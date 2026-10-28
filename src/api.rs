//! HTTP API (see docs/api/README.md). The server binds to loopback by default and rejects
//! cross-origin / non-local Host requests, so a web page in the user's browser cannot drive it.

use std::sync::Arc;

use axum::extract::{Path, Query, Request, State};
use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::agents::{self, CallSpec};
use crate::app::App;
use crate::contracts::*;
use crate::db::Kind;
use crate::gitops;
use crate::hunter;
use crate::pipeline;
use crate::record;
use crate::security::{redact, safe_join};
use crate::util::{new_id, now, truncate};

pub struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error": self.1}))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        let msg = redact(&format!("{e:#}"));
        let code = if msg.contains("not found") {
            StatusCode::NOT_FOUND
        } else if msg.contains("already") {
            StatusCode::CONFLICT
        } else if msg.contains("rate limit") {
            StatusCode::TOO_MANY_REQUESTS
        } else {
            StatusCode::BAD_REQUEST
        };
        ApiError(code, msg)
    }
}

type R<T> = Result<Json<T>, ApiError>;

fn bad(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, msg.into())
}

/// Reject requests whose Host is not local (DNS rebinding) and state-changing requests from other origins (CSRF).
async fn local_only(req: Request, next: Next) -> Result<Response, ApiError> {
    let headers = req.headers();
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("");
    let host_name = host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host);
    if !matches!(host_name, "127.0.0.1" | "localhost" | "[::1]") {
        return Err(ApiError(StatusCode::FORBIDDEN, "requests must target a loopback host".into()));
    }
    if req.method() != Method::GET && req.method() != Method::HEAD {
        if let Some(origin) = headers.get(header::ORIGIN).and_then(|o| o.to_str().ok()) {
            let ok = origin == format!("http://{host}") || origin == format!("https://{host}");
            if !ok {
                return Err(ApiError(StatusCode::FORBIDDEN, "cross-origin request rejected".into()));
            }
        }
    }
    Ok(next.run(req).await)
}

pub fn router(app: Arc<App>) -> Router {
    let api = Router::new()
        .route("/status", get(status))
        .route("/hunt", get(hunt))
        .route("/hunt/explain", post(explain))
        .route("/inheritances", get(list_inheritances).post(create_inheritance))
        .route("/inheritances/{id}", get(detail).delete(delete_inheritance))
        .route("/inheritances/{id}/investigate", post(investigate))
        .route("/inheritances/{id}/restore", post(restore))
        .route("/inheritances/{id}/verify", post(verify))
        .route("/inheritances/{id}/execution-mode", post(set_mode))
        .route("/inheritances/{id}/successor", axum::routing::patch(rename_successor))
        .route("/inheritances/{id}/record", get(get_record))
        .route("/inheritances/{id}/file", get(get_file))
        .route("/inheritances/{id}/diff", get(get_diff))
        .route("/inheritances/{id}/chat", get(get_chat).post(post_chat))
        .route("/inheritances/{id}/calls", get(list_calls))
        .route("/calls/{id}", get(get_call))
        .route("/jobs/{id}/{action}", post(job_action))
        .fallback(|| async { ApiError(StatusCode::NOT_FOUND, "no such API route".into()) });
    let web = tower_http::services::ServeDir::new(&app.cfg.web_dir).append_index_html_on_directories(true);
    Router::new()
        .nest("/api", api)
        .fallback_service(web)
        .layer(axum::middleware::from_fn(local_only))
        .layer(axum::middleware::map_response(|mut r: Response| async move {
            let h = r.headers_mut();
            h.insert("x-content-type-options", "nosniff".parse().unwrap());
            h.insert("x-frame-options", "DENY".parse().unwrap());
            h.insert("referrer-policy", "no-referrer".parse().unwrap());
            // Local tool: always revalidate so a rebuilt UI is never served from a stale cache.
            h.insert("cache-control", "no-cache".parse().unwrap());
            h.insert(
                "content-security-policy",
                "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self'; frame-ancestors 'none'".parse().unwrap(),
            );
            r
        }))
        .with_state(app)
}

// ---------- status ----------

