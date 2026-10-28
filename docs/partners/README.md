# Partner category audit (HF26 Week 4)

Audited on 2026-10-28 (updated the same day after the Mastra, Sentry and DigitalOcean work) against the official category descriptions on the [HF26 Challenge Hub](https://dev.to/challenges/hf26)
and the code in this repository. The official rule is: *"Use only partner tech your project actually uses."*
A category is marked **Implemented** only when the code uses the technology in the product and there is evidence
to show for it.

Deadline: 2026-10-31 23:59 PDT. A submission can win only once, and writing quality is weighted most heavily.

## Summary

| Category | Status | Feasible before deadline? |
| --- | --- | --- |
| Gemma (featured) | **Implemented** | Done |
| Mastra | **Implemented**: Lead Engineer agent + two workflows, used by the product; real runs recorded | Done |
| Sentry Agent Tracing | **Implemented, not yet demonstrated**: engine and companion instrumented, tested offline | Needs the owner's free Sentry DSN to capture real traces |
| DigitalOcean (featured) | **Prepared, not deployed**: compose stack and images built locally | Needs the owner to sign in and confirm credits; nothing billable created |
| Temporal | Not implemented; SQLite checkpoints do the same job today | Possible but high risk: about 1.5 days, second runtime |
| Auth0 for AI Agents | Not implemented, real fit (GitHub actions on the user's behalf, with approval) | Risky: 1–2 days plus Auth0 tenant and GitHub connection setup |
| SerpApi | Not implemented, minor fit | Yes, a few hours. Low product value |
| Ultramarine (Paper Instruments) | Not implemented; it is a hosted model behind an OpenAI-compatible adapter | Yes, a few hours. Conflicts with the local-only default; opt-in comparison only |
| Backboard | Not implemented, weak fit | Possible, low value |
| MongoDB Atlas / Tiger Data | Not implemented; no real need yet | Possible (restoration-recipe retrieval), questionable value with so few runs |
| ElevenLabs | Not implemented | Demo narration qualifies officially, but it isn't a product feature |
| Render (featured) | Not implemented; poor fit for the full engine | Only as a hosted Hunter-only front end. Overlaps DigitalOcean |
| .tech Domain | Not implemented | Only together with a hosted deployment; free first year, renews at $29.99/yr |
| Browserbase | Not implemented | No: the demo repository has no web UI to test |
| GitHub Copilot | Not used | No, not genuinely: Copilot was not used to build this |
| Entire | Not used | No for past work. Future sessions could be captured |
| TabPFN (featured) | Not implemented | No: there's no real historical dataset, and fabricating one is not acceptable |
| Tinker (featured) | Not implemented | No: needs a dataset, a fine-tune, and a measured improvement over a baseline |
| Arduino (featured) | Excluded | No: needs UNO Q hardware |
| Qualcomm | Unknown | The hub shows only a logo with no requirements |

## Details

### Gemma — Implemented
- **Requirement:** "Use Gemma, Google's open-weight model, in building your project", locally, fine-tuned, or served.
- **In the product:** Gemma 4 E4B, running locally in LM Studio, powers every agent role: Lead, Historian, Restorer,
  Reviewer, Tester and Hunter. It uses constrained JSON output.
- **Evidence:** the agent trace in the UI (model name on every call), `docs/demo/evidence/fn-py-run-a-record.json`
  (`provenance.attribution.models`), and the demo runs in `docs/demo/README.md`.
- **Remaining:** none. Gemma 4 12B was downloaded but could not be loaded on this 16 GB machine, and is not claimed.

### Mastra — Implemented
- **Requirement:** "Orchestrate an agent over open models, add memory and tools to a chat agent, or build a workflow
  that triages GitHub issues."
- **In the product** (`companion/mastra`, `@mastra/core` 1.79, Gemma 4 E4B via LM Studio):
  - `leadEngineer`, an agent with five engine tools. The engine's Lead chat goes through it when `IE_MASTRA_URL` is set,
    and the UI marks those answers "Lead · Mastra · Gemma".
  - `answer-lead-question`, a workflow that calls the same tools deterministically, then has `leadEngineerGrounded`
    answer with structured output. It is chosen automatically because LM Studio 0.4.12 cannot render Gemma 4's
    template with tools (verified with direct requests). A runtime that can do tool calling gets native mode.
  - `triage-upstream-issues`, a workflow that triages GitHub issues (Mastra's own listed example). It loads the
    inheritance, fetches the original repository's open issues, classifies them with Gemma, then runs a deterministic
    citation check. It runs as a durable engine job from the Overview tab.
- **Evidence:** `docs/demo/evidence/fn-py-upstream-triage-mastra.json` is a real run on kachayev/fn.py: 24 open
  upstream issues, 8 triaged. #91 "ImportError: cannot import name Iterable from collections" is *addressed*, citing
  repair `rep_84e85726…` and commit `ed0b4e499d`, both verified by the engine. 7 are still relevant. The same result
  came from the UI-started job. See also screenshot `05-mastra-upstream-triage.jpg` and the agent trace entries with
  provider `mastra`. There are 17 vitest tests (citation rule, telemetry scrubbing, request guards, both workflows
  against a local fixture model, the HTTP server).
- **Limits:** native tool calling has not been exercised with Gemma 4 on this machine (runtime limitation above).
  Mastra storage is in memory, since the engine persists results. A 4B model sometimes repeats a heading.

### Sentry Agent Tracing — Implemented, awaiting a DSN for live evidence
- **Requirement:** "Show your agent's work: how it's set up, how it performs (latency, tokens, cost)," what you found
  and debugged, with traces or screenshots in the write-up.
- **In the product:** with `SENTRY_DSN` set, the engine sends one transaction per job (`gen_ai.invoke_agent`), with
  `gen_ai.chat` spans per model call (role, model, input/output/total tokens, latency, ok/error class) and
  `gen_ai.execute_tool` spans per sandbox run (install, syntax, tests, behaviour probe). The companion uses
  `@mastra/sentry` for its agent runs, model generations and tool calls.
- **Privacy:** purposes and errors are reduced to fixed categories. A scrubbing transport (engine) and
  `beforeSendTransaction` plus a span processor (companion) keep only allowlisted metadata keys. `cargo test
  telemetry` injects source code, a file path and an AWS-key-shaped string, then checks the captured envelope contains
  tokens and model names and none of that content.
- **Token usage** is now recorded for every model call and shown in the Agent trace (from the OpenAI-compatible
  `usage` field, or Ollama's eval counts). Cost is $0 per call: the model runs locally.
- **Remaining:** a real Sentry project. The DSN is the owner's to create. Then run one job and capture the trace screenshot.

### DigitalOcean — Prepared, not deployed
- **Requirement:** "Build or deploy your project on DigitalOcean": host an app or agent, or run an open-weight model on a GPU Droplet.
- **Prepared:** `deploy/digitalocean/` contains a compose stack (Ollama + Gemma 4, engine, Mastra companion,
  sandboxes through the host Docker), Dockerfiles, a cloud-init file (SSH-only firewall), and a guide with sizing,
  cost warnings and teardown.
- **Validated locally:** both images build. The companion container starts, serves `/health`, and refuses a non-local
  model endpoint. The engine container, running with the Docker socket, ran a real clone, inventory and sandboxed
  baseline from inside the container, and its privacy policy refused a non-loopback model endpoint. Details are in
  `deploy/digitalocean/README.md`.
- **Not done:** no Droplet exists. The Hacktoberfest promos page requires the owner's MLH sign-in to check
  DigitalOcean credits. Creating a Droplet is billable and needs the owner's account and approval. **Not claimed until
  deployed and demonstrated.**

### Temporal — possible, not recommended
- **Requirement:** "Wrap [your agent] in a Temporal workflow so it survives failures," retries flaky tool calls, and
  picks up where it left off.
- **Status:** the engine already survives failures with SQLite checkpoints. It was killed mid-run and resumed in the demo.
  Temporal has no stable Rust SDK, so integrating it means a TypeScript or Python worker that drives the pipeline steps
  as activities over HTTP, plus a local Temporal dev server.
- **Assessment:** genuine but duplicative, and the riskiest change to the core with 3–4 days left.

### Auth0 for AI Agents — possible, risky
- **Requirement:** sign users in to an agent; call tools on a user's behalf with Token Vault; human approval for risky
  actions; fine-grained authorization for retrieval.
- **Fit:** real. Successor actions on GitHub (create a branch or PR on the user's fork) via Token Vault, gated by human
  approval, exactly as the PRD's approval boundaries describe. Today the engine deliberately has no push path at all.
- **Work:** Auth0 tenant, GitHub connection with Token Vault, local login callback, approval UI, and a new outward-facing
  action that needs careful security review. 1–2 days.

### SerpApi — feasible, low value
- **Requirement:** live web search for an agent, RAG grounding, or tracking news for an open-source project.
- **Possible use:** the Historian searches the web for talks and posts about an inherited project, as context for its
  original intent. GitHub search does not need it, per the PRD.

### Ultramarine (Paper Instruments) — feasible, opt-in only
- **Requirement:** put Ultramarine to work on something real via the OpenAI/Anthropic SDK, and report what the run cost.
- **Possible use:** an opt-in second-opinion Reviewer using the existing OpenAI-compatible adapter, with a cost report.
  It's a hosted model, so it only works outside `local_only` and never touches private repositories.

### Backboard, MongoDB Atlas, Tiger Data
- **Backboard** (memory/RAG, multi-model API): there's no current need. Project memory is local by design.
- **Atlas / Tiger Data** (vector or hybrid search): the genuine future use is a shared, opt-in library of verified
  restoration recipes for the Restorer to retrieve. That needs many more runs to be meaningful. If built, choose one,
  not both.

### ElevenLabs
- "Generate narration for your demo" is an officially listed use. It wouldn't be a product feature. Your call.

### Render and .tech Domain
- **Render** hosts web services without Docker-in-Docker, so the full engine (which must sandbox code) can't run there. A
  hosted Hunter-only front end is possible but overlaps DigitalOcean. The PRD says not to integrate two competing
  infrastructure providers for coverage.
- **.tech** requires the project to be live on the domain, so it only makes sense alongside a hosted deployment.

### Not feasible or not genuine before the deadline
- **TabPFN:** predicting "restorability" would need real labelled outcomes from many restorations; we have three runs.
- **Tinker:** needs a training set, a fine-tune and a measured improvement over a baseline.
- **Browserbase:** needs an inherited project with a web UI. That's a different demo.
- **GitHub Copilot, Entire:** neither was used to build this project, and claiming them retroactively would be false.
- **Arduino:** no UNO Q available. **Qualcomm:** no published requirements found.
