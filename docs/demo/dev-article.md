---
title: "Inheritance Engine: resurrecting abandoned code with a local Gemma team, where the original gets a vote"
published: false
tags: devchallenge, hf26challenge, opensource, ai
---

*This is a submission for the [Hacktoberfest Open-Source AI Challenge Week 4: Ghost in the Machine](https://dev.to/challenges/hacktoberfest-week4-2026-10-26)*

## What I Built

Somewhere on GitHub there is a library that does exactly what you need. Its author wrote the last commit in 2014,
moved on with their life, and the code quietly stopped working. Nobody deleted it. The runtime it was written for just
went away.

**Inheritance Engine** is a workbench for inheriting that kind of software. It treats an abandoned repository the way
you'd treat an inherited workshop: preserve what's there first, then understand it, then repair it, then decide what it
should become. Every agent in it runs on **Gemma 4, on my laptop**.

- **Hunter** searches GitHub by problem or by curiosity. Results show *evidence* (years of history, forks, how long
  it's been quiet, license cautions), never a score.
- **Inherit** keeps the original as a read-only clone at a recorded commit. All work happens in a separate successor
  with pushing disabled. Nothing ever touches the source.
- **Investigate** combines deterministic analysis (import graph, entry points, a scan for APIs that modern runtimes
  removed, a trust passport) with a team of agents: Lead Engineer, Historian, Restorer, Reviewer and Tester.
- **Restore** has the agents patch, review and verify in a locked-down Docker container. Repairs that don't hold up are
  reverted.
- **Preserve** commits an `INHERITANCE.md` to the successor and exports a versioned Inheritance Record: where the code
  came from, who wrote it, what changed, and how each change was proven.

**How I read the theme.** *Ghost in the Machine* is literal here: an abandoned repository is a machine whose maker is
gone, and their decisions are still in it. Día de los Muertos is about remembering people by keeping what they made
present. So the rule of the whole project is that the successor must keep doing **what its creator intended**, in a
world that has moved on. Enforcing that rule turned out to be the hardest engineering problem in the project, and the
most interesting one.

![Investigating fn.py: the Gemma team at work, with the original preserved read-only](https://raw.githubusercontent.com/shaileshvats/inheritance-engine/main/docs/demo/screenshots/02-investigating.jpg)

## Demo

<!-- VIDEO: replace with {% embed <youtube or loom url> %} once recorded -->

The demo inherits [kachayev/fn.py](https://github.com/kachayev/fn.py), "Functional programming in Python" (Apache-2.0,
about 3.4k stars, last commit on its default branch in October 2014). On Python 3.12 its tests don't even start:

```
E   ImportError: cannot import name 'Iterable' from 'collections'
1 error in 0.07s
```

Here is what happened in one recorded run, with nothing staged. Every step is a traced Gemma call or a real container
run.

1. The Restorer's first fix was **approved by the Reviewer and still reverted by the engine**, because the new
   traceback pointed into the very line it had edited.
2. The second attempt **never reached review**. The engine noticed it had dropped `deque` from an import that line 27
   still used.
3. The third attempt held. Test counts didn't move, but the engine recorded that *"the failure moved from
   fn/iters.py:2 to fn/func.py:2"*. Fixing the first fossil had uncovered a second one, `inspect.getargspec`.
4. On that second fossil, Gemma-as-Reviewer **rejected** a patch for a genuine bug: it subtracted an integer from a list.
5. The next patch used `inspect.getfullargspec`. **126 passed, 0 failed**, in `python:3.12-slim` with networking off.

![The Reviewer challenging the Historian's findings, with the engine's citation checks](https://raw.githubusercontent.com/shaileshvats/inheritance-engine/main/docs/demo/screenshots/03-archaeology-reviewer.jpg)

### The bug that taught me what "restored" means

An earlier attempt replaced `getargspec(f)` with `inspect.signature(f).parameters` and left `spec.args` below it.
**All 126 tests passed.** The Reviewer approved. Everything was green. Then I ran the decorator by hand:

```
successor (Python 3.12): AttributeError: 'mappingproxy' object has no attribute 'args'
original  (Python 3.9):  6
```

No test calls `curried()`. Green tests had certified a broken repair.

So after each repair, a **Tester agent** now writes small probe calls for the code that changed, and the engine runs
them twice: on the successor under Python 3.12, and on **the original commit under the Python of its era**. It works
out that era from the decay scan: `collections.Iterable` was removed in 3.10, so the creator's world was 3.9 or
earlier. If the original printed `6` and the successor prints anything else, the repair is reverted and sent back with
the evidence. In a later run this caught a Reviewer-approved fix (original `3`, `6`, `6`; successor `TypeError`) before
it could be committed.

**The ghost gets a vote.**

### And then a human still found one

On the next pass a different fix went green: 126 tests, 3 of 3 probes preserved. I almost moved on. Then I tried
`curried` on a function with `*args`:

```
                          original (3.9)     successor (3.12)
plain(1)(2)               3                  3
star(1)(2)  [*rest]       (1, 2, ())         still curried, waiting
kw(1)(2)    [**opts]      (1, 2, [])         still curried, waiting
```

`signature(f).parameters` counts `*args`, and the old `getargspec(f).args` never did. All three of the Tester's probes
had used plain positional functions.

So **the human gets a vote too**. Any issue can be reopened with your own finding. The Restorer and Tester receive it as
a human finding that outranks passing tests, and the project can't be marked *verified* while it's open. I gave Gemma
more passes at it today. It came close: one patch switched to `getfullargspec` but left `signature()` in the condition,
and another invented an attribute that doesn't exist. **The Reviewer rejected both**, and the successor still says,
accurately:

> 126 passed — not marked verified: the inheritor's reopened issue is unresolved

I'd rather ship a tool that says that than one that shows a green badge.

## Code

{% embed https://github.com/shaileshvats/inheritance-engine %}

To run it you need Docker and a local model server. `lms load google/gemma-4-e4b && lms server start`, then
`cargo run --release` and open `http://127.0.0.1:7878`. The README has the details, including Ollama.

## How I Built It

**The model proposes, the engine verifies.** Gemma 4 E4B runs comfortably on a 16 GB laptop, and it's genuinely
useful. In my runs it wrote correct fixes and caught a real bug as Reviewer. It also wrote plausible wrong fixes, cited
lines that didn't say what it claimed, and occasionally repeated itself. So the trusted part of the system is a Rust
binary that never executes inherited code and checks everything the model says:

- Every citation is **re-read from disk**. In the recorded run, 21 of 22 citations checked out and 1 was flagged as a
  mismatch. A claim the model calls "verified" without a real citation is downgraded to "inferred".
- Edits are exact search/replace operations, applied atomically and only inside the successor. A model can't touch
  `.git`, escape the directory, or create files.
- A repair that makes tests worse, or whose new failure comes from the lines it edited, is reverted.
- **No model ever chooses a shell command.** Test commands come from detection, or from what you type.
- Repository text reaches the model only inside nonce-delimited "untrusted data" blocks. A README that says "ignore
  previous instructions" is just more data.

Every one of those rules exists because a real run broke without it. The repo has a decision log listing which run
taught which lesson.

**The stack:**

- **Gemma 4 E4B** through LM Studio's OpenAI-compatible API, with `json_schema` constrained decoding for every agent
  role. Ollama is supported too. Nothing leaves the machine.
- **Rust** single binary (axum, SQLite, tokio) as the control plane. It spawns `git` and `docker` with argument
  vectors and no shell.
- **Docker** sandbox: `--network none`, `--read-only`, `--cap-drop ALL`, `no-new-privileges`, memory, CPU and PID
  limits, non-root user, no host environment.
- **Durable jobs:** every pipeline step is checkpointed. I killed the server mid-investigation and resumed it; clone,
  inventory, baseline and plan were skipped, not re-run.
- **Vanilla JS** UI with no build step and no external requests, under a strict Content-Security-Policy.
- **Tests:** 53 Rust tests for the safety rules (path traversal, symlink escapes, push disabled, original untouched,
  cleared environment, redaction, fabricated citations, the regression rules, era-runtime selection, telemetry
  scrubbing) and 19 for the Mastra companion.

### Mastra: asking the original repository what still matters

An inheritor doesn't only get code. They get the issue tracker too. fn.py still has 24 open issues. A
[Mastra](https://mastra.ai) workflow (`triage-upstream-issues`) reads them and has Gemma compare each one against what
the successor changed. As always, the engine has the last word: "addressed" only survives if it cites a repair or
commit that really exists.

In the real run, it found that upstream **#91, "ImportError: cannot import name Iterable from collections"**, is fixed
by the successor, citing the exact repair and commit. The other seven it looked at are feature requests and questions
that still stand. That's the inheritor's to-do list, inherited too.

![Mastra triage of fn.py's open upstream issues: #91 addressed, with a verified citation](https://raw.githubusercontent.com/shaileshvats/inheritance-engine/main/docs/demo/screenshots/05-mastra-upstream-triage.jpg)

The Lead Engineer chat also runs as a Mastra agent with read-only tools over the engine's findings. My local LM Studio
build couldn't render Gemma 4's chat template with tools, so the companion probes the runtime at startup. When tool
calling isn't available, it runs a Mastra workflow that calls the same tools deterministically and has Gemma answer
with structured output. Same tools, same rules, and it works on today's runtime.

### Sentry: watching the agents work without leaking the code

Both the Rust engine and the Mastra companion send traces to Sentry. Each investigation shows up as an agent run: the
team's job, every Gemma call with its role, tokens and latency, and every sandbox run as a tool call. Over the week,
Sentry's AI Agents view counted 24 Gemma calls and 53,669 tokens.

![Sentry AI Agents overview: Gemma calls, tokens and the engine's sandbox tool calls](https://raw.githubusercontent.com/shaileshvats/inheritance-engine/main/docs/demo/screenshots/06-sentry-ai-agents.jpg)

The 8.78-minute investigation of one repository breaks down into a 17.85 s plan step and a 3.39-minute Historian call.
The tests "tool" shows in red because the inherited code really did fail.

![The engine's investigation trace in Sentry](https://raw.githubusercontent.com/shaileshvats/inheritance-engine/main/docs/demo/screenshots/07-sentry-engine-trace-waterfall.jpg)

What Sentry *doesn't* see matters as much. Click any model call and the input panel says "No input for this span". No
prompt, no response and no line of inherited source leaves the laptop. The engine enforces this in the transport
itself, and I verified it twice: by auditing every payload before it was sent, and by querying Sentry's stored spans
for content attributes, which returned zero. Getting there exposed a real gap: Mastra's exporter streams spans through
a path that skips the usual Sentry scrubbing hook. The content was already gone by then, but the check now lives where
nothing can bypass it.

![Agent Activity for one Gemma call: tokens and latency, and "No input for this span"](https://raw.githubusercontent.com/shaileshvats/inheritance-engine/main/docs/demo/screenshots/08-sentry-agent-activity-no-input.jpg)

## Why Does Open Innovation Matter?

**Because the whole point is outliving the original maintainer.** A preservation tool that depends on one company's API
staying online and affordable has the same single point of failure as the abandoned projects it rescues. Inheritance
Engine runs Gemma on your own machine. It works offline, costs nothing per run (this whole project cost me ₹0), and the
Inheritance Record it produces is plain JSON you own.

**Because you're feeding it someone else's code.** In `local_only` mode (the default), the engine refuses to send a
single byte of repository content to a non-loopback model endpoint. Inheriting code shouldn't mean uploading it.

**Because I could look inside.** When I needed structured output, I used the runtime's JSON-schema mode. When Ollama's
registry wouldn't serve models to my old client, I pointed the same adapter at LM Studio's OpenAI-compatible server in
an afternoon. When I want a stronger Restorer, one environment variable gives that single role a bigger Gemma. Open
weights and open protocols turned vendor problems into configuration.

**Because it can be inherited too.** The engine is itself built to be picked up by someone else: documented decisions,
a typed API, and tests for every safety rule.

## The Spirit of It

fn.py was written mostly by Alexey Kachayev: 190 of its commits are his, and the last one is from October 2014. When
Inheritance Engine restores a repository, the first thing it writes into the successor isn't a fix. It's an
`INHERITANCE.md` that names him and every other author from the git history, records the exact commit they left
behind, keeps their license, and says plainly that the successor doesn't speak for them.

That file is the closest thing this project has to an ofrenda. The code keeps working, and the people who made it stay
on the page.

## Limitations, honestly

Verification currently works for Python and Node.js, and only for public GitHub repositories. Docker isn't a perfect
security boundary. A 4B model still writes wrong patches; the engine catches many, not all, and probes only test what
the Tester thought to probe. A full run takes 15–25 minutes on a laptop. Inheritance Engine is source-available under
the PolyForm Noncommercial License 1.0.0, which is *not* an OSI open-source license. Inherited projects keep their own
licenses, and the engine never relicenses anything.

The PRD and the implementation were AI-assisted: I worked with Claude as my engineering partner throughout the build,
and the runtime agents are Gemma.

## Prize Categories

- **Best Use of Gemma**: Gemma 4 runs every agent role (Lead, Historian, Restorer, Reviewer, Tester, Hunter), locally,
  with constrained JSON output.
- **Best Use of Mastra**: the Lead Engineer agent with engine tools, a grounded answer workflow, and the upstream-issue
  triage workflow, all over Gemma, all used by the product.
- **Best Use of Sentry Agent Tracing**: real traces from both the Rust engine and the Mastra companion in Sentry's AI
  Agents views, with tokens and latency per call, verified to carry no prompts or code.

---

*Software doesn't have to die with its creator. It can be inherited, understood, repaired, and given a future.*
