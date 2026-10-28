//! Optional Sentry agent tracing (enabled only when `SENTRY_DSN` is set).
//!
//! Trace shape: one transaction per job (`gen_ai.invoke_agent`, the agent team working on an inheritance),
//! a `gen_ai.chat` span per model call (role, model, latency, tokens, outcome), and a `gen_ai.execute_tool`
//! span per sandbox run (install / syntax / tests / behaviour probe).
//!
//! Privacy: spans carry metadata only. Purposes are reduced to fixed categories (issue titles can quote code),
//! errors to fixed classes, and a `before_send_transaction` allowlist removes any other data key. Prompts,
//! responses, source code, file paths, test output and secrets are never sent.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use sentry::protocol::{SpanId, SpanStatus, Value};
use serde_json::json;

use crate::contracts::{AgentCall, Job};

static ENABLED: AtomicBool = AtomicBool::new(false);

fn jobs() -> &'static Mutex<HashMap<String, sentry::Transaction>> {
    static J: OnceLock<Mutex<HashMap<String, sentry::Transaction>>> = OnceLock::new();
    J.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn allowed_key(k: &str) -> bool {
    const EXACT: &[&str] = &[
        "gen_ai.operation.name", "gen_ai.system", "gen_ai.request.model", "gen_ai.response.model", "gen_ai.agent.name",
        "gen_ai.tool.name", "gen_ai.pipeline.name",
    ];
    EXACT.contains(&k) || k.starts_with("gen_ai.usage.") || k.starts_with("ie.") || k.starts_with("sentry.") || k == "thread.id" || k == "thread.name"
}

/// Allowlist every data map in a transaction and drop free-form fields.
pub fn scrub_transaction(mut tx: sentry::protocol::Transaction<'static>) -> sentry::protocol::Transaction<'static> {
    for span in tx.spans.iter_mut() {
        span.data.retain(|k, _| allowed_key(k));
    }
    if let Some(sentry::protocol::Context::Trace(trace)) = tx.contexts.get_mut("trace") {
        trace.data.retain(|k, _| allowed_key(k));
    }
    tx.extra.clear();
    tx.request = None;
    tx.user = None;
    tx
}

/// Transport wrapper: every transaction leaving the process passes through `scrub_transaction`.
/// (The Rust SDK has no `before_send_transaction` hook, so the allowlist is enforced here.)
pub struct ScrubbingTransport(pub std::sync::Arc<dyn sentry::Transport>);

impl sentry::Transport for ScrubbingTransport {
    fn send_envelope(&self, envelope: sentry::Envelope) {
        let mut out = sentry::Envelope::new();
        for item in envelope.items() {
            match item {
                sentry::protocol::EnvelopeItem::Transaction(t) => {
                    let clean = scrub_transaction((**t).clone());
                    audit("transaction", &clean);
                    out.add_item(clean)
                }
                other => out.add_item(other.clone()),
            }
        }
        self.0.send_envelope(out);
    }
    fn flush(&self, timeout: Duration) -> bool {
        self.0.flush(timeout)
    }
    fn shutdown(&self, timeout: Duration) -> bool {
        self.0.shutdown(timeout)
    }
}

/// Optional local audit (`IE_TELEMETRY_AUDIT=path`): append exactly what is handed to the transport, as JSON lines.
/// The envelope header (which carries the DSN) is never written.
fn audit(kind: &str, payload: &impl serde::Serialize) {
    let Ok(path) = std::env::var("IE_TELEMETRY_AUDIT") else { return };
    let line = json!({"process": "rust-engine", "kind": kind, "at": crate::util::now(), "payload": payload});
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        use std::io::Write;
        // One write per record so concurrent writers (engine + companion) cannot interleave inside a line.
        let _ = f.write_all(format!("{line}\n").as_bytes());
    }
}

struct ScrubbingFactory;

