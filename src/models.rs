//! Provider-independent model adapters. Two transports cover most open-weight runtimes:
//! OpenAI-compatible `/chat/completions` (LM Studio, llama.cpp server, vLLM, hosted gateways) and
//! Ollama's native `/api/chat`. Every call passes through the privacy policy check first.

use std::time::Duration;

use serde_json::{json, Value};

use crate::config::{ModelConfig, PrivacyMode, ProviderKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Locality {
    Local,
    Remote,
}

#[derive(Clone)]
pub struct ModelClient {
    pub cfg: ModelConfig,
    pub privacy: PrivacyMode,
    pub allow_cloud: bool,
    http: reqwest::Client,
}

pub fn locality_of(endpoint: &str) -> Locality {
    let host = endpoint
        .split("://")
        .nth(1)
        .unwrap_or(endpoint)
        .split('/')
        .next()
        .unwrap_or("");
    let host = if host.starts_with('[') { host.split(']').next().unwrap_or("").trim_start_matches('[') } else { host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host) };
    match host {
        "127.0.0.1" | "localhost" | "::1" => Locality::Local,
        _ => Locality::Remote,
    }
}

/// Strip code fences and surrounding prose, then parse JSON.
pub fn parse_json_loose(s: &str) -> anyhow::Result<Value> {
    let t = s.trim();
    let t = t.strip_prefix("```json").or_else(|| t.strip_prefix("```")).unwrap_or(t);
    let t = t.strip_suffix("```").unwrap_or(t).trim();
    if let Ok(v) = serde_json::from_str(t) {
        return Ok(v);
    }
    if let (Some(a), Some(b)) = (t.find('{'), t.rfind('}')) {
        if a < b {
            if let Ok(v) = serde_json::from_str(&t[a..=b]) {
                return Ok(v);
            }
        }
    }
    anyhow::bail!("model did not return valid JSON")
}

