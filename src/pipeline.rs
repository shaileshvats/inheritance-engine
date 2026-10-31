//! The inheritance pipelines: investigate (clone → inventory → baseline → agent team → report),
//! restore (diagnose → patch → independent review → verify → commit or revert), and verify.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use serde_json::{json, Value};

use crate::agents::{self, CallSpec};
use crate::app::{App, JobCtx};
use crate::contracts::*;
use crate::db::Kind;
use crate::evidence;
use crate::gitops;
use crate::inventory;
use crate::sandbox::{self, RunRequest};
use crate::security::{redact, safe_join};
use crate::util::{ellipsis, new_id, now, tail_lines, truncate};
use crate::verify;

pub const INVESTIGATE_STEPS: &[&str] = &[
    "clone", "inventory", "baseline verification", "lead: plan", "historian", "restorer: diagnose", "reviewer: challenge", "lead: synthesis",
];

// ---------- shared helpers ----------

/// Deterministic inventory and trust passport of the successor's current tree (no model involved).
pub(crate) async fn build_inventory(app: &App, id: &str) -> anyhow::Result<Inventory> {
    let mut row = app.row(id)?;
    let commit = row.original_commit.clone().unwrap_or_default();
    let root = app.layout(id).successor();
    let name = row.name.clone();
    let c = commit.clone();
    let mut prov = Provenance { source_url: row.source_url.clone(), original_commit: commit, cloned_at: now(), license: row.license.clone() };
    if prov.license.is_none() {
        if let Some(l) = inventory::detect_license_file(&root) {
            prov.license = Some(format!("{l} (detected from LICENSE file)"));
        }
    }
    let (inv, passport) = tokio::task::spawn_blocking(move || inventory::build(inventory::InventoryInput { root: &root, name: &name, commit: &c, provenance: prov })).await??;
    if row.license.is_none() {
        if let Some(l) = &passport.provenance.license {
            row.license = Some(l.clone());
            app.save_row(&mut row)?;
        }
    }
    app.db.put_artifact(id, "inventory", &inv)?;
    app.db.put_artifact(id, "passport", &passport)?;
    Ok(inv)
}

fn inventory_of(app: &App, id: &str) -> anyhow::Result<Inventory> {
    app.db.get_artifact(id, "inventory")?.ok_or_else(|| anyhow::anyhow!("inventory missing; run the investigation first"))
}

fn passport_of(app: &App, id: &str) -> anyhow::Result<Passport> {
    app.db.get_artifact(id, "passport")?.ok_or_else(|| anyhow::anyhow!("trust passport missing; run the investigation first"))
}

pub fn issues_of(app: &App, id: &str) -> Vec<Issue> {
    app.db.list(Kind::Issue, Some(id)).unwrap_or_default()
}

fn save_issue(app: &App, id: &str, issue: &Issue) -> anyhow::Result<()> {
    app.db.put(Kind::Issue, &issue.id, Some(id), &issue.id, issue)
}

pub fn primary_verification(app: &App, id: &str) -> Option<Verification> {
    let vid: String = app.db.get_artifact(id, "primary_verification").ok()??;
    app.db.get(Kind::Verification, &vid).ok()?
}

fn file_exists(app: &App, id: &str, path: &str) -> bool {
    safe_join(&app.layout(id).successor(), path).map(|p| p.is_file()).unwrap_or(false)
}

fn exec_mode(row: &InheritanceRow) -> ExecMode {
    ExecMode::parse(&row.execution_mode).unwrap_or(ExecMode::None)
}

fn store_verification(app: &App, id: &str, v: &Verification) -> anyhow::Result<()> {
    app.db.put(Kind::Verification, &v.id, Some(id), &v.started_at, v)
}

fn not_run(label: &str, mode: &str, reason: &str, rev: Option<String>) -> Verification {
    Verification {
        id: new_id("ver"),
        label: label.into(),
        mode: mode.into(),
        environment: "-".into(),
        command: "-".into(),
        exit_code: None,
        status: "not_run".into(),
        summary: None,
        output_tail: String::new(),
        duration_ms: 0,
        started_at: now(),
        workspace_rev: rev,
        network: "-".into(),
        reason: Some(reason.into()),
    }
}

/// Run the detected verification suite (install → syntax → tests) for the current successor state.
/// Returns the primary verification (tests, else syntax), which is also stored as `primary_verification`.
pub async fn run_suite(app: &App, id: &str, label: &str, custom_test: Option<&str>) -> anyhow::Result<Verification> {
    let row = app.row(id)?;
    let mode = exec_mode(&row);
    let layout = app.layout(id);
    let rev = gitops::head(&layout.successor()).ok();
    let finish = |v: Verification| -> anyhow::Result<Verification> {
        store_verification(app, id, &v)?;
        app.db.put_artifact(id, "primary_verification", &v.id)?;
        Ok(v)
    };
    if mode == ExecMode::None {
        return finish(not_run(&format!("{label} · tests"), "none", "Execution is disabled for this inheritance (static analysis only). Choose Docker or native mode to run tests.", rev));
    }
    let inv = inventory_of(app, id)?;
    let passport = passport_of(app, id)?;
    let plan = verify::plan(&inv, &passport, &app.cfg.python_image, &app.cfg.node_image);
    app.db.put_artifact(id, "verify_plan", &plan)?;
    if plan.ecosystem == "unsupported" && custom_test.is_none() {
        return finish(not_run(&format!("{label} · tests"), mode.as_str(), plan.note.as_deref().unwrap_or("unsupported"), rev));
    }
    if mode == ExecMode::Docker {
        if let Err(e) = sandbox::docker_version().await {
            return finish(not_run(&format!("{label} · tests"), "docker", &format!("Docker is not available ({e}). Start Docker or switch the execution mode explicitly."), rev));
        }
        if let Err(e) = sandbox::ensure_image(&plan.image).await {
            return finish(not_run(&format!("{label} · tests"), "docker", &format!("Could not obtain image {}: {e}", plan.image), rev));
        }
    }
    let timeout = Duration::from_secs(app.cfg.run_timeout_secs);
    let run = |label: String, command: String, network: bool, as_root: bool| {
        let plan = plan.clone();
        let layout = layout.clone();
        let rev = rev.clone();
        async move {
            let out = sandbox::run(RunRequest { mode, layout: &layout, inheritance_id: id, image: &plan.image, command: &command, env: &plan.env, network, as_root, timeout, workdir: None }).await?;
            let output = redact(&out.output);
            let is_test = label.ends_with("tests");
            let summary = if is_test { verify::parse_summary(&output) } else { None };
            let (status, reason) = verify::status_for(out.exit_code, out.timed_out, summary.as_ref(), command.contains("pytest"));
            let tool = if label.ends_with("dependency install") { "sandbox.install" } else if label.ends_with("syntax check") { "sandbox.syntax" } else { "sandbox.tests" };
            crate::telemetry::record_tool(id, tool, mode.as_str(), &status, out.duration_ms);
            anyhow::Ok(Verification {
                id: new_id("ver"),
                label,
                mode: mode.as_str().into(),
                environment: out.environment,
                command: out.command,
                exit_code: out.exit_code,
                status,
                summary,
                output_tail: tail_lines(&output, 150),
                duration_ms: out.duration_ms,
                started_at: now(),
                workspace_rev: rev,
                network: out.network,
                reason,
            })
        }
    };
    // Dependencies are installed once per distinct install command (network on, as root in Docker).
    if let Some(install) = &plan.install {
        let installed: Option<String> = app.db.get_artifact(id, "deps_installed")?;
        let key = format!("{}|{install}", mode.as_str());
        if installed.as_deref() != Some(key.as_str()) {
            let v = run(format!("{label} · dependency install"), install.clone(), true, true).await?;
            let ok = v.status == "passed";
            store_verification(app, id, &v)?;
            if ok {
                app.db.put_artifact(id, "deps_installed", &key)?;
            }
        }
    }
    let mut primary = None;
    if let Some(syntax) = &plan.syntax {
        let v = run(format!("{label} · syntax check"), syntax.clone(), false, false).await?;
        store_verification(app, id, &v)?;
        primary = Some(v);
    }
    let test_cmd = custom_test.map(String::from).or_else(|| plan.test.clone());
    if let Some(test) = test_cmd {
        let v = run(format!("{label} · tests"), test, false, false).await?;
        primary = Some(v);
    }
    match primary {
        Some(v) => finish(v),
        None => finish(not_run(&format!("{label} · tests"), mode.as_str(), plan.note.as_deref().unwrap_or("no verification command available"), rev)),
    }
}

pub(crate) fn verification_brief(v: &Verification) -> String {
    let counts = v.summary.as_ref().map(|s| format!("{} passed, {} failed, {} errors, {} skipped", s.passed, s.failed, s.errors, s.skipped)).unwrap_or_else(|| "no test summary".into());
    format!("{} — status {} (exit code {}), {} — command `{}`{}", v.label, v.status, v.exit_code.map(|c| c.to_string()).unwrap_or_else(|| "none".into()), counts, v.command, v.reason.as_ref().map(|r| format!(" — {r}")).unwrap_or_default())
}

/// Paths mentioned in a traceback / test output that exist in the successor.
fn paths_in_output(app: &App, id: &str, output: &str) -> Vec<(String, usize)> {
    let re = regex::Regex::new(r#"(?:File "(?:/work/)?([^"]+)", line (\d+)|^([\w./\-]+\.(?:py|js|ts)):(\d+))"#).unwrap();
    let mut out = Vec::new();
    for line in output.lines() {
        for c in re.captures_iter(line.trim()) {
            let path = c.get(1).or(c.get(3)).map(|m| m.as_str()).unwrap_or("");
            let ln: usize = c.get(2).or(c.get(4)).and_then(|m| m.as_str().parse().ok()).unwrap_or(1);
            let path = path.trim_start_matches("./");
            if !path.starts_with('/') && file_exists(app, id, path) && !out.iter().any(|(p, l)| p == path && *l == ln) {
                out.push((path.to_string(), ln));
            }
        }
    }
    out
}

struct FilePack {
    text: String,
    files: Vec<String>,
}

/// Pack excerpts of files into an UNTRUSTED prompt section within a character budget.
fn pack_files(app: &App, id: &str, n: &str, files: &[(String, Vec<usize>)], budget: usize) -> FilePack {
    let root = app.layout(id).successor();
    let per = (budget / files.len().max(1)).max(2500);
    let mut text = String::new();
    let mut used = Vec::new();
    for (path, focus) in files {
        if text.len() >= budget {
            break;
        }
        if let Some(ex) = evidence::excerpt(&root, path, focus, per.min(budget - text.len())) {
            text.push_str(&agents::untrusted(n, &format!("FILE {} (lines {} of {})", ex.path, ex.shown, ex.total_lines), &ex.body));
            used.push(ex.path);
        }
    }
    FilePack { text, files: used }
}

fn parse_evidence(v: &Value) -> Vec<Evidence> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|e| {
                    Some(Evidence {
                        path: e["path"].as_str()?.trim().trim_start_matches("./").to_string(),
                        start_line: e["start_line"].as_u64().unwrap_or(1) as usize,
                        end_line: e["end_line"].as_u64().unwrap_or(1) as usize,
                        quote: e["quote"].as_str().unwrap_or("").to_string(),
                        check: "unchecked".into(),
                    })
                })
                .take(4)
                .collect()
        })
        .unwrap_or_default()
}

fn claim_from(app: &App, id: &str, v: &Value, cid: String, agent: &str) -> Claim {
    let mut ev = parse_evidence(&v["evidence"]);
    evidence::check_all(&app.layout(id).successor(), &mut ev);
    let status = evidence::enforce_status(v["status"].as_str().unwrap_or("unknown"), &ev);
    Claim { id: cid, text: v["text"].as_str().unwrap_or("").trim().to_string(), status, agent: agent.into(), evidence: ev }
}

