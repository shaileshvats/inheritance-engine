//! Live web search through SerpApi (optional).
//!
//! The Hunter uses it to find repositories that GitHub's own search misses, and the research job uses it to find forks,
//! successors and documentation for an inheritance. Only the user's search text and public repository names are sent;
//! never repository content. The API key is read from the environment or `.env` on every call (so adding it needs no
//! restart), and it is never logged, stored or put into an error message.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::hunter::validate_full_name;

const ENDPOINT: &str = "https://serpapi.com/search.json";
const MAX_BODY: usize = 2_000_000;

/// Path segments under github.com that are site pages, not repository owners.
const RESERVED_OWNERS: &[&str] = &[
    "about", "apps", "codespaces", "collections", "customer-stories", "enterprise", "events", "explore", "features",
    "join", "login", "marketplace", "new", "notifications", "orgs", "pricing", "readme", "resources", "search",
    "security", "settings", "site", "solutions", "sponsors", "topics", "trending", "users",
];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WebResult {
    /// Stable citation id within one research run (`W1`, `W2`, …). Empty in Hunter results.
    #[serde(default)]
    pub id: String,
    pub query: String,
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub source: String,
    /// `github_repo`, `docs` or `page`.
    pub kind: String,
    /// `owner/repo` when the result is a GitHub repository page.
    pub repo: Option<String>,
}

pub struct WebSearch {
    http: reqwest::Client,
}

impl WebSearch {
    pub fn new() -> anyhow::Result<Self> {
        let http = reqwest::Client::builder().user_agent("inheritance-engine/0.1 (+local)").timeout(std::time::Duration::from_secs(25)).build()?;
        Ok(Self { http })
    }

    /// The key, from the environment first, then `.env` in the working directory.
    fn key() -> Option<String> {
        if let Some(k) = std::env::var("SERPAPI_API_KEY").ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty()) {
            return Some(k);
        }
        let text = std::fs::read_to_string(".env").ok()?;
        key_from_dotenv(&text)
    }

    pub fn enabled() -> bool {
        Self::key().is_some()
    }

    pub async fn search(&self, query: &str, num: u32) -> anyhow::Result<Vec<WebResult>> {
        let key = Self::key().ok_or_else(|| anyhow::anyhow!("web search is off: add SERPAPI_API_KEY to .env (SerpApi's free plan works)"))?;
        let mut url = reqwest::Url::parse(ENDPOINT)?;
        url.query_pairs_mut().append_pair("engine", "google").append_pair("q", query).append_pair("num", &num.clamp(1, 20).to_string()).append_pair("api_key", &key);
        // `without_url` keeps the key (a query parameter) out of every error message.
        let resp = self.http.get(url).send().await.map_err(|e| anyhow::anyhow!("web search request failed: {}", e.without_url()))?;
        let status = resp.status();
        let body = resp.bytes().await.map_err(|e| anyhow::anyhow!("web search response failed: {}", e.without_url()))?;
        if body.len() > MAX_BODY {
            anyhow::bail!("web search response too large ({} bytes)", body.len());
        }
        let v: Value = serde_json::from_slice(&body).map_err(|_| anyhow::anyhow!("web search returned invalid JSON (HTTP {status})"))?;
        if let Some(err) = v["error"].as_str() {
            anyhow::bail!("SerpApi: {}", err.replace(&key, "[key]"));
        }
        if !status.is_success() {
            anyhow::bail!("SerpApi returned HTTP {status}");
        }
        Ok(parse_results(query, &v))
    }
}

fn key_from_dotenv(text: &str) -> Option<String> {
    text.lines().filter_map(|l| l.trim().strip_prefix("SERPAPI_API_KEY=")).map(|v| v.trim().trim_matches('"').trim_matches('\'').to_string()).find(|v| !v.is_empty())
}

