# Inheritance Engine

**Find forgotten software. Understand it. Restore it. Give it a future.**

Inheritance Engine is a local-first workbench for inheriting software whose creator has moved on. It finds a
repository on GitHub, clones it into an isolated workspace without ever touching the original, has a team of AI agents
running on an **open-weight model on your own machine (Gemma 4)** reconstruct what the project was for, diagnoses why it
no longer runs, repairs it in a traceable successor, and proves each repair with real test runs, all recorded as
evidence in a portable Inheritance Record.

Built for the DEV Hacktoberfest 2026 Week 4 challenge, *Ghost in the Machine*.

> Status: hackathon MVP (v0.1.0). It works end to end on real repositories, and it has real limits; see
> [What it can't do yet](#what-it-cant-do-yet). Not production software.

## What it does

| Stage | What happens | Who does it |
| --- | --- | --- |
| **Hunter** | Search GitHub by problem ("I need lazy streams in Python") or by curiosity. Results show evidence signals: history, present usefulness, evolution potential, cautions. They are not a score. | GitHub API + Hunter agent |
| **Inherit** | Bare clone kept **read-only**; successor clone on its own branch with **push disabled**. Original commit and license recorded. | Engine |
| **Investigate** | File tree, languages, entry points, import graph, likely central components ("mother scripts"), a scan for APIs that modern runtimes removed, and a **trust passport** (install scripts, network/filesystem/process/dynamic-code use, secrets, dependencies). | Engine (deterministic) |
| **Baseline** | Dependencies installed and tests run in Docker: network off, read-only rootfs, capabilities dropped. | Engine + Docker |
| **Understand** | The Lead plans and hands each specialist only the files it needs. The Historian reconstructs purpose and architecture with citations. The Restorer diagnoses problems. The Reviewer challenges both. The Lead writes the summary. | Gemma agents |
| **Restore** | Patch, then engine guards, then independent review, then Docker tests, then a behaviour probe against the original, then commit or revert. If tests still fail, the Restorer re-diagnoses from the new failure. | Gemma agents + engine |
| **Preserve** | `INHERITANCE.md` is committed to the successor, and a versioned JSON Inheritance Record is exported. | Engine |

### The engine, not the model, has the last word

Small local models are useful and fallible. Rather than hoping the model is right, the engine checks:

- **Citations are re-read from disk.** A claim marked "verified" with no verifiable citation is downgraded to "inferred".
- **Edits are atomic and contained.** They can't escape the successor, touch `.git`, or create files.
- **Dangling imports are caught before review.** If a patch removes an import that the file still uses, it is reverted.
- **Bad fixes are reverted.** This covers fixes that make tests worse, and fixes after which the failure originates inside the edited lines.
- **Tests aren't the whole story.** A Tester agent writes probe calls for the changed code. The engine runs them on the
  successor with Python 3.12 *and* on the original commit with **the Python of its era**, worked out from the decay scan.
  If the outputs differ, the repair is reverted.
- **No model chooses a shell command.** Verification commands come from detection, or from text you type.

Every one of these rules exists because a real run against a real repository needed it. See
[`docs/demo/README.md`](docs/demo/README.md).

## See it work in a few minutes, without a model

The quickest way to see the engine's central idea is a **replay**. On 2026-10-31, Gemma proposed a fix for fn.py that the
Reviewer agent approved and that kept all 126 tests passing, but that broke behaviour the tests never exercised. The
behaviour probe caught it. A replay feeds that recorded patch, review and probe code back through the engine's **live**
checks, so you can watch the catch happen on your own machine:

```bash
cargo run --release        # then open http://127.0.0.1:7878 → "Replay a recorded catch" → Run this replay
```

Or from a second terminal:

```bash
curl -s -X POST http://127.0.0.1:7878/api/replays -H 'Content-Type: application/json' -d '{}'
```

What is recorded and what is live:

| Recorded model output (Gemma 4 E4B, 2026-10-31) | Done live by your run |
| --- | --- |
| The Restorer's patch (`f.prflags`) and rationale | Clone of kachayev/fn.py at its 2014 commit, kept read-only |
| The Reviewer's verdict: approve, regression risk low | Inventory and decay scan that picks the era runtime (Python 3.9) |
| The Tester's three probe snippets | Tests in Docker before and after the patch (126 → 126 passed) |
| | Probes on the original under Python 3.9 and on the successor under 3.12 |
| | The engine's decision (reverted) and a new Inheritance Record |

It needs Rust, Git, Docker, and network access to GitHub and the Python package index. No model or GPU is needed.
With the Docker images already pulled it takes about 20 seconds; the first run also pulls `python:3.12-slim` and
`python:3.9-slim`. Replays are labelled as replays in the UI and never commit recorded output.

## Quick start (macOS, Linux)

Requirements: Rust 1.89+, Git, Docker (Desktop or Engine), and a local model server: [LM Studio](https://lmstudio.ai)
or [Ollama](https://ollama.com).

```bash
# 1. A local open-weight model (LM Studio shown; Gemma 4 E4B is ~6 GB and runs on a 16 GB laptop)
lms get google/gemma-4-e4b
lms load google/gemma-4-e4b --context-length 16384
lms server start

# 2. The engine
cargo run --release
# → http://127.0.0.1:7878
```

Using Ollama instead:

```bash
IE_MODEL_PROVIDER=ollama IE_MODEL_ENDPOINT=http://127.0.0.1:11434 IE_MODEL=gemma3:4b cargo run --release
```

All configuration is optional and lives in environment variables (see [`.env.example`](.env.example)). The defaults are
local-only: `IE_PRIVACY_MODE=local_only` refuses any non-loopback model endpoint before a single byte is sent.

### Choosing how inherited code runs

You choose this every time you inherit a project. The AI never switches it for you.

- **Docker** (recommended): network off for tests, 1 GiB RAM, 2 CPUs, 256 processes, read-only root filesystem,
  capabilities dropped, non-root user, no host environment. Dependency installation is the only networked step.
- **Native**: a cleared environment inside the successor directory. Network and filesystem access are **not** restricted.
- **None**: static analysis and agent review only.

## Using it

1. **Hunter.** Describe a problem or a topic, and press *Why is this interesting?* on a result.
2. **Inherit.** Pick an execution mode. The investigation starts on its own and you can watch every step.
3. **Overview.** Talk to the Lead Engineer. Try *"Find what is broken and fix the important problems without destroying the
   original behaviour."* Its suggested actions are buttons, and nothing runs without your click.
4. **Restoration.** Choose *Fix essential*, *Fix everything*, or *Fix selected*. Each repair shows its diff, the review, before
   and after test runs, and the behaviour probe.
5. **Agents.** The trace of every model call: role, model, duration, files given, full prompt and response (secrets redacted).
6. **Record.** Lineage from original to successor commits, a diff against the original, and the downloadable Inheritance Record.

The successor is an ordinary Git repository at `data/inheritances/<id>/successor` on branch `successor`. Inheritance
Engine never pushes anything anywhere.

## Architecture

One Rust binary (axum, SQLite) is the trusted control plane, and a dependency-free web UI talks to it over loopback.
Inherited code runs only in Docker, or natively when you explicitly choose that. See
[`docs/architecture`](docs/architecture/README.md), [`docs/security`](docs/security/README.md), and
[`docs/api`](docs/api/README.md).

```
src/
  hunter.rs      GitHub discovery + Hunter agent        inventory.rs  deterministic static analysis
  gitops.rs      hardened clone, read-only original     sandbox.rs    Docker / native execution adapters
  agents.rs      agent roles, schemas, traced calls     models.rs     OpenAI-compatible + Ollama adapters
  pipeline.rs    investigate / restore / verify         probe.rs      differential behaviour probes
  evidence.rs    citation checking                      verify.rs     test plans, result parsing, regression rules
  record.rs      Inheritance Record                     app.rs        durable jobs (pause/resume/cancel/recover)
  api.rs         HTTP API + localhost guards            security.rs   path containment, secret redaction
web/             vanilla ES modules, no build step, no external requests
```

## Optional integrations

All of these are optional. The core journey runs without them.

**Mastra companion** ([`companion/mastra`](companion/mastra/README.md)). This TypeScript service, built on
[Mastra](https://mastra.ai), runs the Lead Engineer chat as a Mastra agent with engine tools, plus a
`triage-upstream-issues` workflow that classifies the original repository's open GitHub issues against what the
successor changed. Both run on the same local Gemma model. The engine keeps the last word: a triage verdict of
"addressed" survives only if it cites a repair or commit that exists.

```bash
cd companion/mastra && npm ci --ignore-scripts && npm run serve     # Node.js 22.18+
IE_MASTRA_URL=http://127.0.0.1:7880 cargo run --release
```

**Sentry agent tracing.** Set `SENTRY_DSN` for the engine and the companion. Each job becomes a trace: the agent
team's work (`gen_ai.invoke_agent`), every model call (`gen_ai.chat` with role, model, tokens, latency, outcome), and
every sandbox run (`gen_ai.execute_tool`). Only metadata leaves the machine. Tests check this by injecting source code
and secret-shaped strings, and it was verified against a real Sentry project (see `docs/partners`). Set
`IE_TELEMETRY_AUDIT=path` to keep a local copy of exactly what is sent.

**Web search (SerpApi).** Set `SERPAPI_API_KEY` (the free plan is enough). The Hunter gets an *Also search the web*
option that finds repositories GitHub's keyword search misses; each is re-fetched from the GitHub API and shows the page
it was found on. On an inheritance, *Research on the web* runs three searches and has Gemma summarise related projects
and documentation. Every item must cite a real result or the engine drops it, and the result feeds the Lead Engineer.
Only search text and public repository names are sent.

**Self-hosting.** [`deploy/self-host`](deploy/self-host/README.md) is a Docker Compose stack (engine, companion,
Ollama with Gemma 4, Docker sandboxes) for a dedicated Linux host, reachable only through SSH. Its images were built
and run locally; it has not been deployed to any cloud provider.

## What runs offline, and what needs the internet

| Works offline (once the repository and its dependencies are cached) | Needs the internet |
| --- | --- |
| Every Gemma agent: Hunter explanations, Historian, Restorer, Reviewer, Tester, Lead Engineer | Cloning the original from GitHub, and Hunter's GitHub search |
| Investigation, decay scan, trust passport, citation checks | Installing the inherited project's dependencies (the only networked sandbox step) |
| Repairs, Docker tests (network off), behaviour probes, revert rules | Pulling Docker images the first time |
| `INHERITANCE.md` and the Inheritance Record | Mastra upstream-issue triage (reads GitHub issues) |
| The Mastra Lead Engineer chat (local model) | SerpApi web search and Sentry tracing, both optional |

Repository content never leaves the machine: SerpApi receives search text and public repository names, and Sentry
receives metadata only.

## Tests

```bash
cargo test
```

The engine has 59 Rust tests and the Mastra companion has 19 vitest tests (`cd companion/mastra && npm test`). They cover path traversal and symlink escapes, atomic edits, locked-down Docker
arguments, the cleared native environment, push being disabled and the original staying untouched, privacy policy
enforcement, secret redaction, fabricated-citation detection, the regression and improvement rules, dangling-import
detection, traceback parsing, test-summary parsing, and era-runtime selection. End-to-end runs against real repositories
are documented with their actual results in [`docs/demo`](docs/demo/README.md).

## Open-source AI, honestly

- **Model:** Gemma 4 E4B (open weights, Google), running locally in LM Studio through its OpenAI-compatible server. The
  adapters also speak Ollama natively, and any OpenAI-compatible server works. Development smoke tests also used Qwen3 4B
  through Ollama.
- **Per-agent models:** for example `IE_MODEL_REVIEWER=google/gemma-4-12b-qat` for a stronger independent reviewer.
- **Orchestration:** the core pipeline is plain Rust in this repository. The optional Mastra companion runs the Lead
  Engineer agent and the upstream-issue triage workflow. Temporal was considered and not used; see
  [`docs/decisions`](docs/decisions/README.md).

## What it can't do yet

- **Languages:** verification is implemented for Python and Node.js projects. Other languages get static analysis and
  agent review only.
- **Repositories:** public GitHub only. There's no OAuth, no private repos, and no GitLab or Bitbucket.
- **Docker isn't a perfect boundary.** The dependency-install step has network access. Native mode has little isolation.
- **Model limits:** a 4B-class model sometimes writes wrong patches. The engine catches many of them, but not all.
  Behaviour probes only cover what the Tester thought to probe.
- **Probes:** the original's era runtime is chosen from the decay scan. Projects with native dependencies may not install
  on that older runtime, and the probe then reports *inconclusive*.
- **No accounts and no collaboration.** It's a single-user local tool.
- **Not yet done:** SAST, container scanning, fuzzing, a malicious-repository fixture suite, and a penetration test.

## License

Inheritance Engine is **source-available** under the [PolyForm Noncommercial License 1.0.0](LICENSE). It is *not* an
OSI-approved open-source license: noncommercial use is permitted, and commercial use is not. See [NOTICE](NOTICE).

Inherited repositories keep their own licenses. The engine displays license metadata and records attribution. It never
relicenses anything, and lineage does not imply endorsement by original authors.

Contributions are welcome under the same terms, with DCO sign-off ([CONTRIBUTING.md](CONTRIBUTING.md)).