impl ModelClient {
    pub fn new(cfg: ModelConfig, privacy: PrivacyMode, allow_cloud: bool) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder().timeout(Duration::from_secs(cfg.timeout_secs)).build()?;
        Ok(Self { cfg, privacy, allow_cloud, http })
    }

    pub fn locality(&self) -> Locality {
        locality_of(&self.cfg.endpoint)
    }

    pub fn provider_name(&self) -> &'static str {
        match self.cfg.provider {
            ProviderKind::OpenAiCompatible => "openai-compatible",
            ProviderKind::Ollama => "ollama",
        }
    }

    /// Refuse to send repository content to a remote provider unless the user's policy allows it.
    pub fn check_policy(&self) -> anyhow::Result<()> {
        if self.locality() == Locality::Local {
            return Ok(());
        }
        match self.privacy {
            PrivacyMode::LocalOnly => anyhow::bail!(
                "blocked by privacy policy: endpoint {} is not local and IE_PRIVACY_MODE=local_only",
                self.cfg.endpoint
            ),
            PrivacyMode::CloudWithPermission if !self.allow_cloud => {
                anyhow::bail!("blocked by privacy policy: remote endpoint requires IE_ALLOW_CLOUD=1 under cloud_with_permission")
            }
            _ => Ok(()),
        }
    }

    pub async fn health(&self) -> (bool, Option<String>) {
        let url = match self.cfg.provider {
            ProviderKind::OpenAiCompatible => format!("{}/models", self.cfg.endpoint),
            ProviderKind::Ollama => format!("{}/api/tags", self.cfg.endpoint),
        };
        let mut req = self.http.get(&url).timeout(Duration::from_secs(4));
        if let Some(k) = &self.cfg.api_key {
            req = req.bearer_auth(k);
        }
        match req.send().await {
            Ok(r) if r.status().is_success() => {
                let v: Value = r.json().await.unwrap_or(Value::Null);
                let names: Vec<String> = match self.cfg.provider {
                    ProviderKind::OpenAiCompatible => v["data"].as_array().map(|a| a.iter().filter_map(|m| m["id"].as_str().map(String::from)).collect()).unwrap_or_default(),
                    ProviderKind::Ollama => v["models"].as_array().map(|a| a.iter().filter_map(|m| m["name"].as_str().map(String::from)).collect()).unwrap_or_default(),
                };
                if names.iter().any(|n| n == &self.cfg.model) {
                    (true, None)
                } else {
                    (false, Some(format!("model '{}' is not available at {} (found: {})", self.cfg.model, self.cfg.endpoint, names.join(", "))))
                }
            }
            Ok(r) => (false, Some(format!("{url} returned {}", r.status()))),
            Err(e) => (false, Some(format!("cannot reach {url}: {e}"))),
        }
    }

    /// One chat completion. With `schema`, the runtime constrains output to that JSON schema.
    pub async fn complete(&self, model: &str, system: &str, user: &str, schema: Option<&Value>, max_tokens: u32) -> anyhow::Result<String> {
        self.check_policy()?;
        match self.cfg.provider {
            ProviderKind::OpenAiCompatible => {
                let mut body = json!({
                    "model": model,
                    "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
                    "temperature": self.cfg.temperature,
                    "max_tokens": max_tokens,
                    "stream": false,
                });
                if let Some(s) = schema {
                    body["response_format"] = json!({"type": "json_schema", "json_schema": {"name": "response", "strict": true, "schema": s}});
                }
                let mut req = self.http.post(format!("{}/chat/completions", self.cfg.endpoint)).json(&body);
                if let Some(k) = &self.cfg.api_key {
                    req = req.bearer_auth(k);
                }
                let resp = req.send().await?;
                let status = resp.status();
                let v: Value = resp.json().await?;
                if !status.is_success() {
                    anyhow::bail!("model endpoint returned {status}: {}", v["error"].to_string().chars().take(400).collect::<String>());
                }
                let content = v["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string();
                if v["choices"][0]["finish_reason"] == "length" {
                    anyhow::bail!("output truncated at max_tokens={max_tokens} ({} chars received)", content.len());
                }
                if content.trim().is_empty() {
                    anyhow::bail!("model returned an empty message (finish_reason: {})", v["choices"][0]["finish_reason"]);
                }
                Ok(content)
            }
            ProviderKind::Ollama => {
                let mut body = json!({
                    "model": model,
                    "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
                    "stream": false,
                    "options": {"temperature": self.cfg.temperature, "num_ctx": self.cfg.context_tokens, "num_predict": max_tokens},
                });
                if let Some(s) = schema {
                    body["format"] = s.clone();
                }
                if let Some(t) = self.cfg.think {
                    body["think"] = json!(t);
                }
                let resp = self.http.post(format!("{}/api/chat", self.cfg.endpoint)).json(&body).send().await?;
                let status = resp.status();
                let v: Value = resp.json().await?;
                if !status.is_success() {
                    anyhow::bail!("ollama returned {status}: {}", v["error"]);
                }
                Ok(v["message"]["content"].as_str().unwrap_or("").to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locality_detection() {
        assert_eq!(locality_of("http://127.0.0.1:1234/v1"), Locality::Local);
        assert_eq!(locality_of("http://localhost:11434"), Locality::Local);
        assert_eq!(locality_of("http://[::1]:8080/v1"), Locality::Local);
        assert_eq!(locality_of("https://api.example.com/v1"), Locality::Remote);
        assert_eq!(locality_of("http://127.0.0.1.evil.com/v1"), Locality::Remote);
    }

    #[test]
    fn local_only_blocks_remote() {
        let cfg = ModelConfig {
            provider: ProviderKind::OpenAiCompatible,
            endpoint: "https://api.example.com/v1".into(),
            model: "m".into(),
            api_key: None,
            temperature: 0.2,
            context_tokens: 4096,
            timeout_secs: 5,
            think: None,
        };
        let c = ModelClient::new(cfg.clone(), PrivacyMode::LocalOnly, false).unwrap();
        assert!(c.check_policy().is_err());
        let c = ModelClient::new(cfg.clone(), PrivacyMode::CloudWithPermission, false).unwrap();
        assert!(c.check_policy().is_err());
        let c = ModelClient::new(cfg, PrivacyMode::CloudWithPermission, true).unwrap();
        assert!(c.check_policy().is_ok());
    }

    #[test]
    fn loose_json() {
        assert_eq!(parse_json_loose("```json\n{\"a\":1}\n```").unwrap()["a"], 1);
        assert_eq!(parse_json_loose("Sure! {\"a\":2} hope it helps").unwrap()["a"], 2);
        assert!(parse_json_loose("nope").is_err());
    }
}
