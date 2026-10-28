# Changelog

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
