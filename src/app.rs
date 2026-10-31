//! Application state and the durable job runner.
//!
//! Jobs are persisted in SQLite with per-step status. Pipelines are written so that each step first
//! checks for its checkpoint artifact, which makes resuming an interrupted job (e.g. after a restart)
//! skip work that already finished.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use crate::agents::Agents;
use crate::config::Config;
use crate::contracts::*;
use crate::db::{Db, Kind};
use crate::gitops::Layout;
use crate::hunter::GitHub;
use crate::util::{new_id, now};

const RUN: u8 = 0;
const PAUSE: u8 = 1;
const CANCEL: u8 = 2;

pub struct App {
    pub cfg: Arc<Config>,
    pub db: Arc<Db>,
    pub agents: Arc<Agents>,
    pub gh: GitHub,
    pub mastra: Option<crate::mastra::MastraClient>,
    /// Optional live web search (SerpApi); inactive until `SERPAPI_API_KEY` is set.
    pub web: crate::websearch::WebSearch,
    controls: Mutex<HashMap<String, Arc<AtomicU8>>>,
}

#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cancelled by user")
    }
}

impl std::error::Error for Cancelled {}

impl App {
    pub fn new(cfg: Arc<Config>, db: Arc<Db>, agents: Arc<Agents>, gh: GitHub, mastra: Option<crate::mastra::MastraClient>, web: crate::websearch::WebSearch) -> Arc<Self> {
        Arc::new(Self { cfg, db, agents, gh, mastra, web, controls: Mutex::new(HashMap::new()) })
    }

    pub fn layout(&self, inheritance_id: &str) -> Layout {
        Layout::new(&self.cfg.data_dir, inheritance_id)
    }

    pub fn row(&self, id: &str) -> anyhow::Result<InheritanceRow> {
        self.db.get(Kind::Inheritance, id)?.ok_or_else(|| anyhow::anyhow!("inheritance {id} not found"))
    }

    pub fn save_row(&self, row: &mut InheritanceRow) -> anyhow::Result<()> {
        row.updated_at = now();
        self.db.put(Kind::Inheritance, &row.id, None, &row.created_at, row)
    }

    pub fn set_stage(&self, id: &str, stage: Stage) -> anyhow::Result<()> {
        let mut row = self.row(id)?;
        row.stage = stage.as_str().into();
        self.save_row(&mut row)
    }

    pub fn jobs_for(&self, id: &str) -> Vec<Job> {
        self.db.list(Kind::Job, Some(id)).unwrap_or_default()
    }

    pub fn active_job(&self, id: &str) -> Option<Job> {
        self.jobs_for(id).into_iter().rev().find(|j| matches!(j.status.as_str(), "queued" | "running" | "paused"))
    }

    /// On startup, any job that was running when the process stopped is marked interrupted (resumable).
    pub fn recover_jobs(&self) -> anyhow::Result<usize> {
        let jobs: Vec<Job> = self.db.list(Kind::Job, None)?;
        let mut n = 0;
        for mut j in jobs {
            if matches!(j.status.as_str(), "queued" | "running" | "paused") {
                j.status = "interrupted".into();
                j.error = Some("The engine stopped while this job was running. Resume to continue from the last completed step.".into());
                for s in j.steps.iter_mut().filter(|s| s.status == "running") {
                    s.status = "pending".into();
                }
                j.updated_at = now();
                self.db.put(Kind::Job, &j.id, Some(&j.inheritance_id), &j.created_at, &j)?;
                n += 1;
            }
        }
        Ok(n)
    }