fn claims_from(app: &App, id: &str, v: &Value, prefix: &str, agent: &str, max: usize) -> Vec<Claim> {
    v.as_array()
        .map(|a| a.iter().take(max).enumerate().map(|(i, c)| claim_from(app, id, c, format!("{prefix}{}", i + 1), agent)).filter(|c| !c.text.is_empty()).collect())
        .unwrap_or_default()
}

fn norm_text(s: &str) -> String {
    s.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ").trim_end_matches('.').to_string()
}

struct DecayGroup {
    gid: String,
    pattern: String,
    detail: String,
    removed_in: String,
    hits: Vec<DecayHit>,
}

fn decay_groups(inv: &Inventory) -> Vec<DecayGroup> {
    let mut map: BTreeMap<String, Vec<DecayHit>> = BTreeMap::new();
    for h in &inv.decay {
        map.entry(h.pattern.clone()).or_default().push(h.clone());
    }
    map.into_iter()
        .enumerate()
        .map(|(i, (pattern, hits))| DecayGroup { gid: format!("D{}", i + 1), detail: hits[0].detail.clone(), removed_in: hits[0].removed_in.clone(), pattern, hits })
        .collect()
}

/// The identifier a failing run would mention, e.g. `collections.Mapping` → `Mapping`.
fn decay_token(h: &DecayHit) -> String {
    let m = h.matched.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
    if h.pattern == "print statement" {
        return "print".into();
    }
    m.rsplit(|c: char| !(c.is_alphanumeric() || c == '_')).find(|s| !s.is_empty()).unwrap_or(m).to_string()
}

fn group_linked_to_failure(g: &DecayGroup, output: &str) -> bool {
    !output.is_empty() && g.hits.iter().any(|h| {
        let t = decay_token(h);
        t.len() >= 4 && output.contains(&t)
    })
}

fn facts_block(row: &InheritanceRow, inv: &Inventory, baseline: Option<&Verification>) -> String {
    let langs: Vec<String> = inv.languages.iter().take(5).map(|l| format!("{} ({} files, {} lines)", l.language, l.files, l.lines)).collect();
    let mothers: Vec<String> = inv.mother_candidates.iter().take(8).map(|m| format!("- {} (score {:.2}: {})", m.path, m.score, m.reasons.join("; "))).collect();
    let eps: Vec<String> = inv.entry_points.iter().take(10).map(|e| format!("- {} [{}] {}", e.path, e.kind, e.detail)).collect();
    let authors: Vec<String> = inv.git.authors.iter().take(5).map(|a| format!("{} ({})", a.name, a.commits)).collect();
    format!(
        "Repository: {} ({})\nGitHub description: {}\nLicense: {}\nGit history: {} commits from {} to {}; top authors: {}\nLanguages: {}\nManifests: {}\nTests: framework {} with {} test files\nEntry points:\n{}\nCentral-component candidates (engine heuristic: imports, history, entry points, README mentions):\n{}\nBaseline verification: {}\n",
        row.full_name,
        row.source_url,
        redact(row.github.description.as_deref().unwrap_or("(none)")),
        row.license.as_deref().unwrap_or("not detected"),
        inv.git.commits,
        inv.git.first_commit_at.as_deref().unwrap_or("?"),
        inv.git.last_commit_at.as_deref().unwrap_or("?"),
        authors.join(", "),
        langs.join(", "),
        inv.manifests.iter().map(|m| m.path.as_str()).collect::<Vec<_>>().join(", "),
        inv.tests.framework,
        inv.tests.files.len(),
        if eps.is_empty() { "- none detected".into() } else { eps.join("\n") },
        if mothers.is_empty() { "- none".into() } else { mothers.join("\n") },
        baseline.map(verification_brief).unwrap_or_else(|| "not run".into()),
    )
}

fn all_paths(inv: &Inventory) -> Vec<String> {
    fn walk(n: &TreeNode, out: &mut Vec<String>) {
        if n.kind == "file" {
            out.push(n.path.clone());
        }
        for c in n.children.iter().flatten() {
            walk(c, out);
        }
    }
    let mut out = Vec::new();
    walk(&inv.tree, &mut out);
    out
}

// ---------- investigate ----------

