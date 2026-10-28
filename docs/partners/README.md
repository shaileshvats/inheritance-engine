# Partner category audit (HF26 Week 4)

Audited on 2026-10-28 against the official category descriptions on the [HF26 Challenge Hub](https://dev.to/challenges/hf26)
and the code in this repository. The official rule is: *"Use only partner tech your project actually uses."*
A category is marked **Implemented** only when the code uses the technology in the product and there is evidence
to show for it.

Deadline: 2026-10-31 23:59 PDT. A submission can win only once, and writing quality is weighted most heavily.

## Summary

| Category | Status | Feasible before deadline? |
| --- | --- | --- |
| Gemma (featured) | **Implemented** | Done |
| Sentry Agent Tracing | Not implemented, strong fit | **Yes**: about half a day, needs a free Sentry project/DSN |
| Mastra | Not implemented, strong fit | **Yes**: about 1 day, adds a Node.js service |
| DigitalOcean (featured) | Not implemented, real fit (cloud worker) | Yes if Hacktoberfest credits are available. Otherwise it costs money and needs your approval |
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

### Sentry Agent Tracing — feasible, recommended
- **Requirement:** "Show your agent's work: how it's set up, how it performs (latency, tokens, cost)," what you found,
  how you debugged it, with traces or screenshots in the write-up.
- **Fit:** strong. Every agent call is already recorded locally with its parent, duration and files. Sentry would add
  hosted traces of the agent tree (Lead → Historian/Restorer/Reviewer → Tester), with token counts and latency.
- **Work:** record token usage from the model responses (not stored yet), then export spans by OTLP or the Rust
  `sentry` crate (0.49.x) using the gen_ai span conventions. It must be opt-in via `SENTRY_DSN`. Send metadata only
  (role, model, durations, tokens, outcome), never prompts or source code, to respect `local_only`.
- **Needs from you:** a free Sentry project and its DSN.

### Mastra — feasible, recommended
- **Requirement:** "Orchestrate an agent over open models, add memory and tools to a chat agent, or build a workflow
  that triages GitHub issues."
- **Fit:** two genuine uses.
  1. **Lead Engineer chat as a Mastra agent** over Gemma (OpenAI-compatible endpoint), with tools that call this engine's
     API: read findings, open a file, run a verification, propose a restore.
  2. **A workflow that triages the original project's open GitHub issues against the successor**: still relevant,
     fixed by the restoration, or obsolete. This matches Mastra's own listed example, and it's useful to an inheritor.
- **Work:** a small TypeScript service (`companion/mastra`), Node.js install, tool contracts, UI wiring. The current
  Mastra version is `@mastra/core` 1.79.0; APIs must be checked against current docs.
- **Bonus:** Sentry's agent monitoring has a Mastra integration, so the two reinforce each other.

### DigitalOcean — feasible with credits
- **Requirement:** "Build or deploy your project on DigitalOcean": host an app or agent, or run an open-weight model on
  a GPU Droplet.
- **Fit:** the PRD's optional cloud worker. A Droplet can run Docker, so the full engine (including the sandbox) can run
  there, unlike most PaaS hosts. CPU-only Gemma inference would be slow; a GPU Droplet costs money.
- **Work:** deployment guide/script, a firewall locking the API to loopback or an SSH tunnel (the API has no accounts),
  and a recorded demo run.
- **Needs from you:** Hacktoberfest DigitalOcean credits, or approval to spend. No spending without approval.

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
