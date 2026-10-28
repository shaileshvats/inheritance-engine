# HTTP API

The control plane listens on `127.0.0.1:7878` by default and serves the web UI at `/` and JSON under `/api`.
All responses are JSON. Errors use HTTP status codes with a body of `{ "error": "human readable message" }`.
Timestamps are RFC 3339 strings. IDs are opaque strings.

## System

`GET /api/status`

```json
{
  "version": "0.1.0",
  "model": { "provider": "openai-compatible", "model": "google/gemma-4-e4b", "endpoint": "http://127.0.0.1:1234/v1",
             "locality": "local", "reachable": true, "detail": null },
  "docker": { "available": true, "version": "29.5.3", "detail": null },
  "privacy_mode": "local_only",
  "github_token": false,
  "data_dir": "./data"
}
```

## Hunter (discovery)

`GET /api/hunt?q=<text>&mode=problem|curiosity&lang=<optional language>`

- `problem` mode: Gemma rewrites the user's problem into a GitHub search query (a real model call, recorded).
- `curiosity` mode: the text is used as topic/keywords and the query is biased towards old, quiet repositories.

```json
{
  "mode": "problem",
  "query_used": "markdown table parser language:python pushed:<2020-01-01",
  "ai_query": { "used": true, "call_id": "call_…", "reasoning": "…" },
  "total_count": 1234,
  "results": [Candidate]
}
```

`Candidate`:

```json
{
  "full_name": "owner/repo", "owner": "owner", "name": "repo", "html_url": "https://github.com/owner/repo",
  "description": "…", "language": "Python", "topics": ["cli"], "stars": 120, "forks": 14, "open_issues": 3,
  "created_at": "…", "pushed_at": "…", "archived": false, "license": "MIT", "size_kb": 340,
  "default_branch": "master", "age_years": 9.1, "idle_years": 6.3,
  "signals": [ { "kind": "historical|present|evolution|caution", "label": "Quiet for 6.3 years", "detail": "Last push 2019-07-02" } ]
}
```

Signals are explainable evidence, never a quality score.

`POST /api/hunt/explain` body `{ "full_name": "owner/repo" }` — Gemma reads the README (fetched from GitHub) and explains why the
project could be worth inheriting.

```json
{
  "full_name": "owner/repo",
  "explanation": { "summary": "…", "historical_value": "…", "present_usefulness": "…", "evolution_potential": "…", "caveats": ["…"] },
  "readme_chars": 4210,
  "call_id": "call_…"
}
```

## Inheritances

`GET /api/inheritances` → `[InheritanceSummary]`

`POST /api/inheritances` body `{ "full_name": "owner/repo", "execution_mode": "docker|native|none" }` → `InheritanceSummary`.
Starts an `investigate` job immediately (clone → inventory → baseline verification → agent team).

`InheritanceSummary`:

```json
{
  "id": "inh_…", "name": "repo", "full_name": "owner/repo", "source_url": "https://github.com/owner/repo",
  "original_commit": "sha or null until cloned", "license": "MIT or null", "stage": "inherited",
  "execution_mode": "docker", "created_at": "…", "updated_at": "…",
  "last_verification": Verification | null,
  "active_job": Job | null,
  "counts": { "findings": 0, "issues": 0, "repairs": 0, "agent_calls": 0 }
}
```

`stage` is one of `discovered, inherited, investigating, understanding, restoring, verified, evolving`. Stages are descriptive and can move backwards.

`GET /api/inheritances/{id}` → `InheritanceDetail` = all summary fields plus:

```json
{
  "github": { "description": "…", "stars": 1, "forks": 0, "pushed_at": "…", "created_at": "…", "default_branch": "master", "owner": "…" },
  "original": { "commit": "sha", "branch": "master", "integrity": { "checked_at": "…", "ok": true, "head": "sha" } },
  "successor": { "name": "repo-successor", "branch": "successor", "head": "sha", "commits_ahead": 2 },
  "inventory": Inventory | null,
  "passport": Passport | null,
  "report": Report | null,
  "issues": [Issue],
  "repairs": [Repair],
  "verifications": [Verification],
  "jobs": [Job],
  "lineage": [LineageEntry]
}
```

