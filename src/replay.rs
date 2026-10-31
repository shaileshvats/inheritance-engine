//! Replays of recorded model output through the engine's live checks.
//!
//! A replay takes what the agents produced in a real, recorded run (a patch, the Reviewer's verdict, the
//! Tester's probe code) and puts it through the same deterministic checks a live repair gets: engine guards,
//! Docker tests, behaviour probes on the original commit's era runtime and on the successor, and the revert
//! rule. No model is called, so anyone can reproduce the result without a GPU or a local model, and it is
//! always labelled as a replay. A replay never commits recorded model output to the successor.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::app::{App, JobCtx};
use crate::contracts::*;
use crate::db::Kind;
use crate::gitops;
use crate::pipeline::{self, run_suite};
use crate::util::{new_id, now};
use crate::verify;

pub const STEPS: &[&str] = &[
    "clone original",
    "inventory",
    "seed successor from recorded run",
    "tests before",
    "recorded patch + engine guards",
    "recorded review",
    "tests after",
    "behaviour probe (live)",
    "engine decision",
    "inheritance record",
];

const FIXTURES: &[(&str, &str)] = &[("fn-py-prflags", include_str!("../fixtures/replay/fn-py-prflags.json"))];

#[derive(Clone, Debug, Deserialize)]
pub struct Fixture {
    pub name: String,
    pub title: String,
    pub about: String,
    pub repository: String,
    pub original_commit: String,
    pub recorded_model: String,
    pub recorded_at: String,
    pub seed: Seed,
    pub issue: Value,
    pub repair: RecordedRepair,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Seed {
    pub description: String,
    pub commits: Vec<String>,
    pub diff: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RecordedRepair {
    pub recorded_id: String,
    pub rationale: String,
    pub diff: String,
    pub files: Vec<String>,
    pub review: Review,
    pub probes: Vec<RecordedProbe>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RecordedProbe {
    pub name: String,
    pub code: String,
}

pub fn fixture(name: &str) -> anyhow::Result<Fixture> {
    let (_, raw) = FIXTURES.iter().find(|(n, _)| *n == name).ok_or_else(|| anyhow::anyhow!("unknown replay fixture {name}"))?;
    Ok(serde_json::from_str(raw)?)
}

pub fn fixtures() -> Vec<Value> {
    FIXTURES.iter().filter_map(|(n, _)| fixture(n).ok()).map(|f| json!({"name": f.name, "title": f.title, "about": f.about, "repository": f.repository})).collect()
}

fn apply_patch(app: &App, id: &str, name: &str, diff: &str) -> anyhow::Result<()> {
    let layout = app.layout(id);
    let path = layout.root.join(format!("{name}.patch"));
    std::fs::write(&path, diff)?;
    // git runs inside the successor, so a relative data dir (the default) must not leak into the path.
    let p = std::fs::canonicalize(&path)?.to_string_lossy().into_owned();
    gitops::git_in(&layout.successor(), &["apply", "--whitespace=nowarn", &p])?;
    Ok(())
}

pub async fn replay_job(ctx: &mut JobCtx) -> anyhow::Result<()> {
    let app = ctx.app.clone();
    let id = ctx.id();
    let name = ctx.job.params["fixture"].as_str().unwrap_or("fn-py-prflags").to_string();
    let fx = fixture(&name)?;
    let layout = app.layout(&id);
    let successor = layout.successor();
    let provenance = format!(
        "Replay of recorded repair {} ({}, recorded {}): the patch, the Reviewer's verdict and the probe code are recorded model output; the tests, the probe runs and this decision ran live",
        fx.repair.recorded_id, fx.recorded_model, fx.recorded_at
    );

    // 1. clone (real)
    ctx.begin("clone original").await?;
    let mut row = app.row(&id)?;
    if row.original_commit.is_some() && successor.exists() {
        ctx.finish("clone original", "skipped", "already cloned");
    } else {
        let url = format!("https://github.com/{}", row.full_name);
        let l2 = layout.clone();
        let res = tokio::task::spawn_blocking(move || gitops::clone_inheritance(&l2, &url)).await??;
        if res.commit != fx.original_commit {
            anyhow::bail!("{} is now at {}, but the recording was made against {}; the replay would not be comparable", row.full_name, &res.commit[..12], &fx.original_commit[..12]);
        }
        row.original_commit = Some(res.commit.clone());
        row.original_branch = Some(res.branch.clone());
        row.stage = Stage::Inherited.as_str().into();
        app.save_row(&mut row)?;
        ctx.finish("clone original", "done", format!("original {} preserved read-only; successor branch created", &res.commit[..12]));
    }

    // 2. inventory (deterministic)
    ctx.begin("inventory").await?;
    let inv = match app.db.get_artifact::<Inventory>(&id, "inventory")? {
        Some(i) => {
            ctx.finish("inventory", "skipped", "already computed");
            i
        }
        None => {
            let i = pipeline::build_inventory(&app, &id).await?;
            ctx.finish("inventory", "done", format!("{} files, {} decay hits", i.files_total, i.decay.len()));
            i
        }
    };
    let era = crate::probe::legacy_python_image(&inv, &app.cfg.python_image).unwrap_or_else(|| app.cfg.python_image.clone());

    // 3. seed the successor with the repairs that were committed in the recorded run
    ctx.begin("seed successor from recorded run").await?;
    if gitops::commits_ahead(&successor, &fx.original_commit) > 0 {
        ctx.finish("seed successor from recorded run", "skipped", "successor already seeded");
    } else {
        apply_patch(&app, &id, "seed", &fx.seed.diff)?;
        let msg = format!(
            "Replay seed: successor state from a recorded run\n\n{}\n\nRecorded-Commits: {}\nOriginal-Commit: {}",
            fx.seed.description,
            fx.seed.commits.join("; "),
            fx.original_commit
        );
        let sha = gitops::commit_all(&successor, &msg)?;
        ctx.finish("seed successor from recorded run", "done", format!("committed {} ({})", &sha[..10], fx.seed.commits.join("; ")));
    }
    app.set_stage(&id, Stage::Restoring)?;

    // The issue the recorded patch was written for, including the inheritor's reopened note.
    let mut issue: Issue = serde_json::from_value(fx.issue.clone())?;
    issue.id = new_id("iss");
    issue.status = "open".into();
    pipeline_save_issue(&app, &id, &issue)?;

    // 4. tests before (live)
    ctx.begin("tests before").await?;
    let before = run_suite(&app, &id, "replay · before recorded patch", None).await?;
    let before_ok = before.status == "passed" || before.summary.is_some();
    ctx.finish("tests before", if before_ok { "done" } else { "failed" }, pipeline::verification_brief(&before));
    if before.status == "not_run" {
        anyhow::bail!("tests could not run: {}. The replay needs Docker.", before.reason.clone().unwrap_or_default());
    }

    let mut repair = Repair {
        id: new_id("rep"),
        issue_id: issue.id.clone(),
        status: "reverted".into(),
        rationale: fx.repair.rationale.clone(),
        diff: fx.repair.diff.clone(),
        files: fx.repair.files.clone(),
        commit: None,
        review: None,
        verification_before: Some(before.id.clone()),
        verification_after: None,
        note: None,
        created_at: now(),
        attempt: 1,
        probe: None,
        replay_of: Some(fx.repair.recorded_id.clone()),
    };
    // Every outcome ends in a revert: a replay never commits recorded model output.
    let decide = |ctx: &mut JobCtx, repair: &mut Repair, issue: &mut Issue, status: &str, why: String| -> anyhow::Result<()> {
        gitops::revert_worktree(&successor)?;
        repair.status = if status == "kept" { "reverted".into() } else { status.into() };
        repair.note = Some(format!("{provenance}. Outcome: {why}"));
        app.db.put(Kind::Repair, &repair.id, Some(&id), &repair.created_at, &*repair)?;
        issue.status = "failed".into();
        pipeline_save_issue(&app, &id, issue)?;
        ctx.finish("engine decision", if status == "kept" { "done" } else { "failed" }, why);
        Ok(())
    };

    // 5. recorded patch + deterministic guards
    ctx.begin("recorded patch + engine guards").await?;
    apply_patch(&app, &id, "recorded-repair", &fx.repair.diff)?;
    let diff = gitops::worktree_diff(&successor)?;
    let dangling = pipeline::dangling_names(&successor, &diff);
    if !dangling.is_empty() {
        let why = dangling.iter().map(|(p, n, l)| format!("'{n}' is no longer imported in {p} but is still used at line {l}")).collect::<Vec<_>>().join("; ");
        ctx.finish("recorded patch + engine guards", "failed", why.clone());
        ctx.begin("engine decision").await?;
        return decide(ctx, &mut repair, &mut issue, "reverted", format!("engine guard before review: {why}; reverted"));
    }
    ctx.finish("recorded patch + engine guards", "done", format!("applied to {}; no dangling imports", fx.repair.files.join(", ")));

    // 6. recorded review, with the engine's consistency rule applied
    ctx.begin("recorded review").await?;
    let mut review = fx.repair.review.clone();
    review.call_id = None;
    if review.verdict == "approve" && (review.problem_is_real == Some(false) || !review.root_cause_addressed || review.preserves_original_behaviour == Some(false)) {
        review.verdict = "request_changes".into();
        review.concerns.push("Engine: verdict was 'approve' but the reviewer's own checks failed.".into());
    }
    repair.review = Some(review.clone());
    ctx.finish("recorded review", "done", format!("recorded verdict: {} (regression risk {}, root cause addressed: {})", review.verdict, review.regression_risk, review.root_cause_addressed));
    if review.verdict != "approve" {
        ctx.begin("engine decision").await?;
        return decide(ctx, &mut repair, &mut issue, "rejected", format!("Reviewer verdict '{}'; reverted", review.verdict));
    }

    // 7. tests after (live)
    ctx.begin("tests after").await?;
    let after = run_suite(&app, &id, "replay · after recorded patch", None).await?;
    repair.verification_after = Some(after.id.clone());
    ctx.finish("tests after", "done", pipeline::verification_brief(&after));
    let regressed = verify::regressed(before.summary.as_ref(), after.summary.as_ref(), &before.status, &after.status);
    let improved = verify::improved(before.summary.as_ref(), after.summary.as_ref(), &before.status, &after.status);
    if regressed {
        ctx.begin("engine decision").await?;
        return decide(ctx, &mut repair, &mut issue, "reverted", format!("verification got worse ({} → {}); reverted", before.status, after.status));
    }

    // 8. behaviour probe (live): the recorded probe code runs on the original commit and on the successor
    ctx.begin("behaviour probe (live)").await?;
    let report = if after.status == "passed" || improved {
        match crate::probe::target(&app, &id, &repair.files)? {
            Some(t) => {
                let probes: Vec<(String, String)> = fx.repair.probes.iter().map(|p| (p.name.clone(), p.code.clone())).collect();
                Some(crate::probe::execute(&app, &id, &t, &probes, None, "replay: probe code recorded from the Tester agent; executed live on both runtimes").await?)
            }
            None => None,
        }
    } else {
        None
    };
    match &report {
        Some(r) => ctx.finish("behaviour probe (live)", if r.verdict == "changed" { "failed" } else { "done" }, format!("original on {} vs successor on {}: {}", r.legacy_image, r.modern_image, r.note.clone().unwrap_or_default())),
        None => ctx.finish("behaviour probe (live)", "skipped", format!("not applicable (tests: {}); era runtime would be {era}", after.status)),
    }
    repair.probe = report.clone();

    // 9. engine decision: the same rule a live repair gets, except that a replay never commits.
    ctx.begin("engine decision").await?;
    match report {
        Some(r) if r.verdict == "changed" => decide(
            ctx,
            &mut repair,
            &mut issue,
            "reverted",
            format!("tests {} but the behaviour probe found the successor ({}) no longer behaves like the original ({}); reverted", after.status, r.modern_image, r.legacy_image),
        )?,
        other => {
            let v = other.map(|r| r.verdict).unwrap_or_else(|| "not run".into());
            decide(ctx, &mut repair, &mut issue, "kept", format!("the checks did not reproduce the recorded catch (probe: {v}); a replay never commits recorded output, so the change was reverted anyway"))?
        }
    }

    // 10. the Inheritance Record (deterministic)
    ctx.begin("inheritance record").await?;
    let rec = crate::record::build(&app, &id)?;
    let authors = rec["provenance"]["attribution"]["original_authors"].as_array().map(|a| a.iter().take(3).filter_map(|x| Some(format!("{} ({} commits)", x["name"].as_str()?, x["commits"]))).collect::<Vec<_>>().join(", ")).unwrap_or_default();
    ctx.finish(
        "inheritance record",
        "done",
        format!("record v{}: {} @ {}; original authors {}; {} successor commit(s); {} attempt(s) on record", rec["record_version"], fx.repository, &fx.original_commit[..12], authors, rec["successor"]["lineage"].as_array().map(|l| l.len().saturating_sub(1)).unwrap_or(0), rec["restoration"]["attempts"].as_array().map(|a| a.len()).unwrap_or(0)),
    );
    Ok(())
}

fn pipeline_save_issue(app: &App, id: &str, issue: &Issue) -> anyhow::Result<()> {
    app.db.put(Kind::Issue, &issue.id, Some(id), &issue.id, issue)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_fixture_is_complete() {
        let f = fixture("fn-py-prflags").unwrap();
        assert_eq!(f.repository, "kachayev/fn.py");
        assert!(f.seed.diff.starts_with("diff --git"));
        assert!(f.repair.diff.contains("f.prflags"));
        assert_eq!(f.repair.review.verdict, "approve");
        assert!(!f.repair.probes.is_empty());
        let issue: Issue = serde_json::from_value(f.issue).unwrap();
        assert!(issue.reopened_note.is_some());
    }
}
