//! GitHub release lookup for the update indicator.
//!
//! Release metadata is fetched from the public GitHub API. The app never
//! downloads or executes an installer itself; "立即更新" opens the release
//! page so the user stays in control of what runs on their machine.

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/ImYoyoData/yoyo-relay/releases/latest";

#[derive(Clone, Debug, Default)]
pub struct ReleaseInfo {
    pub version: String,
    pub url: String,
    pub notes: String,
}

pub async fn fetch_latest_release(client: &reqwest::Client) -> Result<ReleaseInfo> {
    let resp = client
        .get(LATEST_RELEASE_URL)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .with_context(|| "无法连接 GitHub 更新接口")?;
    let status = resp.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(anyhow!("仓库还没有发布任何 Release"));
    }
    if !status.is_success() {
        return Err(anyhow!("GitHub 更新接口返回 HTTP {status}"));
    }
    let value: Value = resp.json().await.context("更新响应不是有效 JSON")?;
    let tag = value
        .get("tag_name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let version = normalize_version(tag);
    if version.is_empty() {
        return Err(anyhow!("Release 缺少版本标签"));
    }
    Ok(ReleaseInfo {
        version,
        url: value
            .get("html_url")
            .and_then(Value::as_str)
            .unwrap_or("https://github.com/ImYoyoData/yoyo-relay/releases")
            .to_string(),
        notes: value
            .get("body")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    })
}

pub fn normalize_version(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('v')
        .trim_start_matches('V')
        .to_string()
}

/// Numeric semver comparison, ignoring pre-release suffixes. `latest` is newer
/// than `current` when its first three numeric components sort higher.
pub fn is_newer(latest: &str, current: &str) -> bool {
    let parse = |value: &str| -> Vec<u64> {
        normalize_version(value)
            .split(['.', '-', '+'])
            .take(3)
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect()
    };
    let latest = parse(latest);
    let current = parse(current);
    for index in 0..3 {
        let a = latest.get(index).copied().unwrap_or(0);
        let b = current.get(index).copied().unwrap_or(0);
        if a != b {
            return a > b;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_tags() {
        assert_eq!(normalize_version("v0.3.0"), "0.3.0");
        assert_eq!(normalize_version("V1.2.3"), "1.2.3");
    }

    #[test]
    fn compares_versions() {
        assert!(is_newer("0.3.0", "0.2.0"));
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.9", "0.2.0"));
        assert!(is_newer("1.0.0-rc1", "0.9.9"));
    }
}