async fn status(State(app): State<Arc<App>>) -> R<Value> {
    let (reachable, detail) = app.agents.client.health().await;
    let docker = crate::sandbox::docker_version().await;
    Ok(Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "model": {
            "provider": app.agents.client.provider_name(),
            "model": app.cfg.model.model,
            "overrides": app.cfg.model_overrides,
            "endpoint": app.cfg.model.endpoint,
            "locality": app.agents.client.locality(),
            "reachable": reachable,
            "detail": detail,
        },
        "docker": {"available": docker.is_ok(), "version": docker.as_ref().ok(), "detail": docker.as_ref().err()},
        "privacy_mode": app.cfg.privacy_mode,
        "github_token": app.cfg.github_token.is_some(),
        "data_dir": app.cfg.data_dir.display().to_string(),
    })))
}

// ---------- hunter ----------

#[derive(Deserialize)]
struct HuntQ {
    q: String,
    mode: Option<String>,
    lang: Option<String>,
}

async fn hunt(State(app): State<Arc<App>>, Query(q): Query<HuntQ>) -> R<Value> {
    let mode = q.mode.as_deref().unwrap_or("problem");
    let mode = if mode == "curiosity" { "curiosity" } else { "problem" };
    let lang = q.lang.as_deref().map(str::trim).filter(|l| !l.is_empty());
    let r = hunter::hunt(&app.gh, &app.agents, &truncate(&q.q, 500), mode, lang).await?;
    Ok(Json(json!({"mode": mode, "query_used": r.query_used, "ai_query": r.ai, "total_count": r.total, "results": r.results})))
}

#[derive(Deserialize)]
struct ExplainBody {
    full_name: String,
}

async fn explain(State(app): State<Arc<App>>, Json(b): Json<ExplainBody>) -> R<Value> {
    let (v, readme_chars, call_id) = hunter::explain(&app.gh, &app.agents, &b.full_name).await?;
    Ok(Json(json!({"full_name": b.full_name, "explanation": v, "readme_chars": readme_chars, "call_id": call_id})))
}

// ---------- inheritances ----------

fn summary(app: &App, row: &InheritanceRow) -> Value {
    let report: Option<Report> = app.db.get_artifact(&row.id, "report").ok().flatten();
    let findings = report.as_ref().map(|r| 1 + r.capabilities.len() + r.data_flows.len() + r.external_services.len()).unwrap_or(0);
    let mut jobs = app.jobs_for(&row.id);
    let active = jobs.iter().rev().find(|j| matches!(j.status.as_str(), "queued" | "running" | "paused")).cloned();
    jobs.clear();
    json!({
        "id": row.id, "name": row.name, "full_name": row.full_name, "source_url": row.source_url,
        "original_commit": row.original_commit, "license": row.license, "stage": row.stage,
        "execution_mode": row.execution_mode, "created_at": row.created_at, "updated_at": row.updated_at,
        "successor_name": row.successor_name, "description": row.github.description,
        "last_verification": pipeline::primary_verification(app, &row.id),
        "active_job": active,
        "counts": {
            "findings": findings,
            "issues": app.db.count(Kind::Issue, &row.id).unwrap_or(0),
            "repairs": app.db.count(Kind::Repair, &row.id).unwrap_or(0),
            "agent_calls": app.db.count(Kind::Call, &row.id).unwrap_or(0),
        },
    })
}

async fn list_inheritances(State(app): State<Arc<App>>) -> R<Vec<Value>> {
    let mut rows: Vec<InheritanceRow> = app.db.list(Kind::Inheritance, None)?;
    rows.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(Json(rows.iter().map(|r| summary(&app, r)).collect()))
}

#[derive(Deserialize)]
struct CreateBody {
    full_name: String,
    execution_mode: String,
}

async fn create_inheritance(State(app): State<Arc<App>>, Json(b): Json<CreateBody>) -> R<Value> {
    let mode = ExecMode::parse(&b.execution_mode).ok_or_else(|| bad("execution_mode must be chosen explicitly: docker, native or none"))?;
    hunter::validate_full_name(&b.full_name)?;
    let (c, meta) = app.gh.repo(&b.full_name).await?;
    let t = now();
    let mut row = InheritanceRow {
        id: new_id("inh"),
        full_name: c.full_name.clone(),
        name: c.name.clone(),
        source_url: c.html_url.clone(),
        license: c.license.clone(),
        stage: Stage::Discovered.as_str().into(),
        execution_mode: mode.as_str().into(),
        original_commit: None,
        original_branch: None,
        successor_name: format!("{}-successor", c.name),
        github: meta,
        created_at: t.clone(),
        updated_at: t,
    };
    app.save_row(&mut row)?;
    app.start_job(&row.id, "investigate", json!({}))?;
    Ok(Json(summary(&app, &row)))
}

