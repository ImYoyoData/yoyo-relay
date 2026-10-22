use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

#[derive(Clone, Debug, Default)]
pub struct UsageRecord {
    pub ts: i64,
    pub request_id: String,
    pub account_id: String,
    pub provider_id: String,
    pub local_key_id: Option<String>,
    pub client_model: String,
    pub upstream_model: String,
    pub protocol: String,
    pub status: u16,
    pub ok: bool,
    pub stream: bool,
    pub cancelled: bool,
    pub prompt_tokens: u64,
    pub cached_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub cost_micros: u64,
    pub latency_ms: u64,
    pub kind: String,
}

#[derive(Clone, Debug, Default)]
pub struct Totals {
    pub requests: u64,
    pub ok: u64,
    pub prompt_tokens: u64,
    pub cached_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub cost_micros: u64,
    pub avg_latency_ms: u64,
}

impl Totals {
    pub fn success_rate(&self) -> f32 {
        if self.requests == 0 {
            0.0
        } else {
            self.ok as f32 / self.requests as f32
        }
    }
}

#[derive(Clone, Debug)]
pub struct GroupRow {
    pub key: String,
    pub totals: Totals,
}

#[derive(Clone, Debug)]
pub struct ModelPairRow {
    pub client_model: String,
    pub upstream_model: String,
    pub totals: Totals,
}

#[derive(Clone, Debug)]
pub struct SeriesPoint {
    pub bucket: i64,
    pub totals: Totals,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct RecentRequest {
    pub id: i64,
    pub ts: i64,
    pub request_id: String,
    pub account_id: String,
    pub provider_id: String,
    pub client_model: String,
    pub upstream_model: String,
    pub protocol: String,
    pub status: u16,
    pub ok: bool,
    pub stream: bool,
    pub cancelled: bool,
    pub prompt_tokens: u64,
    pub cached_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub cost_micros: u64,
    pub latency_ms: u64,
    pub local_key_id: Option<String>,
}

/// One full request record for the call-detail table.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct UsageDetail {
    pub id: i64,
    pub ts: i64,
    pub request_id: String,
    pub account_id: String,
    pub provider_id: String,
    pub local_key_id: Option<String>,
    pub client_model: String,
    pub upstream_model: String,
    pub protocol: String,
    pub status: u16,
    pub ok: bool,
    pub stream: bool,
    pub cancelled: bool,
    pub prompt_tokens: u64,
    pub cached_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub cost_micros: u64,
    pub latency_ms: u64,
    pub kind: String,
}

/// Optional filters for the call-detail page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UsageFilter {
    pub client_model: Option<String>,
    pub account_id: Option<String>,
    pub provider_id: Option<String>,
    pub local_key_id: Option<String>,
    /// `None` = all, `Some(true)` = successful, `Some(false)` = failed.
    pub ok: Option<bool>,
}

impl UsageFilter {
    fn where_clause(&self) -> (String, Vec<Option<String>>, Option<i64>) {
        let mut clauses = Vec::new();
        let mut values: Vec<Option<String>> = Vec::new();
        if let Some(model) = &self.client_model {
            clauses.push("client_model = ?");
            values.push(Some(model.clone()));
        }
        if let Some(account) = &self.account_id {
            clauses.push("account_id = ?");
            values.push(Some(account.clone()));
        }
        if let Some(provider) = &self.provider_id {
            clauses.push("provider_id = ?");
            values.push(Some(provider.clone()));
        }
        if let Some(key) = &self.local_key_id {
            clauses.push("local_key_id = ?");
            values.push(Some(key.clone()));
        }
        // `ok` is bound separately by the callers so integer and string
        // parameters stay in separate lists.
        let sql = if clauses.is_empty() {
            "1=1".to_string()
        } else {
            clauses.join(" AND ")
        };
        // The `ok` flag is an integer in SQLite; keep it separate so the
        // string parameters stay in a uniform vector.
        (sql, values, self.ok.map(|ok| ok as i64))
    }
}

