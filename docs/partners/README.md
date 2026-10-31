# Partner category audit (HF26 Week 4)

Audited on 2026-10-28, updated after the Mastra and Sentry work and again on 2026-10-31 for a zero budget, against the official category descriptions on the [HF26 Challenge Hub](https://dev.to/challenges/hf26)
and the code in this repository. The official rule is: *"Use only partner tech your project actually uses."*
A category is marked **Implemented** only when the code uses the technology in the product and there is evidence
to show for it.

Deadline: 2026-10-31 23:59 PDT. A submission can win only once, and writing quality is weighted most heavily.

## Summary

| Category | Status | Feasible before deadline? |
| --- | --- | --- |
| Gemma (featured) | **Implemented** | Done |
| Mastra | **Implemented**: Lead Engineer agent + two workflows, used by the product; real runs recorded | Done |
| Sentry Agent Tracing | **Implemented and demonstrated**: real traces from both processes, shown in Sentry's AI Agents views | Done |
| DigitalOcean (featured) | **Not pursued**: zero budget, and no DigitalOcean credits were offered | No. A Droplet would be paid; nothing was created |
| Temporal | Not implemented; SQLite checkpoints do the same job today | Possible but high risk: about 1.5 days, second runtime |
| Auth0 for AI Agents | Not implemented, real fit (GitHub actions on the user's behalf, with approval) | Risky: 1–2 days plus Auth0 tenant and GitHub connection setup |
| SerpApi | **Implemented and demonstrated**: Hunter web discovery + cited web research for the agents, on the free plan | Done |
| Ultramarine (Paper Instruments) | Not implemented; it is a hosted model behind an OpenAI-compatible adapter | Yes, a few hours. Conflicts with the local-only default; opt-in comparison only |
| Backboard | Not implemented, weak fit | Possible, low value |
| MongoDB Atlas / Tiger Data | Not implemented; no real need yet | Possible (restoration-recipe retrieval), questionable value with so few runs |
| ElevenLabs | Not implemented | Demo narration qualifies officially, but it isn't a product feature |
| Render (featured) | Not implemented; poor fit for the full engine | Only as a hosted Hunter-only front end; the full engine cannot run there |
| .tech Domain | **Not pursued** | The rule needs the project live on the domain, and there is no free host that can run it |
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
  provider `mastra`. There are 19 vitest tests (citation rule, telemetry scrubbing, request guards, both workflows
  against a local fixture model, the HTTP server).
- **Limits:** native tool calling has not been exercised with Gemma 4 on this machine (runtime limitation above).
  Mastra storage is in memory, since the engine persists results. A 4B model sometimes repeats a heading.

### Sentry Agent Tracing — Implemented and demonstrated
- **Requirement:** "Show your agent's work: how it's set up, how it performs (latency, tokens, cost)," what you found
  and debugged, with traces or screenshots in the write-up.
- **Setup:** one Sentry project (Node.js platform). Both processes use its DSN through native SDKs: the Rust engine via
  the `sentry` crate, the companion via `@mastra/sentry` / `@sentry/node`. No OTLP. The DSN lives only in the git-ignored
  local `.env`.
- **What the engine sends:** one transaction per job (`gen_ai.invoke_agent`, agent name "Inheritance Engine team"), a
  `gen_ai.chat` span per model call (role, model, input/output/total tokens, latency, ok/error class), and a
  `gen_ai.execute_tool` span per sandbox run (install, syntax, tests, behaviour probe). The companion sends Mastra's
  agent runs, model generations and workflow steps.
- **Verified in Sentry (2026-10-28 runs, checked 2026-10-31):**
  - Spans arrived from both SDKs. Rust: `gen_ai.invoke_agent`, `gen_ai.chat`, `gen_ai.execute_tool`. Node: agent,
    model, workflow and processor spans.
  - **AI Agents Overview** for the project (7 days): 24 LLM calls to `google/gemma-4-e4b`, 53,669 tokens, and tool calls
    including the engine's `sandbox.syntax` (screenshot `06-sentry-ai-agents.jpg`).
  - The engine's `inheritance.investigate` trace: 8 spans, 8.78 min, with the Historian at 3.39 min
    (`07-sentry-engine-trace-waterfall.jpg`). Its **Agent Activity** tab shows the engine as an agent with per-call
    tokens. For example plan: 1.9K in + 249 out, Input "No input for this span"
    (`08-sentry-agent-activity-no-input.jpg`).
- **Privacy, verified twice:**
  - **Local:** an opt-in transport-level audit (`IE_TELEMETRY_AUDIT`) of every payload either process handed to Sentry.
    It contained no prompts, responses, source keywords, repository file names, issue text, secret patterns or local
    paths.
  - **Server side:** a query over the project's stored spans found **0** spans with any of 12 content attributes
    (`gen_ai.request.messages`, `gen_ai.response.text`, `gen_ai.agent.prompt`, `gen_ai.input/output.messages`,
    `gen_ai.tool.input/output`, `input`, `output`, `http.url`, `url.full`, `gen_ai.prompt`).
  - **Sent by design:** the public GitHub repository name (`ie.repository`), the opaque inheritance id, and fixed
    category labels.
- **Debugging found along the way:** the Mastra exporter streams agent/model spans as `span` envelope items that bypass
  `beforeSendTransaction`. Payloads were already clean, thanks to the span processor, but enforcement now sits in the
  transport of both processes for every item type.
- **Cost:** Sentry's "Estimated Cost" panel is empty. The model runs locally, with no per-token price, so the real cost is $0.

### DigitalOcean — Not pursued (zero budget)
- **Requirement:** "Build or deploy your project on DigitalOcean": host an app or agent, or run an open-weight model on a GPU Droplet.
- **Credits checked 2026-10-31:** the owner's Hacktoberfest promos page offers .tech, Backboard, Browserbase,
  ElevenLabs, Paper Instruments, Render and Tinker, and **no DigitalOcean credits**. The MLH event offers list only
  ElevenLabs. A Droplet big enough for Gemma 4 E4B (Basic, 16 GB) costs $0.143/hour, and the project's budget is zero.
- **Decision:** no account, payment method, trial or Droplet was created. The category is not claimed.
- **What exists instead:** a generic self-host compose stack in `deploy/self-host`, validated locally only. It is not
  a DigitalOcean deployment and is not presented as one.

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

### SerpApi — Implemented and demonstrated (free plan)
- **Requirement:** "Give an agent live web search, ground a RAG app in fresh results, or track news for an open-source project."
- **Cost:** the owner's account is on SerpApi's Free Plan ($0, 250 searches/month; the dashboard shows "Free Plan").
  No payment method. Development and the demo used a few dozen searches.
- **In the product** (`src/websearch.rs`, `hunter.rs`, `pipeline.rs::research_job`, Overview card):
  - **Hunter:** *Also search the web (SerpApi)*. The same keywords go to Google through SerpApi; GitHub repository URLs
    in the results are re-fetched from the GitHub API (real signals), inserted first, and show the page they came from.
    Other pages are listed as related reading.
  - **Web research job** on an inheritance: three searches (`"owner/repo"`, name + topic, name + owner + documentation),
    then one Gemma call (Historian) summarises related projects and documentation. Every item must cite result ids
    (`W1`…); the engine drops uncited or invalid citations and duplicates, and relabels the inherited repository as
    `original` when the model calls it something else. The result is stored, shown, included in the Inheritance Record,
    and given to the Lead Engineer (engine chat and the Mastra `get-project-overview` tool).
  - **Privacy:** only search text and public repository names are sent. The key is read from `.env` at call time, kept
    out of error messages (`reqwest::Error::without_url`) and never logged. Redirect links that hide their destination
    are discarded.
- **Evidence (2026-10-31):**
  - Hunter, curiosity mode, *"python event signal dispatch library"*: GitHub search alone returned unrelated
    repositories near the top. Web search added `pallets-eco/blinker` and `nitely/python-signals`
    (`docs/demo/screenshots/09-hunter-web-search.jpg`).
  - Web research on kachayev/fn.py: a cited summary, the original and its GitHub Pages documentation
    (`10-web-research.jpg`). Real runs exposed problems that were then fixed: Google redirect links, a query that
    returned a dictionary page, the model echoing schema field names as "documentation" entries, and the model calling
    the original a fork.
  - Tests: link classification, reserved GitHub paths, redirect filtering, `.env` key parsing, and the citation check.
- **Limits:** search engines rarely surface forks of old projects. GitHub's own fork data would be better and is not
  used yet. Result quality for 10-year-old projects is thin.

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
  hosted Hunter-only front end is possible, but it would be a different product from the one being judged.
- **.tech** requires the project to be live on the domain. The free first-year code needs a verified phone on MyMLH
  and renews at $29.99/yr. The engine needs Docker and a 7 GB model, which no free host provides, and exposing the
  owner's laptop publicly is not acceptable for a tool that runs untrusted code. Not pursued, and no code was claimed.

### Not feasible or not genuine before the deadline
- **TabPFN:** predicting "restorability" would need real labelled outcomes from many restorations; we have three runs.
- **Tinker:** needs a training set, a fine-tune and a measured improvement over a baseline.
- **Browserbase:** needs an inherited project with a web UI. That's a different demo.
- **GitHub Copilot, Entire:** neither was used to build this project, and claiming them retroactively would be false.
- **Arduino:** no UNO Q available. **Qualcomm:** no published requirements found.
