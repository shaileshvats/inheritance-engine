//! Typed data contracts shared by the engine and the HTTP API (see docs/api/README.md).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Discovered,
    Inherited,
    Investigating,
    Understanding,
    Restoring,
    Verified,
    Evolving,
}

impl Stage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Stage::Discovered => "discovered",
            Stage::Inherited => "inherited",
            Stage::Investigating => "investigating",
            Stage::Understanding => "understanding",
            Stage::Restoring => "restoring",
            Stage::Verified => "verified",
            Stage::Evolving => "evolving",
        }
    }
    pub fn parse(s: &str) -> Stage {
        serde_json::from_value(serde_json::Value::String(s.to_string())).unwrap_or(Stage::Inherited)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecMode {
    Docker,
    Native,
    None,
}

impl ExecMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ExecMode::Docker => "docker",
            ExecMode::Native => "native",
            ExecMode::None => "none",
        }
    }
    pub fn parse(s: &str) -> Option<ExecMode> {
        match s {
            "docker" => Some(ExecMode::Docker),
            "native" => Some(ExecMode::Native),
            "none" => Some(ExecMode::None),
            _ => None,
        }
    }
}

// ---------- Hunter ----------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Signal {
    pub kind: String,
    pub label: String,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    pub full_name: String,
    pub owner: String,
    pub name: String,
    pub html_url: String,
    pub description: Option<String>,
    pub language: Option<String>,
    pub topics: Vec<String>,
    pub stars: u64,
    pub forks: u64,
    pub open_issues: u64,
    pub created_at: String,
    pub pushed_at: String,
    pub archived: bool,
    pub license: Option<String>,
    pub size_kb: u64,
    pub default_branch: String,
    pub age_years: Option<f64>,
    pub idle_years: Option<f64>,
    pub signals: Vec<Signal>,
}