pub async fn investigate(ctx: &mut JobCtx) -> anyhow::Result<()> {
    let app = ctx.app.clone();
    let id = ctx.id();
    if ctx.job.params.get("fresh").and_then(|v| v.as_bool()).unwrap_or(false) {
        app.db.delete_artifacts_with_prefix(&id, "inv:")?;
        app.db.delete_artifacts_with_prefix(&id, "inventory")?;
        app.db.delete_artifacts_with_prefix(&id, "passport")?;
        ctx.job.params["fresh"] = json!(false);
        ctx.save();
    }

    // 1. clone
    ctx.begin("clone").await?;
    let mut row = app.row(&id)?;
    let layout = app.layout(&id);
    if row.original_commit.is_some() && layout.successor().exists() {
        ctx.finish("clone", "skipped", format!("already cloned at {}", row.original_commit.as_deref().unwrap_or("")));
    } else {
        if row.github.size_kb > app.cfg.max_repo_kb {
            anyhow::bail!("repository is {} KB, above the configured limit of {} KB (IE_MAX_REPO_KB)", row.github.size_kb, app.cfg.max_repo_kb);
        }
        let url = format!("https://github.com/{}", row.full_name);
        let l2 = layout.clone();
        let res = tokio::task::spawn_blocking(move || gitops::clone_inheritance(&l2, &url)).await??;
        row.original_commit = Some(res.commit.clone());
        row.original_branch = Some(res.branch.clone());
        row.stage = Stage::Inherited.as_str().into();
        app.save_row(&mut row)?;
        ctx.finish("clone", "done", format!("original {} on {} preserved read-only; successor branch '{}' created", &res.commit[..12.min(res.commit.len())], res.branch, gitops::SUCCESSOR_BRANCH));
    }

    // 2. inventory
    ctx.begin("inventory").await?;
    let inv: Inventory = match app.db.get_artifact::<Inventory>(&id, "inventory")? {
        Some(inv) => {
            ctx.finish("inventory", "skipped", "already computed");
            inv
        }
        None => {
            app.set_stage(&id, Stage::Investigating)?;
            let inv = build_inventory(&app, &id).await?;
            row = app.row(&id)?;
            ctx.finish("inventory", "done", format!("{} files, {} import edges, {} decay hits, {} entry points", inv.files_total, inv.graph.edges.len(), inv.decay.len(), inv.entry_points.len()));
            inv
        }
    };

    // 3. baseline verification
    ctx.begin("baseline verification").await?;
    let baseline = match app.db.get_artifact::<String>(&id, "inv:baseline")? {
        Some(vid) => {
            ctx.finish("baseline verification", "skipped", "already run");
            app.db.get::<Verification>(Kind::Verification, &vid)?
        }
        None => {
            let v = run_suite(&app, &id, "baseline", None).await?;
            app.db.put_artifact(&id, "inv:baseline", &v.id)?;
            ctx.finish("baseline verification", "done", verification_brief(&v));
            Some(v)
        }
    };
    let baseline_output = baseline.as_ref().map(|v| v.output_tail.clone()).unwrap_or_default();
    let paths = all_paths(&inv);
    let groups = decay_groups(&inv);

    // 4. lead plan
    ctx.begin("lead: plan").await?;
    let plan: Value = match app.db.get_artifact::<Value>(&id, "inv:plan")? {
        Some(p) => {
            ctx.finish("lead: plan", "skipped", "already planned");
            p
        }
        None => {
            let n = agents::nonce();
            let readme = inv.readme_path.as_deref().and_then(|p| std::fs::read_to_string(layout.successor().join(p)).ok()).unwrap_or_default();
            let decay_lines: Vec<String> = groups.iter().map(|g| format!("- {}: {} ({} hits in {}) — {}, removed in {}", g.gid, g.pattern, g.hits.len(), g.hits.iter().map(|h| h.path.as_str()).collect::<BTreeSet<_>>().into_iter().collect::<Vec<_>>().join(", "), g.detail, g.removed_in)).collect();
            let listing: Vec<&str> = paths.iter().map(|s| s.as_str()).take(200).collect();
            let task = format!(
                "You are starting the investigation of an inherited repository. The engine already computed these facts:\n\n{}\nStatic decay scan (APIs removed from modern runtimes):\n{}\n\nFile list:\n{}\n\n{}{}\nDecide what your two specialists should read. The Historian reconstructs purpose and architecture; the Restorer diagnoses what is broken. Choose at most 4 files each from the file list (exact paths). Prefer the README, entry points and central components for the Historian; files with decay hits or files in failing output for the Restorer. Give each a one-sentence focus. Record a first impression (2-3 sentences, clearly provisional) and open questions.",
                facts_block(&row, &inv, baseline.as_ref()),
                if decay_lines.is_empty() { "- none".into() } else { decay_lines.join("\n") },
                listing.join("\n"),
                agents::untrusted(&n, "README excerpt", &truncate(&redact(&readme), 3500)),
                if baseline_output.is_empty() { String::new() } else { agents::untrusted(&n, "baseline verification output (tail)", &tail_lines(&baseline_output, 40)) },
            );
            let spec = CallSpec { agent: "lead", purpose: "Initial inspection and delegation plan".into(), inheritance_id: Some(&id), parent_id: None, input_files: inv.readme_path.iter().cloned().collect(), task, schema: agents::lead_plan_schema(), max_tokens: 900 };
            let (v, call_id) = app.agents.call_json(spec, &n).await?;
            let pick = |key: &str| -> Vec<String> { v[key].as_array().map(|a| a.iter().filter_map(|p| p.as_str()).map(|p| p.trim().trim_start_matches("./").to_string()).filter(|p| paths.contains(p)).take(4).collect()).unwrap_or_default() };
            let mut hist = pick("historian_files");
            let mut rest = pick("restorer_files");
            let mut fallback = Vec::new();
            if hist.is_empty() {
                hist.extend(inv.readme_path.iter().cloned());
                hist.extend(inv.mother_candidates.iter().take(3).map(|m| m.path.clone()));
                fallback.push("historian");
            }
            if rest.is_empty() {
                let mut by_count: BTreeMap<String, usize> = BTreeMap::new();
                for h in &inv.decay {
                    *by_count.entry(h.path.clone()).or_default() += 1;
                }
                for (p, _) in paths_in_output(&app, &id, &baseline_output) {
                    by_count.entry(p).or_insert(100);
                }
                let mut v: Vec<(String, usize)> = by_count.into_iter().collect();
                v.sort_by_key(|e| std::cmp::Reverse(e.1));
                rest = v.into_iter().map(|(p, _)| p).take(4).collect();
                fallback.push("restorer");
            }
            let plan = json!({
                "call_id": call_id,
                "first_impression": v["first_impression"],
                "historian_files": hist,
                "restorer_files": rest,
                "historian_focus": v["historian_focus"],
                "restorer_focus": v["restorer_focus"],
                "open_questions": v["open_questions"],
                "engine_fallback": fallback,
            });
            app.db.put_artifact(&id, "inv:plan", &plan)?;
            ctx.finish("lead: plan", "done", format!("Historian ← {} files, Restorer ← {} files", plan["historian_files"].as_array().map(|a| a.len()).unwrap_or(0), plan["restorer_files"].as_array().map(|a| a.len()).unwrap_or(0)));
            plan
        }
    };
    let plan_call = plan["call_id"].as_str().map(String::from);
    let list = |key: &str| -> Vec<String> { plan[key].as_array().map(|a| a.iter().filter_map(|p| p.as_str().map(String::from)).collect()).unwrap_or_default() };

    // 5. historian
    ctx.begin("historian").await?;
    let hist: Value = match app.db.get_artifact::<Value>(&id, "inv:historian")? {
        Some(h) => {
            ctx.finish("historian", "skipped", "already done");
            h
        }
        None => {
            let n = agents::nonce();
            let files: Vec<(String, Vec<usize>)> = list("historian_files").into_iter().map(|p| (p, vec![])).collect();
            let pack = pack_files(&app, &id, &n, &files, app.cfg.prompt_budget_chars);
            let task = format!(
                "Assignment from the Lead Engineer: {}\n\nEngine facts:\n{}\nFiles assigned to you:\n{}\nReconstruct: the project's purpose (one claim), 3-6 main capabilities, the role of each important component (by path), data flows, external services it talks to, how to build/run it, and what remains unknown. Every claim needs evidence from the files above (path, line range, exact short quote).",
                plan["historian_focus"].as_str().unwrap_or("Understand what this project is for and how it is structured."),
                facts_block(&row, &inv, baseline.as_ref()),
                pack.text
            );
            let spec = CallSpec { agent: "historian", purpose: "Reconstruct purpose and architecture".into(), inheritance_id: Some(&id), parent_id: plan_call.clone(), input_files: pack.files.clone(), task, schema: agents::historian_schema(), max_tokens: 4000 };
            let (v, call_id) = app.agents.call_json(spec, &n).await?;
            let purpose = claim_from(&app, &id, &v["purpose"], "c_purpose".into(), "historian");
            // Small models sometimes repeat a claim; keep each distinct statement once.
            let mut seen: BTreeSet<String> = BTreeSet::from([norm_text(&purpose.text)]);
            let mut dedupe = |claims: Vec<Claim>| -> Vec<Claim> { claims.into_iter().filter(|c| seen.insert(norm_text(&c.text))).collect() };
            let caps = dedupe(claims_from(&app, &id, &v["capabilities"], "c_cap_", "historian", 8));
            let flows = dedupe(claims_from(&app, &id, &v["data_flows"], "c_flow_", "historian", 6));
            let ext = dedupe(claims_from(&app, &id, &v["external_services"], "c_ext_", "historian", 6));
            let root = layout.successor();
            let comps: Vec<Component> = v["components"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .take(12)
                        .filter_map(|c| {
                            let path = c["path"].as_str()?.trim().trim_start_matches("./").to_string();
                            let mut ev = parse_evidence(&c["evidence"]);
                            evidence::check_all(&root, &mut ev);
                            let status = if paths.contains(&path) { evidence::enforce_status(c["status"].as_str().unwrap_or("unknown"), &ev) } else { "unknown".into() };
                            Some(Component { path, role: c["role"].as_str().unwrap_or("").into(), status, evidence: ev })
                        })
                        .collect()
                })
                .unwrap_or_default();
            let out = json!({
                "call_id": call_id, "purpose": purpose, "capabilities": caps, "data_flows": flows, "external_services": ext,
                "components": comps, "build_instructions": v["build_instructions"], "unknowns": v["unknowns"],
            });
            app.db.put_artifact(&id, "inv:historian", &out)?;
            let n_claims = 1 + caps.len() + flows.len() + ext.len();
            let verified = [&vec![purpose.clone()], &caps, &flows, &ext].iter().flat_map(|v| v.iter()).filter(|c| c.status == "verified").count();
            ctx.finish("historian", "done", format!("{n_claims} claims ({verified} verified against source), {} components", comps.len()));
            out
        }
    };

    // 6. restorer diagnose
    ctx.begin("restorer: diagnose").await?;
    if app.db.get_artifact::<Value>(&id, "inv:restorer")?.is_some() {
        ctx.finish("restorer: diagnose", "skipped", "already done");
    } else {
        let n = agents::nonce();
        let mut focus: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for p in list("restorer_files") {
            focus.entry(p).or_default();
        }
        for h in &inv.decay {
            if let Some(f) = focus.get_mut(&h.path) {
                f.push(h.line);
            }
        }
        for (p, l) in paths_in_output(&app, &id, &baseline_output) {
            if let Some(f) = focus.get_mut(&p) {
                f.push(l);
            }
        }
        let files: Vec<(String, Vec<usize>)> = focus.into_iter().collect();
        let pack = pack_files(&app, &id, &n, &files, app.cfg.prompt_budget_chars * 2 / 3);
        let groups_text: Vec<String> = groups
            .iter()
            .map(|g| {
                let samples: Vec<String> = g.hits.iter().take(3).map(|h| format!("    {}:{}: {}", h.path, h.line, redact(&h.text))).collect();
                format!("- {} \"{}\": {} hits. {} (removed in {}). Linked to baseline failure output: {}.\n{}", g.gid, g.pattern, g.hits.len(), g.detail, g.removed_in, if group_linked_to_failure(g, &baseline_output) { "yes" } else { "no" }, samples.join("\n"))
            })
            .collect();
        let task = format!(
            "Assignment from the Lead Engineer: {}\n\nEngine facts:\n{}\nStatic decay groups found by the engine (ids D1, D2, ...):\n{}\n\n{}{}\nDiagnose what is broken. Create one issue per root cause. 'essential' means it stops the software (or its tests) from working on a modern runtime; 'improvement' means optional modernization. Reference decay group ids in decay_groups when an issue covers them. Cite evidence. Propose the minimal fix that preserves the original behaviour. Do not invent problems that the evidence does not show.",
            plan["restorer_focus"].as_str().unwrap_or("Find what prevents this project from working today."),
            facts_block(&row, &inv, baseline.as_ref()),
            if groups_text.is_empty() { "- none".into() } else { groups_text.join("\n") },
            if baseline_output.is_empty() { String::new() } else { agents::untrusted(&n, "baseline verification output (tail)", &tail_lines(&baseline_output, 80)) },
            pack.text
        );
        let spec = CallSpec { agent: "restorer", purpose: "Diagnose decay and failures".into(), inheritance_id: Some(&id), parent_id: plan_call.clone(), input_files: pack.files.clone(), task, schema: agents::restorer_diagnose_schema(), max_tokens: 2500 };
        let (v, call_id) = app.agents.call_json(spec, &n).await?;
        let root = layout.successor();
        let mut covered: BTreeSet<String> = BTreeSet::new();
        let mut created = 0;
        // Replace any previously diagnosed open issues (fresh investigation).
        for mut old in issues_of(&app, &id).into_iter().filter(|i| i.status == "open") {
            old.status = "skipped".into();
            save_issue(&app, &id, &old)?;
        }
        for (i, it) in v["issues"].as_array().cloned().unwrap_or_default().iter().take(12).enumerate() {
            let gids: Vec<String> = it["decay_groups"].as_array().map(|a| a.iter().filter_map(|g| g.as_str().map(|s| s.trim().to_string())).collect()).unwrap_or_default();
            let matched: Vec<&DecayGroup> = groups.iter().filter(|g| gids.contains(&g.gid)).collect();
            let mut ev = parse_evidence(&it["evidence"]);
            // An issue whose own citations land on decay hits covers those groups even if it did not name them.
            let mut matched = matched;
            for g in &groups {
                if !matched.iter().any(|m| m.gid == g.gid) && g.hits.iter().any(|h| ev.iter().any(|e| e.path == h.path && e.start_line <= h.line + 1 && h.line <= e.end_line.max(e.start_line) + 1)) {
                    matched.push(g);
                }
            }
            for g in &matched {
                covered.insert(g.gid.clone());
                for h in g.hits.iter().take(2) {
                    ev.push(Evidence { path: h.path.clone(), start_line: h.line, end_line: h.line, quote: h.text.clone(), check: "unchecked".into() });
                }
            }
            evidence::check_all(&root, &mut ev);
            let issue = Issue {
                id: format!("iss_{}_{}", &call_id[5..13], i + 1),
                title: it["title"].as_str().unwrap_or("Untitled issue").chars().take(120).collect(),
                category: it["category"].as_str().unwrap_or("improvement").into(),
                severity: it["severity"].as_str().unwrap_or("medium").into(),
                diagnosis: it["diagnosis"].as_str().unwrap_or("").into(),
                evidence: ev,
                proposed_fix: it["proposed_fix"].as_str().unwrap_or("").into(),
                confidence: it["confidence"].as_str().unwrap_or("low").into(),
                source: "restorer".into(),
                status: "open".into(),
                pattern: matched.first().map(|g| g.pattern.clone()),
                reopened_note: None,
            };
            save_issue(&app, &id, &issue)?;
            created += 1;
        }
        // Decay the Restorer did not assess is still surfaced, labelled as engine-only findings.
        for g in groups.iter().filter(|g| !covered.contains(&g.gid)) {
            let linked = group_linked_to_failure(g, &baseline_output);
            let mut ev: Vec<Evidence> = g.hits.iter().take(3).map(|h| Evidence { path: h.path.clone(), start_line: h.line, end_line: h.line, quote: h.text.clone(), check: "unchecked".into() }).collect();
            evidence::check_all(&root, &mut ev);
            let issue = Issue {
                id: format!("iss_{}_{}", &call_id[5..13], g.gid.to_lowercase()),
                title: format!("{} ({} use{})", g.pattern, g.hits.len(), if g.hits.len() == 1 { "" } else { "s" }),
                category: if linked { "essential".into() } else { "improvement".into() },
                severity: if linked { "high".into() } else { "low".into() },
                diagnosis: format!("Static scan: {}. Removed in {}.{} The Restorer did not assess this group.", g.detail, g.removed_in, if linked { " The failing baseline output mentions it." } else { "" }),
                evidence: ev,
                proposed_fix: g.detail.clone(),
                confidence: if linked { "medium".into() } else { "low".into() },
                source: "decay-scan".into(),
                status: "open".into(),
                pattern: Some(g.pattern.clone()),
                reopened_note: None,
            };
            save_issue(&app, &id, &issue)?;
            created += 1;
        }
        app.db.put_artifact(&id, "inv:restorer", &json!({"call_id": call_id, "issues": created}))?;
        ctx.finish("restorer: diagnose", "done", format!("{created} issues ({} from the Restorer, {} from the decay scan only)", created - groups.iter().filter(|g| !covered.contains(&g.gid)).count(), groups.iter().filter(|g| !covered.contains(&g.gid)).count()));
    }
    let restorer_call = app.db.get_artifact::<Value>(&id, "inv:restorer")?.and_then(|v| v["call_id"].as_str().map(String::from));

    // 7. reviewer challenge
    let historian_claims = |h: &Value| -> Vec<Claim> {
        let mut out: Vec<Claim> = Vec::new();
        if let Ok(p) = serde_json::from_value::<Claim>(h["purpose"].clone()) {
            out.push(p);
        }
        for k in ["capabilities", "data_flows", "external_services"] {
            out.extend(serde_json::from_value::<Vec<Claim>>(h[k].clone()).unwrap_or_default());
        }
        out
    };
    let claims = historian_claims(&hist);
    ctx.begin("reviewer: challenge").await?;
    let review: Value = match app.db.get_artifact::<Value>(&id, "inv:review")? {
        Some(r) => {
            ctx.finish("reviewer: challenge", "skipped", "already done");
            r
        }
        None => {
            let issues: Vec<Issue> = issues_of(&app, &id).into_iter().filter(|i| i.status == "open").collect();
            let valid: BTreeSet<String> = claims.iter().map(|c| c.id.clone()).chain(issues.iter().map(|i| i.id.clone())).collect();
            let fmt_ev = |ev: &[Evidence]| -> String { ev.iter().map(|e| format!("      {}:{}-{} [engine check: {}] \"{}\"", e.path, e.start_line, e.end_line, e.check, redact(&truncate(&e.quote, 160)))).collect::<Vec<_>>().join("\n") };
            let mut challenges: Vec<Challenge> = Vec::new();
            let mut call_ids: Vec<String> = Vec::new();
            // Up to two passes: the second asks only about items the first pass skipped.
            for pass in 0..2 {
                let done: BTreeSet<String> = challenges.iter().map(|c| c.target.clone()).collect();
                let todo_claims: Vec<&Claim> = claims.iter().filter(|c| !done.contains(&c.id)).collect();
                let todo_issues: Vec<&Issue> = issues.iter().filter(|i| !done.contains(&i.id)).collect();
                if todo_claims.is_empty() && todo_issues.is_empty() {
                    break;
                }
                let n = agents::nonce();
                let mut body = String::from("CLAIMS from the Historian:\n");
                for c in &todo_claims {
                    body.push_str(&format!("- {} [{}] {}\n{}\n", c.id, c.status, c.text, fmt_ev(&c.evidence)));
                }
                body.push_str("\nISSUES from the Restorer and decay scan:\n");
                for i in &todo_issues {
                    body.push_str(&format!("- {} [{} / {} / {}] {}: {}\n{}\n", i.id, i.category, i.severity, i.source, i.title, truncate(&i.diagnosis, 400), fmt_ev(&i.evidence)));
                }
                let ids: Vec<&str> = todo_claims.iter().map(|c| c.id.as_str()).chain(todo_issues.iter().map(|i| i.id.as_str())).collect();
                let task = format!(
                    "Independently review your colleagues' findings. Return exactly one verdict for EACH of these ids: {}. Decide: agree (evidence supports it), disagree (evidence contradicts it or the category/severity is wrong), or insufficient_evidence. Pay special attention to citations whose engine check is not 'verified', to claims about the creator's intent, and to issues marked essential. Give a one-sentence reason each. Baseline verification: {}\n\n{}",
                    ids.join(", "),
                    baseline.as_ref().map(verification_brief).unwrap_or_else(|| "not run".into()),
                    agents::untrusted(&n, "FINDINGS (contain quotes from the repository)", &body)
                );
                let purpose = if pass == 0 { "Challenge historian claims and restorer diagnoses".to_string() } else { format!("Challenge the {} findings skipped in the first pass", ids.len()) };
                let spec = CallSpec { agent: "reviewer", purpose, inheritance_id: Some(&id), parent_id: plan_call.clone(), input_files: vec![], task, schema: agents::reviewer_challenge_schema(), max_tokens: 2000 };
                let (v, call_id) = app.agents.call_json(spec, &n).await?;
                call_ids.push(call_id);
                for c in v["challenges"].as_array().cloned().unwrap_or_default() {
                    let Some(target) = c["target"].as_str().map(|t| t.trim().to_string()) else { continue };
                    if valid.contains(&target) && !challenges.iter().any(|x| x.target == target) {
                        challenges.push(Challenge { target, verdict: c["verdict"].as_str().unwrap_or("insufficient_evidence").into(), reason: c["reason"].as_str().unwrap_or("").into() });
                    }
                }
            }
            let unassessed: Vec<String> = valid.iter().filter(|v| !challenges.iter().any(|c| &c.target == *v)).cloned().collect();
            let disagreements = challenges.iter().filter(|c| c.verdict != "agree").count();
            let out = json!({"call_id": call_ids.first(), "call_ids": call_ids, "challenges": challenges, "unassessed": unassessed});
            app.db.put_artifact(&id, "inv:review", &out)?;
            ctx.finish("reviewer: challenge", "done", format!("{} verdicts, {} disagreements or evidence gaps{}", challenges.len(), disagreements, if unassessed.is_empty() { String::new() } else { format!(", {} not assessed", unassessed.len()) }));
            out
        }
    };
    let challenges: Vec<Challenge> = serde_json::from_value(review["challenges"].clone()).unwrap_or_default();

    // 8. lead synthesis
    ctx.begin("lead: synthesis").await?;
    if app.db.get_artifact::<Report>(&id, "report")?.is_some() && app.db.get_artifact::<Value>(&id, "inv:synthesis")?.is_some() {
        ctx.finish("lead: synthesis", "skipped", "already done");
    } else {
        let n = agents::nonce();
        let issues: Vec<Issue> = issues_of(&app, &id).into_iter().filter(|i| i.status == "open").collect();
        let mut body = String::new();
        for c in &claims {
            body.push_str(&format!("- {} [{}] {}\n", c.id, c.status, c.text));
        }
        for i in &issues {
            body.push_str(&format!("- {} [{} / {}] {}\n", i.id, i.category, i.severity, i.title));
        }
        for ch in &challenges {
            body.push_str(&format!("- reviewer on {}: {} — {}\n", ch.target, ch.verdict, ch.reason));
        }
        let task = format!(
            "Write the investigation summary for the user who just inherited this project. In markdown (max ~250 words): what the creator built (only as far as evidence shows), how it works, which components are essential, its current health (use the verification result exactly; do not claim it works unless tests passed), the important problems, where the Reviewer disagreed (refer to the Reviewer and other specialists in the third person; you are the Lead), and what is unknown. Then list 2-4 concrete next actions.\n\nEngine facts:\n{}\n{}",
            facts_block(&row, &inv, baseline.as_ref()),
            agents::untrusted(&n, "TEAM FINDINGS (contain repository-derived text)", &body)
        );
        let spec = CallSpec { agent: "lead", purpose: "Synthesize the archaeological report".into(), inheritance_id: Some(&id), parent_id: plan_call.clone(), input_files: vec![], task, schema: agents::lead_synthesis_schema(), max_tokens: 1200 };
        let (v, call_id) = app.agents.call_json(spec, &n).await?;
        let purpose: Claim = serde_json::from_value(hist["purpose"].clone()).unwrap_or(Claim { id: "c_purpose".into(), text: "Unknown".into(), status: "unknown".into(), agent: "historian".into(), evidence: vec![] });
        let report = Report {
            lead_summary: v["summary_markdown"].as_str().unwrap_or("").to_string(),
            purpose,
            capabilities: serde_json::from_value(hist["capabilities"].clone()).unwrap_or_default(),
            components: serde_json::from_value(hist["components"].clone()).unwrap_or_default(),
            data_flows: serde_json::from_value(hist["data_flows"].clone()).unwrap_or_default(),
            external_services: serde_json::from_value(hist["external_services"].clone()).unwrap_or_default(),
            build_instructions: hist["build_instructions"].as_str().unwrap_or("").to_string(),
            unknowns: {
                let mut u: Vec<String> = serde_json::from_value(hist["unknowns"].clone()).unwrap_or_default();
                if let Some(un) = review["unassessed"].as_array().filter(|a| !a.is_empty()) {
                    u.push(format!("The Reviewer did not assess: {}", un.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", ")));
                }
                u
            },
            reviewer_challenges: challenges.clone(),
            next_actions: serde_json::from_value(v["next_actions"].clone()).unwrap_or_default(),
            generated_at: now(),
        };
        app.db.put_artifact(&id, "report", &report)?;
        app.db.put_artifact(&id, "inv:synthesis", &json!({"call_id": call_id}))?;
        let essential = issues.iter().filter(|i| i.category == "essential").count();
        let msg = ChatMessage {
            id: new_id("msg"),
            role: "lead".into(),
            content: report.lead_summary.clone(),
            at: now(),
            call_id: Some(call_id),
            suggested_action: Some(if essential > 0 { "restore_essential".into() } else { "none".into() }),
            via: Some("engine".into()),
        };
        app.db.put(Kind::Chat, &msg.id, Some(&id), &msg.at, &msg)?;
        let _ = restorer_call;
        app.set_stage(&id, Stage::Understanding)?;
        ctx.finish("lead: synthesis", "done", format!("report ready; {essential} essential issue(s)"));
    }
    Ok(())
}

