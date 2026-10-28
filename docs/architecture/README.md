# Architecture

Inheritance Engine is one Rust binary (the trusted control plane) plus a dependency-free web UI. Untrusted
repository code never runs inside it: it runs in Docker, or natively only when the user explicitly picks that.

```
 Browser (web/, vanilla ES modules)
        │  HTTP, loopback only, Host/Origin checked
        ▼
 ┌──────────────────────────── control plane (src/) ─────────────────────────────┐
 │ api.rs ── app.rs (durable jobs, pause/resume/cancel, restart recovery)        │
 │              │                                                                 │
 │   pipeline.rs: investigate │ restore │ verify                                  │
 │      │            │             │              │                               │
 │  gitops.rs   inventory.rs   agents.rs ──► models.rs ──► open-weight model      │
 │  (clone,     (static facts, (Lead,        (OpenAI-compatible / Ollama,         │
 │   successor)  decay, trust   Historian,    privacy policy at every call)       │
 │               passport)      Restorer,                                         │
 │                              Reviewer,     evidence.rs (engine re-checks       │
 │                              Hunter)       every citation on disk)             │
 │  hunter.rs (GitHub API)   record.rs (Inheritance Record)   db.rs (SQLite)      │
 └───────────────────────────────┬────────────────────────────────────────────────┘
                                 │ sandbox.rs (no shell in the control plane)
                                 ▼
      docker run --network none --read-only --cap-drop ALL --pids-limit … python:3.12-slim
                    (or: native, cleared environment, explicit user choice)
```

## Data layout

```
data/
  engine.sqlite3                     jobs, issues, repairs, verifications, chat, agent calls, artifacts
  inheritances/<id>/
    original.git/                    bare clone of the source at the recorded commit, made read-only
    successor/                       working copy on branch `successor`; push URLs disabled
    records/record-vN.json           every exported Inheritance Record version
    native-deps/, native-home/       only in native mode
docker volume ie-deps-<id>           dependencies for Docker runs (outside the successor tree)
```

## The journey

1. **Hunter** — `GET /api/hunt`. Problem-first: the Hunter agent turns a problem into keywords + language (recorded call),
   the engine builds and sanitizes the GitHub query. Curiosity-first: quiet repositories sorted by stars. Results carry
   *signals* (historical / present / evolution / caution) rather than a score. `POST /api/hunt/explain` has Gemma read
   the README and reason about the three dimensions separately.
2. **Intake** — `POST /api/inheritances` with an explicit execution mode. Bare clone with hardening flags (no hooks,
   no symlinks, no credential helpers, no `ext::`), made read-only. The successor is a local clone on its own branch.
3. **Inventory** (deterministic) — tree, languages, manifests, entry points, Python/JS import graph, central-component
   ("mother script") scoring from fan-in + history churn + entry points + README mentions (size is a minor term so the
   largest file is not automatically central), a runtime-decay scan, and the trust passport.
4. **Baseline verification** — dependency install (network on, as root in the container, requirements only, npm with
   `--ignore-scripts`), syntax check and tests (network off, unprivileged user).
5. **Agent team** — the Lead reads the engine's facts and decides which ≤4 files each specialist receives (validated
   against the real file list, with deterministic fallbacks). The Historian reconstructs purpose/architecture with
   citations; the Restorer diagnoses issues and maps them to decay groups; the Reviewer challenges every claim and issue
   with the engine's citation checks in view; the Lead writes the summary. Each call is a recorded `AgentCall` with its
   parent, input files, prompt and response.
6. **Restoration** — per issue: Restorer patch (search/replace edits) → atomic application inside the successor → Reviewer
   checks the actual diff → Docker verification → commit with provenance trailers, or revert. When verification still
   fails, the Restorer re-diagnoses from the new output (up to two rounds).
7. **Record** — `GET /api/inheritances/{id}/record` exports a versioned JSON record; `INHERITANCE.md` is committed to the successor.

## Why the engine, not the model, has the last word

Small local models are useful but fallible. The engine enforces the parts that must be true:

| Concern | Enforcement |
| --- | --- |
| Citations are real | `evidence::check` re-reads the cited lines; relocates exact quotes; flags `mismatch`/`missing_file` |
| "Verified" means verified | `enforce_status` downgrades claims without a verified citation to `inferred` |
| Edits stay in the successor | `safe_join` (no `..`, no absolute paths, no `.git`, symlink-safe); no file creation |
| Edits are all-or-nothing | `apply_edits` stages every edit in memory; writes only if all apply |
| Reviewer approval is coherent | an approval contradicting its own checks becomes `request_changes` |
| Broken fixes are not kept | revert on regression, or when the failure's innermost frame is inside the edited lines |
| Commands are not model-chosen | verification commands come from detection or the user's own input |

## Durable jobs

Jobs and steps live in SQLite. Each pipeline step first checks its checkpoint artifact, so `resume` on an interrupted
job (for example after a restart, which marks running jobs `interrupted`) continues without repeating finished model
calls or test runs. Pause and cancel are honoured between steps. This is deliberately simpler than a workflow engine
such as Temporal; the step/checkpoint shape maps onto one if long-running cloud workers are added later.

## Model adapters

`models.rs` speaks two protocols: OpenAI-compatible `/chat/completions` with `response_format: json_schema`, and Ollama
`/api/chat` with `format: <schema>`. Locality is derived from the endpoint host; under `local_only` a non-loopback
endpoint is refused before any request is made. Per-agent model overrides (`IE_MODEL_REVIEWER`, …) allow a larger model
for the independent reviewer.

## Inheritance Record (schema 1.0)

Top-level keys: `schema_version`, `record_version`, `project`, `inheritance`, `archaeology`, `restoration`, `successor`,
`provenance`. The record references evidence (paths, lines, commits, commands, exit codes) and includes agent call
metadata but not prompts, and not whole source files.