    pub fn start_job(self: &Arc<Self>, inheritance_id: &str, kind: &str, params: serde_json::Value) -> anyhow::Result<Job> {
        if let Some(active) = self.active_job(inheritance_id) {
            anyhow::bail!("a {} job is already {} for this inheritance", active.kind, active.status);
        }
        let steps: Vec<&str> = match kind {
            "investigate" => crate::pipeline::INVESTIGATE_STEPS.to_vec(),
            "restore" => vec!["plan"],
            "verify" => vec!["verify"],
            "triage" => vec!["mastra: triage upstream issues"],
            "research" => vec![crate::pipeline::RESEARCH_STEP],
            _ => anyhow::bail!("unknown job kind {kind}"),
        };
        let t = now();
        let job = Job {
            id: new_id("job"),
            inheritance_id: inheritance_id.into(),
            kind: kind.into(),
            status: "queued".into(),
            step: None,
            steps: steps.iter().map(|s| JobStep { name: s.to_string(), status: "pending".into(), detail: None, started_at: None, finished_at: None }).collect(),
            params,
            error: None,
            created_at: t.clone(),
            updated_at: t,
        };
        self.db.put(Kind::Job, &job.id, Some(inheritance_id), &job.created_at, &job)?;
        self.spawn(job.clone());
        Ok(job)
    }

    fn spawn(self: &Arc<Self>, job: Job) {
        let ctl = Arc::new(AtomicU8::new(RUN));
        self.controls.lock().unwrap().insert(job.id.clone(), ctl.clone());
        let app = self.clone();
        tokio::spawn(async move {
            let id = job.id.clone();
            let mut ctx = JobCtx { app: app.clone(), job, ctl };
            ctx.job.status = "running".into();
            ctx.job.error = None;
            ctx.save();
            let repository = app.row(&ctx.job.inheritance_id).map(|r| r.full_name).unwrap_or_default();
            crate::telemetry::job_started(&ctx.job, &repository);
            let result = match ctx.job.kind.as_str() {
                "investigate" => crate::pipeline::investigate(&mut ctx).await,
                "restore" => crate::pipeline::restore(&mut ctx).await,
                "verify" => crate::pipeline::verify_job(&mut ctx).await,
                "triage" => crate::pipeline::triage_job(&mut ctx).await,
                "research" => crate::pipeline::research_job(&mut ctx).await,
                _ => Err(anyhow::anyhow!("unknown job kind")),
            };
            match result {
                Ok(()) => {
                    ctx.job.status = "completed".into();
                    ctx.job.step = None;
                }
                Err(e) if e.downcast_ref::<Cancelled>().is_some() => {
                    ctx.job.status = "cancelled".into();
                    ctx.fail_running("cancelled");
                }
                Err(e) => {
                    tracing::error!(job = %id, error = %format!("{e:#}"), "job failed");
                    ctx.job.status = "failed".into();
                    ctx.job.error = Some(crate::security::redact(&format!("{e:#}")));
                    ctx.fail_running("failed");
                }
            }
            ctx.save();
            crate::telemetry::job_finished(&ctx.job.inheritance_id, &ctx.job.status);
            app.controls.lock().unwrap().remove(&id);
        });
    }

    pub fn control(self: &Arc<Self>, job_id: &str, action: &str) -> anyhow::Result<Job> {
        let mut job: Job = self.db.get(Kind::Job, job_id)?.ok_or_else(|| anyhow::anyhow!("job not found"))?;
        let ctl = self.controls.lock().unwrap().get(job_id).cloned();
        match (action, ctl) {
            ("pause", Some(c)) => c.store(PAUSE, Ordering::SeqCst),
            ("cancel", Some(c)) => c.store(CANCEL, Ordering::SeqCst),
            ("resume", Some(c)) => c.store(RUN, Ordering::SeqCst),
            ("resume", None) if matches!(job.status.as_str(), "interrupted" | "failed" | "cancelled") => {
                if let Some(active) = self.active_job(&job.inheritance_id) {
                    anyhow::bail!("another job ({}) is active", active.id);
                }
                job.status = "queued".into();
                job.error = None;
                for s in job.steps.iter_mut().filter(|s| s.status == "failed") {
                    s.status = "pending".into();
                }
                job.updated_at = now();
                self.db.put(Kind::Job, &job.id, Some(&job.inheritance_id), &job.created_at, &job)?;
                self.spawn(job.clone());
                return Ok(job);
            }
            ("cancel", None) if job.status == "interrupted" => {
                job.status = "cancelled".into();
                job.updated_at = now();
                self.db.put(Kind::Job, &job.id, Some(&job.inheritance_id), &job.created_at, &job)?;
                return Ok(job);
            }
            (a, _) => anyhow::bail!("cannot {a} a job that is {}", job.status),
        }
        // Reflect the requested state promptly; the runner confirms it at the next checkpoint.
        self.db.get(Kind::Job, job_id)?.ok_or_else(|| anyhow::anyhow!("job not found"))
    }
}

