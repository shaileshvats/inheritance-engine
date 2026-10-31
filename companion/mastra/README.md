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
npm test             # 19 tests: rules, workflows against a local fixture model, HTTP server
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

## HTTP API (loopback only, no cross-origin requests)

- `GET /health`: model, tool mode, agents, workflows, and whether Sentry is on.
- `POST /lead/chat` `{inheritanceId, message, history?}`: `{reply, proposed_action, tool_calls, mode, usage, duration_ms}`.
- `POST /triage` `{inheritanceId, maxIssues?}`: `{repository, total_open_upstream, triaged[], counts, run_id}`.
