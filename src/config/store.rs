use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

use super::presets;
use super::secret::{generate_local_key, is_encrypted, MasterKey, Paths};
use super::types::*;

pub struct LoadOutcome {
    pub config: AppConfig,
    pub warnings: Vec<String>,
}

pub struct ConfigStore {
    pub paths: Paths,
    master: Arc<MasterKey>,
}

impl ConfigStore {
    pub fn open() -> Result<Self> {
        let paths = Paths::discover()?;
        let master = MasterKey::load_or_create(&paths.master_key_file)?;
        Ok(Self {
            paths,
            master: Arc::new(master),
        })
    }

    #[cfg(test)]
    pub fn for_test(paths: Paths) -> Result<Self> {
        fs::create_dir_all(&paths.config_dir)?;
        let master = MasterKey::load_or_create(&paths.master_key_file)?;
        Ok(Self {
            paths,
            master: Arc::new(master),
        })
    }

    pub fn master(&self) -> &Arc<MasterKey> {
        &self.master
    }

    pub fn encrypt(&self, plaintext: &str) -> Result<String> {
        self.master.encrypt_if_needed(plaintext)
    }

    pub fn decrypt(&self, envelope: &str) -> Result<String> {
        self.master.decrypt(envelope)
    }

    pub fn masked_secret(&self, envelope: &str) -> String {
        match self.master.decrypt(envelope) {
            Ok(plain) if !plain.is_empty() => mask_secret(&plain),
            _ => "未设置".to_string(),
        }
    }

    pub fn load(&self) -> Result<LoadOutcome> {
        let mut warnings = Vec::new();
        let existed = self.paths.config_file.exists();

        let mut config = if !existed {
            let mut cfg = AppConfig::default();
            let key = generate_local_key();
            cfg.local_keys.push(LocalKey {
                id: new_id(),
                name: "默认 Key".into(),
                key: self.encrypt(&key)?,
                enabled: true,
                allowed_models: Vec::new(),
                max_tokens: None,
                max_requests: None,
                created_at: now_ts(),
                last_used_at: None,
            });
            warnings.push(format!(
                "首次启动：已生成默认本地 Key，可在「本地 Key」页复制。配置目录 {}",
                self.paths.config_dir.display()
            ));
            cfg
        } else {
            let text = fs::read_to_string(&self.paths.config_file)
                .with_context(|| format!("读取配置失败：{}", self.paths.config_file.display()))?;
            let trimmed = text.trim_start_matches('\u{feff}');
            let value: Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(err) => {
                    let backup = self.paths.config_file.with_file_name(format!(
                        "config.corrupt-{}.json",
                        chrono::Utc::now().format("%Y%m%d-%H%M%S")
                    ));
                    let _ = fs::copy(&self.paths.config_file, &backup);
                    return Err(anyhow!(
                        "配置文件 JSON 无效：{err}。原文件已备份到 {}",
                        backup.display()
                    ));
                }
            };
            let version = value
                .get("schema_version")
                .and_then(Value::as_u64)
                .unwrap_or(0) as u32;
            if version > CONFIG_SCHEMA_VERSION {
                warnings.push(format!(
                    "配置版本 {version} 高于当前程序支持的 {CONFIG_SCHEMA_VERSION}，未知字段会被忽略。"
                ));
            }
            let migrated = migrate_value(value);
            match serde_json::from_value::<AppConfig>(migrated.clone()) {
                Ok(cfg) => cfg,
                Err(err) => {
                    return Err(anyhow!("配置结构无效：{err}"));
                }
            }
        };

