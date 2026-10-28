//! Hunter: repository discovery on GitHub, with explainable signals instead of a quality score.

use serde_json::{json, Value};

use crate::agents::{self, Agents, CallSpec};
use crate::contracts::{Candidate, GithubMeta, Signal};
use crate::util::{truncate, years_since};

pub struct GitHub {
    http: reqwest::Client,
    token: Option<String>,
}

impl GitHub {
    pub fn new(token: Option<String>) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder().user_agent("inheritance-engine/0.1 (+local)").timeout(std::time::Duration::from_secs(20)).build()?;
        Ok(Self { http, token })
    }

    async fn get(&self, url: &str, accept: &str) -> anyhow::Result<reqwest::Response> {
        let mut req = self.http.get(url).header("Accept", accept).header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(t) = &self.token {
            req = req.bearer_auth(t);
        }
        let resp = req.send().await?;
        if resp.status() == reqwest::StatusCode::FORBIDDEN || resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let reset = resp.headers().get("x-ratelimit-reset").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
            anyhow::bail!("GitHub rate limit reached{}. Set GITHUB_TOKEN (read-only, no scopes needed) to raise the limit.", if reset.is_empty() { String::new() } else { format!(" (resets at unix time {reset})") });
        }
        if !resp.status().is_success() {
            anyhow::bail!("GitHub returned {} for {}", resp.status(), url);
        }
        Ok(resp)
    }

    pub async fn search(&self, q: &str, sort: Option<&str>) -> anyhow::Result<(u64, Vec<Candidate>)> {
        let mut url = reqwest::Url::parse("https://api.github.com/search/repositories")?;
        url.query_pairs_mut().append_pair("q", q).append_pair("per_page", "20");
        if let Some(s) = sort {
            url.query_pairs_mut().append_pair("sort", s).append_pair("order", "desc");
        }
        let v: Value = self.get(url.as_str(), "application/vnd.github+json").await?.json().await?;
        let total = v["total_count"].as_u64().unwrap_or(0);
        let items = v["items"].as_array().cloned().unwrap_or_default();
        Ok((total, items.iter().map(candidate_from).collect()))
    }

    pub async fn repo(&self, full_name: &str) -> anyhow::Result<(Candidate, GithubMeta)> {
        validate_full_name(full_name)?;
        let v: Value = self.get(&format!("https://api.github.com/repos/{full_name}"), "application/vnd.github+json").await?.json().await?;
        if v["private"].as_bool().unwrap_or(false) {
            anyhow::bail!("private repositories are not supported in this version");
        }
        let c = candidate_from(&v);
        let meta = GithubMeta {
            description: c.description.clone(),
            stars: c.stars,
            forks: c.forks,
            pushed_at: Some(c.pushed_at.clone()),
            created_at: Some(c.created_at.clone()),
            default_branch: c.default_branch.clone(),
            owner: c.owner.clone(),
            size_kb: c.size_kb,
            archived: c.archived,
            clone_url: format!("https://github.com/{}.git", c.full_name),
        };
        Ok((c, meta))
    }

    pub async fn readme(&self, full_name: &str) -> anyhow::Result<String> {
        validate_full_name(full_name)?;
        Ok(self.get(&format!("https://api.github.com/repos/{full_name}/readme"), "application/vnd.github.raw").await?.text().await?)
    }
}