#[derive(Clone, Debug)]
pub struct BalanceRow {
    pub account_id: String,
    pub amount: f64,
    pub currency: String,
    pub source: String,
    pub updated_at: i64,
    pub note: String,
}

pub struct Db {
    conn: Mutex<Connection>,
}

#[allow(dead_code)]
impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("打开数据库失败：{}", path.display()))?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            PRAGMA foreign_keys = OFF;
            CREATE TABLE IF NOT EXISTS usage_requests (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts INTEGER NOT NULL,
                request_id TEXT NOT NULL,
                account_id TEXT NOT NULL DEFAULT '',
                provider_id TEXT NOT NULL DEFAULT '',
                local_key_id TEXT,
                client_model TEXT NOT NULL DEFAULT '',
                upstream_model TEXT NOT NULL DEFAULT '',
                protocol TEXT NOT NULL DEFAULT '',
                status INTEGER NOT NULL DEFAULT 0,
                ok INTEGER NOT NULL DEFAULT 0,
                stream INTEGER NOT NULL DEFAULT 0,
                cancelled INTEGER NOT NULL DEFAULT 0,
                prompt_tokens INTEGER NOT NULL DEFAULT 0,
                cached_tokens INTEGER NOT NULL DEFAULT 0,
                completion_tokens INTEGER NOT NULL DEFAULT 0,
                total_tokens INTEGER NOT NULL DEFAULT 0,
                cost_micros INTEGER NOT NULL DEFAULT 0,
                latency_ms INTEGER NOT NULL DEFAULT 0,
                kind TEXT NOT NULL DEFAULT 'chat'
            );
            CREATE INDEX IF NOT EXISTS idx_usage_ts ON usage_requests(ts);
            CREATE INDEX IF NOT EXISTS idx_usage_account_ts ON usage_requests(account_id, ts);
            CREATE INDEX IF NOT EXISTS idx_usage_provider_ts ON usage_requests(provider_id, ts);
            CREATE INDEX IF NOT EXISTS idx_usage_model_ts ON usage_requests(client_model, ts);
            CREATE INDEX IF NOT EXISTS idx_usage_key_ts ON usage_requests(local_key_id, ts);
            CREATE TABLE IF NOT EXISTS balances (
                account_id TEXT PRIMARY KEY,
                amount REAL NOT NULL,
                currency TEXT NOT NULL,
                source TEXT NOT NULL,
                updated_at INTEGER NOT NULL,
                note TEXT NOT NULL DEFAULT ''
            );
            CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            "#,
        )?;
        // Databases created before the cached-token column existed need the
        // column added in place; CREATE TABLE IF NOT EXISTS will not do it.
        let has_cached: bool = conn
            .prepare("PRAGMA table_info(usage_requests)")?
            .query_map([], |r| r.get::<_, String>(1))?
            .filter_map(Result::ok)
            .any(|name| name == "cached_tokens");
        if !has_cached {
            conn.execute_batch(
                "ALTER TABLE usage_requests ADD COLUMN cached_tokens INTEGER NOT NULL DEFAULT 0",
            )?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        match self.conn.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    pub fn record_usage(&self, rec: &UsageRecord) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO usage_requests
             (ts, request_id, account_id, provider_id, local_key_id, client_model, upstream_model,
              protocol, status, ok, stream, cancelled, prompt_tokens, cached_tokens,
              completion_tokens, total_tokens, cost_micros, latency_ms, kind)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)",
            params![
                rec.ts,
                rec.request_id,
                rec.account_id,
                rec.provider_id,
                rec.local_key_id,
                rec.client_model,
                rec.upstream_model,
                rec.protocol,
                rec.status as i64,
                rec.ok as i64,
                rec.stream as i64,
                rec.cancelled as i64,
                rec.prompt_tokens as i64,
                rec.cached_tokens as i64,
                rec.completion_tokens as i64,
                rec.total_tokens as i64,
                rec.cost_micros as i64,
                rec.latency_ms as i64,
                rec.kind,
            ],
        )?;
        // Keep the history bounded: only the newest 1000 rows are retained.
        conn.execute(
            "DELETE FROM usage_requests WHERE id <= (
                 SELECT id FROM usage_requests ORDER BY id DESC LIMIT 1 OFFSET 1000
             )",
            [],
        )?;
        Ok(())
    }

    /// Total matching rows for the call-detail page.
    pub fn usage_count(&self, filter: &UsageFilter) -> Result<u64> {
        let (sql, values, ok) = filter.where_clause();
        let conn = self.lock();
        let query = format!(
            "SELECT COUNT(*) FROM usage_requests WHERE {sql}{}",
            if ok.is_some() { " AND ok = ?" } else { "" }
        );
        let mut stmt = conn.prepare(&query)?;
        let mut index = 1;
        for value in &values {
            stmt.raw_bind_parameter(index, value)?;
            index += 1;
        }
        if let Some(ok) = ok {
            stmt.raw_bind_parameter(index, ok)?;
        }
        let count: i64 = stmt
            .raw_query()
            .next()?
            .map(|row| row.get(0))
            .transpose()?
            .unwrap_or(0);
        Ok(count as u64)
    }

    /// One page of full records, newest first.
    pub fn usage_page(
        &self,
        filter: &UsageFilter,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<UsageDetail>> {
        let (sql, values, ok) = filter.where_clause();
        let conn = self.lock();
        let query = format!(
            "SELECT id, ts, request_id, account_id, provider_id, local_key_id,
                    client_model, upstream_model, protocol, status, ok, stream, cancelled,
                    prompt_tokens, cached_tokens, completion_tokens, total_tokens,
                    cost_micros, latency_ms, kind
             FROM usage_requests WHERE {sql}{}
             ORDER BY id DESC LIMIT ? OFFSET ?",
            if ok.is_some() { " AND ok = ?" } else { "" }
        );
        let mut stmt = conn.prepare(&query)?;
        let mut index = 1;
        for value in &values {
            stmt.raw_bind_parameter(index, value)?;
            index += 1;
        }
        if let Some(ok) = ok {
            stmt.raw_bind_parameter(index, ok)?;
            index += 1;
        }
        stmt.raw_bind_parameter(index, limit as i64)?;
        stmt.raw_bind_parameter(index + 1, offset as i64)?;
        let mut raw_rows = stmt.raw_query();
        let mut out = Vec::new();
        while let Some(r) = raw_rows.next()? {
            out.push(UsageDetail {
                id: r.get(0)?,
                ts: r.get(1)?,
                request_id: r.get(2)?,
                account_id: r.get(3)?,
                provider_id: r.get(4)?,
                local_key_id: r.get(5)?,
                client_model: r.get(6)?,
                upstream_model: r.get(7)?,
                protocol: r.get(8)?,
                status: r.get::<_, i64>(9)? as u16,
                ok: r.get::<_, i64>(10)? != 0,
                stream: r.get::<_, i64>(11)? != 0,
                cancelled: r.get::<_, i64>(12)? != 0,
                prompt_tokens: r.get::<_, i64>(13)? as u64,
                cached_tokens: r.get::<_, i64>(14)? as u64,
                completion_tokens: r.get::<_, i64>(15)? as u64,
                total_tokens: r.get::<_, i64>(16)? as u64,
                cost_micros: r.get::<_, i64>(17)? as u64,
                latency_ms: r.get::<_, i64>(18)? as u64,
                kind: r.get(19)?,
            });
        }
        Ok(out)
    }

    pub fn delete_usage(&self, id: i64) -> Result<bool> {
        let conn = self.lock();
        let n = conn.execute("DELETE FROM usage_requests WHERE id = ?1", params![id])?;
        Ok(n > 0)
    }

    pub fn clear_usage(&self) -> Result<usize> {
        let conn = self.lock();
        let n = conn.execute("DELETE FROM usage_requests", [])?;
        Ok(n)
    }

    pub fn sum_account_since(&self, account_id: &str, since: i64) -> Result<(u64, u64)> {
        let conn = self.lock();
        let row = conn.query_row(
            "SELECT COALESCE(SUM(total_tokens),0), COUNT(*) FROM usage_requests
             WHERE account_id = ?1 AND ts >= ?2 AND ok = 1",
            params![account_id, since],
            |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64)),
        )?;
        Ok(row)
    }

    pub fn sum_local_key_since(&self, key_id: &str, since: i64) -> Result<(u64, u64)> {
        let conn = self.lock();
        let row = conn.query_row(
            "SELECT COALESCE(SUM(total_tokens),0), COUNT(*) FROM usage_requests
             WHERE local_key_id = ?1 AND ts >= ?2",
            params![key_id, since],
            |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64)),
        )?;
        Ok(row)
    }

    pub fn cost_account_since(&self, account_id: &str, since: i64) -> Result<u64> {
        let conn = self.lock();
        let v: i64 = conn.query_row(
            "SELECT COALESCE(SUM(cost_micros),0) FROM usage_requests
             WHERE account_id = ?1 AND ts >= ?2 AND ok = 1",
            params![account_id, since],
            |r| r.get(0),
        )?;
        Ok(v as u64)
    }

    pub fn totals_since(&self, since: i64) -> Result<Totals> {
        self.totals_since_key(since, None)
    }

    /// Totals for a period, optionally restricted to one local key.
    /// A `None` key includes every request, including unauthenticated ones.
    pub fn totals_since_key(&self, since: i64, key: Option<&str>) -> Result<Totals> {
        let conn = self.lock();
        let row = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(ok),0),
                    COALESCE(SUM(prompt_tokens),0), COALESCE(SUM(cached_tokens),0),
                    COALESCE(SUM(completion_tokens),0),
                    COALESCE(SUM(total_tokens),0), COALESCE(SUM(cost_micros),0),
                    COALESCE(CAST(AVG(CASE WHEN ok=1 THEN latency_ms END) AS INTEGER),0)
             FROM usage_requests WHERE ts >= ?1 AND (?2 IS NULL OR local_key_id = ?2)",
            params![since, key],
            |r| {
                Ok(Totals {
                    requests: r.get::<_, i64>(0)? as u64,
                    ok: r.get::<_, i64>(1)? as u64,
                    prompt_tokens: r.get::<_, i64>(2)? as u64,
                    cached_tokens: r.get::<_, i64>(3)? as u64,
                    completion_tokens: r.get::<_, i64>(4)? as u64,
                    total_tokens: r.get::<_, i64>(5)? as u64,
                    cost_micros: r.get::<_, i64>(6)? as u64,
                    avg_latency_ms: r.get::<_, i64>(7)? as u64,
                })
            },
        )?;
        Ok(row)
    }

    pub fn totals_for_model_since(&self, client_model: &str, since: i64) -> Result<Totals> {
        let conn = self.lock();
        let row = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(ok),0),
                    COALESCE(SUM(prompt_tokens),0), COALESCE(SUM(cached_tokens),0),
                    COALESCE(SUM(completion_tokens),0),
                    COALESCE(SUM(total_tokens),0), COALESCE(SUM(cost_micros),0),
                    COALESCE(CAST(AVG(CASE WHEN ok=1 THEN latency_ms END) AS INTEGER),0)
             FROM usage_requests WHERE ts >= ?1 AND client_model = ?2",
            params![since, client_model],
            |r| {
                Ok(Totals {
                    requests: r.get::<_, i64>(0)? as u64,
                    ok: r.get::<_, i64>(1)? as u64,
                    prompt_tokens: r.get::<_, i64>(2)? as u64,
                    cached_tokens: r.get::<_, i64>(3)? as u64,
                    completion_tokens: r.get::<_, i64>(4)? as u64,
                    total_tokens: r.get::<_, i64>(5)? as u64,
                    cost_micros: r.get::<_, i64>(6)? as u64,
                    avg_latency_ms: r.get::<_, i64>(7)? as u64,
                })
            },
        )?;
        Ok(row)
    }

    pub fn group_since(&self, dimension: &str, since: i64) -> Result<Vec<GroupRow>> {
        self.group_since_key(dimension, since, None)
    }

    pub fn group_since_key(
        &self,
        dimension: &str,
        since: i64,
        key: Option<&str>,
    ) -> Result<Vec<GroupRow>> {
        let column = match dimension {
            "provider" => "provider_id",
            "account" => "account_id",
            "model" => "client_model",
            _ => "client_model",
        };
        let sql = format!(
            "SELECT {column}, COUNT(*), COALESCE(SUM(ok),0),
                    COALESCE(SUM(prompt_tokens),0), COALESCE(SUM(cached_tokens),0),
                    COALESCE(SUM(completion_tokens),0),
                    COALESCE(SUM(total_tokens),0), COALESCE(SUM(cost_micros),0),
                    COALESCE(CAST(AVG(CASE WHEN ok=1 THEN latency_ms END) AS INTEGER),0)
             FROM usage_requests WHERE ts >= ?1 AND (?2 IS NULL OR local_key_id = ?2)
             GROUP BY {column} ORDER BY SUM(total_tokens) DESC"
        );
        let conn = self.lock();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![since, key], |r| {
            Ok(GroupRow {
                key: r.get::<_, String>(0)?,
                totals: Totals {
                    requests: r.get::<_, i64>(1)? as u64,
                    ok: r.get::<_, i64>(2)? as u64,
                    prompt_tokens: r.get::<_, i64>(3)? as u64,
                    cached_tokens: r.get::<_, i64>(4)? as u64,
                    completion_tokens: r.get::<_, i64>(5)? as u64,
                    total_tokens: r.get::<_, i64>(6)? as u64,
                    cost_micros: r.get::<_, i64>(7)? as u64,
                    avg_latency_ms: r.get::<_, i64>(8)? as u64,
                },
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn model_upstream_since(&self, since: i64) -> Result<Vec<ModelPairRow>> {
        self.model_upstream_since_key(since, None)
    }

    pub fn model_upstream_since_key(
        &self,
        since: i64,
        key: Option<&str>,
    ) -> Result<Vec<ModelPairRow>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT client_model, upstream_model, COUNT(*), COALESCE(SUM(ok),0),
                    COALESCE(SUM(prompt_tokens),0), COALESCE(SUM(cached_tokens),0),
                    COALESCE(SUM(completion_tokens),0),
                    COALESCE(SUM(total_tokens),0), COALESCE(SUM(cost_micros),0)
             FROM usage_requests WHERE ts >= ?1 AND (?2 IS NULL OR local_key_id = ?2)
             GROUP BY client_model, upstream_model
             ORDER BY SUM(total_tokens) DESC",
        )?;
        let rows = stmt.query_map(params![since, key], |r| {
            Ok(ModelPairRow {
                client_model: r.get(0)?,
                upstream_model: r.get(1)?,
                totals: Totals {
                    requests: r.get::<_, i64>(2)? as u64,
                    ok: r.get::<_, i64>(3)? as u64,
                    prompt_tokens: r.get::<_, i64>(4)? as u64,
                    cached_tokens: r.get::<_, i64>(5)? as u64,
                    completion_tokens: r.get::<_, i64>(6)? as u64,
                    total_tokens: r.get::<_, i64>(7)? as u64,
                    cost_micros: r.get::<_, i64>(8)? as u64,
                    avg_latency_ms: 0,
                },
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn series_since(&self, since: i64, bucket_secs: i64) -> Result<Vec<SeriesPoint>> {
        self.series_since_key(since, bucket_secs, None)
    }

    pub fn series_since_key(
        &self,
        since: i64,
        bucket_secs: i64,
        key: Option<&str>,
    ) -> Result<Vec<SeriesPoint>> {
        let bucket_secs = bucket_secs.max(60);
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT ((ts - ?1) / ?2) * ?2 + ?1 AS bucket,
                    COUNT(*), COALESCE(SUM(ok),0), COALESCE(SUM(total_tokens),0),
                    COALESCE(SUM(cost_micros),0)
             FROM usage_requests WHERE ts >= ?1 AND (?3 IS NULL OR local_key_id = ?3)
             GROUP BY bucket ORDER BY bucket",
        )?;
        let rows = stmt.query_map(params![since, bucket_secs, key], |r| {
            Ok(SeriesPoint {
                bucket: r.get::<_, i64>(0)?,
                totals: Totals {
                    requests: r.get::<_, i64>(1)? as u64,
                    ok: r.get::<_, i64>(2)? as u64,
                    total_tokens: r.get::<_, i64>(3)? as u64,
                    cost_micros: r.get::<_, i64>(4)? as u64,
                    ..Totals::default()
                },
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn recent_requests(&self, limit: usize) -> Result<Vec<RecentRequest>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, ts, request_id, account_id, provider_id, client_model, upstream_model,
                    protocol, status, ok, stream, cancelled, prompt_tokens, cached_tokens,
                    completion_tokens, total_tokens, cost_micros, latency_ms, local_key_id
             FROM usage_requests ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            Ok(RecentRequest {
                id: r.get(0)?,
                ts: r.get(1)?,
                request_id: r.get(2)?,
                account_id: r.get(3)?,
                provider_id: r.get(4)?,
                client_model: r.get(5)?,
                upstream_model: r.get(6)?,
                protocol: r.get(7)?,
                status: r.get::<_, i64>(8)? as u16,
                ok: r.get::<_, i64>(9)? != 0,
                stream: r.get::<_, i64>(10)? != 0,
                cancelled: r.get::<_, i64>(11)? != 0,
                prompt_tokens: r.get::<_, i64>(12)? as u64,
                cached_tokens: r.get::<_, i64>(13)? as u64,
                completion_tokens: r.get::<_, i64>(14)? as u64,
                total_tokens: r.get::<_, i64>(15)? as u64,
                cost_micros: r.get::<_, i64>(16)? as u64,
                latency_ms: r.get::<_, i64>(17)? as u64,
                local_key_id: r.get(18)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn cleanup(&self, retention_days: u32) -> Result<usize> {
        let cutoff = chrono::Utc::now().timestamp() - (retention_days.max(1) as i64) * 86_400;
        let conn = self.lock();
        let n = conn.execute("DELETE FROM usage_requests WHERE ts < ?1", params![cutoff])?;
        Ok(n)
    }

    pub fn balance_upsert(&self, row: &BalanceRow) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO balances(account_id, amount, currency, source, updated_at, note)
             VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(account_id) DO UPDATE SET
               amount=excluded.amount, currency=excluded.currency,
               source=excluded.source, updated_at=excluded.updated_at, note=excluded.note",
            params![
                row.account_id,
                row.amount,
                row.currency,
                row.source,
                row.updated_at,
                row.note
            ],
        )?;
        Ok(())
    }

    pub fn balances(&self) -> Result<Vec<BalanceRow>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT account_id, amount, currency, source, updated_at, note FROM balances",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(BalanceRow {
                account_id: r.get(0)?,
                amount: r.get(1)?,
                currency: r.get(2)?,
                source: r.get(3)?,
                updated_at: r.get(4)?,
                note: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn meta_set(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO meta(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn meta_get(&self, key: &str) -> Result<Option<String>> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT value FROM meta WHERE key = ?1")?;
        let mut rows = stmt.query(params![key])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    pub fn count_usage(&self) -> Result<u64> {
        let conn = self.lock();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM usage_requests", [], |r| r.get(0))?;
        Ok(n as u64)
    }
}

pub fn today_start() -> i64 {
    use chrono::{Local, TimeZone};
    let now = Local::now();
    now.date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|d| Local.from_local_datetime(&d).single())
        .map(|d| d.timestamp())
        .unwrap_or_else(|| now.timestamp() - 86_400)
}

pub fn range_start(days: u32) -> i64 {
    if days <= 1 {
        today_start()
    } else {
        chrono::Utc::now().timestamp() - (days as i64) * 86_400
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(account: &str, tokens: u64, ok: bool) -> UsageRecord {
        UsageRecord {
            ts: chrono::Utc::now().timestamp(),
            request_id: uuid::Uuid::new_v4().to_string(),
            account_id: account.into(),
            provider_id: "p1".into(),
            local_key_id: Some("k1".into()),
            client_model: "gpt-4o".into(),
            upstream_model: "gpt-4o-2024".into(),
            protocol: "openai-chat".into(),
            status: if ok { 200 } else { 500 },
            ok,
            stream: true,
            cancelled: false,
            prompt_tokens: tokens,
            cached_tokens: 0,
            completion_tokens: 0,
            total_tokens: tokens,
            cost_micros: 100,
            latency_ms: 42,
            kind: "chat".into(),
        }
    }

    #[test]
    fn records_and_aggregates() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("t.db")).unwrap();
        db.record_usage(&sample("a1", 10, true)).unwrap();
        db.record_usage(&sample("a1", 5, true)).unwrap();
        db.record_usage(&sample("a2", 20, false)).unwrap();
        let totals = db.totals_since(0).unwrap();
        assert_eq!(totals.requests, 3);
        assert_eq!(totals.ok, 2);
        assert_eq!(totals.total_tokens, 35);
        let (tokens, reqs) = db.sum_account_since("a1", 0).unwrap();
        assert_eq!((tokens, reqs), (15, 2));
        let groups = db.group_since("account", 0).unwrap();
        assert_eq!(groups.len(), 2);
        let series = db.series_since(0, 3600).unwrap();
        assert!(!series.is_empty());
    }

    #[test]
    fn filters_usage_by_local_key() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("t.db")).unwrap();
        let mut first = sample("a1", 10, true);
        first.local_key_id = Some("k1".into());
        let mut second = sample("a1", 20, true);
        second.local_key_id = Some("k2".into());
        let mut third = sample("a2", 30, true);
        third.local_key_id = None;
        db.record_usage(&first).unwrap();
        db.record_usage(&second).unwrap();
        db.record_usage(&third).unwrap();

        let totals = db.totals_since_key(0, Some("k1")).unwrap();
        assert_eq!(totals.requests, 1);
        assert_eq!(totals.total_tokens, 10);
        let totals = db.totals_since_key(0, None).unwrap();
        assert_eq!(totals.requests, 3);
        assert_eq!(totals.total_tokens, 60);
        let groups = db.group_since_key("model", 0, Some("k2")).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].totals.total_tokens, 20);
        let pairs = db.model_upstream_since_key(0, Some("k1")).unwrap();
        assert_eq!(pairs.len(), 1);
        assert!(!db.series_since_key(0, 3600, Some("k1")).unwrap().is_empty());
    }

    #[test]
    fn sums_cached_tokens_separately() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("t.db")).unwrap();
        let mut rec = sample("a1", 100, true);
        rec.cached_tokens = 40;
        db.record_usage(&rec).unwrap();
        let mut second = sample("a1", 50, true);
        second.cached_tokens = 10;
        db.record_usage(&second).unwrap();

        let totals = db.totals_since(0).unwrap();
        assert_eq!(totals.prompt_tokens, 150);
        assert_eq!(totals.cached_tokens, 50);
        let groups = db.group_since("account", 0).unwrap();
        assert_eq!(groups[0].totals.cached_tokens, 50);
    }

    #[test]
    fn adds_cached_column_to_legacy_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE usage_requests (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    ts INTEGER NOT NULL,
                    request_id TEXT NOT NULL,
                    account_id TEXT NOT NULL DEFAULT '',
                    provider_id TEXT NOT NULL DEFAULT '',
                    local_key_id TEXT,
                    client_model TEXT NOT NULL DEFAULT '',
                    upstream_model TEXT NOT NULL DEFAULT '',
                    protocol TEXT NOT NULL DEFAULT '',
                    status INTEGER NOT NULL DEFAULT 0,
                    ok INTEGER NOT NULL DEFAULT 0,
                    stream INTEGER NOT NULL DEFAULT 0,
                    cancelled INTEGER NOT NULL DEFAULT 0,
                    prompt_tokens INTEGER NOT NULL DEFAULT 0,
                    completion_tokens INTEGER NOT NULL DEFAULT 0,
                    total_tokens INTEGER NOT NULL DEFAULT 0,
                    cost_micros INTEGER NOT NULL DEFAULT 0,
                    latency_ms INTEGER NOT NULL DEFAULT 0,
                    kind TEXT NOT NULL DEFAULT 'chat'
                );
                INSERT INTO usage_requests (ts, request_id, prompt_tokens, completion_tokens, total_tokens)
                VALUES (1, 'old', 10, 5, 15);",
            )
            .unwrap();
        }
        let db = Db::open(&path).unwrap();
        let totals = db.totals_since(0).unwrap();
        assert_eq!(totals.total_tokens, 15);
        assert_eq!(totals.cached_tokens, 0);
        // New writes work after the in-place migration.
        db.record_usage(&sample("a1", 5, true)).unwrap();
        assert_eq!(db.totals_since(0).unwrap().requests, 2);
    }

    #[test]
    fn pages_filters_and_deletes_call_details() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("t.db")).unwrap();
        for index in 0..25 {
            let mut rec = sample(if index % 2 == 0 { "a1" } else { "a2" }, 10, index != 3);
            rec.client_model = if index < 10 { "gpt-4o" } else { "claude" }.into();
            db.record_usage(&rec).unwrap();
        }

        let page = db.usage_page(&UsageFilter::default(), 0, 20).unwrap();
        assert_eq!(page.len(), 20);
        assert_eq!(db.usage_count(&UsageFilter::default()).unwrap(), 25);
        let page = db.usage_page(&UsageFilter::default(), 20, 20).unwrap();
        assert_eq!(page.len(), 5);

        let filter = UsageFilter {
            client_model: Some("gpt-4o".into()),
            ..UsageFilter::default()
        };
        assert_eq!(db.usage_count(&filter).unwrap(), 10);
        assert_eq!(
            db.usage_page(&filter, 0, 20).unwrap()[0].client_model,
            "gpt-4o"
        );

        let failed = UsageFilter {
            ok: Some(false),
            ..UsageFilter::default()
        };
        assert_eq!(db.usage_count(&failed).unwrap(), 1);

        let id = page[0].id;
        assert!(db.delete_usage(id).unwrap());
        assert_eq!(db.usage_count(&UsageFilter::default()).unwrap(), 24);
        assert_eq!(db.clear_usage().unwrap(), 24);
        assert_eq!(db.usage_count(&UsageFilter::default()).unwrap(), 0);
    }

    #[test]
    fn keeps_only_newest_thousand_records() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("t.db")).unwrap();
        for _ in 0..1005 {
            db.record_usage(&sample("a1", 1, true)).unwrap();
        }
        assert_eq!(db.usage_count(&UsageFilter::default()).unwrap(), 1000);
    }

    #[test]
    fn balance_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("t.db")).unwrap();
        db.balance_upsert(&BalanceRow {
            account_id: "a1".into(),
            amount: 12.5,
            currency: "USD".into(),
            source: "auto".into(),
            updated_at: 1,
            note: "x".into(),
        })
        .unwrap();
        let rows = db.balances().unwrap();
        assert_eq!(rows[0].amount, 12.5);
    }
}
