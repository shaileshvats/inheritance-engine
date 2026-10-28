//! Differential behaviour probes: does a repair preserve what the original code did?
//!
//! Passing tests only prove what the tests exercise. After a repair, a Tester agent writes small probe
//! calls for the code the diff touched. The engine runs them twice in Docker — on the successor with the
//! modern runtime, and on the original commit with the runtime of its era (derived from the decay scan) —
//! and compares the printed results. Probes are model-written code, so they never run natively.

use std::collections::BTreeSet;
use std::time::Duration;

use serde_json::{json, Value};

use crate::agents::{self, CallSpec};
use crate::app::App;
use crate::contracts::*;
use crate::db::Kind;
use crate::evidence;
use crate::gitops;
use crate::sandbox::{self, RunRequest};
use crate::security::redact;
use crate::util::{new_id, now, tail_lines, truncate};

const PROBE_DIR: &str = ".ie-probe";
const MARK: &str = "@@IE-PROBE@@";

/// The newest Python that still had every API the decay scan says the project relies on.
pub fn legacy_python_image(inv: &Inventory, modern: &str) -> Option<String> {
    let mut min_removed: Option<(u32, u32)> = None;
    for h in &inv.decay {
        let Some(v) = h.removed_in.strip_prefix("Python ") else { continue };
        let mut parts = v.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty());
        let (Some(maj), Some(min)) = (parts.next().and_then(|m| m.parse::<u32>().ok()), parts.next().and_then(|m| m.parse::<u32>().ok())) else { continue };
        if min_removed.map(|m| (maj, min) < m).unwrap_or(true) {
            min_removed = Some((maj, min));
        }
    }
    let (maj, min) = min_removed?;
    let image = if maj <= 2 || (maj == 3 && min == 0) { "python:2.7-slim".to_string() } else { format!("python:{}.{}-slim", maj, min.saturating_sub(1).max(7)) };
    (image != modern).then_some(image)
}

/// Wrap each probe so one failure does not hide the others, and normalise what is printed.
fn render_script(probes: &[(String, String)]) -> String {
    let mut s = String::from("from __future__ import print_function\nimport sys\nsys.path.insert(0, '.')\nsys.path.insert(0, './src')\n");
    for (name, code) in probes {
        let body: String = code.lines().map(|l| format!("    {l}\n")).collect();
        s.push_str(&format!(
            "print('{MARK} {name}')\ntry:\n{body}except BaseException as _e:\n    print('EXCEPTION ' + type(_e).__name__)\n",
            name = name.replace(['\'', '\\', '\n'], "_"),
        ));
    }
    s
}

fn split_outputs(output: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in output.lines() {
        if let Some(name) = line.strip_prefix(MARK) {
            out.push((name.trim().to_string(), String::new()));
        } else if let Some(last) = out.last_mut() {
            if !line.trim().is_empty() && !line.contains("Warning:") && !line.trim_start().starts_with("warnings.warn") {
                last.1.push_str(line.trim_end());
                last.1.push('\n');
            }
        }
    }
    out
}

fn export_original(app: &App, id: &str, commit: &str) -> anyhow::Result<std::path::PathBuf> {
    let layout = app.layout(id);
    let dir = layout.root.join("original-export");
    if !dir.join(".ie-exported").exists() {
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        let archive = std::process::Command::new("git").arg("--git-dir").arg(layout.original()).args(["archive", "--format=tar", commit]).output()?;
        if !archive.status.success() {
            anyhow::bail!("git archive failed");
        }
        let mut tar = std::process::Command::new("tar").args(["-x", "-C"]).arg(&dir).stdin(std::process::Stdio::piped()).spawn()?;
        use std::io::Write;
        tar.stdin.take().unwrap().write_all(&archive.stdout)?;
        if !tar.wait()?.success() {
            anyhow::bail!("extracting the original commit failed");
        }
        std::fs::write(dir.join(".ie-exported"), commit)?;
    }
    Ok(dir)
}

fn ensure_excluded(successor: &std::path::Path) {
    let exclude = successor.join(".git").join("info").join("exclude");
    let cur = std::fs::read_to_string(&exclude).unwrap_or_default();
    if !cur.contains(PROBE_DIR) {
        let _ = std::fs::write(&exclude, format!("{cur}{PROBE_DIR}/\n"));
    }
}