### Inventory (deterministic static analysis, no AI)

```json
{
  "files_total": 31, "bytes_total": 120394,
  "languages": [ { "language": "Python", "files": 20, "lines": 3100 } ],
  "tree": { "name": "repo", "path": "", "kind": "dir", "size": 0, "children": [TreeNode] },
  "manifests": [ { "path": "setup.py", "kind": "setuptools" } ],
  "entry_points": [ { "path": "tool/cli.py", "kind": "__main__ guard", "detail": "line 88" } ],
  "tests": { "framework": "pytest|unittest|npm|none", "files": ["tests/test_x.py"] },
  "mother_candidates": [ { "path": "tool/core.py", "score": 0.82, "reasons": ["imported by 7 modules", "touched by 41 commits"] } ],
  "graph": { "nodes": [ { "id": "tool/core.py", "path": "tool/core.py", "lang": "Python", "fan_in": 7, "fan_out": 2, "entry": false, "test": false } ],
             "edges": [ { "from": "tool/cli.py", "to": "tool/core.py" } ] },
  "git": { "commits": 120, "first_commit_at": "…", "last_commit_at": "…",
           "authors": [ { "name": "…", "commits": 90 } ],
           "recent": [ { "sha": "…", "date": "…", "author": "…", "subject": "…" } ] },
  "decay": [ { "path": "tool/core.py", "line": 12, "pattern": "collections.Mapping", "detail": "Moved to collections.abc", "removed_in": "Python 3.10" } ]
}
```

`TreeNode`: `{ "name", "path", "kind": "dir|file", "size", "children"?: [TreeNode] }`.

### Passport (trust passport, deterministic)

```json
{
  "provenance": { "source_url": "…", "original_commit": "…", "cloned_at": "…", "license": "MIT" },
  "install_scripts": [Signal], "network_access": [Signal], "filesystem_access": [Signal],
  "process_execution": [Signal], "dynamic_code": [Signal],
  "secrets": [ { "path": "…", "line": 3, "kind": "AWS access key", "redacted": "AKIA…[redacted]" } ],
  "dependencies": [ { "ecosystem": "pypi", "name": "requests", "spec": ">=2", "source_path": "requirements.txt" } ],
  "unresolved": ["…"]
}
```

`Signal`: `{ "path", "line", "detail" }`.

### Report (agent team output)

```json
{
  "lead_summary": "markdown text written by the Lead Engineer",
  "purpose": Claim,
  "capabilities": [Claim],
  "components": [ { "path": "…", "role": "…", "evidence": [Evidence], "status": "verified|inferred|unknown" } ],
  "data_flows": [Claim],
  "external_services": [Claim],
  "build_instructions": "…",
  "unknowns": ["…"],
  "reviewer_challenges": [ { "target": "claim id or issue id", "verdict": "agree|disagree|insufficient_evidence", "reason": "…" } ],
  "next_actions": ["…"],
  "generated_at": "…"
}
```

`Claim`: `{ "id": "c1", "text": "…", "status": "verified|inferred|unknown", "agent": "historian", "evidence": [Evidence] }`

`Evidence`: `{ "path", "start_line", "end_line", "quote", "check": "verified|mismatch|missing_file|out_of_range|unchecked" }`.
`check` is computed by the engine (not the model) by reading the cited lines from disk.

### Issues, repairs, verification

`Issue`:
```json
{ "id": "iss_…", "title": "…", "category": "essential|improvement", "severity": "high|medium|low",
  "diagnosis": "…", "evidence": [Evidence], "proposed_fix": "…", "confidence": "high|medium|low",
  "source": "decay-scan|verification|restorer", "status": "open|repaired|rejected|failed|skipped" }
```