async fn detail(State(app): State<Arc<App>>, Path(id): Path<String>) -> R<Value> {
    let row = app.row(&id)?;
    let layout = app.layout(&id);
    let mut v = summary(&app, &row);
    let integrity = match (&row.original_commit, &row.original_branch) {
        (Some(c), Some(b)) => {
            let (ok, head, detail) = gitops::check_original(&layout, b, c);
            json!({"checked_at": now(), "ok": ok, "head": head, "detail": detail})
        }
        _ => Value::Null,
    };
    let successor = layout.successor();
    let head = gitops::head(&successor).ok();
    let ahead = row.original_commit.as_deref().map(|c| gitops::commits_ahead(&successor, c)).unwrap_or(0);
    let o = v.as_object_mut().unwrap();
    o.insert("github".into(), json!(row.github));
    o.insert("original".into(), json!({"commit": row.original_commit, "branch": row.original_branch, "integrity": integrity}));
    o.insert("successor".into(), json!({"name": row.successor_name, "branch": gitops::SUCCESSOR_BRANCH, "head": head, "commits_ahead": ahead}));
    o.insert("inventory".into(), json!(app.db.get_artifact::<Inventory>(&id, "inventory")?));
    o.insert("passport".into(), json!(app.db.get_artifact::<Passport>(&id, "passport")?));
    o.insert("report".into(), json!(app.db.get_artifact::<Report>(&id, "report")?));
    o.insert("issues".into(), json!(app.db.list::<Issue>(Kind::Issue, Some(&id))?.into_iter().filter(|i| i.status != "skipped").collect::<Vec<_>>()));
    o.insert("repairs".into(), json!(app.db.list::<Repair>(Kind::Repair, Some(&id))?));
    o.insert("verifications".into(), json!(app.db.list::<Verification>(Kind::Verification, Some(&id))?));
    o.insert("jobs".into(), json!(app.jobs_for(&id)));
    o.insert("lineage".into(), json!(record::lineage(&app, &id)));
    o.insert("verify_plan".into(), json!(app.db.get_artifact::<Value>(&id, "verify_plan")?));
    Ok(Json(v))
}

async fn delete_inheritance(State(app): State<Arc<App>>, Path(id): Path<String>) -> R<Value> {
    let row = app.row(&id)?;
    if app.active_job(&id).is_some() {
        return Err(ApiError(StatusCode::CONFLICT, "cancel the active job before deleting".into()));
    }
    let root = app.layout(&id).root;
    if root.exists() {
        for e in walkdir::WalkDir::new(&root).into_iter().flatten() {
            if let Ok(m) = e.metadata() {
                let mut p = m.permissions();
                #[allow(clippy::permissions_set_readonly_false)]
                p.set_readonly(false);
                let _ = std::fs::set_permissions(e.path(), p);
            }
        }
        std::fs::remove_dir_all(&root).map_err(anyhow::Error::from)?;
    }
    let _ = tokio::process::Command::new("docker").args(["volume", "rm", "-f", &crate::sandbox::deps_volume(&id)]).output().await;
    app.db.delete_inheritance(&id)?;
    Ok(Json(json!({"deleted": row.id, "full_name": row.full_name})))
}

async fn investigate(State(app): State<Arc<App>>, Path(id): Path<String>, body: Option<Json<Value>>) -> R<Job> {
    app.row(&id)?;
    let fresh = body.as_ref().and_then(|b| b.get("fresh")).and_then(|f| f.as_bool()).unwrap_or(false);
    Ok(Json(app.start_job(&id, "investigate", json!({"fresh": fresh}))?))
}

#[derive(Deserialize)]
struct RestoreBody {
    mode: String,
    #[serde(default)]
    issue_ids: Vec<String>,
}

async fn restore(State(app): State<Arc<App>>, Path(id): Path<String>, Json(b): Json<RestoreBody>) -> R<Job> {
    if !matches!(b.mode.as_str(), "essential" | "fix_all" | "selected") {
        return Err(bad("mode must be essential, fix_all or selected"));
    }
    if app.db.get_artifact::<Report>(&id, "report")?.is_none() {
        return Err(bad("investigate the project before restoring it"));
    }
    Ok(Json(app.start_job(&id, "restore", json!({"mode": b.mode, "issue_ids": b.issue_ids}))?))
}