impl sentry::TransportFactory for ScrubbingFactory {
    fn create_transport_with_options(&self, options: sentry::TransportOptions) -> std::sync::Arc<dyn sentry::Transport> {
        let inner = sentry::transports::DefaultTransportFactory.create_transport_with_options(options);
        std::sync::Arc::new(ScrubbingTransport(inner))
    }
}

/// Initialise Sentry from `SENTRY_DSN`. Returns the guard that flushes on drop.
pub fn init() -> Option<sentry::ClientInitGuard> {
    let dsn = std::env::var("SENTRY_DSN").ok().filter(|d| !d.trim().is_empty())?;
    let mut options = sentry::ClientOptions::default().traces_sample_rate(1.0);
    options.dsn = dsn.parse().ok();
    options.release = Some(concat!("inheritance-engine@", env!("CARGO_PKG_VERSION")).into());
    options.environment = Some(std::env::var("SENTRY_ENVIRONMENT").unwrap_or_else(|_| "local".into()).into());
    options.send_default_pii = false;
    options.transport = Some(std::sync::Arc::new(ScrubbingFactory));
    let guard = sentry::init(options);
    ENABLED.store(guard.is_enabled(), Ordering::Relaxed);
    Some(guard)
}

pub fn status() -> serde_json::Value {
    json!({"enabled": enabled(), "content_sent": false})
}

/// Fixed categories instead of free text: purposes can contain issue titles that quote repository code.
pub fn purpose_kind(purpose: &str) -> &'static str {
    const KINDS: &[(&str, &str)] = &[
        ("Initial inspection", "plan"), ("Reconstruct", "historian"), ("Diagnose", "diagnose"), ("Re-diagnose", "rediagnose"),
        ("Challenge", "challenge"), ("Synthesize", "synthesis"), ("Patch", "patch"), ("Review repair", "review"),
        ("Behaviour probes", "probe"), ("Answer the user", "chat"), ("Translate problem", "hunt"), ("Explain why", "explain"),
        ("Triage", "triage"),
    ];
    KINDS.iter().find(|(p, _)| purpose.starts_with(p)).map(|(_, k)| *k).unwrap_or("other")
}

/// Fixed error classes instead of messages (messages can echo model or repository text).
pub fn error_class(error: &str) -> &'static str {
    let e = error.to_ascii_lowercase();
    if e.contains("truncated") {
        "truncated_output"
    } else if e.contains("valid json") {
        "invalid_json"
    } else if e.contains("privacy policy") {
        "blocked_by_privacy_policy"
    } else if e.contains("timed out") || e.contains("timeout") {
        "timeout"
    } else if e.contains("returned 4") || e.contains("returned 5") || e.contains("connect") {
        "endpoint_error"
    } else {
        "other"
    }
}

fn ts(rfc3339: &str) -> SystemTime {
    chrono::DateTime::parse_from_rfc3339(rfc3339).map(SystemTime::from).unwrap_or_else(|_| SystemTime::now())
}

pub fn job_started(job: &Job, repository: &str) {
    if !enabled() {
        return;
    }
    let ctx = sentry::TransactionContext::new(&format!("inheritance.{}", job.kind), "gen_ai.invoke_agent");
    let tx = sentry::start_transaction(ctx);
    tx.set_data("gen_ai.operation.name", Value::from("invoke_agent"));
    tx.set_data("gen_ai.agent.name", Value::from("Inheritance Engine team"));
    tx.set_data("ie.job_kind", Value::from(job.kind.clone()));
    tx.set_data("ie.inheritance_id", Value::from(job.inheritance_id.clone()));
    tx.set_data("ie.repository", Value::from(repository.to_string()));
    jobs().lock().unwrap().insert(job.inheritance_id.clone(), tx);
}

pub fn job_finished(inheritance_id: &str, status: &str) {
    if let Some(tx) = jobs().lock().unwrap().remove(inheritance_id) {
        tx.set_status(match status {
            "completed" => SpanStatus::Ok,
            "cancelled" => SpanStatus::Cancelled,
            _ => SpanStatus::InternalError,
        });
        tx.set_data("ie.job_status", Value::from(status.to_string()));
        tx.finish();
    }
}

