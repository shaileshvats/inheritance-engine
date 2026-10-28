//! The Inheritance Record: a portable, versioned JSON document describing where a successor came
//! from, what was learned, what was changed, and how each change was verified.
//!
//! Records reference evidence (paths, line ranges, commits, commands) rather than copying source files.

use serde_json::{json, Value};

use crate::app::App;
use crate::contracts::*;
use crate::db::Kind;
use crate::gitops;
use crate::util::now;

pub const SCHEMA_VERSION: &str = "1.0";

pub fn lineage(app: &App, id: &str) -> Vec<Value> {
    let Ok(row) = app.row(id) else { return vec![] };
    let Some(base) = row.original_commit.clone() else { return vec![] };
    let inv: Option<Inventory> = app.db.get_artifact(id, "inventory").ok().flatten();
    let last = inv.as_ref().and_then(|i| i.git.recent.first().cloned());
    let mut out = vec![json!({
        "kind": "original",
        "sha": base,
        "label": format!("{} @ {} (original, read-only)", row.full_name, row.original_branch.as_deref().unwrap_or("?")),
        "date": last.as_ref().map(|c| c.date.clone()),
        "author": last.as_ref().map(|c| c.author.clone()),
        "parent": Value::Null,
    })];
    let successor = app.layout(id).successor();
    let mut commits = gitops::successor_commits(&successor, &base).unwrap_or_default();
    commits.reverse();
    let mut parent = base.clone();
    for c in commits {
        out.push(json!({"kind": "successor", "sha": c.sha, "label": c.subject, "date": c.date, "author": c.author, "parent": parent}));
        parent = c.sha;
    }
    out
}

pub fn build(app: &App, id: &str) -> anyhow::Result<Value> {
    let row = app.row(id)?;
    let inv: Option<Inventory> = app.db.get_artifact(id, "inventory")?;
    let passport: Option<Passport> = app.db.get_artifact(id, "passport")?;
    let report: Option<Report> = app.db.get_artifact(id, "report")?;
    let issues: Vec<Issue> = app.db.list(Kind::Issue, Some(id))?;
    let repairs: Vec<Repair> = app.db.list(Kind::Repair, Some(id))?;
    let verifications: Vec<Verification> = app.db.list(Kind::Verification, Some(id))?;
    let calls: Vec<AgentCall> = app.db.list(Kind::Call, Some(id))?;
    let successor = app.layout(id).successor();
    let head = gitops::head(&successor).ok();
    let version: u64 = app.db.get_artifact::<u64>(id, "record_version")?.unwrap_or(0) + 1;

    let claims: Vec<Claim> = report
        .as_ref()
        .map(|r| std::iter::once(r.purpose.clone()).chain(r.capabilities.clone()).chain(r.data_flows.clone()).chain(r.external_services.clone()).collect())
        .unwrap_or_default();
    let all_evidence: Vec<&Evidence> = claims.iter().flat_map(|c| c.evidence.iter()).chain(issues.iter().flat_map(|i| i.evidence.iter())).collect();
    let count = |s: &str| all_evidence.iter().filter(|e| e.check == s).count();

    let record = json!({
        "schema_version": SCHEMA_VERSION,
        "record_version": version,
        "generated_at": now(),
        "generator": {"name": "inheritance-engine", "version": env!("CARGO_PKG_VERSION")},
        "project": {
            "id": format!("github:{}@{}", row.full_name, row.original_commit.as_deref().unwrap_or("?")),
            "name": row.name,
            "source_url": row.source_url,
            "original_commit": row.original_commit,
            "original_branch": row.original_branch,
            "original_owner": row.github.owner,
            "original_license": row.license,
            "description": row.github.description,
        },
        "inheritance": {
            "created_at": row.created_at,
            "inheritor": "local-user",
            "workspace_id": row.id,
            "execution_mode": row.execution_mode,
            "stage": row.stage,
        },
        "archaeology": {
            "purpose": report.as_ref().map(|r| &r.purpose),
            "summary": report.as_ref().map(|r| &r.lead_summary),
            "architecture": inv.as_ref().map(|i| json!({
                "languages": i.languages, "manifests": i.manifests, "files_total": i.files_total,
                "import_graph": {"nodes": i.graph.nodes.len(), "edges": i.graph.edges},
            })),
            "entry_points": inv.as_ref().map(|i| &i.entry_points),
            "core_components": {
                "engine_candidates": inv.as_ref().map(|i| &i.mother_candidates),
                "agent_components": report.as_ref().map(|r| &r.components),
            },
            "dependencies": passport.as_ref().map(|p| &p.dependencies),
            "findings": claims,
            "reviewer_challenges": report.as_ref().map(|r| &r.reviewer_challenges),
            "unknowns": report.as_ref().map(|r| &r.unknowns),
            "decay": inv.as_ref().map(|i| &i.decay),
            "git_history": inv.as_ref().map(|i| &i.git),
        },
        "restoration": {
            "issues": issues,
            "attempts": repairs,
            "changes": lineage(app, id).into_iter().filter(|l| l["kind"] == "successor").collect::<Vec<_>>(),
            "verification": verifications,
            "upstream_triage": app.db.get_artifact::<Value>(id, "upstream_triage")?,
        },
        "successor": {
            "identity": {"name": row.successor_name, "branch": gitops::SUCCESSOR_BRANCH, "head": head},
            "parent_project_id": format!("github:{}@{}", row.full_name, row.original_commit.as_deref().unwrap_or("?")),
            "lineage": lineage(app, id),
        },
        "provenance": {
            "evidence": {
                "citations_total": all_evidence.len(),
                "verified": count("verified"), "mismatch": count("mismatch"), "missing_file": count("missing_file"),
                "out_of_range": count("out_of_range"), "unchecked": count("unchecked"),
                "method": "Each citation was re-read from the successor working tree by the engine; claims marked verified by a model without a verified citation were downgraded to inferred.",
            },
            "attribution": {
                "original_authors": inv.as_ref().map(|i| &i.git.authors),
                "original_license": row.license,
                "models": calls.iter().map(|c| c.model.clone()).collect::<std::collections::BTreeSet<_>>(),
                "note": "Lineage does not imply endorsement by the original authors.",
            },
            "agent_calls": calls.iter().map(|c| json!({
                "id": c.id, "agent": c.agent, "purpose": c.purpose, "model": c.model, "provider": c.provider,
                "started_at": c.started_at, "duration_ms": c.duration_ms, "ok": c.ok, "parent_id": c.parent_id, "input_files": c.input_files,
            })).collect::<Vec<_>>(),
            "trust_passport": passport,
        },
    });
    app.db.put_artifact(id, "record_version", &version)?;
    let dir = app.layout(id).root.join("records");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(format!("record-v{version}.json")), serde_json::to_vec_pretty(&record)?)?;
    Ok(record)
}