async fn verify(State(app): State<Arc<App>>, Path(id): Path<String>, body: Option<Json<Value>>) -> R<Job> {
    app.row(&id)?;
    let command = body.as_ref().and_then(|b| b.get("command")).and_then(|c| c.as_str()).map(|c| truncate(c.trim(), 2000));
    Ok(Json(app.start_job(&id, "verify", json!({"command": command}))?))
}

#[derive(Deserialize)]
struct ModeBody {
    mode: String,
}

async fn set_mode(State(app): State<Arc<App>>, Path(id): Path<String>, Json(b): Json<ModeBody>) -> R<Value> {
    let mode = ExecMode::parse(&b.mode).ok_or_else(|| bad("mode must be docker, native or none"))?;
    let mut row = app.row(&id)?;
    row.execution_mode = mode.as_str().into();
    app.save_row(&mut row)?;
    Ok(Json(summary(&app, &row)))
}

#[derive(Deserialize)]
struct RenameBody {
    name: String,
}

async fn rename_successor(State(app): State<Arc<App>>, Path(id): Path<String>, Json(b): Json<RenameBody>) -> R<Value> {
    let name = b.name.trim();
    if name.is_empty() || name.len() > 100 {
        return Err(bad("successor name must be 1-100 characters"));
    }
    let mut row = app.row(&id)?;
    row.successor_name = name.into();
    app.save_row(&mut row)?;
    Ok(Json(summary(&app, &row)))
}