/// Organic results from a SerpApi Google response, classified.
pub fn parse_results(query: &str, v: &Value) -> Vec<WebResult> {
    v["organic_results"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or_default()
        .iter()
        .filter_map(|r| {
            let url = r["link"].as_str()?.to_string();
            if !(url.starts_with("https://") || url.starts_with("http://")) || is_search_redirect(&url) {
                return None;
            }
            let repo = repo_from_url(&url);
            let kind = if repo.is_some() {
                "github_repo"
            } else if is_docs(&url) {
                "docs"
            } else {
                "page"
            };
            Some(WebResult {
                id: String::new(),
                query: query.to_string(),
                title: crate::util::ellipsis(r["title"].as_str().unwrap_or(""), 160),
                snippet: crate::util::ellipsis(r["snippet"].as_str().unwrap_or(""), 320),
                source: r["source"].as_str().or(r["displayed_link"].as_str()).map(|s| crate::util::ellipsis(s, 80)).unwrap_or_default(),
                kind: kind.into(),
                repo,
                url,
            })
        })
        .collect()
}

/// `owner/repo` for a GitHub repository URL (including deeper paths such as `/blob/...`), otherwise `None`.
pub fn repo_from_url(url: &str) -> Option<String> {
    let u = reqwest::Url::parse(url).ok()?;
    if !matches!(u.host_str()?, "github.com" | "www.github.com") {
        return None;
    }
    let mut segs = u.path_segments()?.filter(|s| !s.is_empty());
    let owner = segs.next()?;
    let repo = segs.next()?.trim_end_matches(".git");
    if RESERVED_OWNERS.contains(&owner.to_ascii_lowercase().as_str()) {
        return None;
    }
    let full = format!("{owner}/{repo}");
    validate_full_name(&full).ok().map(|_| full)
}

/// Google redirect links hide the real destination, so they can't be cited.
fn is_search_redirect(url: &str) -> bool {
    reqwest::Url::parse(url).ok().and_then(|u| u.host_str().map(|h| h == "google.com" || h.starts_with("www.google.") || h.starts_with("google."))).unwrap_or(false)
}

fn is_docs(url: &str) -> bool {
    let Ok(u) = reqwest::Url::parse(url) else { return false };
    let host = u.host_str().unwrap_or("");
    host.ends_with("readthedocs.io") || host.ends_with(".github.io") || host.ends_with("readthedocs.org") || host.starts_with("docs.") || host == "pypi.org" || host == "www.npmjs.com"
        || u.path().starts_with("/docs") || u.path().contains("/documentation")
}

/// Removes duplicate URLs and numbers results `W1`, `W2`, … for citation.
pub fn number(results: Vec<WebResult>, max: usize) -> Vec<WebResult> {
    let mut seen = std::collections::HashSet::new();
    results
        .into_iter()
        .filter(|r| seen.insert(r.url.trim_end_matches('/').to_ascii_lowercase()))
        .take(max)
        .enumerate()
        .map(|(i, mut r)| {
            r.id = format!("W{}", i + 1);
            r
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn repository_urls_are_recognised() {
        assert_eq!(repo_from_url("https://github.com/kachayev/fn.py").as_deref(), Some("kachayev/fn.py"));
        assert_eq!(repo_from_url("https://github.com/kachayev/fn.py/blob/master/README.rst").as_deref(), Some("kachayev/fn.py"));
        assert_eq!(repo_from_url("https://www.github.com/a/b.git").as_deref(), Some("a/b"));
        assert_eq!(repo_from_url("https://github.com/topics/functional-programming"), None);
        assert_eq!(repo_from_url("https://github.com/kachayev"), None);
        assert_eq!(repo_from_url("https://gist.github.com/a/b"), None);
        assert_eq!(repo_from_url("https://example.com/github.com/a/b"), None);
    }

    #[test]
    fn results_are_parsed_classified_and_numbered() {
        let v = json!({"organic_results": [
            {"title": "kachayev/fn.py: Functional programming in Python", "link": "https://github.com/kachayev/fn.py", "snippet": "Missing batteries", "source": "GitHub"},
            {"title": "fn.py docs", "link": "https://fnpy.readthedocs.io/en/latest/", "snippet": "Docs"},
            {"title": "A blog post", "link": "https://blog.example.com/fp-python", "snippet": "About fn.py", "displayed_link": "blog.example.com"},
            {"title": "Duplicate", "link": "https://github.com/kachayev/fn.py/", "snippet": "dup"},
            {"title": "Not http", "link": "javascript:alert(1)"},
            {"title": "Redirect", "link": "https://www.google.com/goto?url=abc"}
        ]});
        let r = number(parse_results("fn.py", &v), 10);
        assert_eq!(r.len(), 3);
        assert_eq!((r[0].id.as_str(), r[0].kind.as_str(), r[0].repo.as_deref()), ("W1", "github_repo", Some("kachayev/fn.py")));
        assert_eq!(r[1].kind, "docs");
        assert_eq!((r[2].kind.as_str(), r[2].source.as_str()), ("page", "blog.example.com"));
    }

    #[test]
    fn key_is_read_from_dotenv_text() {
        assert_eq!(key_from_dotenv("# x\nSENTRY_DSN=y\nSERPAPI_API_KEY=\"abc123\"\n").as_deref(), Some("abc123"));
        assert_eq!(key_from_dotenv("SERPAPI_API_KEY=\n"), None);
    }
}