pub fn validate_full_name(full_name: &str) -> anyhow::Result<()> {
    let ok = full_name.split('/').count() == 2
        && full_name.split('/').all(|p| !p.is_empty() && p != "." && p != ".." && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')));
    if !ok {
        anyhow::bail!("'{full_name}' is not a valid owner/repo name");
    }
    Ok(())
}

fn candidate_from(v: &Value) -> Candidate {
    let created_at = v["created_at"].as_str().unwrap_or("").to_string();
    let pushed_at = v["pushed_at"].as_str().unwrap_or("").to_string();
    let mut c = Candidate {
        full_name: v["full_name"].as_str().unwrap_or("").into(),
        owner: v["owner"]["login"].as_str().unwrap_or("").into(),
        name: v["name"].as_str().unwrap_or("").into(),
        html_url: v["html_url"].as_str().unwrap_or("").into(),
        description: v["description"].as_str().map(String::from),
        language: v["language"].as_str().map(String::from),
        topics: v["topics"].as_array().map(|a| a.iter().filter_map(|t| t.as_str().map(String::from)).collect()).unwrap_or_default(),
        stars: v["stargazers_count"].as_u64().unwrap_or(0),
        forks: v["forks_count"].as_u64().unwrap_or(0),
        open_issues: v["open_issues_count"].as_u64().unwrap_or(0),
        age_years: years_since(&created_at),
        idle_years: years_since(&pushed_at),
        created_at,
        pushed_at,
        archived: v["archived"].as_bool().unwrap_or(false),
        license: v["license"]["spdx_id"].as_str().filter(|s| *s != "NOASSERTION").map(String::from),
        size_kb: v["size"].as_u64().unwrap_or(0),
        default_branch: v["default_branch"].as_str().unwrap_or("main").into(),
        signals: vec![],
    };
    c.signals = signals(&c);
    c
}

/// Evidence-based signals. They describe; they do not rank or score.
pub fn signals(c: &Candidate) -> Vec<Signal> {
    let mut s = Vec::new();
    let sig = |kind: &str, label: String, detail: String| Signal { kind: kind.into(), label, detail };
    if let Some(age) = c.age_years {
        if age >= 6.0 {
            s.push(sig("historical", format!("{age:.0} years of history"), format!("Created {}", c.created_at.get(..10).unwrap_or(""))));
        }
    }
    if c.forks >= 10 {
        s.push(sig("historical", format!("{} forks", c.forks), "Other people have built on this code".into()));
    }
    if c.stars >= 50 {
        s.push(sig("historical", format!("{} stars", c.stars), "Popularity is context, not proof of quality".into()));
    }
    if c.description.as_deref().map(|d| d.len() > 20).unwrap_or(false) {
        s.push(sig("present", "Stated purpose".into(), truncate(c.description.as_deref().unwrap_or(""), 140)));
    }
    if c.open_issues > 0 {
        s.push(sig("present", format!("{} open issues", c.open_issues), "Includes pull requests; may show remaining demand".into()));
    }
    if let Some(idle) = c.idle_years {
        if idle >= 3.0 {
            s.push(sig("evolution", format!("Quiet for {idle:.1} years"), format!("Last push {}; dependencies have probably drifted", c.pushed_at.get(..10).unwrap_or(""))));
        }
    }
    if c.size_kb > 0 && c.size_kb < 5_000 {
        s.push(sig("evolution", "Small codebase".into(), format!("{} KB repository; feasible to understand end-to-end", c.size_kb)));
    }
    if !c.topics.is_empty() {
        s.push(sig("evolution", "Tagged topics".into(), c.topics.iter().take(5).cloned().collect::<Vec<_>>().join(", ")));
    }
    if c.archived {
        s.push(sig("caution", "Archived".into(), "The owner made it read-only; a successor would be a separate project".into()));
    }
    match &c.license {
        None => s.push(sig("caution", "No license detected".into(), "Fine for private study; redistribution needs the author's permission".into())),
        Some(l) => s.push(sig("present", format!("License: {l}"), "Reported by GitHub; obligations still apply to successors".into())),
    }
    if c.size_kb > 100_000 {
        s.push(sig("caution", "Large repository".into(), format!("{} MB; cloning and analysis will be slow", c.size_kb / 1024)));
    }
    s
}

fn sanitize_keywords(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == ' ' || c == '+' || c == '#' { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .take(6)
        .collect::<Vec<_>>()
        .join(" ")
}

fn sanitize_language(s: &str) -> Option<String> {
    let l: String = s.chars().filter(|c| c.is_alphanumeric() || *c == '+' || *c == '#').collect();
    if l.is_empty() || l.eq_ignore_ascii_case("any") || l.eq_ignore_ascii_case("none") {
        None
    } else {
        Some(l)
    }
}

pub struct HuntResult {
    pub query_used: String,
    pub ai: Value,
    pub total: u64,
    pub results: Vec<Candidate>,
}

pub async fn hunt(gh: &GitHub, agents: &Agents, text: &str, mode: &str, lang: Option<&str>) -> anyhow::Result<HuntResult> {
    let text = text.trim();
    if text.is_empty() {
        anyhow::bail!("describe a problem or a topic to search for");
    }
    let mut ai = json!({"used": false});
    let (keywords, language) = if mode == "problem" {
        let n = agents::nonce();
        let task = format!(
            "A user describes a problem. Turn it into a short GitHub repository search: 2-5 plain keywords that would appear in a matching project's name or description, plus the most likely programming language (or 'any').\n\n{}",
            agents::untrusted(&n, "USER PROBLEM", text)
        );
        let spec = CallSpec { agent: "hunter", purpose: "Translate problem into GitHub search".into(), inheritance_id: None, parent_id: None, input_files: vec![], task, schema: agents::hunter_query_schema(), max_tokens: 400 };
        match agents.call_json(spec, &n).await {
            Ok((v, call_id)) => {
                let kw = sanitize_keywords(v["keywords"].as_str().unwrap_or(text));
                let lg = lang.map(String::from).or_else(|| sanitize_language(v["language"].as_str().unwrap_or("")));
                ai = json!({"used": true, "call_id": call_id, "reasoning": v["reasoning"]});
                (if kw.is_empty() { sanitize_keywords(text) } else { kw }, lg)
            }
            Err(e) => {
                ai = json!({"used": false, "error": e.to_string()});
                (sanitize_keywords(text), lang.map(String::from))
            }
        }
    } else {
        (sanitize_keywords(text), lang.map(String::from))
    };
    let mut q = format!("{keywords} in:name,description,readme");
    if let Some(l) = language.as_deref().and_then(sanitize_language) {
        q.push_str(&format!(" language:{l}"));
    }
    let sort = if mode == "curiosity" {
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(365 * 4)).format("%Y-%m-%d");
        q.push_str(&format!(" pushed:<{cutoff} stars:>10"));
        Some("stars")
    } else {
        q.push_str(" stars:>3");
        None
    };
    let (total, results) = gh.search(&q, sort).await?;
    Ok(HuntResult { query_used: q, ai, total, results })
}

pub async fn explain(gh: &GitHub, agents: &Agents, full_name: &str) -> anyhow::Result<(Value, usize, String)> {
    let (c, _) = gh.repo(full_name).await?;
    let readme = gh.readme(full_name).await.unwrap_or_default();
    let readme_excerpt = truncate(&crate::security::redact(&readme), 7000);
    let n = agents::nonce();
    let meta = json!({
        "full_name": c.full_name, "description": c.description, "language": c.language, "topics": c.topics,
        "stars": c.stars, "forks": c.forks, "created_at": c.created_at, "pushed_at": c.pushed_at,
        "archived": c.archived, "license": c.license, "size_kb": c.size_kb
    });
    let task = format!(
        "Explain whether this repository could be worth inheriting. Judge three separate dimensions without merging them into a score: historical value (what it represents or preserved), present usefulness (does it solve a problem people still have), evolution potential (could it be restored or extended). For each dimension write one or two sentences that point to concrete evidence from the metadata or README; never answer with a rating word such as High/Medium/Low. Popularity and inactivity are context, not verdicts. List concrete caveats. If the README is empty, say what cannot be known.\n\nGitHub metadata (trusted):\n{}\n\n{}",
        serde_json::to_string_pretty(&meta)?,
        agents::untrusted(&n, "README", if readme_excerpt.is_empty() { "(no README found)" } else { &readme_excerpt })
    );
    let spec = CallSpec { agent: "hunter", purpose: format!("Explain why {full_name} may be worth inheriting"), inheritance_id: None, parent_id: None, input_files: vec!["README (via GitHub API)".into()], task, schema: agents::hunter_explain_schema(), max_tokens: 900 };
    let (v, call_id) = agents.call_json(spec, &n).await?;
    Ok((v, readme.len(), call_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_name_validation() {
        assert!(validate_full_name("psf/requests").is_ok());
        assert!(validate_full_name("a/b/c").is_err());
        assert!(validate_full_name("../x").is_err());
        assert!(validate_full_name("a/..").is_err());
        assert!(validate_full_name("a/b?x=1").is_err());
    }

    #[test]
    fn keywords_are_sanitized() {
        assert_eq!(sanitize_keywords("markdown \"tables\" OR stars:>1 && rm -rf"), "markdown tables OR stars 1 rm");
    }

    #[test]
    fn signals_describe_without_scoring() {
        let v = json!({
            "full_name": "o/r", "owner": {"login": "o"}, "name": "r", "html_url": "https://github.com/o/r",
            "description": "A tiny library for parsing things nicely", "language": "Python", "topics": ["parser"],
            "stargazers_count": 120, "forks_count": 15, "open_issues_count": 2,
            "created_at": "2012-01-01T00:00:00Z", "pushed_at": "2016-01-01T00:00:00Z", "archived": false,
            "license": null, "size": 300, "default_branch": "master"
        });
        let c = candidate_from(&v);
        let kinds: Vec<&str> = c.signals.iter().map(|s| s.kind.as_str()).collect();
        assert!(kinds.contains(&"historical"));
        assert!(kinds.contains(&"evolution"));
        assert!(kinds.contains(&"caution"));
        assert!(c.signals.iter().any(|s| s.label == "No license detected"));
    }
}
