# Changelog

## Unreleased — 2026-10-31 (partner integrations)

- GitHub Actions CI (`.github/workflows/ci.yml`): clippy and Rust tests, Mastra and Temporal suites, restoration-memory
  tests against Tiger Data's Postgres image, and the replay end to end in Docker on a clean runner, uploading the
  Inheritance Record. `scripts/replay-check.sh` asserts the catch against any engine.
- Temporal companion (`companion/temporal`): durable `inheritAndRestore` workflow over the engine's jobs (waits out
  services, resumes after worker or engine restarts, bounded re-runs, skips unconfigured partners). 11 tests.
- Restoration memory in the Mastra companion: repair attempts from all inheritances in Postgres + pgvector +
  pgvectorscale (DiskANN), local embeddings, hybrid keyword + vector search, a Lead Engineer tool. 7 tests.
- No engine (Rust) code changed.

## Unreleased — 2026-10-28

- Mastra companion (`companion/mastra`): Lead Engineer agent with engine tools, grounded `answer-lead-question`
  workflow, `triage-upstream-issues` workflow with an engine-side citation check; engine integration (chat, durable
  `triage` job, UI card, Inheritance Record). 19 vitest tests.
- Sentry agent tracing (engine + companion), metadata only, enforced by scrubbing transport / span processor; tests
  inject source and secrets to prove nothing leaks. Token usage recorded for every model call.
- Self-host Docker Compose stack (`deploy/self-host`: compose, Dockerfiles, cloud-init), images built and run locally;
  never deployed to a cloud provider (zero budget).
- SerpApi web search (optional, free plan): Hunter "Also search the web" with GitHub re-verification, and a
  `research` job whose Gemma summary must cite real results (engine-checked); fed to the Lead and the Mastra agent.
- Human-in-the-loop reopen of issues; reviewer rejections get one informed retry; humans outrank green tests.

## 0.1.0 — 2026-10-28 (Hacktoberfest Week 4 build)

- Hunter: GitHub search (problem-first via the Hunter agent, curiosity-first), explainable signals, AI explanations.
- Intake: hardened bare clone kept read-only, successor clone on its own branch with push disabled.
- Inventory: tree, languages, manifests, entry points, Python/JS import graph, central-component scoring,
  runtime-decay scan, trust passport (install scripts, network/fs/process/dynamic-code signals, secrets, dependencies).
- Agent team: Lead (plan, synthesis, chat), Historian, Restorer (diagnose, patch, re-diagnose), Reviewer (challenge, repair review),
  Hunter — real model calls with constrained JSON, nonce-delimited untrusted content, full traces.
- Evidence: every citation re-read from disk by the engine; unsupported "verified" claims downgraded.
- Restoration: atomic search/replace edits, independent review, Docker verification, revert on regression or when the
  failure originates in the edited lines, retry with real error output, commit with provenance trailers.
- Verification: Docker (network off, read-only rootfs, caps dropped, limits) or explicit native mode; real exit codes.
- Inheritance Record: versioned JSON export with provenance and lineage; INHERITANCE.md written into the successor.
- Durable jobs in SQLite with pause/resume/cancel and recovery after restart.