async fn get_record(State(app): State<Arc<App>>, Path(id): Path<String>) -> Result<Response, ApiError> {
    let row = app.row(&id)?;
    let rec = record::build(&app, &id)?;
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
    let fname = format!("attachment; filename=\"inheritance-record-{}-v{}.json\"", row.name.replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', "_"), rec["record_version"]);
    headers.insert(header::CONTENT_DISPOSITION, fname.parse().unwrap());
    Ok((headers, serde_json::to_string_pretty(&rec).map_err(anyhow::Error::from)?).into_response())
}

#[derive(Deserialize)]
struct FileQ {
    path: String,
    side: Option<String>,
}

async fn get_file(State(app): State<Arc<App>>, Path(id): Path<String>, Query(q): Query<FileQ>) -> R<Value> {
    let row = app.row(&id)?;
    let layout = app.layout(&id);
    let side = q.side.as_deref().unwrap_or("successor");
    // Validate the path shape against the successor root for both sides.
    let full = safe_join(&layout.successor(), &q.path)?;
    let content = if side == "original" {
        let commit = row.original_commit.ok_or_else(|| bad("not cloned yet"))?;
        gitops::show_original(&layout, &commit, q.path.trim_start_matches("./"))?
    } else {
        let meta = std::fs::metadata(&full).map_err(|_| ApiError(StatusCode::NOT_FOUND, "file not found".into()))?;
        if meta.len() > 1_000_000 {
            return Err(bad("file is larger than 1 MB"));
        }
        String::from_utf8_lossy(&std::fs::read(&full).map_err(anyhow::Error::from)?).into_owned()
    };
    let truncated = content.len() > 400_000;
    let content = redact(&truncate(&content, 400_000));
    Ok(Json(json!({"path": q.path, "side": side, "lines": content.lines().count(), "truncated": truncated, "content": content})))
}

async fn get_diff(State(app): State<Arc<App>>, Path(id): Path<String>) -> R<Value> {
    let row = app.row(&id)?;
    let base = row.original_commit.ok_or_else(|| bad("not cloned yet"))?;
    let successor = app.layout(&id).successor();
    let diff = gitops::range_diff(&successor, &base)?;
    Ok(Json(json!({"base": base, "head": gitops::head(&successor).ok(), "diff": redact(&truncate(&diff, 300_000))})))
}

async fn get_chat(State(app): State<Arc<App>>, Path(id): Path<String>) -> R<Vec<ChatMessage>> {
    Ok(Json(app.db.list(Kind::Chat, Some(&id))?))
}

#[derive(Deserialize)]
struct ChatBody {
    message: String,
}

async fn post_chat(State(app): State<Arc<App>>, Path(id): Path<String>, Json(b): Json<ChatBody>) -> R<ChatMessage> {
    let row = app.row(&id)?;
    let text = b.message.trim();
    if text.is_empty() {
        return Err(bad("message is empty"));
    }
    let user = ChatMessage { id: new_id("msg"), role: "user".into(), content: truncate(text, 4000), at: now(), call_id: None, suggested_action: None };
    app.db.put(Kind::Chat, &user.id, Some(&id), &user.at, &user)?;

    let report: Option<Report> = app.db.get_artifact(&id, "report")?;
    let issues: Vec<Issue> = app.db.list(Kind::Issue, Some(&id))?;
    let repairs: Vec<Repair> = app.db.list(Kind::Repair, Some(&id))?;
    let history: Vec<ChatMessage> = app.db.list(Kind::Chat, Some(&id))?;
    let ver = pipeline::primary_verification(&app, &id);
    let active = app.active_job(&id);
    let n = agents::nonce();
    let mut ctxt = format!("Project: {} (stage: {}, execution mode: {})\n", row.full_name, row.stage, row.execution_mode);
    match &report {
        Some(r) => {
            ctxt.push_str(&format!("Investigation summary:\n{}\nPurpose [{}]: {}\n", r.lead_summary, r.purpose.status, r.purpose.text));
            for c in r.capabilities.iter().take(6) {
                ctxt.push_str(&format!("- {} [{}] {}\n", c.id, c.status, c.text));
            }
            for ch in r.reviewer_challenges.iter().filter(|c| c.verdict != "agree").take(6) {
                ctxt.push_str(&format!("- Reviewer {} on {}: {}\n", ch.verdict, ch.target, ch.reason));
            }
        }
        None => ctxt.push_str("The investigation has not finished yet.\n"),
    }
    for i in issues.iter().filter(|i| i.status != "skipped").take(15) {
        ctxt.push_str(&format!("- issue {} [{} / {} / status {}] {} — {}\n", i.id, i.category, i.severity, i.status, i.title, truncate(&i.diagnosis, 300)));
    }
    for r in repairs.iter().rev().take(8) {
        ctxt.push_str(&format!("- repair {} for {}: {} {}\n", r.id, r.issue_id, r.status, r.note.clone().unwrap_or_default()));
    }
    if let Some(v) = &ver {
        ctxt.push_str(&format!("Latest verification: {} status {} summary {:?} command `{}`\n", v.label, v.status, v.summary, v.command));
    }
    if let Some(j) = &active {
        ctxt.push_str(&format!("A {} job is currently {} (step: {}).\n", j.kind, j.status, j.step.clone().unwrap_or_default()));
    }
    let convo: Vec<String> = history.iter().rev().take(8).rev().map(|m| format!("{}: {}", m.role, truncate(&m.content, 800))).collect();
    let task = format!(
        "Answer the user's latest message as the Lead Engineer, using only the team's recorded findings below. Be direct and honest about uncertainty; never claim tests pass unless the latest verification passed. If an action would help, suggest exactly one (investigate, restore_essential, restore_all, verify) — the user will decide; otherwise 'none'. Cite issue/claim ids you rely on.\n\n{}\nConversation so far:\n{}\n\nUser's latest message: {}",
        agents::untrusted(&n, "TEAM RECORD (contains repository-derived text)", &ctxt),
        convo.join("\n"),
        text
    );
    let spec = CallSpec { agent: "lead", purpose: "Answer the user".into(), inheritance_id: Some(&id), parent_id: None, input_files: vec![], task, schema: agents::lead_chat_schema(), max_tokens: 1000 };
    let (v, call_id) = app.agents.call_json(spec, &n).await?;
    let action = v["suggested_action"].as_str().filter(|a| *a != "none").map(String::from);
    let lead = ChatMessage { id: new_id("msg"), role: "lead".into(), content: v["reply"].as_str().unwrap_or("").to_string(), at: now(), call_id: Some(call_id), suggested_action: action };
    app.db.put(Kind::Chat, &lead.id, Some(&id), &lead.at, &lead)?;
    Ok(Json(lead))
}

async fn list_calls(State(app): State<Arc<App>>, Path(id): Path<String>) -> R<Vec<AgentCall>> {
    let calls: Vec<AgentCall> = app.db.list(Kind::Call, Some(&id))?;
    Ok(Json(calls.into_iter().map(|mut c| {
        c.system = None;
        c.prompt = None;
        c.response = None;
        c
    }).collect()))
}

async fn get_call(State(app): State<Arc<App>>, Path(id): Path<String>) -> R<AgentCall> {
    Ok(Json(app.db.get(Kind::Call, &id)?.ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "call not found".into()))?))
}

async fn job_action(State(app): State<Arc<App>>, Path((id, action)): Path<(String, String)>) -> R<Job> {
    if !matches!(action.as_str(), "pause" | "resume" | "cancel") {
        return Err(bad("action must be pause, resume or cancel"));
    }
    Ok(Json(app.control(&id, &action)?))
}