        // Normalize identities and timestamps, and encrypt any legacy plaintext.
        let mut needs_save = !existed;
        let now = now_ts();
        let mut provider_ids = HashSet::new();
        for p in &mut config.providers {
            if p.id.trim().is_empty() {
                p.id = new_id();
                needs_save = true;
            }
            if !provider_ids.insert(p.id.clone()) {
                warnings.push(format!("供应商 ID 重复：{}", p.id));
            }
            if p.created_at == 0 {
                p.created_at = now;
                needs_save = true;
            }
        }
        for a in &mut config.accounts {
            if a.id.trim().is_empty() {
                a.id = new_id();
                needs_save = true;
            }
            if a.created_at == 0 {
                a.created_at = now;
                needs_save = true;
            }
            if !a.api_key.is_empty() && !is_encrypted(&a.api_key) {
                match self.encrypt(&a.api_key) {
                    Ok(enc) => {
                        a.api_key = enc;
                        needs_save = true;
                    }
                    Err(err) => warnings.push(format!("账号 {} 的 Key 加密失败：{err}", a.name)),
                }
            }
        }
        for r in &mut config.routes {
            if r.id.trim().is_empty() {
                r.id = new_id();
                needs_save = true;
            }
            if r.created_at == 0 {
                r.created_at = now;
                needs_save = true;
            }
        }
        for k in &mut config.local_keys {
            if k.id.trim().is_empty() {
                k.id = new_id();
                needs_save = true;
            }
            if k.created_at == 0 {
                k.created_at = now;
                needs_save = true;
            }
            if !k.key.is_empty() && !is_encrypted(&k.key) {
                match self.encrypt(&k.key) {
                    Ok(enc) => {
                        k.key = enc;
                        needs_save = true;
                    }
                    Err(err) => warnings.push(format!("本地 Key {} 加密失败：{err}", k.name)),
                }
            }
        }

        config.schema_version = CONFIG_SCHEMA_VERSION;

        if config.server.port == 0 {
            config.server.port = 7789;
            needs_save = true;
        }

        if needs_save {
            if let Err(err) = self.save(&mut config) {
                warnings.push(format!("写入规范化配置失败：{err}"));
            }
        }

        warnings.extend(validate(&config));
        Ok(LoadOutcome { config, warnings })
    }

    pub fn save(&self, config: &mut AppConfig) -> Result<()> {
        config.schema_version = CONFIG_SCHEMA_VERSION;
        config.ui.taskbar = config.ui.taskbar.normalized();
        let errors = validate(config);
        if !errors.is_empty() {
            return Err(anyhow!(errors.join("；")));
        }
        for a in &mut config.accounts {
            if !a.api_key.is_empty() && !is_encrypted(&a.api_key) {
                a.api_key = self.encrypt(&a.api_key)?;
            }
        }
        for k in &mut config.local_keys {
            if !k.key.is_empty() && !is_encrypted(&k.key) {
                k.key = self.encrypt(&k.key)?;
            }
        }
        for p in &mut config.providers {
            for h in &mut p.extra_headers {
                // Headers are not encrypted in v1, but keep whitespace tidy.
                h.name = h.name.trim().to_string();
            }
        }

        let json = serde_json::to_vec_pretty(config)?;
        let tmp = self.paths.config_file.with_extension("json.tmp");
        fs::write(&tmp, &json).with_context(|| format!("写入临时配置失败：{}", tmp.display()))?;
        if let Err(err) = fs::rename(&tmp, &self.paths.config_file) {
            // Windows may refuse to replace an existing destination.
            let _ = fs::remove_file(&self.paths.config_file);
            fs::rename(&tmp, &self.paths.config_file)
                .map_err(|_| anyhow!("替换配置文件失败：{err}（临时文件 {}）", tmp.display()))?;
        }
        Ok(())
    }

    /// Export a shareable copy with every credential removed.
    pub fn export_without_secrets(&self, config: &AppConfig) -> Result<PathBuf> {
        let mut copy = config.clone();
        for a in &mut copy.accounts {
            a.api_key.clear();
        }
        for k in &mut copy.local_keys {
            k.key.clear();
        }
        for p in &mut copy.providers {
            for h in &mut p.extra_headers {
                if is_sensitive_header(&h.name) {
                    h.value.clear();
                }
            }
        }
        fs::create_dir_all(&self.paths.exports_dir)?;
        let file = self.paths.exports_dir.join(format!(
            "yoyo-relay-export-{}.json",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        ));
        let json = serde_json::to_vec_pretty(&copy)?;
        fs::write(&file, json)?;
        Ok(file)
    }

    /// Import a configuration file. Credentials are deliberately cleared;
    /// the user must re-enter them (the export never contains them).
    pub fn import_without_secrets(&self, path: &Path) -> Result<AppConfig> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("读取导入文件失败：{}", path.display()))?;
        let value: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))?;
        let value = migrate_value(value);
        let mut config: AppConfig = serde_json::from_value(value)?;
        for a in &mut config.accounts {
            a.api_key.clear();
        }
        for k in &mut config.local_keys {
            k.key.clear();
        }
        config.ui.onboarding_done = true;
        Ok(config)
    }

    /// Export reusable price plans as a standalone JSON document. Unlike the
    /// full configuration export this file contains no credentials at all, so
    /// it is safe to share.
    pub fn export_price_plans(&self, plans: &[PricePlan]) -> Result<PathBuf> {
        fs::create_dir_all(&self.paths.exports_dir)?;
        let file = self.paths.exports_dir.join(format!(
            "yoyo-relay-prices-{}.json",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        ));
        let payload = serde_json::json!({
            "kind": "yoyo-relay-price-plans",
            "version": 1,
            "exported_at": now_ts(),
            "plans": plans,
        });
        fs::write(&file, serde_json::to_vec_pretty(&payload)?)?;
        Ok(file)
    }

    /// Read a price-plan document. Accepts either the wrapper written by
    /// `export_price_plans` or a bare array of plans.
    pub fn import_price_plans(&self, path: &Path) -> Result<Vec<PricePlan>> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("读取价格方案失败：{}", path.display()))?;
        let value: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))?;
        let plans = match value {
            Value::Array(items) => serde_json::from_value(Value::Array(items))?,
            Value::Object(mut obj) => {
                let plans = obj.remove("plans").unwrap_or(Value::Null);
                serde_json::from_value(plans)?
            }
            _ => return Err(anyhow!("价格方案文件格式不受支持")),
        };
        Ok(plans)
    }

    pub fn new_local_key(&self, name: &str) -> Result<LocalKey> {
        let plaintext = generate_local_key();
        Ok(LocalKey {
            id: new_id(),
            name: name.to_string(),
            key: self.encrypt(&plaintext)?,
            enabled: true,
            allowed_models: Vec::new(),
            max_tokens: None,
            max_requests: None,
            created_at: now_ts(),
            last_used_at: None,
        })
    }

    pub fn preset_provider(&self, preset: &str) -> Provider {
        presets::provider_from_preset(preset)
    }
}