// ---------- restore ----------

struct EditOutcome {
    files: Vec<String>,
}

/// Apply search/replace edits atomically to the successor. Nothing is written unless every edit applies.
fn apply_edits(root: &std::path::Path, edits: &[Value]) -> Result<EditOutcome, Vec<String>> {
    let mut errors = Vec::new();
    let mut staged: BTreeMap<String, String> = BTreeMap::new();
    if edits.is_empty() {
        return Err(vec!["no edits were proposed".into()]);
    }
    for (i, e) in edits.iter().enumerate().take(20) {
        let path = e["path"].as_str().unwrap_or("").trim().trim_start_matches("./").to_string();
        let find = e["find"].as_str().unwrap_or("");
        let replace = e["replace"].as_str().unwrap_or("");
        let all = e["all_occurrences"].as_bool().unwrap_or(false);
        let full = match safe_join(root, &path) {
            Ok(p) => p,
            Err(err) => {
                errors.push(format!("edit {}: {err}", i + 1));
                continue;
            }
        };
        if !full.is_file() {
            errors.push(format!("edit {}: '{path}' is not an existing file (creating files is not allowed)", i + 1));
            continue;
        }
        if find.is_empty() {
            errors.push(format!("edit {}: 'find' must not be empty", i + 1));
            continue;
        }
        let current = match staged.get(&path) {
            Some(s) => s.clone(),
            None => match std::fs::read_to_string(&full) {
                Ok(s) => s,
                Err(err) => {
                    errors.push(format!("edit {}: cannot read '{path}': {err}", i + 1));
                    continue;
                }
            },
        };
        let count = current.matches(find).count();
        if count == 0 {
            errors.push(format!("edit {}: the 'find' text was not found in '{path}' (it must match the file exactly, without line-number prefixes)", i + 1));
            continue;
        }
        if count > 1 && !all {
            errors.push(format!("edit {}: the 'find' text occurs {count} times in '{path}'; include more context or set all_occurrences", i + 1));
            continue;
        }
        let next = if all { current.replace(find, replace) } else { current.replacen(find, replace, 1) };
        staged.insert(path, next);
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    for (path, content) in &staged {
        if let Ok(full) = safe_join(root, path) {
            if let Err(e) = std::fs::write(&full, content) {
                return Err(vec![format!("write failed for {path}: {e}")]);
            }
        }
    }
    Ok(EditOutcome { files: staged.keys().cloned().collect() })
}

pub async fn restore(ctx: &mut JobCtx) -> anyhow::Result<()> {
    let app = ctx.app.clone();
    let id = ctx.id();
    let row = app.row(&id)?;
    let layout = app.layout(&id);
    let successor = layout.successor();
    let inv = inventory_of(&app, &id)?;
    let mode = ctx.job.params["mode"].as_str().unwrap_or("essential").to_string();
    let selected: Vec<String> = ctx.job.params["issue_ids"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();

    ctx.begin("plan").await?;
    let sev = |s: &str| match s {
        "high" => 0,
        "medium" => 1,
        _ => 2,
    };
    let mut todo: Vec<Issue> = issues_of(&app, &id)
        .into_iter()
        // Rejected issues are retried only when the user selects them explicitly.
        .filter(|i| matches!(i.status.as_str(), "open" | "failed") || (mode == "selected" && i.status == "rejected"))
        .filter(|i| match mode.as_str() {
            "fix_all" => true,
            "selected" => selected.contains(&i.id),
            _ => i.category == "essential",
        })
        .collect();
    todo.sort_by_key(|i| (i.category != "essential", sev(&i.severity)));
    if todo.is_empty() {
        ctx.finish("plan", "done", "no open issues match this request");
        return Ok(());
    }
    let step_names: Vec<String> = todo.iter().map(|i| format!("repair: {}", ellipsis(&i.title, 60))).collect();
    for s in &step_names {
        ctx.add_step(s);
    }
    ctx.add_step("final verification");
    ctx.add_step("inheritance notice");
    ctx.finish("plan", "done", format!("{} issue(s) queued ({mode})", todo.len()));
    app.set_stage(&id, Stage::Restoring)?;

    // Make sure there is a "before" measurement for the current successor state.
    let head = gitops::head(&successor)?;
    let mut current = match primary_verification(&app, &id) {
        Some(v) if v.workspace_rev.as_deref() == Some(head.as_str()) => Some(v),
        _ if exec_mode(&row) != ExecMode::None => Some(run_suite(&app, &id, "before restoration", None).await?),
        _ => primary_verification(&app, &id),
    };

    for (issue, step) in todo.into_iter().zip(step_names) {
        if ctx.step_status(&step).as_deref() == Some("done") {
            continue;
        }
        ctx.begin(&step).await?;
        // A repair job may be resumed; never stack edits on uncommitted changes.
        gitops::revert_worktree(&successor)?;
        if let Some(by) = resolved_by_rescan(&app, &id, &inv, &issue) {
            let mut issue = issue;
            issue.status = "repaired".into();
            issue.diagnosis.push_str(&format!("\n\nResolved by an earlier repair: {by}"));
            save_issue(&app, &id, &issue)?;
            ctx.finish(&step, "skipped", format!("already resolved — {by}"));
            continue;
        }
        let (status, detail, after) = repair_issue(&app, &id, &inv, issue, current.clone(), ctx).await?;
        if let Some(a) = after {
            current = Some(a);
        }
        ctx.finish(&step, if status == "applied" { "done" } else { "failed" }, detail);
    }

    // Fixing one problem often uncovers the next (e.g. an import error hides later failures).
    // While verification still fails, let the Restorer diagnose the new output and repair again.
    if mode != "selected" && exec_mode(&row) != ExecMode::None {
        for round in 1..=3 {
            let Some(cur) = current.clone() else { break };
            if !matches!(cur.status.as_str(), "failed" | "error") || cur.output_tail.is_empty() {
                break;
            }
            let step = format!("restorer: re-diagnose (round {round})");
            ctx.begin(&step).await?;
            let new_issues = rediagnose(&app, &id, &inv, &cur).await?;
            let wanted: Vec<Issue> = new_issues.into_iter().filter(|i| mode == "fix_all" || i.category == "essential").collect();
            if wanted.is_empty() {
                ctx.finish(&step, "done", "no new essential problem identified from the failing output");
                break;
            }
            ctx.finish(&step, "done", format!("{} new issue(s) found in the failing output", wanted.len()));
            for issue in wanted {
                let rstep = format!("repair: {}", ellipsis(&issue.title, 60));
                ctx.begin(&rstep).await?;
                gitops::revert_worktree(&successor)?;
                let (status, detail, after) = repair_issue(&app, &id, &inv, issue, current.clone(), ctx).await?;
                if let Some(a) = after {
                    current = Some(a);
                }
                ctx.finish(&rstep, if status == "applied" { "done" } else { "failed" }, detail);
            }
        }
    }

    // Issues resolved as a side effect of other repairs are closed by a static re-scan, not left open.
    for mut i in issues_of(&app, &id).into_iter().filter(|i| matches!(i.status.as_str(), "open" | "failed")) {
        if let Some(by) = resolved_by_rescan(&app, &id, &inv, &i) {
            i.status = "repaired".into();
            i.diagnosis.push_str(&format!("\n\nResolved by an earlier repair: {by}"));
            save_issue(&app, &id, &i)?;
        }
    }

    ctx.begin("final verification").await?;
    let final_v = if exec_mode(&row) != ExecMode::None { Some(run_suite(&app, &id, "after restoration", None).await?) } else { None };
    let passed = final_v.as_ref().map(|v| v.status == "passed").unwrap_or(false);
    // A finding the inheritor reopened outranks green tests until it is repaired.
    let human_open: Vec<String> = issues_of(&app, &id).into_iter().filter(|i| i.reopened_note.is_some() && i.status != "repaired").map(|i| i.title).collect();
    let mut detail = final_v.as_ref().map(verification_brief).unwrap_or_else(|| "not run (execution disabled)".into());
    if passed && !human_open.is_empty() {
        detail.push_str(&format!(" — not marked verified: the inheritor's reopened issue is unresolved ({})", human_open.join("; ")));
    }
    ctx.finish("final verification", "done", detail);
    app.set_stage(&id, if passed && human_open.is_empty() { Stage::Verified } else { Stage::Restoring })?;

    ctx.begin("inheritance notice").await?;
    let detail = write_inheritance_notice(&app, &id)?;
    ctx.finish("inheritance notice", "done", detail);
    Ok(())
}

async fn repair_issue(app: &App, id: &str, inv: &Inventory, mut issue: Issue, before: Option<Verification>, ctx: &mut JobCtx) -> anyhow::Result<(String, String, Option<Verification>)> {
    let successor = app.layout(id).successor();
    let row = app.row(id)?;
    let before_out = before.as_ref().map(|v| v.output_tail.clone()).unwrap_or_default();
    let mut focus: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for e in &issue.evidence {
        if file_exists(app, id, &e.path) {
            focus.entry(e.path.clone()).or_default().push(e.start_line);
        }
    }
    if let Some(p) = &issue.pattern {
        for h in inv.decay.iter().filter(|h| &h.pattern == p) {
            focus.entry(h.path.clone()).or_default().push(h.line);
        }
    }
    for (p, l) in paths_in_output(app, id, &before_out) {
        if focus.len() < 4 || focus.contains_key(&p) {
            focus.entry(p).or_default().push(l);
        }
    }
    let files: Vec<(String, Vec<usize>)> = focus.into_iter().take(4).collect();
    let mut repair = Repair {
        id: new_id("rep"),
        issue_id: issue.id.clone(),
        status: "failed".into(),
        rationale: String::new(),
        diff: String::new(),
        files: vec![],
        commit: None,
        review: None,
        verification_before: before.as_ref().map(|v| v.id.clone()),
        verification_after: None,
        note: None,
        created_at: now(),
        attempt: 0,
        probe: None,
        replay_of: None,
    };
    let mut feedback = String::new();
    let mut parent: Option<String> = None;
    for attempt in 0..2u32 {
        repair.attempt = attempt + 1;
        ctx.checkpoint().await?;
        let n = agents::nonce();
        let pack = pack_files(app, id, &n, &files, app.cfg.prompt_budget_chars * 3 / 4);
        let task = format!(
            "Repair this issue in the successor copy (the original is preserved separately).\n\nIssue {}: {}\nCategory: {} / severity {}\nDiagnosis: {}\nProposed fix: {}{}\n\nCurrent verification: {}\n{}{}\n{}\nReturn search/replace edits. Each 'find' must be copied EXACTLY from the file content (without the `N| ` line-number prefixes), include enough context to be unique, or set all_occurrences=true for a mechanical rename. Change only what is needed to fix the root cause; keep the original behaviour and style. Do not create new files.",
            issue.id,
            issue.title,
            issue.category,
            issue.severity,
            redact(&issue.diagnosis),
            redact(&issue.proposed_fix),
            issue.reopened_note.as_ref().map(|note| format!("\n\nThe inheritor (a human) reopened this issue after an earlier repair. Their finding takes priority over passing tests:\n{}", agents::untrusted(&n, "INHERITOR NOTE", note))).unwrap_or_default(),
            before.as_ref().map(verification_brief).unwrap_or_else(|| "not run".into()),
            if before_out.is_empty() { String::new() } else { agents::untrusted(&n, "verification output (tail)", &tail_lines(&before_out, 60)) },
            pack.text,
            feedback
        );
        let spec = CallSpec { agent: "restorer", purpose: format!("Patch: {}", ellipsis(&issue.title, 80)), inheritance_id: Some(id), parent_id: parent.clone(), input_files: pack.files.clone(), task, schema: agents::restorer_patch_schema(), max_tokens: 2500 };
        let (v, patch_call) = match app.agents.call_json(spec, &n).await {
            Ok(r) => r,
            Err(e) => {
                repair.note = Some(format!("Restorer call failed: {e:#}"));
                break;
            }
        };
        parent = Some(patch_call.clone());
        repair.rationale = v["rationale"].as_str().unwrap_or("").to_string();
        let edits = v["edits"].as_array().cloned().unwrap_or_default();
        let applied = match apply_edits(&successor, &edits) {
            Ok(a) => a,
            Err(errs) => {
                gitops::revert_worktree(&successor)?;
                feedback = format!("\nYour previous edits could not be applied:\n- {}\nFix them.", errs.join("\n- "));
                repair.note = Some(format!("edits did not apply: {}", errs.join("; ")));
                continue;
            }
        };
        let diff = gitops::worktree_diff(&successor)?;
        if diff.trim().is_empty() {
            feedback = "\nYour previous edits produced no change. Propose edits that actually change the code.".into();
            repair.note = Some("edits produced no change".into());
            continue;
        }
        repair.files = applied.files.clone();
        repair.diff = truncate(&diff, 60_000);

        // Deterministic guard: an import the diff removed must not still be used in that file.
        let dangling = dangling_names(&successor, &diff);
        if !dangling.is_empty() {
            let why = dangling.iter().map(|(p, name, line)| format!("'{name}' is no longer imported in {p} but is still used at line {line}")).collect::<Vec<_>>().join("; ");
            gitops::revert_worktree(&successor)?;
            repair.status = "reverted".into();
            repair.note = Some(format!("Engine check before review: {why}; change reverted."));
            app.db.put(Kind::Repair, &repair.id, Some(id), &repair.created_at, &repair)?;
            if attempt == 0 {
                feedback = format!("\nYour previous edits were reverted by the engine before review: {why}. Keep every name the file still uses importable, and update all its uses if you rename an API.");
                repair.id = new_id("rep");
                repair.created_at = now();
                continue;
            }
            issue.status = "failed".into();
            save_issue(app, id, &issue)?;
            return Ok(("reverted".into(), why, before.clone()));
        }

        // Independent review of the actual diff.
        let rn = agents::nonce();
        let rtask = format!(
            "Review this repair independently and answer each check honestly from the diff and evidence. Approve only if the problem is real, the diff fixes its root cause, and original behaviour is preserved. Check that every name the diff imports or calls really exists where the diff says it does. In 'concerns', write specific problems you found in THIS diff (empty list if none); do not restate the checks.\n\nIssue {}: {}\nDiagnosis: {}\nRestorer's rationale: {}\nVerification before the change: {}\n\n{}{}",
            issue.id,
            issue.title,
            redact(&issue.diagnosis),
            redact(&repair.rationale),
            before.as_ref().map(verification_brief).unwrap_or_else(|| "not run".into()),
            agents::untrusted(&rn, "DIFF (successor working tree)", &redact(&truncate(&diff, 14_000))),
            if before_out.is_empty() { String::new() } else { agents::untrusted(&rn, "verification output before (tail)", &tail_lines(&before_out, 40)) },
        );
        let rspec = CallSpec { agent: "reviewer", purpose: format!("Review repair: {}", ellipsis(&issue.title, 80)), inheritance_id: Some(id), parent_id: Some(patch_call.clone()), input_files: applied.files.clone(), task: rtask, schema: agents::reviewer_repair_schema(), max_tokens: 900 };
        let (rv, review_call) = match app.agents.call_json(rspec, &rn).await {
            Ok(r) => r,
            Err(e) => {
                gitops::revert_worktree(&successor)?;
                repair.note = Some(format!("Reviewer call failed, change reverted: {e:#}"));
                break;
            }
        };
        let concerns: Vec<String> = serde_json::from_value::<Vec<String>>(rv["concerns"].clone())
            .unwrap_or_default()
            .into_iter()
            .filter(|c| !c.trim().is_empty() && !c.trim_end().ends_with('?'))
            .collect();
        let mut review = Review {
            verdict: rv["verdict"].as_str().unwrap_or("reject").into(),
            problem_is_real: rv["problem_is_real"].as_bool(),
            evidence_supports_diagnosis: rv["evidence_supports_diagnosis"].as_bool(),
            preserves_original_behaviour: rv["preserves_original_behaviour"].as_bool(),
            tests_exercise_change: rv["tests_exercise_change"].as_bool(),
            root_cause_addressed: rv["root_cause_addressed"].as_bool().unwrap_or(false),
            regression_risk: rv["regression_risk"].as_str().unwrap_or("high").into(),
            concerns,
            call_id: Some(review_call),
        };
        // An approval that contradicts the reviewer's own checks is not an approval.
        if review.verdict == "approve" && (review.problem_is_real == Some(false) || !review.root_cause_addressed || review.preserves_original_behaviour == Some(false)) {
            review.verdict = "request_changes".into();
            review.concerns.push("Engine: verdict was 'approve' but the reviewer's own checks failed (problem real / root cause / behaviour preserved).".into());
        }
        repair.review = Some(review.clone());
        if review.verdict != "approve" {
            gitops::revert_worktree(&successor)?;
            // One more attempt with the reviewer's concerns, whether it asked for changes or rejected outright.
            if attempt == 0 && !review.concerns.is_empty() {
                feedback = format!("\nThe Reviewer requested changes to your previous attempt:\n- {}\nAddress these concerns.", review.concerns.join("\n- "));
                repair.status = "rejected".into();
                repair.note = Some("Reviewer requested changes; retrying once".into());
                app.db.put(Kind::Repair, &repair.id, Some(id), &repair.created_at, &repair)?;
                repair.id = new_id("rep");
                repair.created_at = now();
                continue;
            }
            repair.status = "rejected".into();
            repair.note = Some(format!("Reviewer verdict '{}'; change reverted", review.verdict));
            app.db.put(Kind::Repair, &repair.id, Some(id), &repair.created_at, &repair)?;
            issue.status = "rejected".into();
            save_issue(app, id, &issue)?;
            return Ok(("rejected".into(), format!("Reviewer: {} — {}", review.verdict, review.concerns.join("; ")), None));
        }

        // Verify the change for real before keeping it.
        let after = if exec_mode(&row) != ExecMode::None { Some(run_suite(app, id, &format!("after {}", ellipsis(&issue.title, 40)), None).await?) } else { None };
        let mut progress_note: Option<String> = None;
        if let Some(a) = &after {
            repair.verification_after = Some(a.id.clone());
            let b_sum = before.as_ref().and_then(|b| b.summary.clone());
            let b_status = before.as_ref().map(|b| b.status.clone()).unwrap_or_default();
            let regressed = verify::regressed(b_sum.as_ref(), a.summary.as_ref(), &b_status, &a.status);
            let improved = verify::improved(b_sum.as_ref(), a.summary.as_ref(), &b_status, &a.status);
            let changed = changed_lines(&diff);
            let culprit = verify::innermost_frames(&a.output_tail).into_iter().find(|(p, l)| changed.iter().any(|(cp, lo, hi)| cp == p && *l + 2 >= *lo && *l <= hi + 2));
            let verdict = if regressed {
                Some(format!("verification got worse ({b_status} → {})", a.status))
            } else if !improved && a.status != "passed" {
                culprit.as_ref().map(|(p, l)| format!("the failure now originates in the edited code ({p}:{l})"))
            } else {
                None
            };
            if let Some(why) = verdict {
                gitops::revert_worktree(&successor)?;
                repair.status = "reverted".into();
                repair.note = Some(format!("Engine check: {why}; change reverted."));
                app.db.put(Kind::Repair, &repair.id, Some(id), &repair.created_at, &repair)?;
                if attempt == 0 {
                    feedback = format!(
                        "\nYour previous attempt was applied, tested, and reverted because {why}. Output after that attempt:\n{}\nPropose a corrected repair.",
                        agents::untrusted(&n, "verification output after previous attempt", &tail_lines(&a.output_tail, 30))
                    );
                    repair.id = new_id("rep");
                    repair.created_at = now();
                    repair.verification_after = None;
                    continue;
                }
                issue.status = "failed".into();
                save_issue(app, id, &issue)?;
                return Ok(("reverted".into(), format!("{why}; reverted"), before.clone()));
            }
            if !improved && b_status != "passed" {
                let was = verify::innermost_frames(&before_out).last().map(|(p, l)| format!("{p}:{l}"));
                let now_at = verify::innermost_frames(&a.output_tail).last().map(|(p, l)| format!("{p}:{l}"));
                progress_note = Some(match (was, now_at) {
                    (Some(w), Some(n2)) if w != n2 => format!("Test counts unchanged, but the failure moved from {w} to {n2} (outside the edited lines)."),
                    _ => "No measurable change in verification; kept because the reviewer approved and nothing regressed.".into(),
                });
            }
        }
        // Tests only prove what they exercise: compare behaviour against the original in its own era.
        if after.as_ref().map(|a| a.status == "passed" || verify::improved(before.as_ref().and_then(|b| b.summary.as_ref()), a.summary.as_ref(), before.as_ref().map(|b| b.status.as_str()).unwrap_or(""), &a.status)).unwrap_or(false) {
            match crate::probe::run(app, id, &issue, &diff, &repair.files, repair.review.as_ref().and_then(|r| r.call_id.clone())).await {
                Ok(Some(report)) => {
                    let changed = report.verdict == "changed";
                    repair.probe = Some(report.clone());
                    if changed {
                        gitops::revert_worktree(&successor)?;
                        repair.status = "reverted".into();
                        repair.note = Some(format!("Behaviour probe: the successor ({}) no longer behaves like the original ({}) on code the tests do not cover; change reverted.", report.modern_image, report.legacy_image));
                        app.db.put(Kind::Repair, &repair.id, Some(id), &repair.created_at, &repair)?;
                        if attempt == 0 {
                            feedback = format!(
                                "\nYour previous attempt passed the tests but was reverted: behaviour probes show it changes what the original code does.\n{}\nPropose a repair that keeps the original behaviour on the modern runtime.",
                                agents::untrusted(&n, "behaviour probe results", &crate::probe::feedback(&report))
                            );
                            repair.id = new_id("rep");
                            repair.created_at = now();
                            repair.verification_after = None;
                            repair.probe = None;
                            continue;
                        }
                        issue.status = "failed".into();
                        save_issue(app, id, &issue)?;
                        let restored = if exec_mode(&row) != ExecMode::None { Some(run_suite(app, id, "after revert", None).await?) } else { None };
                        return Ok(("reverted".into(), "behaviour probe showed changed behaviour; reverted".into(), restored));
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    repair.probe = Some(ProbeReport { tester_call_id: None, legacy_image: String::new(), modern_image: String::new(), probes: vec![], verdict: "inconclusive".into(), note: Some(format!("probe could not run: {e:#}")) });
                }
            }
        }
        let summary_line = |v: Option<&Verification>| v.map(|v| match &v.summary {
            Some(s) => format!("{} ({} passed, {} failed, {} errors)", v.status, s.passed, s.failed, s.errors),
            None => v.status.clone(),
        }).unwrap_or_else(|| "not run".into());
        let message = format!(
            "Restore: {}\n\n{}\n\nIssue: {}\nReviewed-by: Reviewer agent ({}) - verdict approve, regression risk {}\nVerification: {} -> {}{}\nInheritance-Engine-Repair: {}\nOriginal-Commit: {}",
            ellipsis(&issue.title, 72),
            truncate(&repair.rationale, 600),
            issue.id,
            app.cfg.model_for("reviewer"),
            review.regression_risk,
            summary_line(before.as_ref()),
            summary_line(after.as_ref()),
            repair.probe.as_ref().map(|p| format!("\nBehaviour-Probe: {} ({} vs original on {})", p.verdict, p.modern_image, p.legacy_image)).unwrap_or_default(),
            repair.id,
            row.original_commit.as_deref().unwrap_or("?"),
        );
        let sha = gitops::commit_all(&successor, &message)?;
        repair.commit = Some(sha.clone());
        repair.status = "applied".into();
        repair.note = if after.is_none() { Some("Committed without verification because execution is disabled; treat as unverified.".into()) } else { progress_note };
        app.db.put(Kind::Repair, &repair.id, Some(id), &repair.created_at, &repair)?;
        issue.status = "repaired".into();
        save_issue(app, id, &issue)?;
        let probe_note = repair.probe.as_ref().map(|p| format!("; behaviour probe {} vs {}: {}", p.modern_image, p.legacy_image, p.verdict)).unwrap_or_default();
        let detail = format!("committed {} — {}{probe_note}", &sha[..10], summary_line(after.as_ref()));
        return Ok(("applied".into(), detail, after));
    }
    gitops::revert_worktree(&successor)?;
    app.db.put(Kind::Repair, &repair.id, Some(id), &repair.created_at, &repair)?;
    issue.status = "failed".into();
    save_issue(app, id, &issue)?;
    Ok(("failed".into(), repair.note.clone().unwrap_or_else(|| "repair failed".into()), None))
}

/// Python names whose import a diff removed but which the edited file still uses: (path, name, line).
pub(crate) fn dangling_names(root: &std::path::Path, diff: &str) -> Vec<(String, String, usize)> {
    let from_imp = regex::Regex::new(r"^\s*from\s+[\w.]+\s+import\s+\(?([^#]+)").unwrap();
    let plain_imp = regex::Regex::new(r"^\s*import\s+([^#]+)").unwrap();
    let names_of = |line: &str| -> Vec<String> {
        let list = if let Some(c) = from_imp.captures(line) { c[1].to_string() } else if let Some(c) = plain_imp.captures(line) { c[1].to_string() } else { return vec![] };
        list.replace([')', '\\'], " ")
            .split(',')
            .filter_map(|n| {
                let parts: Vec<&str> = n.split_whitespace().collect();
                let bound = if parts.len() == 3 && parts[1] == "as" { parts[2] } else { *parts.first()? };
                let bound = bound.split('.').next()?;
                (!bound.is_empty() && bound != "*").then(|| bound.to_string())
            })
            .collect()
    };
    let mut per_file: BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)> = BTreeMap::new();
    let mut file = String::new();
    for line in diff.lines() {
        if let Some(p) = line.strip_prefix("+++ b/") {
            file = p.to_string();
            continue;
        }
        if line.starts_with("---") || line.starts_with("+++") || !file.ends_with(".py") {
            continue;
        }
        if let Some(l) = line.strip_prefix('-') {
            per_file.entry(file.clone()).or_default().0.extend(names_of(l));
        } else if let Some(l) = line.strip_prefix('+') {
            per_file.entry(file.clone()).or_default().1.extend(names_of(l));
        }
    }
    let mut out = Vec::new();
    for (path, (removed, added)) in per_file {
        let Ok(full) = safe_join(root, &path) else { continue };
        let Ok(text) = std::fs::read_to_string(full) else { continue };
        let still_imported: BTreeSet<String> = text.lines().flat_map(names_of).collect();
        for name in removed.difference(&added).filter(|n| !still_imported.contains(*n)) {
            let word = regex::Regex::new(&format!(r"\b{}\b", regex::escape(name))).unwrap();
            if let Some((i, _)) = text.lines().enumerate().find(|(_, l)| {
                let t = l.trim_start();
                !t.starts_with('#') && !t.starts_with("import ") && !t.starts_with("from ") && word.is_match(l)
            }) {
                out.push((path.clone(), name.clone(), i + 1));
            }
        }
    }
    out
}

/// For decay issues: re-scan the affected files as they are now. If no use of the pattern remains,
/// an earlier repair already resolved it.
fn resolved_by_rescan(app: &App, id: &str, inv: &Inventory, issue: &Issue) -> Option<String> {
    // A human reopened it: the static scan being clean is exactly what they disagreed with.
    if issue.reopened_note.is_some() {
        return None;
    }
    let pattern = issue.pattern.as_ref()?;
    let paths: BTreeSet<&str> = inv.decay.iter().filter(|h| &h.pattern == pattern).map(|h| h.path.as_str()).collect();
    if paths.is_empty() {
        return None;
    }
    let root = app.layout(id).successor();
    let files: Vec<inventory::SourceFile> = paths
        .iter()
        .filter_map(|p| {
            let text = std::fs::read_to_string(safe_join(&root, p).ok()?).ok()?;
            Some(inventory::SourceFile { path: p.to_string(), size: text.len() as u64, lang: inventory::language_of(p), text: Some(text) })
        })
        .collect();
    let remaining = inventory::scan_decay(&files).into_iter().filter(|h| &h.pattern == pattern).count();
    (remaining == 0).then(|| format!("a static re-scan of {} finds no remaining '{}'", paths.into_iter().collect::<Vec<_>>().join(", "), pattern))
}

/// (path, first_line, last_line) of the new-side hunks in a unified diff.
pub(crate) fn changed_lines(diff: &str) -> Vec<(String, usize, usize)> {
    let hunk = regex::Regex::new(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@").unwrap();
    let mut out = Vec::new();
    let mut file = String::new();
    for line in diff.lines() {
        if let Some(p) = line.strip_prefix("+++ b/") {
            file = p.to_string();
        } else if let Some(c) = hunk.captures(line) {
            let start: usize = c[1].parse().unwrap_or(1);
            let len: usize = c.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(1);
            out.push((file.clone(), start, start + len.max(1) - 1));
        }
    }
    out
}

/// Diagnose a still-failing verification after earlier repairs. Creates and returns new open issues.
async fn rediagnose(app: &App, id: &str, inv: &Inventory, failing: &Verification) -> anyhow::Result<Vec<Issue>> {
    let n = agents::nonce();
    let mut focus: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (p, l) in paths_in_output(app, id, &failing.output_tail) {
        focus.entry(p).or_default().push(l);
    }
    let repaired: BTreeSet<String> = issues_of(app, id).into_iter().filter(|i| i.status == "repaired").filter_map(|i| i.pattern).collect();
    let groups: Vec<DecayGroup> = decay_groups(inv).into_iter().filter(|g| !repaired.contains(&g.pattern)).collect();
    for g in groups.iter().filter(|g| group_linked_to_failure(g, &failing.output_tail)) {
        for h in &g.hits {
            focus.entry(h.path.clone()).or_default().push(h.line);
        }
    }
    let files: Vec<(String, Vec<usize>)> = focus.into_iter().take(4).collect();
    let pack = pack_files(app, id, &n, &files, app.cfg.prompt_budget_chars / 2);
    let groups_text: Vec<String> = groups
        .iter()
        .map(|g| format!("- {} \"{}\": {} hits, e.g. {}:{} — {} (removed in {}). Mentioned in the failing output: {}.", g.gid, g.pattern, g.hits.len(), g.hits[0].path, g.hits[0].line, g.detail, g.removed_in, if group_linked_to_failure(g, &failing.output_tail) { "yes" } else { "no" }))
        .collect();
    let recent: Vec<String> = app.db.list::<Repair>(Kind::Repair, Some(id))?.iter().filter(|r| r.status == "applied").map(|r| format!("- {} changed {}", r.commit.as_deref().map(|c| &c[..10]).unwrap_or("?"), r.files.join(", "))).collect();
    let task = format!(
        "Earlier repairs were applied and kept:\n{}\n\nVerification still does not pass: {}\n\nRemaining static decay groups:\n{}\n\n{}{}\nDiagnose the problem(s) the failing output shows NOW. Only report issues the output or cited code demonstrates; mark them essential when they stop the tests from running or passing. Reference decay group ids when relevant.",
        if recent.is_empty() { "- none".into() } else { recent.join("\n") },
        verification_brief(failing),
        if groups_text.is_empty() { "- none".into() } else { groups_text.join("\n") },
        agents::untrusted(&n, "verification output (tail)", &tail_lines(&failing.output_tail, 80)),
        pack.text,
    );
    let spec = CallSpec { agent: "restorer", purpose: "Re-diagnose remaining failure".into(), inheritance_id: Some(id), parent_id: None, input_files: pack.files.clone(), task, schema: agents::restorer_diagnose_schema(), max_tokens: 1800 };
    let (v, call_id) = app.agents.call_json(spec, &n).await?;
    let root = app.layout(id).successor();
    let mut out = Vec::new();
    for (i, it) in v["issues"].as_array().cloned().unwrap_or_default().iter().take(4).enumerate() {
        let gids: Vec<String> = it["decay_groups"].as_array().map(|a| a.iter().filter_map(|g| g.as_str().map(|s| s.trim().to_string())).collect()).unwrap_or_default();
        let matched: Vec<&DecayGroup> = groups.iter().filter(|g| gids.contains(&g.gid)).collect();
        let mut ev = parse_evidence(&it["evidence"]);
        for g in &matched {
            for h in g.hits.iter().take(2) {
                ev.push(Evidence { path: h.path.clone(), start_line: h.line, end_line: h.line, quote: h.text.clone(), check: "unchecked".into() });
            }
        }
        evidence::check_all(&root, &mut ev);
        // Superseded open issues for the same decay pattern are folded into this one.
        if let Some(g) = matched.first() {
            for mut old in issues_of(app, id).into_iter().filter(|o| o.status == "open" && o.pattern.as_deref() == Some(g.pattern.as_str())) {
                old.status = "skipped".into();
                save_issue(app, id, &old)?;
            }
        }
        let issue = Issue {
            id: format!("iss_{}_{}", &call_id[5..13], i + 1),
            title: it["title"].as_str().unwrap_or("Untitled issue").chars().take(120).collect(),
            category: it["category"].as_str().unwrap_or("essential").into(),
            severity: it["severity"].as_str().unwrap_or("high").into(),
            diagnosis: it["diagnosis"].as_str().unwrap_or("").into(),
            evidence: ev,
            proposed_fix: it["proposed_fix"].as_str().unwrap_or("").into(),
            confidence: it["confidence"].as_str().unwrap_or("low").into(),
            source: "verification".into(),
            status: "open".into(),
            pattern: matched.first().map(|g| g.pattern.clone()),
            reopened_note: None,
        };
        save_issue(app, id, &issue)?;
        out.push(issue);
    }
    Ok(out)
}

/// Write INHERITANCE.md into the successor: attribution, provenance and lineage. Deterministic.
fn write_inheritance_notice(app: &App, id: &str) -> anyhow::Result<String> {
    let row = app.row(id)?;
    let successor = app.layout(id).successor();
    let base = row.original_commit.clone().unwrap_or_default();
    let repairs: Vec<Repair> = app.db.list::<Repair>(Kind::Repair, Some(id))?.into_iter().filter(|r| r.status == "applied").collect();
    if repairs.is_empty() {
        return Ok("no applied repairs; notice not written".into());
    }
    let issues = issues_of(app, id);
    let inv = inventory_of(app, id).ok();
    let authors = inv.as_ref().map(|i| i.git.authors.iter().take(5).map(|a| format!("{} ({} commits)", a.name, a.commits)).collect::<Vec<_>>().join(", ")).unwrap_or_default();
    let mut md = format!(
        "# Inheritance notice\n\nThis is a **successor** of [{full}](https://github.com/{full}), maintained independently.\nThe lineage below records where the code came from; it does not imply endorsement by the original authors.\n\n## Original\n\n- Source: https://github.com/{full}\n- Original commit: `{base}` (branch `{branch}`)\n- License (as reported/detected): {license}\n- Original authors (from git history): {authors}\n\nAll original work remains the work of its authors under its original license. This file does not change that license.\n\n## Restoration\n\n",
        full = row.full_name,
        base = base,
        branch = row.original_branch.as_deref().unwrap_or("?"),
        license = row.license.as_deref().unwrap_or("not detected"),
        authors = if authors.is_empty() { "unknown".into() } else { authors },
    );
    for r in &repairs {
        let title = issues.iter().find(|i| i.id == r.issue_id).map(|i| i.title.clone()).unwrap_or_default();
        let after = r.verification_after.as_deref().and_then(|v| app.db.get::<Verification>(Kind::Verification, v).ok().flatten());
        md.push_str(&format!(
            "- `{}` {} — reviewed ({}), verification: {}\n",
            r.commit.as_deref().map(|c| &c[..10]).unwrap_or("?"),
            title,
            r.review.as_ref().map(|v| v.verdict.as_str()).unwrap_or("none"),
            after.map(|a| format!("{} via `{}` in {}", a.status, a.command.lines().next().unwrap_or(""), a.environment)).unwrap_or_else(|| "not run".into())
        ));
    }
    md.push_str("\nRepairs were proposed by AI agents running a local open-weight model, independently reviewed by a second agent, executed in an isolated environment, and kept only when verification did not regress. Details are in the exported Inheritance Record.\n");
    let path = successor.join("INHERITANCE.md");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    if existing == md {
        return Ok("notice already up to date".into());
    }
    if successor.join("INHERITANCE.md").exists() && gitops::show_original(&app.layout(id), &base, "INHERITANCE.md").is_ok() {
        return Ok("the original already has an INHERITANCE.md; not overwritten".into());
    }
    std::fs::write(&path, md)?;
    let sha = gitops::commit_all(&successor, "docs: record inheritance lineage and attribution\n\nGenerated by Inheritance Engine from recorded provenance.")?;
    Ok(format!("INHERITANCE.md committed as {}", &sha[..10]))
}

// ---------- verify ----------

pub async fn verify_job(ctx: &mut JobCtx) -> anyhow::Result<()> {
    let app = ctx.app.clone();
    let id = ctx.id();
    ctx.begin("verify").await?;
    let custom = ctx.job.params["command"].as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let v = run_suite(&app, &id, if custom.is_some() { "manual" } else { "check" }, custom.as_deref()).await?;
    ctx.finish("verify", "done", verification_brief(&v));
    let human_open = issues_of(&app, &id).iter().any(|i| i.reopened_note.is_some() && i.status != "repaired");
    if v.status == "passed" && !human_open {
        app.set_stage(&id, Stage::Verified)?;
    }
    Ok(())
}

// ---------- web research (SerpApi + Gemma, engine-checked citations) ----------

pub const RESEARCH_STEP: &str = "web research: related projects and documentation";

fn research_queries(row: &InheritanceRow) -> Vec<String> {
    let topic = crate::hunter::sanitize_keywords(row.github.description.as_deref().unwrap_or(""));
    let topic: String = topic.split_whitespace().take(4).collect::<Vec<_>>().join(" ");
    vec![
        format!("\"{}\"", row.full_name),
        format!("{} {}", row.name, topic).trim().to_string(),
        format!("{} {} documentation", row.name, row.github.owner),
    ]
}

/// Keeps only items that cite results which really exist, drops invalid ids, attaches a source URL (a documentation
/// page when one is cited, for documentation items), and drops items that repeat an earlier item's source.
pub fn check_research(v: &Value, results: &[crate::websearch::WebResult], own_repo: &str) -> Value {
    let url_of = |cid: &str| results.iter().find(|r| r.id == cid).map(|r| r.url.clone());
    let docs_url = |cites: &[String]| cites.iter().find_map(|c| results.iter().find(|r| &r.id == c && r.kind == "docs")).map(|r| r.url.clone());
    let valid = |a: &Value| -> Vec<String> { a.as_array().map(|a| a.as_slice()).unwrap_or_default().iter().filter_map(|c| c.as_str()).map(|c| c.trim().trim_matches(|ch| ch == '[' || ch == ']').to_string()).filter(|c| url_of(c).is_some()).collect() };
    let mut dropped = 0;
    let mut keep = |items: &Value, docs: bool| -> Vec<Value> {
        let mut seen = BTreeSet::new();
        items.as_array().map(|a| a.as_slice()).unwrap_or_default().iter().filter_map(|it| {
            let cites = valid(&it["cites"]);
            if cites.is_empty() {
                dropped += 1;
                return None;
            }
            let url = if docs { docs_url(&cites).or_else(|| url_of(&cites[0])) } else { url_of(&cites[0]) };
            if !seen.insert(url.clone()) {
                dropped += 1;
                return None;
            }
            let mut it = it.clone();
            // The model sometimes calls the inherited repository itself a fork; the engine knows better.
            let is_own = results.iter().any(|r| r.id == cites[0] && r.repo.as_deref().is_some_and(|x| x.eq_ignore_ascii_case(own_repo)));
            if !docs && is_own {
                it["relation"] = json!("original");
            }
            it["url"] = json!(url);
            it["cites"] = json!(cites);
            Some(it)
        }).collect()
    };
    let related = keep(&v["related"], false);
    let documentation = keep(&v["documentation"], true);
    let summary_cites = valid(&v["summary_cites"]);
    json!({
        "summary": v["summary"],
        "summary_cites": summary_cites,
        "summary_check": if summary_cites.is_empty() { "uncited" } else { "citations verified" },
        "related": related,
        "documentation": documentation,
        "dropped_uncited": dropped,
    })
}

pub async fn research_job(ctx: &mut JobCtx) -> anyhow::Result<()> {
    let app = ctx.app.clone();
    let id = ctx.id();
    ctx.begin(RESEARCH_STEP).await?;
    let row = app.row(&id)?;
    let queries = research_queries(&row);
    let mut found = Vec::new();
    let mut errors = Vec::new();
    for q in &queries {
        match app.web.search(q, 8).await {
            Ok(r) => found.extend(r),
            Err(e) => errors.push(e.to_string()),
        }
    }
    let results = crate::websearch::number(found, 18);
    if results.is_empty() {
        anyhow::bail!("web search found nothing{}", if errors.is_empty() { String::new() } else { format!(": {}", errors.join("; ")) });
    }
    let n = agents::nonce();
    let listing = results
        .iter()
        .map(|r| format!("[{}] {} | {}{}\n    {}", r.id, r.title, r.url, r.repo.as_ref().map(|x| format!(" (GitHub repository {x})")).unwrap_or_default(), r.snippet))
        .collect::<Vec<_>>()
        .join("\n");
    let task = format!(
        "Research the web context of an inherited open-source project, {} (\"{}\"). Below are live web search results. Identify (1) related projects: the original itself, forks, successors, maintained alternatives, or notable mentions; (2) documentation for the original. Every item must cite the ids (like W3) of the results that show it. Use only these results: never invent projects, names or URLs. Empty lists are fine. Then write a two-sentence summary for the person inheriting the project, citing ids in summary_cites.\n\n{}",
        row.full_name,
        row.github.description.as_deref().unwrap_or(""),
        agents::untrusted(&n, "WEB SEARCH RESULTS", &listing)
    );
    let spec = CallSpec { agent: "historian", purpose: format!("Research the web context of {}", row.full_name), inheritance_id: Some(&id), parent_id: None, input_files: vec![format!("{} web results (SerpApi)", results.len())], task, schema: agents::research_schema(), max_tokens: 1400 };
    let (v, call_id) = app.agents.call_json(spec, &n).await?;
    let mut out = check_research(&v, &results, &row.full_name);
    out["queries"] = json!(queries);
    out["results"] = json!(results);
    out["search_errors"] = json!(errors);
    out["provider"] = json!("serpapi");
    out["call_id"] = json!(call_id);
    out["researched_at"] = json!(now());
    app.db.put_artifact(&id, "web_research", &out)?;
    ctx.finish(RESEARCH_STEP, "done", format!(
        "{} searches, {} results; {} related project(s) and {} documentation source(s) with verified citations; {} uncited item(s) dropped",
        queries.len(), results.len(), out["related"].as_array().map(|a| a.len()).unwrap_or(0),
        out["documentation"].as_array().map(|a| a.len()).unwrap_or(0), out["dropped_uncited"]
    ));
    Ok(())
}

/// A compact, cited summary of the web research for the Lead's context, if there is one.
pub fn web_research_brief(app: &App, id: &str) -> Option<String> {
    let w: Value = app.db.get_artifact(id, "web_research").ok()??;
    let mut s = format!("Web research ({}): {}\n", w["researched_at"].as_str().unwrap_or(""), w["summary"].as_str().unwrap_or(""));
    for r in w["related"].as_array().into_iter().flatten().take(6) {
        s.push_str(&format!("- related {} [{}]: {} ({})\n", r["name"].as_str().unwrap_or(""), r["relation"].as_str().unwrap_or(""), r["note"].as_str().unwrap_or(""), r["url"].as_str().unwrap_or("")));
    }
    for d in w["documentation"].as_array().into_iter().flatten().take(4) {
        s.push_str(&format!("- docs {}: {}\n", d["title"].as_str().unwrap_or(""), d["url"].as_str().unwrap_or("")));
    }
    Some(s)
}

// ---------- upstream issue triage (Mastra workflow) ----------

pub async fn triage_job(ctx: &mut JobCtx) -> anyhow::Result<()> {
    let app = ctx.app.clone();
    let id = ctx.id();
    let step = "mastra: triage upstream issues";
    ctx.begin(step).await?;
    let m = app.mastra.clone().ok_or_else(|| anyhow::anyhow!("the Mastra companion is not configured"))?;
    let max = ctx.job.params["max_issues"].as_u64().unwrap_or(8) as u32;
    let started_at = now();
    let t0 = std::time::Instant::now();
    let result = m.triage(&id, max).await;
    let call = AgentCall {
        id: new_id("call"),
        inheritance_id: Some(id.clone()),
        parent_id: None,
        agent: "triager".into(),
        purpose: "Triage upstream GitHub issues (Mastra workflow)".into(),
        provider: "mastra".into(),
        model: result.as_ref().ok().and_then(|v| v["model"].as_str().map(String::from)).unwrap_or_else(|| app.cfg.model.model.clone()),
        started_at: started_at.clone(),
        duration_ms: t0.elapsed().as_millis() as u64,
        input_files: vec!["workflow: triage-upstream-issues".into()],
        prompt_chars: 0,
        response_chars: result.as_ref().map(|v| v.to_string().len()).unwrap_or(0),
        ok: result.is_ok(),
        error: result.as_ref().err().map(|e| e.to_string()),
        input_tokens: None,
        output_tokens: None,
        system: Some(format!("Mastra companion at {}; run id {}", m.base(), result.as_ref().ok().and_then(|v| v["run_id"].as_str()).unwrap_or("-"))),
        prompt: None,
        response: result.as_ref().ok().map(|v| truncate(&redact(&v.to_string()), 40_000)),
    };
    app.db.put(Kind::Call, &call.id, Some(&id), &started_at, &call)?;
    crate::telemetry::record_call(&call);
    let mut v = result?;
    v["triaged_at"] = json!(now());
    v["call_id"] = json!(call.id);
    app.db.put_artifact(&id, "upstream_triage", &v)?;
    let counts = v["counts"].clone();
    ctx.finish(step, "done", format!(
        "{} open upstream issue(s); triaged {}: {} addressed by the successor, {} still relevant, {} obsolete, {} need a human",
        v["total_open_upstream"], v["triaged"].as_array().map(|a| a.len()).unwrap_or(0),
        counts["addressed_by_successor"], counts["still_relevant"], counts["obsolete"], counts["needs_human"]
    ));
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn research_items_need_real_citations() {
        let wr = |id: &str, url: &str| crate::websearch::WebResult { id: id.into(), query: "q".into(), title: "t".into(), url: url.into(), snippet: String::new(), source: String::new(), kind: if url.contains("readthedocs") { "docs".into() } else { "page".into() }, repo: None };
        let mut results = vec![wr("W1", "https://github.com/a/fork"), wr("W2", "https://a.readthedocs.io/"), wr("W3", "https://github.com/o/r")];
        results[2].repo = Some("o/r".into());
        let v = json!({
            "summary": "A fork exists [W1].", "summary_cites": ["W1"],
            "related": [
                {"name": "a/fork", "relation": "fork", "note": "active fork", "cites": ["W1", "W9"]},
                {"name": "invented/project", "relation": "successor", "note": "made up", "cites": ["W7"]},
                {"name": "no cites", "relation": "mention", "note": "", "cites": []},
                {"name": "o/r", "relation": "fork", "note": "the inherited repo itself", "cites": ["W3"]}
            ],
            "documentation": [{"title": "Docs", "note": "", "cites": ["W1", "[W2]"]}, {"title": "summary_cites", "note": "", "cites": ["W2"]}]
        });
        let out = check_research(&v, &results, "o/r");
        assert_eq!(out["related"].as_array().unwrap().len(), 2);
        assert_eq!(out["related"][1]["relation"], json!("original"));
        assert_eq!(out["related"][0]["cites"], json!(["W1"]));
        assert_eq!(out["related"][0]["url"], json!("https://github.com/a/fork"));
        assert_eq!(out["documentation"][0]["url"], json!("https://a.readthedocs.io/"));
        assert_eq!(out["documentation"].as_array().unwrap().len(), 1);
        assert_eq!(out["dropped_uncited"], json!(3));
        assert_eq!(out["summary_check"], json!("citations verified"));
    }

    #[test]
    fn edits_apply_atomically_and_validate() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.py"), "from collections import Iterable\nx = 1\nx = 1\n").unwrap();
        // One good edit + one bad edit: nothing is written.
        let edits = vec![
            json!({"path": "a.py", "find": "from collections import Iterable", "replace": "from collections.abc import Iterable", "all_occurrences": false}),
            json!({"path": "a.py", "find": "x = 1", "replace": "x = 2", "all_occurrences": false}),
        ];
        let err = apply_edits(d.path(), &edits).err().unwrap();
        assert!(err[0].contains("occurs 2 times"));
        assert!(std::fs::read_to_string(d.path().join("a.py")).unwrap().contains("from collections import Iterable"));

        let edits = vec![json!({"path": "a.py", "find": "from collections import Iterable", "replace": "from collections.abc import Iterable", "all_occurrences": false}), json!({"path": "a.py", "find": "x = 1", "replace": "x = 2", "all_occurrences": true})];
        let ok = apply_edits(d.path(), &edits).unwrap();
        assert_eq!(ok.files, vec!["a.py".to_string()]);
        let s = std::fs::read_to_string(d.path().join("a.py")).unwrap();
        assert!(s.contains("collections.abc"));
        assert_eq!(s.matches("x = 2").count(), 2);
    }

    #[test]
    fn edits_cannot_escape_or_create() {
        let d = tempfile::tempdir().unwrap();
        let e = vec![json!({"path": "../evil.py", "find": "a", "replace": "b", "all_occurrences": false})];
        assert!(apply_edits(d.path(), &e).is_err());
        let e = vec![json!({"path": "new.py", "find": "a", "replace": "b", "all_occurrences": false})];
        assert!(apply_edits(d.path(), &e).err().unwrap()[0].contains("not an existing file"));
        let e = vec![json!({"path": ".git/config", "find": "a", "replace": "b", "all_occurrences": false})];
        assert!(apply_edits(d.path(), &e).is_err());
    }

    #[test]
    fn detects_dangling_imports() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("fn")).unwrap();
        // The real fn.py failure mode: Iterable moved to collections.abc but deque dropped.
        std::fs::write(d.path().join("fn/iters.py"), "from collections.abc import Iterable\n\ndef consume(i):\n    deque(i, maxlen=0)\n").unwrap();
        let diff = "--- a/fn/iters.py\n+++ b/fn/iters.py\n@@ -1 +1 @@\n-from collections import deque, Iterable\n+from collections.abc import Iterable\n";
        assert_eq!(dangling_names(d.path(), diff), vec![("fn/iters.py".to_string(), "deque".to_string(), 4)]);
        // Correct fix: nothing dangling.
        std::fs::write(d.path().join("fn/iters.py"), "from collections import deque\nfrom collections.abc import Iterable\n\ndef consume(i):\n    deque(i, maxlen=0)\n").unwrap();
        let diff2 = "--- a/fn/iters.py\n+++ b/fn/iters.py\n@@ -1 +1,2 @@\n-from collections import deque, Iterable\n+from collections import deque\n+from collections.abc import Iterable\n";
        assert!(dangling_names(d.path(), diff2).is_empty());
    }

    #[test]
    fn changed_lines_from_diff() {
        let d = "diff --git a/fn/iters.py b/fn/iters.py\n--- a/fn/iters.py\n+++ b/fn/iters.py\n@@ -1,3 +1,4 @@\n-x\n+y\n";
        assert_eq!(changed_lines(d), vec![("fn/iters.py".to_string(), 1, 4)]);
    }

    #[test]
    fn decay_tokens_link_failures() {
        let h = DecayHit { path: "a.py".into(), line: 1, pattern: "collections ABC import".into(), detail: String::new(), removed_in: String::new(), text: "from collections import Iterable".into(), matched: "from collections import Iterable".into() };
        assert_eq!(decay_token(&h), "Iterable");
        let g = DecayGroup { gid: "D1".into(), pattern: h.pattern.clone(), detail: String::new(), removed_in: String::new(), hits: vec![h] };
        assert!(group_linked_to_failure(&g, "ImportError: cannot import name 'Iterable' from 'collections'"));
        assert!(!group_linked_to_failure(&g, "all good"));
        let h2 = DecayHit { path: "t.py".into(), line: 1, pattern: "deprecated unittest alias".into(), detail: String::new(), removed_in: String::new(), text: "self.assertEquals(a, b)".into(), matched: ".assertEquals(".into() };
        assert_eq!(decay_token(&h2), "assertEquals");
    }
}