fn span_under_job(inheritance_id: Option<&str>, op: &str, description: &str, start: SystemTime) -> Option<sentry::TransactionOrSpan> {
    let map = jobs().lock().unwrap();
    match inheritance_id.and_then(|id| map.get(id)) {
        Some(tx) => Some(sentry::TransactionOrSpan::Span(tx.start_child_with_details(op, description, SpanId::default(), start))),
        None => {
            drop(map);
            let ctx = sentry::TransactionContext::new(description, op);
            Some(sentry::TransactionOrSpan::Transaction(sentry::start_transaction_with_timestamp(ctx, start)))
        }
    }
}

/// One model call (engine agent or Mastra companion) as a `gen_ai.chat` span.
pub fn record_call(call: &AgentCall) {
    if !enabled() {
        return;
    }
    let start = ts(&call.started_at);
    let kind = purpose_kind(&call.purpose);
    let Some(span) = span_under_job(call.inheritance_id.as_deref(), "gen_ai.chat", &format!("chat {} ({kind})", call.model), start) else { return };
    span.set_data("gen_ai.operation.name", Value::from("chat"));
    span.set_data("gen_ai.system", Value::from(call.provider.clone()));
    span.set_data("gen_ai.request.model", Value::from(call.model.clone()));
    span.set_data("gen_ai.agent.name", Value::from(call.agent.clone()));
    span.set_data("ie.purpose", Value::from(kind));
    span.set_data("ie.input_files", Value::from(call.input_files.len() as u64));
    if let Some(i) = call.input_tokens {
        span.set_data("gen_ai.usage.input_tokens", Value::from(i));
    }
    if let Some(o) = call.output_tokens {
        span.set_data("gen_ai.usage.output_tokens", Value::from(o));
    }
    if let (Some(i), Some(o)) = (call.input_tokens, call.output_tokens) {
        span.set_data("gen_ai.usage.total_tokens", Value::from(i + o));
    }
    if call.ok {
        span.set_status(SpanStatus::Ok);
    } else {
        span.set_status(SpanStatus::InternalError);
        span.set_data("ie.error_class", Value::from(error_class(call.error.as_deref().unwrap_or(""))));
    }
    span.finish_with_timestamp(start + Duration::from_millis(call.duration_ms));
}

