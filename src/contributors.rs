//! GitHub contributor list for the About page.

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

const CONTRIBUTORS_URL: &str =
    "https://api.github.com/repos/ImYoyoData/yoyo-relay/contributors?per_page=12";

#[derive(Clone, Debug)]
pub struct Contributor {
    pub login: String,
    pub contributions: u64,
    pub url: String,
    pub avatar: Option<Arc<Vec<u8>>>,
}

fn parse_contributors(items: &[Value]) -> Vec<Contributor> {
    let mut contributors: Vec<Contributor> = items
        .iter()
        .filter_map(|item| {
            let login = item.get("login").and_then(Value::as_str)?.to_string();
            Some(Contributor {
                login,
                contributions: item
                    .get("contributions")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                url: item
                    .get("html_url")
                    .and_then(Value::as_str)
                    .unwrap_or("https://github.com/ImYoyoData/yoyo-relay/graphs/contributors")
                    .to_string(),
                avatar: None,
            })
        })
        .collect();
    contributors.sort_by(|a, b| b.contributions.cmp(&a.contributions));
    contributors
}

/// Fetch the top contributors and their avatars. Avatar failures are ignored;
/// the UI falls back to an initial-letter tile.
pub async fn fetch(client: &reqwest::Client) -> Result<Vec<Contributor>> {
    let resp = client
        .get(CONTRIBUTORS_URL)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .context("无法连接 GitHub 贡献者接口")?;
    let status = resp.status();
    if !status.is_success() {
        return Err(anyhow!("GitHub 贡献者接口返回 HTTP {status}"));
    }
    let value: Value = resp.json().await.context("贡献者响应不是有效 JSON")?;
    let Some(items) = value.as_array() else {
        return Err(anyhow!("贡献者响应格式不受支持"));
    };
    let mut contributors = parse_contributors(items);
    let avatar_urls: Vec<String> = items
        .iter()
        .filter_map(|item| {
            item.get("avatar_url")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    let downloads = avatar_urls.into_iter().map(|url| {
        let client = client.clone();
        async move {
            let resp = client
                .get(url)
                .timeout(std::time::Duration::from_secs(10))
                .send()
                .await
                .ok()?;
            if !resp.status().is_success() {
                return None;
            }
            let bytes = resp.bytes().await.ok()?;
            if bytes.is_empty() || bytes.len() > 1024 * 1024 {
                return None;
            }
            Some(Arc::new(bytes.to_vec()))
        }
    });
    let avatars = futures_util::future::join_all(downloads).await;
    for (contributor, avatar) in contributors.iter_mut().zip(avatars) {
        contributor.avatar = avatar;
    }
    Ok(contributors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_sorts_contributors() {
        let items = vec![
            serde_json::json!({"login":"a","contributions":3,"html_url":"https://github.com/a"}),
            serde_json::json!({"login":"b","contributions":9,"html_url":"https://github.com/b"}),
            serde_json::json!({"login":"c"}),
        ];
        let parsed = parse_contributors(&items);
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].login, "b");
        assert_eq!(parsed[0].contributions, 9);
        assert_eq!(parsed[2].contributions, 0);
    }
}