fn migrate_value(mut value: Value) -> Value {
    let obj = match value.as_object_mut() {
        Some(obj) => obj,
        None => return value,
    };
    let version = obj
        .get("schema_version")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if version < 1 {
        // v0 -> v1: every field has a serde default; only the marker changes.
        obj.insert("schema_version".into(), Value::from(1));
    }
    if version < 2 {
        // v1 -> v2: model profiles are derived lazily from routes when absent,
        // so only the marker needs to move. Existing route metadata remains.
        obj.insert("schema_version".into(), Value::from(2));
    }
    if version < 3 {
        // v2 -> v3: live FX metadata, taskbar badge settings and the optional
        // balance formula all have serde defaults, so only the marker moves.
        obj.insert("schema_version".into(), Value::from(3));
    }
    if version < 4 {
        // v3 -> v4: per-route currency, cached-input pricing, peak/off-peak
        // periods and the price-plan library. Routes that only carry the old
        // flat USD pair keep working through `ModelRoute::effective_pricing`.
        obj.insert("schema_version".into(), Value::from(4));
    }
    if version < 5 {
        // v4 -> v5: update-check settings, percent-based balance conversion and
        // custom badge text colour all have serde defaults, so only the marker
        // moves.
        obj.insert("schema_version".into(), Value::from(5));
    }
    if version < 6 {
        // v5 -> v6: plugin host settings. Plugin code lives in
        // `<config_dir>/plugins`, so only default settings are added.
        obj.insert("schema_version".into(), Value::from(6));
    }
    value
}

