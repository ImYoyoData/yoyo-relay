use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{Local, TimeZone};

use crate::config::{now_ts, Account, AppConfig, ModelRoute, Protocol, Provider, SchedulerConfig};
use crate::db::Db;
use crate::protocol::{route_supports_ingress, Ingress};

// ---------------------------------------------------------------------------
// Account runtime state machine
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountState {
    Healthy,
    Cooling {
        until: i64,
        reason: String,
        fail_streak: u32,
    },
    HalfOpen,
    Exhausted {
        reset_at: i64,
    },
    Disabled,
}

impl AccountState {
    pub fn badge(&self) -> &'static str {
        match self {
            AccountState::Healthy => "健康",
            AccountState::Cooling { .. } => "冷却中",
            AccountState::HalfOpen => "探测中",
            AccountState::Exhausted { .. } => "额度用尽",
            AccountState::Disabled => "已禁用",
        }
    }

    pub fn is_available_for_traffic(&self) -> bool {
        matches!(self, AccountState::Healthy)
    }
}

#[derive(Default)]
struct ProbeState {
    inflight: bool,
    last_started: i64,
}

#[derive(Default)]
struct QuotaCache {
    checked_at: i64,
    window_start: i64,
    used_tokens: u64,
    used_requests: u64,
}

pub struct AccountRuntime {
    pub state: Mutex<AccountState>,
    fail_streak: Mutex<u32>,
    pub inflight: AtomicUsize,
    quota: Mutex<QuotaCache>,
    probe: Mutex<ProbeState>,
}

impl AccountRuntime {
    fn new() -> Self {
        Self {
            state: Mutex::new(AccountState::Healthy),
            fail_streak: Mutex::new(0),
            inflight: AtomicUsize::new(0),
            quota: Mutex::new(QuotaCache::default()),
            probe: Mutex::new(ProbeState::default()),
        }
    }

    pub fn state(&self) -> AccountState {
        self.state
            .lock()
            .map(|g| g.clone())
            .unwrap_or(AccountState::Healthy)
    }

