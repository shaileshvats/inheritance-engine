# Mastra companion

A small TypeScript service built with [Mastra](https://mastra.ai) (`@mastra/core` 1.79). It gives Inheritance Engine:

- **Lead Engineer agent** (`leadEngineer`): answers the inheritor's questions with engine tools (`get-project-overview`,
  `list-issues`, `list-repairs`, `read-source-file`, `propose-action`), over the same local Gemma model. The tools only
  read; `propose-action` only *proposes*, and the engine shows it as a button you must press.
- **`answer-lead-question` workflow**: the same tools called deterministically, then the grounded Lead
  (`leadEngineerGrounded`) answers with structured output. It's used automatically when the local runtime can't do
  native tool calling. LM Studio 0.4.12 can't render Gemma 4's chat template with tools.
- **`triage-upstream-issues` workflow**: load the inheritance from the engine, fetch the original repository's open
  GitHub issues, classify each with the Issue Triager agent (Gemma), then run a deterministic check. "Addressed by the
  successor" survives only if it cites a repair or commit that really exists. Otherwise it is downgraded to
  `needs_human`.
- **Sentry agent tracing** (optional, `SENTRY_DSN`) via `@mastra/sentry`. Content is stripped twice: by a span processor
  (inputs, outputs, prompts, previews) and by an allowlist in Sentry's `beforeSendTransaction`. Only metadata is sent.

## Run

Requires Node.js 22.18 or newer (the server runs TypeScript directly with Node's built-in type stripping).

```bash
npm ci --ignore-scripts
npm run serve        # narrow loopback API on 127.0.0.1:7880, used by the engine (IE_MASTRA_URL)
npm run dev          # optional: Mastra Studio for exploring the agents and workflows
npm test             # 24 tests, plus 2 more with IE_TEST_MEMORY_DATABASE_URL set: rules, workflows, HTTP server, memory
```

Then start the engine with `IE_MASTRA_URL=http://127.0.0.1:7880`.

| Variable | Default | Meaning |
| --- | --- | --- |
| `IE_ENGINE_URL` | `http://127.0.0.1:7878` | engine API (read only) |
| `IE_MASTRA_HOST` / `IE_MASTRA_PORT` | `127.0.0.1` / `7880` | where this service listens (keep loopback) |
| `IE_MODEL_ENDPOINT` | `http://127.0.0.1:1234/v1` | OpenAI-compatible model server; must be loopback |
| `IE_MASTRA_MODEL` or `IE_MODEL` | `google/gemma-4-e4b` | model id |
| `IE_MASTRA_TOOL_MODE` | auto | `native` or `grounded`; auto-detected by probing the runtime |
| `SENTRY_DSN` / `SENTRY_ENVIRONMENT` | unset / `local` | optional agent tracing (metadata only) |
| `GITHUB_TOKEN` | unset | optional; raises GitHub API rate limits for triage |
| `IE_MEMORY_DATABASE_URL` | unset | optional restoration memory (Postgres with pgvector + pgvectorscale); must be loopback |
| `IE_EMBED_ENDPOINT` / `IE_EMBED_MODEL` | model endpoint / `text-embedding-nomic-embed-text-v1.5` | local embeddings for the memory; must be loopback |

## Restoration memory (optional, Tiger Data)

Each inheritance learns on its own, so a fix that worked or a bad fix that got caught in one project means nothing to
the next one. The restoration memory fixes that. It indexes every repair attempt the engine has recorded, across all
inheritances (replays are excluded), into Postgres. Each attempt is stored with its outcome, the engine's reason, the
diagnosis, the rationale, the review and the probe verdict. The Lead Engineer then searches it with **hybrid keyword +
vector search**: `tsvector` full-text ranking and pgvector cosine distance, fused by reciprocal rank. Results always
leave out the inheritance that's asking, so they're precedents from *other* projects, cited by repair id.

It runs on [Tiger Data](https://www.tigerdata.com)'s open-source Postgres image with pgvector and **pgvectorscale**
(DiskANN index). Embeddings come from the local LM Studio server (`nomic-embed-text`, 768 dimensions). The database and
the embedding endpoint must both be loopback, because repair text can quote inherited source. That's why this runs
self-hosted rather than on Tiger Cloud.

```bash
docker run -d --name ie-memory -p 127.0.0.1:5433:5432 -e POSTGRES_PASSWORD=ie \
  -v ie-memory-data:/home/postgres/pgdata/data timescale/timescaledb-ha:pg17
docker exec ie-memory psql -U postgres -c 'CREATE DATABASE memory'
lms load text-embedding-nomic-embed-text-v1.5      # or let LM Studio load it on first use
export IE_MEMORY_DATABASE_URL=postgres://postgres:ie@127.0.0.1:5433/memory
npm run memory -- index                             # also refreshed automatically before every Lead search
npm run memory -- search "cannot import name Iterable from collections"
```

With the variable set, the Lead gets a `search-restoration-memory` tool. In grounded mode, the answer workflow calls
it for each question plus the open issues. Without the variable, nothing changes.

Retrieval is checkable, and the cited repair ids and statuses are real. Gemma's *summary* of why a precedent failed
can still be wrong, so the ids are there to check. Treat answers built on the memory as leads, not evidence.

## HTTP API (loopback only, no cross-origin requests)

- `GET /health`: model, tool mode, agents, workflows, and whether Sentry is on.
- `POST /lead/chat` `{inheritanceId, message, history?}`: `{reply, proposed_action, tool_calls, mode, usage, duration_ms}`.
- `POST /triage` `{inheritanceId, maxIssues?}`: `{repository, total_open_upstream, triaged[], counts, run_id}`.