pub fn validate(config: &AppConfig) -> Vec<String> {
    let mut errors = Vec::new();
    let mut ids = HashSet::new();

    if config.server.port == 0 {
        errors.push("端口不能为 0".into());
    }
    if config.server.max_global_concurrency == 0 {
        errors.push("全局并发上限不能为 0".into());
    }
    if config.server.bind.trim().is_empty() {
        errors.push("监听地址不能为空".into());
    }

    for p in &config.providers {
        if !ids.insert(format!("p:{}", p.id)) {
            errors.push(format!("供应商 ID 重复：{}", p.id));
        }
        if p.name.trim().is_empty() {
            errors.push("供应商名称不能为空".into());
        }
        if !(p.base_url.starts_with("http://") || p.base_url.starts_with("https://")) {
            errors.push(format!(
                "供应商「{}」的 Base URL 必须以 http:// 或 https:// 开头",
                p.name
            ));
        }
        for h in &p.extra_headers {
            if h.name.trim().is_empty() && !h.value.trim().is_empty() {
                errors.push(format!("供应商「{}」存在没有名称的 Header", p.name));
            }
        }
    }

    for a in &config.accounts {
        if !ids.insert(format!("a:{}", a.id)) {
            errors.push(format!("账号 ID 重复：{}", a.id));
        }
        if a.name.trim().is_empty() {
            errors.push("账号名称不能为空".into());
        }
        if !config.providers.iter().any(|p| p.id == a.provider_id) {
            errors.push(format!("账号「{}」引用了不存在的供应商", a.name));
        }
        if let Some(q) = &a.quota {
            if q.limit_tokens.is_none() && q.limit_requests.is_none() {
                errors.push(format!("账号「{}」的额度窗口没有设置任何上限", a.name));
            }
            if q.window == QuotaWindow::Rolling && (q.rolling_hours == 0 || q.rolling_hours > 720) {
                errors.push(format!("账号「{}」的滚动窗口必须在 1–720 小时之间", a.name));
            }
        }
    }

    for r in &config.routes {
        if !ids.insert(format!("r:{}", r.id)) {
            errors.push(format!("路由 ID 重复：{}", r.id));
        }
        if r.client_model.trim().is_empty() {
            errors.push("路由的客户端模型名不能为空".into());
        }
        if r.upstream_model.trim().is_empty() {
            errors.push(format!("路由「{}」的上游模型名不能为空", r.client_model));
        }
        if !config.providers.iter().any(|p| p.id == r.provider_id) {
            errors.push(format!("路由「{}」引用了不存在的供应商", r.client_model));
        }
    }

    let mut profile_names = HashSet::new();
    for p in &config.model_profiles {
        let name = p.client_model.trim();
        if name.is_empty() {
            errors.push("模型档案的客户端模型名不能为空".into());
        } else if !profile_names.insert(name.to_string()) {
            errors.push(format!("模型档案重复：{name}"));
        }
        if let Some(context) = p.context_window {
            if context == 0 {
                errors.push(format!("模型「{name}」的上下文窗口必须大于 0"));
            }
        }
        if let (Some(context), Some(max_output)) = (p.context_window, p.max_output_tokens) {
            if max_output > context {
                errors.push(format!("模型「{name}」的最大输出不能大于上下文窗口"));
            }
        }
    }

    for k in &config.local_keys {
        if !ids.insert(format!("k:{}", k.id)) {
            errors.push(format!("本地 Key ID 重复：{}", k.id));
        }
        if k.name.trim().is_empty() {
            errors.push("本地 Key 名称不能为空".into());
        }
    }

    errors
}

