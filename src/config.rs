//! Runtime configuration, read from environment variables (and an optional `.env` file).
//!
//! Nothing here is a secret except `GITHUB_TOKEN` and `IE_MODEL_API_KEY`; both are kept in memory only,
//! never logged, never written to the database, and never passed to inherited code.

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyMode {
    /// Only models reachable on a loopback address may receive repository content.
    LocalOnly,
    /// Remote models may be used only when `IE_ALLOW_CLOUD=1` is set explicitly.
    CloudWithPermission,
    /// Remote models are allowed.
    CloudFirst,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    /// Any server implementing `POST /chat/completions` (LM Studio, llama.cpp server, vLLM, hosted APIs).
    OpenAiCompatible,
    /// Ollama's native `/api/chat`.
    Ollama,
}

#[derive(Clone, Debug)]
pub struct ModelConfig {
    pub provider: ProviderKind,
    pub endpoint: String,
    pub model: String,
    pub api_key: Option<String>,
    pub temperature: f32,
    pub context_tokens: u32,
    pub timeout_secs: u64,
    /// For runtimes/models with a reasoning toggle (Ollama `think`); None leaves the runtime default.
    pub think: Option<bool>,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub bind: String,
    pub data_dir: PathBuf,
    pub web_dir: PathBuf,
    pub model: ModelConfig,
    /// Per-agent model overrides, e.g. `IE_MODEL_REVIEWER=google/gemma-4-12b`.
    pub model_overrides: Vec<(String, String)>,
    pub privacy_mode: PrivacyMode,
    pub allow_cloud: bool,
    pub github_token: Option<String>,
    pub python_image: String,
    pub node_image: String,
    pub run_timeout_secs: u64,
    pub max_repo_kb: u64,
    /// Character budget for repository content inside a single model prompt.
    pub prompt_budget_chars: usize,
    /// Optional Mastra companion (loopback) running the Lead agent and the triage workflow.
    pub mastra_url: Option<String>,
}

fn var(key: &str) -> Option<String> {
    std::env::var(key).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// Minimal `.env` loader: `KEY=VALUE` lines, `#` comments, existing environment wins.
pub fn load_dotenv(path: &std::path::Path) {
    let Ok(text) = std::fs::read_to_string(path) else { return };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let k = k.trim();
            let v = v.trim().trim_matches('"').trim_matches('\'');
            if std::env::var_os(k).is_none() {
                std::env::set_var(k, v);
            }
        }
    }
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let provider = match var("IE_MODEL_PROVIDER").as_deref() {
            Some("ollama") => ProviderKind::Ollama,
            Some("openai") | Some("openai-compatible") | None => ProviderKind::OpenAiCompatible,
            Some(other) => anyhow::bail!("unknown IE_MODEL_PROVIDER '{other}' (use 'openai-compatible' or 'ollama')"),
        };
        let default_endpoint = match provider {
            ProviderKind::OpenAiCompatible => "http://127.0.0.1:1234/v1",
            ProviderKind::Ollama => "http://127.0.0.1:11434",
        };
        let privacy_mode = match var("IE_PRIVACY_MODE").as_deref() {
            None | Some("local_only") => PrivacyMode::LocalOnly,
            Some("cloud_with_permission") => PrivacyMode::CloudWithPermission,
            Some("cloud_first") => PrivacyMode::CloudFirst,
            Some(other) => anyhow::bail!("unknown IE_PRIVACY_MODE '{other}'"),
        };
        let model_overrides = ["LEAD", "HISTORIAN", "RESTORER", "REVIEWER", "TESTER", "HUNTER"]
            .iter()
            .filter_map(|role| var(&format!("IE_MODEL_{role}")).map(|m| (role.to_lowercase(), m)))
            .collect();
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        Ok(Self {
            bind: var("IE_BIND").unwrap_or_else(|| "127.0.0.1:7878".into()),
            data_dir: var("IE_DATA_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("data")),
            web_dir: var("IE_WEB_DIR").map(PathBuf::from).unwrap_or_else(|| manifest_dir.join("web")),
            model: ModelConfig {
                provider,
                endpoint: var("IE_MODEL_ENDPOINT").unwrap_or_else(|| default_endpoint.into()).trim_end_matches('/').to_string(),
                model: var("IE_MODEL").unwrap_or_else(|| "google/gemma-4-e4b".into()),
                api_key: var("IE_MODEL_API_KEY"),
                temperature: var("IE_MODEL_TEMPERATURE").and_then(|v| v.parse().ok()).unwrap_or(0.2),
                context_tokens: var("IE_MODEL_CONTEXT").and_then(|v| v.parse().ok()).unwrap_or(16384),
                timeout_secs: var("IE_MODEL_TIMEOUT_SECS").and_then(|v| v.parse().ok()).unwrap_or(600),
                think: var("IE_MODEL_THINK").map(|v| v == "1" || v == "true"),
            },
            model_overrides,
            privacy_mode,
            allow_cloud: var("IE_ALLOW_CLOUD").as_deref() == Some("1"),
            github_token: var("GITHUB_TOKEN"),
            python_image: var("IE_PYTHON_IMAGE").unwrap_or_else(|| "python:3.12-slim".into()),
            node_image: var("IE_NODE_IMAGE").unwrap_or_else(|| "node:22-slim".into()),
            run_timeout_secs: var("IE_RUN_TIMEOUT_SECS").and_then(|v| v.parse().ok()).unwrap_or(600),
            max_repo_kb: var("IE_MAX_REPO_KB").and_then(|v| v.parse().ok()).unwrap_or(150_000),
            prompt_budget_chars: var("IE_PROMPT_BUDGET_CHARS").and_then(|v| v.parse().ok()).unwrap_or(28_000),
            mastra_url: var("IE_MASTRA_URL"),
        })
    }

    pub fn model_for(&self, role: &str) -> String {
        self.model_overrides
            .iter()
            .find(|(r, _)| r == role)
            .map(|(_, m)| m.clone())
            .unwrap_or_else(|| self.model.model.clone())
    }
}