pub struct JobCtx {
    pub app: Arc<App>,
    pub job: Job,
    ctl: Arc<AtomicU8>,
}

impl JobCtx {
    pub fn save(&mut self) {
        self.job.updated_at = now();
        if let Err(e) = self.app.db.put(Kind::Job, &self.job.id, Some(&self.job.inheritance_id), &self.job.created_at, &self.job) {
            tracing::error!(error = %e, "failed to persist job");
        }
    }

    fn fail_running(&mut self, status: &str) {
        let t = now();
        for s in self.job.steps.iter_mut().filter(|s| s.status == "running") {
            s.status = status.into();
            s.finished_at = Some(t.clone());
        }
    }

    pub fn id(&self) -> String {
        self.job.inheritance_id.clone()
    }

    /// Honour pause/cancel between steps.
    pub async fn checkpoint(&mut self) -> anyhow::Result<()> {
        loop {
            match self.ctl.load(Ordering::SeqCst) {
                CANCEL => return Err(Cancelled.into()),
                PAUSE => {
                    if self.job.status != "paused" {
                        self.job.status = "paused".into();
                        self.save();
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                }
                _ => {
                    if self.job.status != "running" {
                        self.job.status = "running".into();
                        self.save();
                    }
                    return Ok(());
                }
            }
        }
    }

    /// Add a step if missing. Steps discovered mid-run are inserted before the closing steps so the list reads in order.
    pub fn add_step(&mut self, name: &str) {
        if !self.job.steps.iter().any(|s| s.name == name) {
            let step = JobStep { name: name.into(), status: "pending".into(), detail: None, started_at: None, finished_at: None };
            let closing = ["final verification", "inheritance notice"];
            match self.job.steps.iter().position(|s| closing.contains(&s.name.as_str()) && s.status == "pending") {
                Some(i) if !closing.contains(&name) => self.job.steps.insert(i, step),
                _ => self.job.steps.push(step),
            }
            self.save();
        }
    }

    pub fn step_status(&self, name: &str) -> Option<String> {
        self.job.steps.iter().find(|s| s.name == name).map(|s| s.status.clone())
    }

    /// Start a step (after a pause/cancel checkpoint).
    pub async fn begin(&mut self, name: &str) -> anyhow::Result<()> {
        self.checkpoint().await?;
        self.add_step(name);
        let t = now();
        if let Some(s) = self.job.steps.iter_mut().find(|s| s.name == name) {
            s.status = "running".into();
            s.started_at = Some(t);
            s.detail = None;
        }
        self.job.step = Some(name.into());
        self.save();
        Ok(())
    }

    pub fn finish(&mut self, name: &str, status: &str, detail: impl Into<String>) {
        let t = now();
        let detail = detail.into();
        if let Some(s) = self.job.steps.iter_mut().find(|s| s.name == name) {
            s.status = status.into();
            s.finished_at = Some(t);
            s.detail = if detail.is_empty() { None } else { Some(detail) };
        }
        self.save();
    }

    pub fn note(&mut self, name: &str, detail: impl Into<String>) {
        if let Some(s) = self.job.steps.iter_mut().find(|s| s.name == name) {
            s.detail = Some(detail.into());
        }
        self.save();
    }
}