// ---------- Inventory ----------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TreeNode {
    pub name: String,
    pub path: String,
    pub kind: String,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<TreeNode>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LanguageStat {
    pub language: String,
    pub files: usize,
    pub lines: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub path: String,
    pub kind: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EntryPoint {
    pub path: String,
    pub kind: String,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TestInfo {
    pub framework: String,
    pub files: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MotherCandidate {
    pub path: String,
    pub score: f64,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub path: String,
    pub lang: String,
    pub fan_in: usize,
    pub fan_out: usize,
    pub entry: bool,
    pub test: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Graph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthorStat {
    pub name: String,
    pub commits: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommitInfo {
    pub sha: String,
    pub date: String,
    pub author: String,
    pub subject: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct GitSummary {
    pub commits: usize,
    pub first_commit_at: Option<String>,
    pub last_commit_at: Option<String>,
    pub authors: Vec<AuthorStat>,
    pub recent: Vec<CommitInfo>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DecayHit {
    pub path: String,
    pub line: usize,
    pub pattern: String,
    pub detail: String,
    pub removed_in: String,
    /// The offending source line (trimmed); this is real file content and doubles as verified evidence.
    pub text: String,
    /// The exact text the rule matched, e.g. `collections.Mapping`.
    #[serde(default)]
    pub matched: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Inventory {
    pub files_total: usize,
    pub bytes_total: u64,
    pub languages: Vec<LanguageStat>,
    pub tree: TreeNode,
    pub manifests: Vec<Manifest>,
    pub entry_points: Vec<EntryPoint>,
    pub tests: TestInfo,
    pub mother_candidates: Vec<MotherCandidate>,
    pub graph: Graph,
    pub git: GitSummary,
    pub decay: Vec<DecayHit>,
    pub readme_path: Option<String>,
}

// ---------- Trust passport ----------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PassportSignal {
    pub path: String,
    pub line: usize,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SecretHit {
    pub path: String,
    pub line: usize,
    pub kind: String,
    pub redacted: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dependency {
    pub ecosystem: String,
    pub name: String,
    pub spec: String,
    pub source_path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Provenance {
    pub source_url: String,
    pub original_commit: String,
    pub cloned_at: String,
    pub license: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Passport {
    pub provenance: Provenance,
    pub install_scripts: Vec<PassportSignal>,
    pub network_access: Vec<PassportSignal>,
    pub filesystem_access: Vec<PassportSignal>,
    pub process_execution: Vec<PassportSignal>,
    pub dynamic_code: Vec<PassportSignal>,
    pub secrets: Vec<SecretHit>,
    pub dependencies: Vec<Dependency>,
    pub unresolved: Vec<String>,
}

// ---------- Evidence, report ----------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Evidence {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    #[serde(default)]
    pub quote: String,
    #[serde(default = "unchecked")]
    pub check: String,
}

fn unchecked() -> String {
    "unchecked".into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Claim {
    pub id: String,
    pub text: String,
    pub status: String,
    pub agent: String,
    pub evidence: Vec<Evidence>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Component {
    pub path: String,
    pub role: String,
    pub status: String,
    pub evidence: Vec<Evidence>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Challenge {
    pub target: String,
    pub verdict: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub lead_summary: String,
    pub purpose: Claim,
    pub capabilities: Vec<Claim>,
    pub components: Vec<Component>,
    pub data_flows: Vec<Claim>,
    pub external_services: Vec<Claim>,
    pub build_instructions: String,
    pub unknowns: Vec<String>,
    pub reviewer_challenges: Vec<Challenge>,
    pub next_actions: Vec<String>,
    pub generated_at: String,
}

// ---------- Restoration ----------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Issue {
    pub id: String,
    pub title: String,
    pub category: String,
    pub severity: String,
    pub diagnosis: String,
    pub evidence: Vec<Evidence>,
    pub proposed_fix: String,
    pub confidence: String,
    pub source: String,
    pub status: String,
    /// Decay pattern this issue groups, when it came from the static decay scan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    /// Set when the inheritor reopened the issue with their own finding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reopened_note: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Review {
    pub verdict: String,
    #[serde(default)]
    pub problem_is_real: Option<bool>,
    #[serde(default)]
    pub evidence_supports_diagnosis: Option<bool>,
    #[serde(default)]
    pub preserves_original_behaviour: Option<bool>,
    #[serde(default)]
    pub tests_exercise_change: Option<bool>,
    pub root_cause_addressed: bool,
    pub regression_risk: String,
    pub concerns: Vec<String>,
    pub call_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProbeResult {
    pub name: String,
    pub code: String,
    pub original_output: String,
    pub successor_output: String,
    /// preserved | changed | inconclusive
    pub verdict: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProbeReport {
    pub tester_call_id: Option<String>,
    pub legacy_image: String,
    pub modern_image: String,
    pub probes: Vec<ProbeResult>,
    /// preserved | changed | inconclusive
    pub verdict: String,
    pub note: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Repair {
    pub id: String,
    pub issue_id: String,
    pub status: String,
    pub rationale: String,
    pub diff: String,
    pub files: Vec<String>,
    pub commit: Option<String>,
    pub review: Option<Review>,
    pub verification_before: Option<String>,
    pub verification_after: Option<String>,
    pub note: Option<String>,
    pub created_at: String,
    #[serde(default)]
    pub attempt: u32,
    #[serde(default)]
    pub probe: Option<ProbeReport>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct TestSummary {
    pub passed: u32,
    pub failed: u32,
    pub errors: u32,
    pub skipped: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Verification {
    pub id: String,
    pub label: String,
    pub mode: String,
    pub environment: String,
    pub command: String,
    pub exit_code: Option<i32>,
    pub status: String,
    pub summary: Option<TestSummary>,
    pub output_tail: String,
    pub duration_ms: u64,
    pub started_at: String,
    pub workspace_rev: Option<String>,
    pub network: String,
    pub reason: Option<String>,
}

// ---------- Jobs ----------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobStep {
    pub name: String,
    pub status: String,
    pub detail: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub inheritance_id: String,
    pub kind: String,
    pub status: String,
    pub step: Option<String>,
    pub steps: Vec<JobStep>,
    pub params: serde_json::Value,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

// ---------- Inheritance ----------

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct GithubMeta {
    pub description: Option<String>,
    pub stars: u64,
    pub forks: u64,
    pub pushed_at: Option<String>,
    pub created_at: Option<String>,
    pub default_branch: String,
    pub owner: String,
    pub size_kb: u64,
    pub archived: bool,
    pub clone_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Integrity {
    pub checked_at: String,
    pub ok: bool,
    pub head: Option<String>,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InheritanceRow {
    pub id: String,
    pub full_name: String,
    pub name: String,
    pub source_url: String,
    pub license: Option<String>,
    pub stage: String,
    pub execution_mode: String,
    pub original_commit: Option<String>,
    pub original_branch: Option<String>,
    pub successor_name: String,
    pub github: GithubMeta,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub at: String,
    pub call_id: Option<String>,
    pub suggested_action: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentCall {
    pub id: String,
    pub inheritance_id: Option<String>,
    pub parent_id: Option<String>,
    pub agent: String,
    pub purpose: String,
    pub provider: String,
    pub model: String,
    pub started_at: String,
    pub duration_ms: u64,
    pub input_files: Vec<String>,
    pub prompt_chars: usize,
    pub response_chars: usize,
    pub ok: bool,
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<String>,
}