/// One sandbox execution as a `gen_ai.execute_tool` span. `label` must be a fixed tool name, not free text.
pub fn record_tool(inheritance_id: &str, tool: &'static str, mode: &str, status: &str, duration_ms: u64) {
    if !enabled() {
        return;
    }
    let end = SystemTime::now();
    let start = end - Duration::from_millis(duration_ms);
    let Some(span) = span_under_job(Some(inheritance_id), "gen_ai.execute_tool", &format!("execute_tool {tool}"), start) else { return };
    span.set_data("gen_ai.operation.name", Value::from("execute_tool"));
    span.set_data("gen_ai.tool.name", Value::from(tool));
    span.set_data("ie.execution_mode", Value::from(mode.to_string()));
    span.set_data("ie.result", Value::from(status.to_string()));
    span.set_status(if matches!(status, "passed") { SpanStatus::Ok } else if status == "not_run" { SpanStatus::UnknownError } else { SpanStatus::InternalError });
    span.finish_with_timestamp(end);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_never_carry_free_text() {
        assert_eq!(purpose_kind("Patch: ImportError: cannot import name 'getargspec'"), "patch");
        assert_eq!(purpose_kind("Review repair: def curried(func):"), "review");
        assert_eq!(purpose_kind("something new"), "other");
        assert_eq!(error_class("model endpoint returned 400: {\"error\":\"def secret()\"}"), "endpoint_error");
        assert_eq!(error_class("output truncated at max_tokens=4000"), "truncated_output");
    }

    #[test]
    fn allowlist() {
        for k in ["gen_ai.usage.input_tokens", "gen_ai.request.model", "gen_ai.agent.name", "ie.purpose"] {
            assert!(allowed_key(k), "{k}");
        }
        for k in ["input", "output", "gen_ai.agent.prompt", "gen_ai.response.text", "gen_ai.request.messages", "db.statement"] {
            assert!(!allowed_key(k), "{k}");
        }
    }

    #[test]
    fn traces_carry_metadata_but_no_content() {
        let source = "def curried(func):\n    spec = getargspec(f)  # AKIAABCDEFGHIJKLMNOP";
        let test_options = sentry::ClientOptions::default().traces_sample_rate(1.0);
        let envelopes = sentry::test::with_captured_envelopes_options(
            || {
                ENABLED.store(true, Ordering::Relaxed);
                let job = Job {
                    id: "job_t".into(), inheritance_id: "inh_test0001".into(), kind: "restore".into(), status: "running".into(),
                    step: None, steps: vec![], params: json!({}), error: None, created_at: crate::util::now(), updated_at: crate::util::now(),
                };
                job_started(&job, "kachayev/fn.py");
                let call = AgentCall {
                    id: "call_t".into(), inheritance_id: Some("inh_test0001".into()), parent_id: None, agent: "restorer".into(),
                    purpose: format!("Patch: {source}"), provider: "openai-compatible".into(), model: "google/gemma-4-e4b".into(),
                    started_at: crate::util::now(), duration_ms: 1234, input_files: vec!["fn/func.py".into()], prompt_chars: 10, response_chars: 10,
                    ok: false, error: Some(format!("model endpoint returned 400: {source}")), input_tokens: Some(5535), output_tokens: Some(268),
                    system: Some(source.into()), prompt: Some(source.into()), response: Some(source.into()),
                };
                record_call(&call);
                record_tool("inh_test0001", "sandbox.tests", "docker", "passed", 900);
                job_finished("inh_test0001", "completed");
                ENABLED.store(false, Ordering::Relaxed);
            },
            test_options,
        );
        assert_eq!(envelopes.len(), 1);
        let mut body = Vec::new();
        envelopes[0].to_writer(&mut body).unwrap();
        let text = String::from_utf8_lossy(&body);
        for forbidden in ["curried", "getargspec", "AKIA", "fn/func.py"] {
            assert!(!text.contains(forbidden), "trace leaked {forbidden}: {text}");
        }
        for expected in ["gen_ai.chat", "gen_ai.execute_tool", "gen_ai.usage.input_tokens", "5535", "google/gemma-4-e4b", "sandbox.tests", "endpoint_error", "inheritance.restore"] {
            assert!(text.contains(expected), "trace missing {expected}");
        }
    }

    #[test]
    fn scrubbing_transport_strips_injected_content() {
        use sentry::Transport;
        let source = "def curried(func): AKIAABCDEFGHIJKLMNOP";
        let mut span = sentry::protocol::Span::default();
        span.data.insert("gen_ai.usage.input_tokens".into(), Value::from(10u64));
        span.data.insert("gen_ai.response.text".into(), Value::from(source));
        span.data.insert("input".into(), Value::from(source));
        let mut tx = sentry::protocol::Transaction::default();
        tx.spans.push(span);
        tx.extra.insert("file".into(), Value::from(source));
        let mut env = sentry::Envelope::new();
        env.add_item(tx);
        let inner = std::sync::Arc::new(sentry::test::TestTransport::new());
        ScrubbingTransport(inner.clone()).send_envelope(env);
        let sent = inner.fetch_and_clear_envelopes();
        let mut body = Vec::new();
        sent[0].to_writer(&mut body).unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(!text.contains("curried") && !text.contains("AKIA"), "{text}");
        assert!(text.contains("gen_ai.usage.input_tokens"));
    }
}