/// Run behaviour probes for a repair. Returns None when probing does not apply.
pub async fn run(app: &App, id: &str, issue: &Issue, diff: &str, files: &[String], parent: Option<String>) -> anyhow::Result<Option<ProbeReport>> {
    let row = app.row(id)?;
    if row.execution_mode != "docker" || !files.iter().any(|f| f.ends_with(".py")) || files.iter().all(|f| crate::inventory::is_test_path(f)) {
        return Ok(None);
    }
    let inv: Inventory = match app.db.get_artifact(id, "inventory")? {
        Some(i) => i,
        None => return Ok(None),
    };
    let modern = app.cfg.python_image.clone();
    let legacy = legacy_python_image(&inv, &modern).unwrap_or_else(|| modern.clone());
    let commit = row.original_commit.clone().unwrap_or_default();
    let layout = app.layout(id);
    let successor = layout.successor();

    // 1. The Tester writes probes for the changed code.
    let n = agents::nonce();
    let focus: Vec<(String, Vec<usize>)> = files.iter().filter(|f| f.ends_with(".py")).map(|f| (f.clone(), vec![])).take(3).collect();
    let mut pack = String::new();
    for (p, _) in &focus {
        if let Some(ex) = evidence::excerpt(&successor, p, &[], app.cfg.prompt_budget_chars / 3) {
            pack.push_str(&agents::untrusted(&n, &format!("FILE {} after the repair (lines {})", ex.path, ex.shown), &ex.body));
        }
    }
    let task = format!(
        "A repair was just applied for: {}\nThe existing tests pass or improved, but they may not exercise the changed code. Write 1-4 behaviour probes: short Python snippets that import the changed module(s) from the project root and CALL the changed functions/classes with small, deterministic inputs, then print() the results with repr(). Each probe must work on both an old Python ({legacy}) and a new one ({modern}): no f-strings, no walrus, no type hints, no network, no file writes, no randomness, no time. Prefer exercising code paths that the diff touched. Use the package's public import paths as shown in the files.\n\n{}{}",
        issue.title,
        agents::untrusted(&n, "DIFF", &redact(&truncate(diff, 8000))),
        pack
    );
    let schema = json!({"type": "object", "additionalProperties": false, "required": ["probes", "rationale"], "properties": {
        "rationale": {"type": "string"},
        "probes": {"type": "array", "items": {"type": "object", "additionalProperties": false, "required": ["name", "code"], "properties": {"name": {"type": "string"}, "code": {"type": "string"}}}}
    }});
    let spec = CallSpec { agent: "tester", purpose: format!("Behaviour probes: {}", truncate(&issue.title, 70)), inheritance_id: Some(id), parent_id: parent, input_files: focus.iter().map(|f| f.0.clone()).collect(), task, schema, max_tokens: 1200 };
    let (v, call_id) = app.agents.call_json(spec, &n).await?;
    let probes: Vec<(String, String)> = v["probes"]
        .as_array()
        .map(|a| a.iter().filter_map(|p| Some((p["name"].as_str()?.chars().take(60).collect(), p["code"].as_str()?.to_string()))).filter(|(_, c): &(String, String)| !c.trim().is_empty()).take(4).collect())
        .unwrap_or_default();
    if probes.is_empty() {
        return Ok(Some(ProbeReport { tester_call_id: Some(call_id), legacy_image: legacy, modern_image: modern, probes: vec![], verdict: "inconclusive".into(), note: Some("The Tester proposed no probes.".into()) }));
    }
    let script = render_script(&probes);

    // 2. Run on the successor (modern runtime) and on the original commit (era runtime).
    let original_dir = export_original(app, id, &commit)?;
    ensure_excluded(&successor);
    for dir in [&successor, &original_dir] {
        std::fs::create_dir_all(dir.join(PROBE_DIR))?;
        std::fs::write(dir.join(PROBE_DIR).join("probe.py"), &script)?;
    }
    let env = vec![("PYTHONDONTWRITEBYTECODE".to_string(), "1".to_string()), ("PYTHONPATH".to_string(), "{WORK}:{WORK}/src:{DEPS}".to_string())];
    let mut outputs = Vec::new();
    let runs = [("successor", modern.clone(), successor.clone()), ("original", legacy.clone(), original_dir.clone())];
    for (label, image, workdir) in runs {
        let image = image.as_str();
        if let Err(e) = sandbox::ensure_image(image).await {
            let _ = std::fs::remove_dir_all(successor.join(PROBE_DIR));
            return Ok(Some(ProbeReport { tester_call_id: Some(call_id), legacy_image: legacy.clone(), modern_image: modern.clone(), probes: vec![], verdict: "inconclusive".into(), note: Some(format!("could not obtain {image}: {e}")) }));
        }
        let out = sandbox::run(RunRequest {
            mode: ExecMode::Docker,
            layout: &layout,
            inheritance_id: id,
            image,
            command: &format!("python {PROBE_DIR}/probe.py"),
            env: &env,
            network: false,
            as_root: false,
            timeout: Duration::from_secs(120),
            workdir: Some(workdir),
        })
        .await?;
        let output = redact(&out.output);
        let ver = Verification {
            id: new_id("ver"),
            label: format!("behaviour probe · {label} ({image})"),
            mode: "docker".into(),
            environment: out.environment.clone(),
            command: out.command.clone(),
            exit_code: out.exit_code,
            status: if out.exit_code == Some(0) { "passed".into() } else { "error".into() },
            summary: None,
            output_tail: tail_lines(&output, 120),
            duration_ms: out.duration_ms,
            started_at: now(),
            workspace_rev: if label == "successor" { gitops::head(&successor).ok() } else { Some(commit.clone()) },
            network: out.network.clone(),
            reason: Some("probe script written by the Tester agent; compares successor behaviour with the original".into()),
        };
        app.db.put(Kind::Verification, &ver.id, Some(id), &ver.started_at, &ver)?;
        outputs.push(split_outputs(&output));
    }
    let _ = std::fs::remove_dir_all(successor.join(PROBE_DIR));

    // 3. Compare.
    let (succ, orig) = (&outputs[0], &outputs[1]);
    let mut results = Vec::new();
    let mut seen = BTreeSet::new();
    for (name, code) in &probes {
        let key = name.replace(['\'', '\\', '\n'], "_");
        if !seen.insert(key.clone()) {
            continue;
        }
        let o = orig.iter().find(|(n, _)| n == &key).map(|(_, v)| v.trim().to_string());
        let s = succ.iter().find(|(n, _)| n == &key).map(|(_, v)| v.trim().to_string());
        let verdict = match (&o, &s) {
            (Some(o), _) if o.starts_with("EXCEPTION") || o.is_empty() => "inconclusive",
            (Some(o), Some(s)) if o == s => "preserved",
            (Some(_), _) => "changed",
            (None, _) => "inconclusive",
        };
        results.push(ProbeResult { name: key, code: code.clone(), original_output: o.unwrap_or_default(), successor_output: s.unwrap_or_default(), verdict: verdict.into() });
    }
    let changed = results.iter().filter(|r| r.verdict == "changed").count();
    let preserved = results.iter().filter(|r| r.verdict == "preserved").count();
    let verdict = if changed > 0 { "changed" } else if preserved > 0 { "preserved" } else { "inconclusive" };
    Ok(Some(ProbeReport {
        tester_call_id: Some(call_id),
        legacy_image: legacy,
        modern_image: modern,
        note: Some(format!("{preserved} preserved, {changed} changed, {} inconclusive", results.len() - preserved - changed)),
        probes: results,
        verdict: verdict.into(),
    }))
}

