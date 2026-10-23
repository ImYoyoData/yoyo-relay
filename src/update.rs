//! GitHub release lookup and self-update asset selection.
//!
//! Release metadata is fetched from the public GitHub API. "立即更新" downloads
//! the matching release archive, verifies its size/digest, replaces the
//! running executable and restarts it. A failed install leaves the original
//! executable untouched and falls back to the release page.

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/ImYoyoData/yoyo-relay/releases/latest";

/// Client for GitHub release checks/downloads. It explicitly honours the
/// system HTTP/SOCKS proxy (Windows registry, or HTTPS_PROXY/ALL_PROXY) so a
/// VPN/proxy is used when one is configured.
pub fn build_client() -> reqwest::Client {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(20))
        .pool_max_idle_per_host(16)
        .tcp_nodelay(true)
        .user_agent(concat!("YoyoRelay/", env!("CARGO_PKG_VERSION")));
    if let Some(proxy) = system_proxy_url() {
        if let Ok(proxy) = reqwest::Proxy::all(&proxy) {
            builder = builder.proxy(proxy);
        }
    }
    builder.build().unwrap_or_else(|_| reqwest::Client::new())
}

fn system_proxy_url() -> Option<String> {
    for key in ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"] {
        if let Ok(value) = std::env::var(key) {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    #[cfg(windows)]
    {
        return windows_proxy_url();
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[cfg(windows)]
fn windows_proxy_url() -> Option<String> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings";
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let query = |name: &str| -> Option<String> {
        let output = Command::new("reg")
            .args(["query", KEY, "/v", name])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout).to_string();
        let value = text
            .split("REG_SZ")
            .nth(1)
            .or_else(|| text.split("REG_DWORD").nth(1))?;
        Some(value.trim().to_string())
    };

    let enabled = query("ProxyEnable")?;
    if !enabled.contains("0x1") && enabled.trim() != "1" {
        return None;
    }
    let server = query("ProxyServer")?;
    if server.is_empty() {
        return None;
    }
    let proxy = server
        .split(';')
        .find_map(|part| part.trim().strip_prefix("https="))
        .or_else(|| {
            server
                .split(';')
                .find_map(|part| part.trim().strip_prefix("http="))
        })
        .unwrap_or(server.as_str())
        .trim();
    if proxy.is_empty() {
        return None;
    }
    if proxy.contains("://") {
        Some(proxy.to_string())
    } else {
        Some(format!("http://{proxy}"))
    }
}

#[derive(Clone, Debug, Default)]
pub struct ReleaseInfo {
    pub version: String,
    pub url: String,
    pub notes: String,
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Clone, Debug, Default)]
pub struct ReleaseAsset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// `sha256:<hex>` when GitHub provides a digest for the asset.
    pub digest: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveKind {
    Zip,
    TarGz,
}

impl ReleaseAsset {
    pub fn archive_kind(&self) -> Option<ArchiveKind> {
        if self.name.ends_with(".zip") {
            Some(ArchiveKind::Zip)
        } else if self.name.ends_with(".tar.gz") {
            Some(ArchiveKind::TarGz)
        } else {
            None
        }
    }
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
    let assets = value
        .get("assets")
        .and_then(Value::as_array)
        .map(|assets| {
            assets
                .iter()
                .filter_map(|asset| {
                    let name = asset.get("name").and_then(Value::as_str)?.to_string();
                    let url = asset
                        .get("browser_download_url")
                        .and_then(Value::as_str)?
                        .to_string();
                    let size = asset.get("size").and_then(Value::as_u64).unwrap_or(0);
                    let digest = asset
                        .get("digest")
                        .and_then(Value::as_str)
                        .filter(|digest| digest.starts_with("sha256:"))
                        .map(str::to_string);
                    Some(ReleaseAsset {
                        name,
                        url,
                        size,
                        digest,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
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
        assets,
    })
}

/// Pick the non-UPX archive for the running platform. UPX builds are only a
/// fallback when a plain build is missing.
pub fn select_asset(assets: &[ReleaseAsset]) -> Option<&ReleaseAsset> {
    select_asset_for(
        assets,
        if cfg!(target_os = "windows") {
            "windows"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else {
            "linux"
        },
        if cfg!(target_arch = "aarch64") {
            "aarch64"
        } else {
            "x86_64"
        },
    )
}

pub fn select_asset_for<'a>(
    assets: &'a [ReleaseAsset],
    os: &str,
    arch: &str,
) -> Option<&'a ReleaseAsset> {
    let platform = match (os, arch) {
        ("windows", "x86_64" | "aarch64") => "windows-x64",
        ("linux", "x86_64") => "linux-x64",
        ("macos", "aarch64") => "macos-arm64",
        ("macos", "x86_64") => "macos-x64",
        _ => return None,
    };
    let candidates: Vec<&ReleaseAsset> = assets
        .iter()
        .filter(|asset| asset.name.contains(platform) && asset.archive_kind().is_some())
        .collect();
    candidates
        .iter()
        .copied()
        .find(|asset| !asset.name.contains("-upx"))
        .or_else(|| candidates.first().copied())
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

    fn asset(name: &str) -> ReleaseAsset {
        ReleaseAsset {
            name: name.into(),
            url: format!("https://example.com/{name}"),
            size: 10,
            digest: None,
        }
    }

    #[test]
    fn selects_plain_platform_archive_before_upx() {
        let assets = vec![
            asset("yoyo-relay-v0.5.0-windows-x64-upx.zip"),
            asset("yoyo-relay-v0.5.0-windows-x64.zip"),
            asset("yoyo-relay-v0.5.0-linux-x64.tar.gz"),
            asset("yoyo-relay-v0.5.0-macos-arm64.tar.gz"),
            asset("yoyo-relay-v0.5.0-macos-x64.tar.gz"),
        ];
        assert_eq!(
            select_asset_for(&assets, "windows", "x86_64").unwrap().name,
            "yoyo-relay-v0.5.0-windows-x64.zip"
        );
        assert_eq!(
            select_asset_for(&assets, "linux", "x86_64").unwrap().name,
            "yoyo-relay-v0.5.0-linux-x64.tar.gz"
        );
        assert_eq!(
            select_asset_for(&assets, "macos", "aarch64").unwrap().name,
            "yoyo-relay-v0.5.0-macos-arm64.tar.gz"
        );
        assert_eq!(
            select_asset_for(&assets, "macos", "x86_64").unwrap().name,
            "yoyo-relay-v0.5.0-macos-x64.tar.gz"
        );
    }

    #[test]
    fn recognizes_archive_types() {
        assert_eq!(
            asset("x-windows-x64.zip").archive_kind(),
            Some(ArchiveKind::Zip)
        );
        assert_eq!(
            asset("x-linux-x64.tar.gz").archive_kind(),
            Some(ArchiveKind::TarGz)
        );
        assert_eq!(asset("x.txt").archive_kind(), None);
    }
}