`Repair`:
```json
{ "id": "rep_…", "issue_id": "iss_…", "status": "applied|reverted|rejected|failed",
  "rationale": "…", "diff": "unified diff", "files": ["…"], "commit": "sha or null",
  "review": Review | null, "verification_before": "ver_… or null", "verification_after": "ver_… or null",
  "note": "why reverted/failed, if so", "created_at": "…" }
```

`Review`: `{ "verdict": "approve|request_changes|reject", "root_cause_addressed": true, "regression_risk": "low|medium|high", "concerns": ["…"], "call_id": "call_…" }`

`Verification`:
```json
{ "id": "ver_…", "label": "baseline tests", "mode": "docker|native", "environment": "python:3.12-slim",
  "command": "python -m pytest -q", "exit_code": 1, "status": "passed|failed|error|not_run",
  "summary": { "passed": 10, "failed": 3, "errors": 1, "skipped": 0 } | null,
  "output_tail": "last ~120 lines (secrets redacted)", "duration_ms": 8123, "started_at": "…",
  "workspace_rev": "sha", "network": "none", "reason": "set when not_run" }
```

`Job`:
```json
{ "id": "job_…", "kind": "investigate|restore|verify", "status": "queued|running|paused|completed|failed|cancelled|interrupted",
  "step": "current step name", "steps": [ { "name": "clone", "status": "pending|running|done|skipped|failed", "detail": "…", "started_at": "…", "finished_at": "…" } ],
  "error": null, "created_at": "…", "updated_at": "…" }
```

`LineageEntry`: `{ "kind": "original|successor", "sha": "…", "label": "…", "date": "…", "parent": "sha or null", "author": "…" }`

### Actions

| Method | Path | Body | Result |
| --- | --- | --- | --- |
| POST | `/api/inheritances/{id}/investigate` | – | `Job` (re-runs only incomplete steps) |
| POST | `/api/inheritances/{id}/restore` | `{ "mode": "essential\|fix_all\|selected", "issue_ids": [] }` | `Job` |
| POST | `/api/inheritances/{id}/verify` | `{ "command": "optional user-edited command" }` | `Job` |
| POST | `/api/inheritances/{id}/execution-mode` | `{ "mode": "docker\|native\|none" }` | `InheritanceSummary` |
| PATCH | `/api/inheritances/{id}/successor` | `{ "name": "new name" }` | `InheritanceSummary` |
| POST | `/api/jobs/{id}/cancel` \| `/pause` \| `/resume` | – | `Job` |
| GET | `/api/inheritances/{id}/record` | – | Inheritance Record JSON (see docs/architecture) |
| GET | `/api/inheritances/{id}/file?path=…&side=successor\|original` | – | `{ "path", "side", "content", "lines", "truncated" }` |
| GET | `/api/inheritances/{id}/diff` | – | `{ "base": "sha", "head": "sha", "diff": "…" }` |

### Lead Engineer chat and agent traces

`GET /api/inheritances/{id}/chat` → `[ { "role": "user|lead", "content": "…", "at": "…", "call_id": "…|null", "suggested_action": "none|investigate|restore_essential|restore_all|verify|null" } ]`

`POST /api/inheritances/{id}/chat` body `{ "message": "…" }` → the new lead message (same shape). Suggested actions are
never executed automatically; the UI shows them as buttons.

`GET /api/inheritances/{id}/calls` →
```json
[ { "id": "call_…", "agent": "lead|historian|restorer|reviewer|hunter", "purpose": "…", "provider": "…", "model": "…",
    "started_at": "…", "duration_ms": 9123, "input_files": ["…"], "prompt_chars": 8000, "response_chars": 900,
    "ok": true, "error": null, "parent_id": "call_…|null" } ]
```

`GET /api/calls/{id}` → the same plus `"system"`, `"prompt"` and `"response"` (secrets redacted).