pub fn feedback(report: &ProbeReport) -> String {
    let lines: Vec<String> = report
        .probes
        .iter()
        .filter(|p| p.verdict == "changed")
        .map(|p| format!("- probe '{}':\n{}\n  original ({}) printed: {}\n  successor ({}) printed: {}", p.name, p.code, report.legacy_image, p.original_output, report.modern_image, p.successor_output))
        .collect();
    lines.join("\n")
}

pub fn summary(report: &ProbeReport) -> Value {
    json!({"verdict": report.verdict, "legacy_image": report.legacy_image, "note": report.note})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(removed_in: &str) -> DecayHit {
        DecayHit { path: "a.py".into(), line: 1, pattern: "p".into(), detail: String::new(), removed_in: removed_in.into(), text: String::new(), matched: String::new() }
    }

    fn inv(hits: Vec<DecayHit>) -> Inventory {
        Inventory {
            files_total: 0, bytes_total: 0, languages: vec![],
            tree: TreeNode { name: String::new(), path: String::new(), kind: "dir".into(), size: 0, children: None },
            manifests: vec![], entry_points: vec![], tests: TestInfo { framework: "none".into(), files: vec![] },
            mother_candidates: vec![], graph: Graph::default(), git: GitSummary::default(), decay: hits, readme_path: None,
        }
    }

    #[test]
    fn era_runtime_from_decay() {
        assert_eq!(legacy_python_image(&inv(vec![hit("Python 3.10"), hit("Python 3.12")]), "python:3.12-slim").as_deref(), Some("python:3.9-slim"));
        assert_eq!(legacy_python_image(&inv(vec![hit("Python 3.0")]), "python:3.12-slim").as_deref(), Some("python:2.7-slim"));
        assert_eq!(legacy_python_image(&inv(vec![hit("Python 3.12 (setuptools no longer bundled)")]), "python:3.12-slim").as_deref(), Some("python:3.11-slim"));
        assert_eq!(legacy_python_image(&inv(vec![hit("Node.js 10 (deprecated)")]), "python:3.12-slim"), None);
    }

    #[test]
    fn script_isolates_probes_and_outputs_split() {
        let s = render_script(&[("a".into(), "print(1)".into()), ("b".into(), "raise ValueError('x')".into())]);
        assert!(s.contains("print('@@IE-PROBE@@ a')"));
        assert!(s.contains("    raise ValueError('x')"));
        let out = format!("{MARK} a\n1\n{MARK} b\nEXCEPTION ValueError\n");
        let parts = split_outputs(&out);
        assert_eq!(parts, vec![("a".to_string(), "1\n".to_string()), ("b".to_string(), "EXCEPTION ValueError\n".to_string())]);
    }
}