pub fn is_sensitive_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("authorization")
        || lower.contains("api-key")
        || lower.contains("token")
        || lower.contains("secret")
        || lower.contains("key")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn roundtrip_config_with_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::for_test(dir.path().to_path_buf());
        fs::create_dir_all(&paths.config_dir).unwrap();
        let store = ConfigStore::for_test(paths).unwrap();
        let mut config = AppConfig::default();
        config
            .providers
            .push(presets::provider_from_preset("openai"));
        let provider_id = config.providers[0].id.clone();
        config.accounts.push(Account {
            id: new_id(),
            provider_id,
            name: "test".into(),
            api_key: "sk-plaintext-secret".into(),
            ..Account::default()
        });
        config.model_profiles.push(ClientModelProfile {
            client_model: "test-model".into(),
            context_window: Some(128_000),
            max_output_tokens: Some(8_192),
            input_modalities: vec!["text".into(), "image".into()],
            reasoning_levels: vec!["low".into(), "high".into()],
            ..ClientModelProfile::default()
        });
        store.save(&mut config).unwrap();
        assert!(is_encrypted(&config.accounts[0].api_key));
        let out = store.load().unwrap();
        assert_eq!(out.config.accounts.len(), 1);
        assert_eq!(out.config.model_profiles.len(), 1);
        assert_eq!(out.config.model_profiles[0].context_window, Some(128_000));
        let plain = store.decrypt(&out.config.accounts[0].api_key).unwrap();
        assert_eq!(plain, "sk-plaintext-secret");
        let raw = fs::read_to_string(&store.paths.config_file).unwrap();
        assert!(!raw.contains("sk-plaintext-secret"));
    }

    #[test]
    fn price_plans_roundtrip_without_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::for_test(dir.path().to_path_buf());
        fs::create_dir_all(&paths.config_dir).unwrap();
        let store = ConfigStore::for_test(paths).unwrap();
        let plans = vec![PricePlan {
            name: "GLM 峰谷".into(),
            note: "2026 秋季价".into(),
            currency: PriceCurrency::Cny,
            base: PriceTier {
                input_per_mtok: 2.0,
                cached_input_per_mtok: 0.5,
                output_per_mtok: 8.0,
            },
            periods: vec![PricePeriod {
                name: "峰时".into(),
                start_hour: 9,
                end_hour: 18,
                days: vec![0, 1, 2, 3, 4],
                tier: PriceTier {
                    input_per_mtok: 4.0,
                    cached_input_per_mtok: 1.0,
                    output_per_mtok: 16.0,
                },
            }],
        }];
        let path = store.export_price_plans(&plans).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("api_key"));
        assert!(!text.contains("sk-"));
        let loaded = store.import_price_plans(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "GLM 峰谷");
        assert_eq!(loaded[0].currency, PriceCurrency::Cny);
        assert!(loaded[0].periods[0].matches(
            chrono::Local
                .with_ymd_and_hms(2026, 10, 19, 10, 0, 0)
                .single()
                .unwrap()
        ));
    }

    #[test]
    fn imports_bare_plan_array() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::for_test(dir.path().to_path_buf());
        fs::create_dir_all(&paths.config_dir).unwrap();
        let file = paths.config_dir.join("bare.json");
        let store = ConfigStore::for_test(paths).unwrap();
        fs::write(
            &file,
            serde_json::to_vec(&serde_json::json!([{
                "name": "USD 基础",
                "currency": "usd",
                "base": {"input_per_mtok": 1.0, "cached_input_per_mtok": 0.1, "output_per_mtok": 2.0}
            }]))
            .unwrap(),
        )
        .unwrap();
        let loaded = store.import_price_plans(&file).unwrap();
        assert_eq!(loaded[0].name, "USD 基础");
        assert_eq!(loaded[0].base.output_per_mtok, 2.0);
    }

    #[test]
    fn migrates_v3_config_and_keeps_legacy_prices() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::for_test(dir.path().to_path_buf());
        fs::create_dir_all(&paths.config_dir).unwrap();
        let legacy = serde_json::json!({
            "schema_version": 3,
            "providers": [{
                "id": "p1",
                "name": "P",
                "base_url": "https://example.com/v1"
            }],
            "routes": [{
                "id": "r1",
                "client_model": "m",
                "provider_id": "p1",
                "upstream_model": "up",
                "input_price_per_mtok": 1.5,
                "output_price_per_mtok": 6.0
            }],
            "ui": {"fx_rates": {"USD": 1.0, "CNY": 7.2}}
        });
        fs::write(
            &paths.config_file,
            serde_json::to_vec_pretty(&legacy).unwrap(),
        )
        .unwrap();
        let store = ConfigStore::for_test(paths).unwrap();
        let outcome = store.load().unwrap();
        assert_eq!(outcome.config.schema_version, CONFIG_SCHEMA_VERSION);
        let route = outcome.config.route("r1").unwrap();
        let pricing = route.effective_pricing();
        assert_eq!(pricing.currency, PriceCurrency::Usd);
        assert!((pricing.base.input_per_mtok - 1.5).abs() < 1e-9);
        assert!((pricing.base.output_per_mtok - 6.0).abs() < 1e-9);
        assert!(outcome.config.price_plans.is_empty());
        // Old badges without the new switches still load.
        assert!(!outcome.config.ui.taskbar.transparent_bg);
    }

    #[test]
    fn export_strips_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::for_test(dir.path().to_path_buf());
        fs::create_dir_all(&paths.config_dir).unwrap();
        let store = ConfigStore::for_test(paths).unwrap();
        let mut config = AppConfig::default();
        config
            .providers
            .push(presets::provider_from_preset("openai"));
        let provider_id = config.providers[0].id.clone();
        config.accounts.push(Account {
            id: new_id(),
            provider_id,
            name: "test".into(),
            api_key: "sk-secret".into(),
            ..Account::default()
        });
        store.save(&mut config).unwrap();
        let path = store.export_without_secrets(&config).unwrap();
        let text = fs::read_to_string(path).unwrap();
        assert!(!text.contains("sk-secret"));
        assert!(!text.contains("enc:v1:"));
    }
}
