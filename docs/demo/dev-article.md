---
title: "Inheritance Engine: resurrecting abandoned code with a local Gemma team, where the original gets a vote"
published: false
tags: devchallenge, hf26challenge, opensource, ai
---

*This is a submission for the [Hacktoberfest Open-Source AI Challenge Week 4: Ghost in the Machine](https://dev.to/challenges/hacktoberfest-week4-2026-10-26)*

On the morning of October 31, a model running on my laptop proposed a fix for a Python library last touched in 2014. A
second model reviewed the fix and called it correct. All 126 tests passed.

The fix was wrong. What caught it was the library itself: the original author's code, run on the Python it was written
for, was asked the same questions as the repaired code and gave different answers. The engine reverted the patch.

That is the idea behind **Inheritance Engine**: when you inherit software from someone who has moved on, the original
gets a vote on what "restored" means.

## What I Built

Somewhere on GitHub there is a library that does exactly what you need. Its author moved on years ago, and the code
quietly stopped working. Nobody deleted it. The runtime it was written for just went away.

Inheritance Engine is a workbench for inheriting that kind of software. It treats an abandoned repository like an
inherited workshop: preserve what's there, understand it, repair it, and record where everything came from. Every
agent in it runs on **Gemma 4, on my laptop**.

- **Hunter** finds candidates on GitHub (and, optionally, the web) and shows *evidence*, never a score.
- **Inherit** keeps the original as a read-only clone at a recorded commit. All work happens in a separate successor
  with pushing disabled.
- **Investigate** combines deterministic analysis (import graph, a scan for APIs that modern runtimes removed, a trust
  passport) with a team of agents: Lead Engineer, Historian, Restorer, Reviewer and Tester.
- **Restore** has the agents patch, review and verify in a locked-down Docker container. Repairs that don't hold up are
  reverted.
- **Preserve** writes an `INHERITANCE.md` into the successor and exports a versioned **Inheritance Record**: who wrote
  the code, what it was for, what changed, and how each change was proven.

![Investigating fn.py: the Gemma team at work, with the original preserved read-only](https://raw.githubusercontent.com/shaileshvats/inheritance-engine/main/docs/demo/screenshots/02-investigating.jpg)

## Demo

> 🎥 **VIDEO PLACEHOLDER:** the demo video is not recorded yet. This block will be replaced with the YouTube or Loom embed before publishing.

The demo inherits [kachayev/fn.py](https://github.com/kachayev/fn.py), "Functional programming in Python" (Apache-2.0,
about 3.4k stars, last commit on its default branch in October 2014). On Python 3.12 its tests don't even start:

```
E   ImportError: cannot import name 'Iterable' from 'collections'
1 error in 0.07s
```

### The catch, step by step

Gemma restored the imports, and the tests went from "can't start" to **126 passed**. Then I noticed that `@curried`
behaved differently for functions with `*args`. I reopened the issue with that finding, and on submission day Gemma
tried again:

1. **The original's behaviour.** Python 3.9 is the newest runtime fn.py's own code still works on, so the engine treats
   it as the creator's world and keeps the original commit there as the reference.
2. **The patch.** The Restorer replaced `getargspec` with `getfullargspec`, then used `f.prflags` to count arguments.
   That attribute doesn't exist.
3. **The review.** Gemma-as-Reviewer **approved** it: "root cause addressed: yes, regression risk: low".
4. **The tests.** 126 passed before, 126 passed after. No test calls `curried` on a variadic function.
5. **The behaviour probe.** The Tester wrote three probes, the first one straight from my reopened note. The engine
   ran them on the original commit under Python 3.9 and on the successor under Python 3.12. Original: `(1, 2)`, then a
   `TypeError`. Successor: `NameError`. Three probes, three changed.
6. **The decision.** Reverted, with the evidence attached. The successor stays honestly *restoring*:

   > 126 passed — not marked verified: the inheritor's reopened issue is unresolved

![Replay of the catch: the Reviewer approved, 126 tests passed before and after, and the behaviour probe against the original reverted it](https://raw.githubusercontent.com/shaileshvats/inheritance-engine/main/docs/demo/screenshots/12-replay-probe-catch.jpg)

### Try it yourself, without a model

A model won't write the same wrong patch twice on request, so the engine can **replay** this one. A replay feeds the
recorded patch, Reviewer verdict and probe code through the engine's *live* checks. The clone, the Docker tests, the
probes on Python 3.9 and 3.12, the revert and a fresh Inheritance Record all happen on your machine. You need Rust, Git
and Docker, and no GPU:

```bash
git clone https://github.com/shaileshvats/inheritance-engine && cd inheritance-engine
cargo run --release      # open http://127.0.0.1:7878 → "Replay a recorded catch" → Run this replay
```

It takes about 20 seconds once the Docker images are pulled. I've run it four times, including from a fresh clone and
with the model server unreachable, and it caught the patch every time. The UI labels it as a replay, and recorded
output is never committed.

### How the engine learned to be suspicious

Every check in that chain exists because a real run needed it:

- On the first restoration, the Restorer's first fix was approved by the Reviewer and **still reverted by the engine**,
  because the new traceback pointed into the line it had just edited. The second attempt never reached review: it
  dropped `deque` from an import that line 27 still used.
- An earlier `getargspec` fix used `inspect.signature(f).parameters` and left `spec.args` below it. **All 126 tests
  passed** and the Reviewer approved, but calling the decorator by hand gave
  `AttributeError: 'mappingproxy' object has no attribute 'args'`. No test calls `curried()`. That bug is why behaviour
  probes exist.
- The next fix passed the tests *and* three probes, but changed how `curried` treats `*args`. All three probes had
  used plain positional functions. That is why a human can reopen any issue, why their finding outranks green tests,
  and why the Tester must now cover distinct input shapes.

![The Reviewer challenging the Historian's findings, with the engine's citation checks](https://raw.githubusercontent.com/shaileshvats/inheritance-engine/main/docs/demo/screenshots/03-archaeology-reviewer.jpg)

## Code

{% embed https://github.com/shaileshvats/inheritance-engine %}

For the full experience you also need a local model server: `lms load google/gemma-4-e4b && lms server start`, then
`cargo run --release`. The README covers Ollama and the optional companions.

## How I Built It

**The model proposes, the engine verifies.** Gemma 4 E4B runs comfortably on a 16 GB laptop and is genuinely useful:
in my runs it wrote correct fixes and caught a real bug as Reviewer. It also wrote plausible wrong fixes, cited lines
that didn't say what it claimed, and approved a patch that invented an attribute. So the trusted part of the system is
a Rust binary that never executes inherited code and checks everything the model says:

- Every citation is **re-read from disk**. In the recorded investigation, 21 of 22 citations checked out and 1 was
  flagged as a mismatch. A claim the model calls "verified" without a real citation is downgraded to "inferred".
- Edits are exact search/replace operations, applied atomically and only inside the successor.
- A repair that makes tests worse, whose new failure comes from the lines it edited, or that changes what the original
  did, is reverted.
- **No model ever chooses a shell command.** Test commands come from detection, or from what you type.
- Repository text reaches the model only inside nonce-delimited "untrusted data" blocks.

**The stack:** Gemma 4 E4B through LM Studio's OpenAI-compatible API with `json_schema` constrained decoding for every
agent role (Ollama works too); a single Rust binary (axum, SQLite, tokio) that spawns `git` and `docker` with argument
vectors and no shell; a Docker sandbox with `--network none`, a read-only root filesystem, all capabilities dropped and
resource limits; checkpointed jobs that survive a restart; and a vanilla JS UI with no build step. **61 Rust tests**
cover the safety rules (path traversal, push disabled, original untouched, fabricated citations, the revert rules,
telemetry scrubbing, web citation checks), plus 19 for the Mastra companion.

### Mastra: asking the original repository what still matters

An inheritor doesn't only get code. They get the issue tracker too. fn.py still has 24 open issues. A
[Mastra](https://mastra.ai) workflow (`triage-upstream-issues`) reads them and has Gemma compare each one against what
the successor changed. "Addressed" only survives if it cites a repair or commit that really exists.

In the first run it found that upstream **#91, "ImportError: cannot import name Iterable from collections"**, is fixed
by the successor, citing the exact repair and commit. When I re-ran it on submission day, Gemma again said #91 was
addressed but cited nothing, and the engine downgraded it to *needs a human*. Same model, same issue, different day:
that is why the check exists.

![Mastra triage of fn.py's open upstream issues (first run): #91 addressed, with a verified citation](https://raw.githubusercontent.com/shaileshvats/inheritance-engine/main/docs/demo/screenshots/05-mastra-upstream-triage.jpg)

The Lead Engineer chat also runs as a Mastra agent with read-only tools over the engine's findings. My LM Studio build
couldn't render Gemma 4's chat template with tools, so the companion detects that at startup and runs a Mastra workflow
that calls the same tools deterministically, then has Gemma answer with structured output.

### SerpApi: when GitHub search isn't enough

GitHub's search matches keywords in names and descriptions, and abandoned projects often describe themselves badly. For
*"python event signal dispatch library"*, GitHub's own search returned a JavaScript essay, a Flask template, a
system-design primer and a PHP daemon. The Hunter's web search through [SerpApi](https://serpapi.com) added
`pallets-eco/blinker` and `nitely/python-signals`, the libraries that actually do that. Each one is re-fetched from the
GitHub API, so its evidence is real.

![Hunter results with web search: repositories found on the web, each with its source page](https://raw.githubusercontent.com/shaileshvats/inheritance-engine/main/docs/demo/screenshots/09-hunter-web-search.jpg)

For an inheritance, a **web research** job runs three searches and has Gemma summarise related projects and docs, and
every item must cite a real result like `[W2]`. For fn.py it found the original and its documentation site, and the
Lead Engineer now answers *"where are the docs?"* with that source.

![Web research for fn.py: a cited summary, the original and its documentation site](https://raw.githubusercontent.com/shaileshvats/inheritance-engine/main/docs/demo/screenshots/10-web-research.jpg)

Testing it on a second, more obscure library taught me two things. The search for its docs returned the author's
personal social-media profiles, which a tool like this should never surface, so those pages are now filtered out. And
Gemma called Python's built-in `signal` module a "successor" of the library. The citation was real, but the claim
wasn't. So a lineage claim now needs a cited source that actually names the project, or the engine downgrades it to a
plain mention. Everything ran on SerpApi's free plan, and only search text and public repository names are sent.

### Sentry: watching the agents without leaking the code

Both the Rust engine and the Mastra companion send traces to Sentry: each job as an agent run, every Gemma call with its
role, tokens and latency, and every sandbox run as a tool call. In the 7-day window shown, Sentry's AI Agents view
counted 24 Gemma calls and 53,669 tokens.

![Sentry AI Agents overview: Gemma calls, tokens and the engine's sandbox tool calls](https://raw.githubusercontent.com/shaileshvats/inheritance-engine/main/docs/demo/screenshots/06-sentry-ai-agents.jpg)

The 8.78-minute investigation of one repository breaks down into a 17.85 s plan step and a 3.39-minute Historian call.
The tests "tool" shows in red because the inherited code really did fail.

![The engine's investigation trace in Sentry](https://raw.githubusercontent.com/shaileshvats/inheritance-engine/main/docs/demo/screenshots/07-sentry-engine-trace-waterfall.jpg)

What Sentry *doesn't* see matters as much. Click any model call and the input panel says "No input for this span". The
engine enforces this in the transport itself. I checked it three ways: by auditing every payload before it was sent, by
querying Sentry's stored spans for content attributes (zero), and on submission day by scanning all 26 audited payloads
for lines of the inherited source (none). Getting there exposed a real gap: Mastra's exporter streams spans through a
path that skips the usual Sentry scrubbing hook, so the check now lives where nothing can bypass it.

![Agent Activity for one Gemma call: tokens and latency, and "No input for this span"](https://raw.githubusercontent.com/shaileshvats/inheritance-engine/main/docs/demo/screenshots/08-sentry-agent-activity-no-input.jpg)

## Why Does Open Innovation Matter?

**Because preservation can't depend on a service that might disappear.** A tool for rescuing abandoned software has to
outlive the things it depends on. Gemma's weights are a file on my disk. The Inheritance Record names the exact model
that wrote every patch, and that model stays on my disk even if an API is retired. The whole project cost ₹0.

**Because you're handing it someone else's code.** In `local_only` mode (the default), the engine refuses to send a
single byte of repository content to a non-loopback model endpoint. Inheriting code shouldn't mean uploading it.

**Because I could look inside.** When I needed structured output, I used the runtime's JSON-schema mode. When Ollama's
registry wouldn't serve models to my old client, I pointed the same adapter at LM Studio. When I want a stronger
Reviewer, one environment variable gives that single role a bigger Gemma.

**What works with the internet off, and what doesn't:**

| Works offline (once the repository and its dependencies are cached) | Needs the internet |
| --- | --- |
| Every Gemma agent: Historian, Restorer, Reviewer, Tester, Lead Engineer | Cloning the original, and Hunter's GitHub search |
| Investigation, citation checks, Docker tests (network off), behaviour probes, reverts | Installing the inherited project's dependencies, and pulling Docker images the first time |
| `INHERITANCE.md` and the Inheritance Record | Mastra's upstream-issue triage (it reads GitHub issues) |
| The Mastra Lead Engineer chat | SerpApi web search and Sentry tracing, both optional |

## The Spirit of It

*Ghost in the Machine* is literal here. An abandoned repository is a machine whose maker is gone, and their decisions
are still in it: in the code, in the git history, and in the behaviour the tests never wrote down. Día de los Muertos is
about remembering people by keeping what they made present. So the rule of the whole project is that the successor must
keep doing **what its creator intended**, in a world that has moved on, and must never forget who that creator was.

That's what the **Inheritance Record** is for. It isn't a changelog. It's provenance: the exact commit the creator left
behind, their name and how much of the code is theirs, their license, what the project was for, every change made since,
and the evidence for each one, including the changes that were *rejected*. This is the real record for fn.py, trimmed:

```json
{
  "project": {
    "id": "github:kachayev/fn.py@df53f5d6bf0e94a37f44f6be57d1c87c9b7a6c26",
    "original_license": "Apache-2.0 (detected from LICENSE file)"
  },
  "provenance": {
    "attribution": {
      "original_authors": [
        { "name": "Alexey Kachayev", "commits": 190 },
        { "name": "han semaj", "commits": 19 },
        { "name": "Nikolaos-Digenis Karagiannis", "commits": 7 }
      ],
      "note": "Lineage does not imply endorsement by the original authors."
    }
  },
  "restoration": {
    "attempts": [{
      "id": "rep_3c4ccc574d204f3f",
      "status": "reverted",
      "review": { "verdict": "approve", "regression_risk": "low" },
      "probe": { "verdict": "changed", "legacy_image": "python:3.9-slim", "modern_image": "python:3.12-slim" }
    }]
  }
}
```

![The Inheritance Record for fn.py: lineage from Alexey Kachayev's last commit in October 2014 to the successor](https://raw.githubusercontent.com/shaileshvats/inheritance-engine/main/docs/demo/screenshots/13-inheritance-record.jpg)

Every round of repairs also ends with an `INHERITANCE.md` committed to the successor. It names Alexey Kachayev and
every other author, records the commit they left behind, keeps their license, and says plainly that the successor
doesn't speak for them. That file is the closest thing this project has to an ofrenda: the code keeps working, and the
people who made it stay on the page.

## Limitations, honestly

- **fn.py is not finished.** The reopened `*args` issue is still open after four Gemma patches, so the successor is
  *restoring*, not *verified*.
- **A second library, [shaunduncan/smokesignal](https://github.com/shaunduncan/smokesignal)** (last commit 2016),
  went from tests that couldn't start (`No module named 'mock'`) to **7 passing**. The engine reverted Gemma's first two
  fixes because they dropped a name that line 19 still used, and kept the third. The remaining 28 failures are Python 3
  changes to bound methods. Gemma hasn't solved them: one patch was reverted because the new failure came from its own
  edit, and the Reviewer rejected another that would have dropped `self`.
- **Web research for obscure projects is often irrelevant.** The engine can check that a source exists, not that it's
  relevant. For smokesignal, the "documentation" Gemma found was about Python's `signal` module.
- **Probes only test what the Tester thought to probe**, and a 4B model writes wrong patches. The engine catches many,
  not all. A full run takes 15–25 minutes on a laptop.
- Verification works for Python and Node.js, and only for public GitHub repositories. Docker isn't a perfect security
  boundary. Replays exist for one recorded case so far.
- Inheritance Engine is source-available under the PolyForm Noncommercial License 1.0.0, which is *not* an OSI
  open-source license. Inherited projects keep their own licenses, and the engine never relicenses anything.

The PRD and the implementation were AI-assisted: I worked with Claude as my engineering partner throughout the build,
and the runtime agents are Gemma.

## Prize Categories

- **Best Use of Gemma**: Gemma 4 runs every agent role (Lead, Historian, Restorer, Reviewer, Tester, Hunter), locally,
  with constrained JSON output.
- **Best Use of Mastra**: the Lead Engineer agent with engine tools, a grounded answer workflow, and the upstream-issue
  triage workflow, all over Gemma, all used by the product.
- **Best Use of Sentry Agent Tracing**: real traces from both the Rust engine and the Mastra companion in Sentry's AI
  Agents views, with tokens and latency per call, verified to carry no prompts or code.
- **Best Use of SerpApi**: live web search in the Hunter to discover repositories GitHub's search misses, and a web
  research job that gives the agents cited, engine-checked sources about an inherited project.

---

*Software doesn't have to die with its creator. It can be inherited, understood, repaired, and given a future.*
