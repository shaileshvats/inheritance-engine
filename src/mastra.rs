//! Client for the Mastra companion (`companion/mastra`), which runs the Lead Engineer agent and the
//! upstream-issue triage workflow over the same local Gemma model. Optional: when `IE_MASTRA_URL` is unset
//! or the companion is unreachable, the engine's built-in Lead answers instead and says so.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone)]
pub struct MastraClient {
    base: String,
    http: reqwest::Client,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LeadReply {
    pub reply: String,
    #[serde(default)]
    pub proposed_action: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<Value>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub workflow: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub usage: Option<Usage>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

const ACTIONS: &[&str] = &["investigate", "restore_essential", "restore_all", "verify"];

impl LeadReply {
    /// Only known actions survive; anything else from the companion is dropped.
    pub fn action(&self) -> Option<String> {
        self.proposed_action.clone().filter(|a| ACTIONS.contains(&a.as_str()))
    }

    pub fn tool_names(&self) -> Vec<String> {
        self.tool_calls.iter().filter_map(|t| t["tool"].as_str().map(String::from)).collect()
    }
}

impl MastraClient {
    pub fn new(base: &str) -> anyhow::Result<Self> {
        let base = base.trim_end_matches('/').to_string();
        let host = base.split("://").nth(1).unwrap_or("").split(['/', ':']).next().unwrap_or("");
        if !matches!(host, "127.0.0.1" | "localhost") {
            anyhow::bail!("IE_MASTRA_URL must point to a loopback address (got {base})");
        }
        // Long timeout: a triage run classifies several issues on a laptop model.
        let http = reqwest::Client::builder().timeout(Duration::from_secs(1800)).build()?;
        Ok(Self { base, http })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub async fn health(&self) -> Result<Value, String> {
        let r = self.http.get(format!("{}/health", self.base)).timeout(Duration::from_secs(3)).send().await.map_err(|e| e.to_string())?;
        if !r.status().is_success() {
            return Err(format!("companion health returned {}", r.status()));
        }
        r.json().await.map_err(|e| e.to_string())
    }

    async fn post(&self, path: &str, body: Value) -> anyhow::Result<Value> {
        let r = self.http.post(format!("{}{path}", self.base)).json(&body).send().await?;
        let status = r.status();
        let v: Value = r.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            anyhow::bail!("Mastra companion {path} returned {status}: {}", v["error"].as_str().unwrap_or("error"));
        }
        Ok(v)
    }

    pub async fn lead_chat(&self, inheritance_id: &str, message: &str, history: &[(String, String)]) -> anyhow::Result<LeadReply> {
        let history: Vec<Value> = history.iter().map(|(role, content)| json!({"role": if role == "lead" { "assistant" } else { "user" }, "content": content})).collect();
        let v = self.post("/lead/chat", json!({"inheritanceId": inheritance_id, "message": message, "history": history})).await?;
        Ok(serde_json::from_value(v)?)
    }

    pub async fn triage(&self, inheritance_id: &str, max_issues: u32) -> anyhow::Result<Value> {
        let v = self.post("/triage", json!({"inheritanceId": inheritance_id, "maxIssues": max_issues})).await?;
        validate_triage(&v)?;
        Ok(v)
    }
}

/// Shape check on the companion's triage output before the engine stores it.
pub fn validate_triage(v: &Value) -> anyhow::Result<()> {
    let triaged = v["triaged"].as_array().ok_or_else(|| anyhow::anyhow!("triage result has no 'triaged' list"))?;
    for t in triaged {
        let verdict = t["verdict"].as_str().unwrap_or("");
        if !matches!(verdict, "addressed_by_successor" | "still_relevant" | "obsolete" | "needs_human") {
            anyhow::bail!("unknown triage verdict '{verdict}'");
        }
        if t["number"].as_u64().is_none() || t["engine_check"].as_str().is_none() {
            anyhow::bail!("triage entry is missing its issue number or engine check");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_loopback_companions() {
        assert!(MastraClient::new("http://127.0.0.1:7880").is_ok());
        assert!(MastraClient::new("http://localhost:7880/").is_ok());
        assert!(MastraClient::new("https://mastra.example.com").is_err());
        assert!(MastraClient::new("http://127.0.0.1.evil.com:7880").is_err());
    }

    #[test]
    fn lead_reply_filters_actions_and_parses_usage() {
        let v = json!({"reply": "r", "proposed_action": "rm -rf /", "tool_calls": [{"tool": "get-project-overview", "args": {}}], "mode": "grounded", "usage": {"input_tokens": 5535, "output_tokens": 268}});
        let r: LeadReply = serde_json::from_value(v).unwrap();
        assert_eq!(r.action(), None);
        assert_eq!(r.tool_names(), vec!["get-project-overview"]);
        assert_eq!(r.usage, Some(Usage { input_tokens: Some(5535), output_tokens: Some(268) }));
        let ok: LeadReply = serde_json::from_value(json!({"reply": "r", "proposed_action": "verify"})).unwrap();
        assert_eq!(ok.action().as_deref(), Some("verify"));
    }

    #[test]
    fn validates_triage_shape() {
        let good = json!({"triaged": [{"number": 91, "verdict": "addressed_by_successor", "engine_check": "verified: cites rep_1"}]});
        assert!(validate_triage(&good).is_ok());
        assert!(validate_triage(&json!({"triaged": [{"number": 1, "verdict": "fixed", "engine_check": "x"}]})).is_err());
        assert!(validate_triage(&json!({"nope": 1})).is_err());
    }

    #[tokio::test]
    async fn unreachable_companion_is_an_error_not_a_hang() {
        let c = MastraClient::new("http://127.0.0.1:9").unwrap();
        assert!(c.health().await.is_err());
        assert!(c.lead_chat("inh_x", "hi", &[]).await.is_err());
    }
}