    pub fn fail_streak(&self) -> u32 {
        *self.fail_streak.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub struct AccountSlot {
    rt: Arc<AccountRuntime>,
}

impl Drop for AccountSlot {
    fn drop(&mut self) {
        self.rt.inflight.fetch_sub(1, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------------------
// Candidate chain
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FallbackLayer {
    Account,
    Provider,
    Mapping,
}

#[allow(dead_code)]
impl FallbackLayer {
    pub fn label(self) -> &'static str {
        match self {
            FallbackLayer::Account => "账号级",
            FallbackLayer::Provider => "供应商级",
            FallbackLayer::Mapping => "映射级",
        }
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct Attempt {
    pub account: Account,
    pub route: ModelRoute,
    pub provider: Provider,
    pub upstream_protocol: Protocol,
    pub layer: FallbackLayer,
}

#[derive(Clone, Debug, Default)]
pub struct Chain {
    pub attempts: Vec<Attempt>,
    pub notes: Vec<String>,
    pub fallback_model: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum FailureKind {
    ConcurrencyLimit,
    RateLimited,
    Auth,
    Server,
    Timeout,
    Network,
    /// Upstream accepted the request but did not produce a first byte /
    /// first SSE chunk within the configured deadline.
    Slow,
    NotFound,
}

#[derive(Clone, Debug, Default)]
pub struct QuotaSnapshot {
    pub used_tokens: u64,
    pub used_requests: u64,
    pub limit_tokens: Option<u64>,
    pub limit_requests: Option<u64>,
    pub window_label: String,
    pub reset_at: i64,
}

// ---------------------------------------------------------------------------
// Circuit breaker
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct Breaker {
    consecutive_failures: AtomicU32,
    open_until: AtomicI64,
}

#[derive(Clone, Debug)]
pub struct BreakerSnapshot {
    pub open: bool,
    pub open_until: i64,
    pub consecutive_failures: u32,
    pub threshold: u32,
}

impl Breaker {
    pub fn allow(&self, cfg: &SchedulerConfig, now: i64) -> bool {
        let until = self.open_until.load(Ordering::SeqCst);
        if until > now {
            return false;
        }
        if until != 0 {
            self.open_until.store(0, Ordering::SeqCst);
        }
        let _ = cfg;
        true
    }

    pub fn retry_after(&self, now: i64) -> u64 {
        (self.open_until.load(Ordering::SeqCst) - now).max(0) as u64
    }

    pub fn record_success(&self) {
        self.consecutive_failures.store(0, Ordering::SeqCst);
    }

    /// Returns true when this failure opened the breaker.
    pub fn record_all_failed(&self, cfg: &SchedulerConfig, now: i64) -> bool {
        let n = self.consecutive_failures.fetch_add(1, Ordering::SeqCst) + 1;
        if n >= cfg.breaker_threshold.max(1) {
            self.open_until
                .store(now + cfg.breaker_cooldown_secs as i64, Ordering::SeqCst);
            self.consecutive_failures.store(0, Ordering::SeqCst);
            true
        } else {
            false
        }
    }

    #[allow(dead_code)]
    pub fn force_open(&self, cfg: &SchedulerConfig, now: i64) {
        self.consecutive_failures.store(0, Ordering::SeqCst);
        self.open_until
            .store(now + cfg.breaker_cooldown_secs as i64, Ordering::SeqCst);
    }

    pub fn snapshot(&self, cfg: &SchedulerConfig, now: i64) -> BreakerSnapshot {
        let until = self.open_until.load(Ordering::SeqCst);
        BreakerSnapshot {
            open: until > now,
            open_until: until,
            consecutive_failures: self.consecutive_failures.load(Ordering::SeqCst),
            threshold: cfg.breaker_threshold,
        }
    }
}

// ---------------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------------

pub struct Scheduler {
    accounts: Mutex<HashMap<String, Arc<AccountRuntime>>>,
    cursors: Mutex<HashMap<String, usize>>,
    pub breaker: Breaker,
    db: Arc<Db>,
}

impl Scheduler {
    pub fn new(db: Arc<Db>) -> Self {
        Self {
            accounts: Mutex::new(HashMap::new()),
            cursors: Mutex::new(HashMap::new()),
            breaker: Breaker::default(),
            db,
        }
    }

    pub fn runtime_for(&self, account_id: &str) -> Arc<AccountRuntime> {
        let mut map = self.accounts.lock().unwrap_or_else(|e| e.into_inner());
        map.entry(account_id.to_string())
            .or_insert_with(|| Arc::new(AccountRuntime::new()))
            .clone()
    }

    pub fn remove_missing_accounts(&self, cfg: &AppConfig) {
        let ids: std::collections::HashSet<&str> =
            cfg.accounts.iter().map(|a| a.id.as_str()).collect();
        let mut map = self.accounts.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|id, _| ids.contains(id.as_str()));
    }

    pub fn reset_account(&self, account_id: &str) {
        if let Some(rt) = self
            .accounts
            .lock()
            .ok()
            .and_then(|m| m.get(account_id).cloned())
        {
            *rt.state.lock().unwrap_or_else(|e| e.into_inner()) = AccountState::Healthy;
            *rt.fail_streak.lock().unwrap_or_else(|e| e.into_inner()) = 0;
            let mut q = rt.quota.lock().unwrap_or_else(|e| e.into_inner());
            q.checked_at = 0;
        }
    }

    pub fn on_success(&self, account_id: &str) {
        let rt = self.runtime_for(account_id);
        *rt.state.lock().unwrap_or_else(|e| e.into_inner()) = AccountState::Healthy;
        *rt.fail_streak.lock().unwrap_or_else(|e| e.into_inner()) = 0;
        let mut q = rt.quota.lock().unwrap_or_else(|e| e.into_inner());
        q.checked_at = 0;
    }

    pub fn on_failure(&self, account: &Account, kind: FailureKind, cfg: &SchedulerConfig) {
        self.on_failure_at(account, kind, cfg, Local::now().timestamp());
    }

    pub fn on_failure_at(
        &self,
        account: &Account,
        kind: FailureKind,
        cfg: &SchedulerConfig,
        now: i64,
    ) {
        if matches!(kind, FailureKind::NotFound) {
            return;
        }
        let rt = self.runtime_for(&account.id);
        let streak = {
            let mut s = rt.fail_streak.lock().unwrap_or_else(|e| e.into_inner());
            *s = s.saturating_add(1);
            *s
        };
        let base = account
            .cooldown_override_secs
            .unwrap_or(cfg.cooldown_secs)
            .max(1);
        let (seconds, reason) = match kind {
            FailureKind::Auth => (cfg.auth_cooldown_secs.max(1), "鉴权失败"),
            FailureKind::ConcurrencyLimit => (cfg.concurrency_cooldown_secs.max(1), "上游并发超限"),
            FailureKind::RateLimited => {
                let factor = 1u64 << (streak.saturating_sub(1)).min(10);
                (
                    base.saturating_mul(factor)
                        .min(cfg.max_cooldown_secs.max(1)),
                    "429 限流",
                )
            }
            FailureKind::Server => {
                let factor = 1u64 << (streak.saturating_sub(1)).min(10);
                (
                    base.saturating_mul(factor)
                        .min(cfg.max_cooldown_secs.max(1)),
                    "上游 5xx",
                )
            }
            FailureKind::Timeout => {
                let factor = 1u64 << (streak.saturating_sub(1)).min(10);
                (
                    base.saturating_mul(factor)
                        .min(cfg.max_cooldown_secs.max(1)),
                    "连接超时",
                )
            }
            FailureKind::Network => {
                let factor = 1u64 << (streak.saturating_sub(1)).min(10);
                (
                    base.saturating_mul(factor)
                        .min(cfg.max_cooldown_secs.max(1)),
                    "连接失败",
                )
            }
            FailureKind::Slow => (cfg.slow_cooldown_secs.max(1), "首包/首 token 超时"),
            FailureKind::NotFound => (0, ""),
        };
        *rt.state.lock().unwrap_or_else(|e| e.into_inner()) = AccountState::Cooling {
            until: now + seconds as i64,
            reason: reason.to_string(),
            fail_streak: streak,
        };
    }

    pub fn mark_exhausted(&self, account_id: &str, reset_at: i64) {
        let rt = self.runtime_for(account_id);
        *rt.state.lock().unwrap_or_else(|e| e.into_inner()) = AccountState::Exhausted { reset_at };
    }

    pub fn try_acquire_slot(
        &self,
        account_id: &str,
        max_concurrency: usize,
    ) -> Option<AccountSlot> {
        let rt = self.runtime_for(account_id);
        let mut current = rt.inflight.load(Ordering::SeqCst);
        loop {
            if max_concurrency > 0 && current >= max_concurrency {
                return None;
            }
            match rt.inflight.compare_exchange_weak(
                current,
                current + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return Some(AccountSlot { rt }),
                Err(actual) => current = actual,
            }
        }
    }

    pub fn note_account_usage(&self, account_id: &str, tokens: u64, requests: u64) {
        let rt = self.runtime_for(account_id);
        let mut q = rt.quota.lock().unwrap_or_else(|e| e.into_inner());
        if q.checked_at > 0 {
            q.used_tokens = q.used_tokens.saturating_add(tokens);
            q.used_requests = q.used_requests.saturating_add(requests);
        }
    }

    // -- state inspection ---------------------------------------------------

    fn normalize_state(&self, rt: &AccountRuntime, account: &Account, now: i64) {
        let mut state = rt.state.lock().unwrap_or_else(|e| e.into_inner());
        match &*state {
            AccountState::Cooling { until, .. } if now >= *until => {
                *state = AccountState::HalfOpen;
            }
            AccountState::Exhausted { reset_at } if now >= *reset_at => {
                *state = AccountState::Healthy;
            }
            AccountState::Disabled if account.enabled => {
                *state = AccountState::Healthy;
            }
            _ => {}
        }
        if !account.enabled {
            *state = AccountState::Disabled;
        }
    }

    pub fn state_of(&self, account: &Account, now: i64) -> AccountState {
        let rt = self.runtime_for(&account.id);
        self.normalize_state(&rt, account, now);
        rt.state()
    }

    pub fn inflight_of(&self, account_id: &str) -> usize {
        self.accounts
            .lock()
            .ok()
            .and_then(|m| m.get(account_id).cloned())
            .map(|rt| rt.inflight.load(Ordering::SeqCst))
            .unwrap_or(0)
    }

    fn quota_values(
        &self,
        rt: &AccountRuntime,
        account: &Account,
        cfg: &SchedulerConfig,
        now: i64,
    ) -> Option<(u64, u64, i64)> {
        let quota = account.quota.as_ref()?;
        let local_now = Local
            .timestamp_opt(now, 0)
            .single()
            .unwrap_or_else(Local::now);
        let window_start = quota.window_start(local_now);
        let reset_at = quota.reset_at(local_now);
        let mut cache = rt.quota.lock().unwrap_or_else(|e| e.into_inner());
        let ttl = cfg.quota_cache_secs.max(1) as i64;
        let stale = cache.checked_at == 0
            || cache.window_start != window_start
            || now - cache.checked_at >= ttl;
        if stale {
            match self.db.sum_account_since(&account.id, window_start) {
                Ok((tokens, requests)) => {
                    cache.used_tokens = tokens;
                    cache.used_requests = requests;
                    cache.window_start = window_start;
                    cache.checked_at = now;
                }
                Err(err) => {
                    tracing::warn!("quota query failed for {}: {err}", account.name);
                    return None;
                }
            }
        }
        Some((cache.used_tokens, cache.used_requests, reset_at))
    }

    fn quota_exceeded(
        &self,
        rt: &AccountRuntime,
        account: &Account,
        cfg: &SchedulerConfig,
        now: i64,
    ) -> bool {
        let Some(quota) = &account.quota else {
            return false;
        };
        let Some((used_tokens, used_requests, reset_at)) = self.quota_values(rt, account, cfg, now)
        else {
            return false;
        };
        let tokens_exceeded = quota
            .limit_tokens
            .map(|limit| used_tokens >= limit)
            .unwrap_or(false);
        let requests_exceeded = quota
            .limit_requests
            .map(|limit| used_requests >= limit)
            .unwrap_or(false);
        if tokens_exceeded || requests_exceeded {
            self.mark_exhausted(&account.id, reset_at);
            true
        } else {
            false
        }
    }

    pub fn quota_snapshot(
        &self,
        account: &Account,
        cfg: &SchedulerConfig,
        now: i64,
    ) -> Option<QuotaSnapshot> {
        let quota = account.quota.as_ref()?;
        let rt = self.runtime_for(&account.id);
        let (used_tokens, used_requests, reset_at) = self.quota_values(&rt, account, cfg, now)?;
        Some(QuotaSnapshot {
            used_tokens,
            used_requests,
            limit_tokens: quota.limit_tokens,
            limit_requests: quota.limit_requests,
            window_label: match quota.window {
                crate::config::QuotaWindow::Rolling => {
                    format!("滚动 {}h", quota.rolling_hours)
                }
                other => other.label().to_string(),
            },
            reset_at,
        })
    }

    fn account_eligible(
        &self,
        rt: &AccountRuntime,
        account: &Account,
        cfg: &SchedulerConfig,
        now: i64,
    ) -> bool {
        if !account.enabled {
            return false;
        }
        self.normalize_state(rt, account, now);
        let state = rt.state();
        if !state.is_available_for_traffic() {
            return false;
        }
        if self.quota_exceeded(rt, account, cfg, now) {
            return false;
        }
        true
    }

    // -- chain building -----------------------------------------------------

    pub fn build_chain(
        &self,
        cfg: &AppConfig,
        client_model: &str,
        ingress: Ingress,
        advance_cursor: bool,
    ) -> Chain {
        let now = now_ts();
        let mut chain = Chain::default();
        if !cfg.model_enabled(client_model) {
            chain
                .notes
                .push(format!("客户端模型「{client_model}」已被禁用"));
            return chain;
        }
        let routes = cfg.routes_for(client_model);
        if routes.is_empty() {
            chain
                .notes
                .push(format!("客户端模型「{client_model}」没有启用的路由"));
            return chain;
        }

        let primary_attempts = self.build_layer_attempts(
            cfg,
            &routes,
            ingress,
            FallbackLayer::Account,
            &mut chain.notes,
            now,
            advance_cursor,
        );
        let normal_cap = (cfg.scheduler.max_retries as usize + 1).max(1);
        // Account-level retries are capped, but every distinct upstream model
        // must still get one chance. Otherwise a provider with many accounts
        // could consume the whole budget and a failing model would never fall
        // through to the next upstream.
        let mut selected: Vec<Attempt> =
            primary_attempts.iter().take(normal_cap).cloned().collect();
        for attempt in &primary_attempts {
            if selected
                .iter()
                .any(|chosen| chosen.route.id == attempt.route.id)
            {
                continue;
            }
            if selected.len() >= normal_cap.max(32) {
                break;
            }
            selected.push(attempt.clone());
        }
        chain.attempts.extend(selected);

        // Third layer: another client-model mapping chain, only when explicitly configured.
        let fallback_model = cfg.fallback_for(client_model);
        if let Some(fallback_model) = fallback_model {
            chain.fallback_model = Some(fallback_model.clone());
            if cfg.scheduler.fallback_retries > 0 {
                let fallback_routes = cfg.routes_for(&fallback_model);
                if fallback_routes.is_empty() {
                    chain
                        .notes
                        .push(format!("映射级兜底模型「{fallback_model}」没有启用路由"));
                } else {
                    let extra = self.build_layer_attempts(
                        cfg,
                        &fallback_routes,
                        ingress,
                        FallbackLayer::Mapping,
                        &mut chain.notes,
                        now,
                        false,
                    );
                    chain.attempts.extend(
                        extra
                            .into_iter()
                            .take(cfg.scheduler.fallback_retries as usize),
                    );
                }
            }
        }

        // Remove exact duplicates while preserving order.
        let mut seen = std::collections::HashSet::new();
        chain
            .attempts
            .retain(|a| seen.insert((a.route.id.clone(), a.account.id.clone())));
        chain
    }

    fn build_layer_attempts(
        &self,
        cfg: &AppConfig,
        routes: &[ModelRoute],
        ingress: Ingress,
        first_layer: FallbackLayer,
        notes: &mut Vec<String>,
        now: i64,
        advance_cursor: bool,
    ) -> Vec<Attempt> {
        let mut groups: Vec<(String, Vec<ModelRoute>)> = Vec::new();
        for route in routes {
            let Some(provider) = cfg.provider(&route.provider_id) else {
                notes.push(format!(
                    "路由「{}」引用了不存在的供应商",
                    route.client_model
                ));
                continue;
            };
            let upstream = provider.protocol_for_ingress(
                &route.upstream_model,
                route.protocol,
                ingress.protocol(),
            );
            if !route_supports_ingress(route, ingress, upstream) {
                notes.push(format!(
                    "路由 {} → {} 的协议组合（{} → {}）当前不支持",
                    route.client_model,
                    route.upstream_model,
                    ingress.protocol().map(|p| p.label()).unwrap_or("generic"),
                    upstream.label()
                ));
                continue;
            }
            match groups.iter_mut().find(|(id, _)| id == &provider.id) {
                Some((_, list)) => list.push(route.clone()),
                None => groups.push((provider.id.clone(), vec![route.clone()])),
            }
        }

        let mut attempts = Vec::new();
        for (group_index, (provider_id, group_routes)) in groups.iter().enumerate() {
            let Some(provider) = cfg.provider(provider_id) else {
                continue;
            };
            let accounts = self.eligible_accounts(cfg, provider, now, advance_cursor);
            if accounts.is_empty() {
                notes.push(format!(
                    "供应商「{}」暂无可用账号（冷却/并发/额度/禁用）",
                    provider.name
                ));
                continue;
            }
            let layer = if first_layer == FallbackLayer::Mapping {
                FallbackLayer::Mapping
            } else if group_index == 0 {
                first_layer
            } else {
                FallbackLayer::Provider
            };
            for route in group_routes {
                let upstream = provider.protocol_for_ingress(
                    &route.upstream_model,
                    route.protocol,
                    ingress.protocol(),
                );
                for account in &accounts {
                    attempts.push(Attempt {
                        account: account.clone(),
                        route: route.clone(),
                        provider: provider.clone(),
                        upstream_protocol: upstream,
                        layer,
                    });
                }
            }
        }
        attempts
    }

    fn eligible_accounts(
        &self,
        cfg: &AppConfig,
        provider: &Provider,
        now: i64,
        advance_cursor: bool,
    ) -> Vec<Account> {
        let mut accounts: Vec<Account> = cfg
            .accounts_of(&provider.id)
            .into_iter()
            .filter(|a| {
                let rt = self.runtime_for(&a.id);
                self.account_eligible(&rt, a, &cfg.scheduler, now) && self.try_acquire_slot_probe(a)
            })
            .collect();
        if accounts.is_empty() {
            return accounts;
        }
        accounts.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| a.created_at.cmp(&b.created_at))
                .then_with(|| a.name.cmp(&b.name))
        });
        let top_priority = accounts[0].priority;
        let split = accounts
            .iter()
            .position(|a| a.priority != top_priority)
            .unwrap_or(accounts.len());
        let mut top: Vec<Account> = accounts.drain(..split).collect();
        let rest = accounts;

        let cursor = {
            let mut map = self.cursors.lock().unwrap_or_else(|e| e.into_inner());
            let entry = map.entry(provider.id.clone()).or_insert(0);
            let value = *entry;
            if advance_cursor {
                *entry = entry.wrapping_add(1);
            }
            value
        };
        if !top.is_empty() {
            let len = top.len();
            top.rotate_left(cursor % len);
        }
        top.extend(rest);
        top
    }

    /// Pure check used while filtering; does not reserve a slot.
    fn try_acquire_slot_probe(&self, account: &Account) -> bool {
        if account.max_concurrency == 0 {
            return true;
        }
        let inflight = self.inflight_of(&account.id);
        inflight < account.max_concurrency
    }

    // -- half-open probes ---------------------------------------------------

    /// Move expired cooling accounts to HalfOpen and return those due for a probe.
    pub fn due_probes(&self, cfg: &AppConfig, now: i64) -> Vec<Account> {
        let mut out = Vec::new();
        for account in &cfg.accounts {
            if !account.enabled {
                continue;
            }
            let rt = self.runtime_for(&account.id);
            self.normalize_state(&rt, account, now);
            let state = rt.state();
            let needs = matches!(state, AccountState::HalfOpen);
            if !needs {
                continue;
            }
            let mut probe = rt.probe.lock().unwrap_or_else(|e| e.into_inner());
            if probe.inflight {
                continue;
            }
            if now - probe.last_started < cfg.scheduler.half_open_probe_secs as i64 {
                continue;
            }
            probe.inflight = true;
            probe.last_started = now;
            out.push(account.clone());
        }
        out
    }

    pub fn probe_finished(&self, account: &Account, ok: bool, cfg: &SchedulerConfig, now: i64) {
        let rt = self.runtime_for(&account.id);
        if let Ok(mut probe) = rt.probe.lock() {
            probe.inflight = false;
        }
        if ok {
            self.on_success(&account.id);
        } else {
            self.on_failure_at(account, FailureKind::Server, cfg, now);
        }
    }

    pub fn snapshot_states(&self, cfg: &AppConfig, now: i64) -> HashMap<String, AccountState> {
        let mut map = HashMap::new();
        for account in &cfg.accounts {
            let rt = self.runtime_for(&account.id);
            self.normalize_state(&rt, account, now);
            map.insert(account.id.clone(), rt.state());
        }
        map
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{new_id, Account, AppConfig, ModelRoute, ProtocolStrategyMode, Provider};

    fn setup() -> (Scheduler, AppConfig, String, String) {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(Db::open(&dir.path().join("t.db")).unwrap());
        let scheduler = Scheduler::new(db);
        let mut cfg = AppConfig::default();
        let mut provider = Provider {
            id: new_id(),
            name: "P1".into(),
            base_url: "https://example.com/v1".into(),
            ..Provider::default()
        };
        provider.static_protocol = Protocol::OpenAiChat;
        cfg.providers.push(provider.clone());
        let account = Account {
            id: new_id(),
            provider_id: provider.id.clone(),
            name: "A1".into(),
            enabled: true,
            max_concurrency: 2,
            ..Account::default()
        };
        cfg.accounts.push(account.clone());
        cfg.routes.push(ModelRoute {
            id: new_id(),
            client_model: "gpt-4o".into(),
            provider_id: provider.id.clone(),
            upstream_model: "gpt-4o".into(),
            ..ModelRoute::default()
        });
        (scheduler, cfg, account.id, provider.id)
    }

    #[test]
    fn cooling_backoff_doubles_and_caps() {
        let (s, cfg, account_id, _) = setup();
        let account = cfg.account(&account_id).unwrap().clone();
        let now = 1_000_000;
        s.on_failure_at(&account, FailureKind::Server, &cfg.scheduler, now);
        match s.state_of(&account, now + 1) {
            AccountState::Cooling { until, .. } => {
                assert_eq!(until, now + cfg.scheduler.cooldown_secs as i64);
            }
            other => panic!("unexpected {other:?}"),
        }
        s.on_failure_at(&account, FailureKind::Server, &cfg.scheduler, now + 2);
        match s.state_of(&account, now + 3) {
            AccountState::Cooling {
                until, fail_streak, ..
            } => {
                assert_eq!(fail_streak, 2);
                assert_eq!(until, now + 2 + 2 * cfg.scheduler.cooldown_secs as i64);
            }
            other => panic!("unexpected {other:?}"),
        }
        s.on_success(&account_id);
        assert_eq!(s.state_of(&account, now + 4), AccountState::Healthy);
    }

    #[test]
    fn chain_respects_priority_and_rotates() {
        let (s, mut cfg, account_id, provider_id) = setup();
        let second = Account {
            id: new_id(),
            provider_id: provider_id.clone(),
            name: "A2".into(),
            enabled: true,
            priority: 5,
            max_concurrency: 2,
            ..Account::default()
        };
        cfg.accounts.push(second.clone());
        let c1 = s.build_chain(&cfg, "gpt-4o", Ingress::Chat, true);
        let c2 = s.build_chain(&cfg, "gpt-4o", Ingress::Chat, true);
        assert_eq!(c1.attempts[0].account.id, second.id);
        assert_eq!(c2.attempts[0].account.id, second.id);
        assert!(c1.attempts.iter().any(|a| a.account.id == account_id));
    }

    #[test]
    fn second_provider_used_when_first_has_no_account() {
        let (s, mut cfg, account_id, _) = setup();
        // Disable the only account of P1, add P2 with an account.
        if let Some(a) = cfg.accounts.iter_mut().find(|a| a.id == account_id) {
            a.enabled = false;
        }
        let p2 = Provider {
            id: new_id(),
            name: "P2".into(),
            base_url: "https://example.org/v1".into(),
            ..Provider::default()
        };
        cfg.providers.push(p2.clone());
        let a2 = Account {
            id: new_id(),
            provider_id: p2.id.clone(),
            name: "B1".into(),
            enabled: true,
            ..Account::default()
        };
        cfg.accounts.push(a2.clone());
        cfg.routes.push(ModelRoute {
            id: new_id(),
            client_model: "gpt-4o".into(),
            provider_id: p2.id.clone(),
            upstream_model: "gpt-4o-mini".into(),
            priority: -1,
            ..ModelRoute::default()
        });
        let chain = s.build_chain(&cfg, "gpt-4o", Ingress::Chat, true);
        assert!(!chain.attempts.is_empty());
        assert_eq!(chain.attempts[0].account.id, a2.id);
        assert_eq!(chain.attempts[0].layer, FallbackLayer::Provider);
    }

    #[test]
    fn chain_order_prefers_same_provider_accounts_then_next_provider() {
        let (s, mut cfg, _account_id, provider_id) = setup();
        let second = Account {
            id: new_id(),
            provider_id: provider_id.clone(),
            name: "A2".into(),
            enabled: true,
            max_concurrency: 2,
            ..Account::default()
        };
        cfg.accounts.push(second.clone());
        let p2 = Provider {
            id: new_id(),
            name: "P2".into(),
            base_url: "https://example.org/v1".into(),
            ..Provider::default()
        };
        cfg.providers.push(p2.clone());
        let a3 = Account {
            id: new_id(),
            provider_id: p2.id.clone(),
            name: "B1".into(),
            enabled: true,
            ..Account::default()
        };
        cfg.accounts.push(a3.clone());
        cfg.routes.push(ModelRoute {
            id: new_id(),
            client_model: "gpt-4o".into(),
            provider_id: p2.id.clone(),
            upstream_model: "gpt-4o-mini".into(),
            priority: -1,
            ..ModelRoute::default()
        });

        let chain = s.build_chain(&cfg, "gpt-4o", Ingress::Chat, true);
        let ordered: Vec<&str> = chain
            .attempts
            .iter()
            .map(|a| a.account.id.as_str())
            .collect();
        let last_p1 = ordered
            .iter()
            .rposition(|id| *id == second.id.as_str() || *id == _account_id.as_str())
            .unwrap();
        let first_p2 = ordered.iter().position(|id| *id == a3.id.as_str()).unwrap();
        assert!(
            last_p1 < first_p2,
            "same-provider accounts must be tried before other providers: {ordered:?}"
        );
    }

    #[test]
    fn every_route_gets_one_attempt_even_with_small_retry_cap() {
        let (s, mut cfg, _account_id, provider_id) = setup();
        // Four accounts on the same provider: with the old behaviour these
        // could consume the whole retry budget and the second upstream model
        // would never be tried.
        for i in 0..3 {
            cfg.accounts.push(Account {
                id: new_id(),
                provider_id: provider_id.clone(),
                name: format!("A{}", i + 2),
                enabled: true,
                max_concurrency: 2,
                ..Account::default()
            });
        }
        let second_route = ModelRoute {
            id: new_id(),
            client_model: "gpt-4o".into(),
            provider_id: provider_id.clone(),
            upstream_model: "gpt-4o-backup".into(),
            priority: -1,
            ..ModelRoute::default()
        };
        cfg.routes.push(second_route.clone());
        cfg.scheduler.max_retries = 1;

        let chain = s.build_chain(&cfg, "gpt-4o", Ingress::Chat, true);
        assert!(
            chain
                .attempts
                .iter()
                .any(|attempt| attempt.route.id == second_route.id),
            "second upstream model must get an attempt: {:?}",
            chain
                .attempts
                .iter()
                .map(|attempt| &attempt.route.upstream_model)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ingress_protocol_mode_follows_caller() {
        let (s, mut cfg, _account_id, provider_id) = setup();
        if let Some(provider) = cfg.providers.iter_mut().find(|p| p.id == provider_id) {
            provider.protocol_strategy.mode = ProtocolStrategyMode::Ingress;
            provider.static_protocol = Protocol::OpenAiChat;
        }
        let chain = s.build_chain(&cfg, "gpt-4o", Ingress::Anthropic, true);
        assert_eq!(
            chain.attempts[0].upstream_protocol,
            Protocol::AnthropicMessages
        );
        let chain = s.build_chain(&cfg, "gpt-4o", Ingress::Responses, true);
        assert_eq!(
            chain.attempts[0].upstream_protocol,
            Protocol::OpenAiResponses
        );
    }

    #[test]
    fn slow_failure_uses_slow_cooldown() {
        let (s, cfg, account_id, _) = setup();
        let account = cfg.account(&account_id).unwrap().clone();
        let now = 1_000_000;
        s.on_failure_at(&account, FailureKind::Slow, &cfg.scheduler, now);
        match s.state_of(&account, now + 1) {
            AccountState::Cooling { until, .. } => {
                assert_eq!(until, now + cfg.scheduler.slow_cooldown_secs as i64);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn breaker_opens_after_threshold() {
        let b = Breaker::default();
        let mut cfg = SchedulerConfig::default();
        cfg.breaker_threshold = 3;
        let now = 100;
        assert!(!b.record_all_failed(&cfg, now));
        assert!(!b.record_all_failed(&cfg, now));
        assert!(b.record_all_failed(&cfg, now));
        assert!(!b.allow(&cfg, now));
        assert!(b.allow(&cfg, now + cfg.breaker_cooldown_secs as i64 + 1));
    }

    #[test]
    fn quota_marks_exhausted() {
        let (s, mut cfg, account_id, _) = setup();
        cfg.accounts[0].quota = Some(crate::config::QuotaConfig {
            window: crate::config::QuotaWindow::Rolling,
            rolling_hours: 5,
            limit_tokens: Some(10),
            limit_requests: None,
        });
        let account = cfg.account(&account_id).unwrap().clone();
        let now = Local::now().timestamp();
        // Hydrate the cache, then simulate usage that exceeds the configured limit.
        let _ = s.quota_values(&s.runtime_for(&account_id), &account, &cfg.scheduler, now);
        let rt = s.runtime_for(&account_id);
        {
            let mut q = rt.quota.lock().unwrap();
            q.checked_at = now;
            q.used_tokens = 100;
        }
        assert!(!s.account_eligible(&rt, &account, &cfg.scheduler, now));
        assert!(matches!(
            s.state_of(&account, now),
            AccountState::Exhausted { .. }
        ));
    }
}
