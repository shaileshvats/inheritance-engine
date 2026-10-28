---
title: "Inheritance Engine: giving abandoned code a successor, with Gemma running on my laptop"
published: false
tags: devchallenge, hf26challenge, opensource, ai
---

*This is a submission for the [Hacktoberfest 2026 Open-Source AI Challenge, Week 4: Ghost in the Machine](https://dev.to/challenges/hacktoberfest-week4-2026-10-26).*

## What I Built

Somewhere on GitHub there is a library that does exactly what you need. Its author wrote the last commit in 2014,
moved on with their life, and the code quietly stopped working. Nobody deleted it. The runtime it was written for just
went away.

**Inheritance Engine** is a workbench for inheriting that kind of software. It treats an abandoned repository the
way you'd treat an inherited workshop: preserve what's there first, then understand it, then repair it, then decide
what it should become.

- **Hunter** searches GitHub by problem or by curiosity. Results show *evidence* (years of history, forks, how long
  it's been quiet, license cautions), never a score.
- **Inherit** keeps the original as a read-only clone at a recorded commit. All work happens in a separate successor
  with pushing disabled. Nothing ever touches the source.
- **Investigate** combines deterministic analysis (import graph, entry points, likely central "mother scripts", a scan
  for APIs that modern runtimes removed, a trust passport) with a team of agents. A Lead Engineer, a Historian, a
  Restorer, a Reviewer and a Tester all run on **Gemma 4, locally**.
- **Restore** has the agents patch, review, and verify in a locked-down Docker container. Repairs that don't hold up get
  reverted.
- **Preserve** commits an `INHERITANCE.md` to the successor and exports a versioned Inheritance Record: where the code
  came from, what was learned, what changed, and how each change was proven.

<!-- screenshot: docs/demo/screenshots -->

## The ghost in the machine

The theme asked for something that honours the past. Old code is a record of someone's decisions, and the
most respectful thing a successor can do is keep the software doing what its creator intended, on a world that has
moved on.

That turned out to be the hardest engineering problem in the project, and the most interesting one.

## Demo

**Repository:** `<repo link>` · **Video:** `<video link>`

The demo inherits [kachayev/fn.py](https://github.com/kachayev/fn.py), "Functional programming in Python" (Apache-2.0,
about 3.4k stars, last commit on its default branch in 2014). On Python 3.12 its tests don't even start:

```
E   ImportError: cannot import name 'Iterable' from 'collections'
1 error in 0.07s
```

Here's what happened in one recorded run, with nothing staged. Every step is a traced Gemma call or a real container run.

1. The Restorer's first fix was **approved by the Reviewer and still reverted by the engine**, because the new traceback
   pointed into the very line it had edited.
2. The second attempt **never reached review**. The engine noticed it had dropped `deque` from an import that line 27
   still used.
3. The third attempt held. Test counts didn't move, but the engine recorded that *"the failure moved from
   fn/iters.py:2 to fn/func.py:2"*. Fixing the first fossil had uncovered a second one, `inspect.getargspec`.
4. On that second fossil, Gemma-as-Reviewer **rejected** a patch for a genuine bug: it subtracted an integer from a list.
5. The next patch used `inspect.getfullargspec`. **126 passed, 0 failed**, in `python:3.12-slim` with networking off.

## The bug that taught me what "restored" means

An earlier attempt replaced `getargspec(f)` with `inspect.signature(f).parameters` and left `spec.args` below it.
**All 126 tests passed.** The Reviewer approved. Everything was green.

Then I ran the decorator by hand:

```
successor (Python 3.12): AttributeError: 'mappingproxy' object has no attribute 'args'
original  (Python 3.9):  6
```

No test calls `curried()`. Green tests had certified a broken repair.

So the engine now does something I haven't seen elsewhere. After each repair, a **Tester agent** writes small probe
calls for the code that changed. The engine runs them twice: on the successor under Python 3.12, and on **the original
commit under the Python of its era**. It works out that era from the decay scan: `collections.Iterable` was removed in
3.10, so the creator's world was 3.9 or earlier. If the original printed `6` and the successor prints anything else,
the repair is reverted and sent back with the evidence.

The ghost gets a vote.

## And then a human still found one

In a later run, the probe caught a broken fix (original `3`, `6`, `6`, successor `TypeError`) and reverted it. On the
next pass a different fix went green: 126 tests, 3 of 3 probes preserved. I almost moved on.

Then I tried `curried` on a function with `*args`. The original on Python 3.9 called through after two arguments. The
successor waited forever for a third. `signature(f).parameters` counts `*args`, and the old `getargspec(f).args` never
did. All three of the Tester's probes had used plain two- and three-argument functions.

So the human gets a vote too. Any issue can be reopened with your own finding. The Restorer and Tester receive it as a
human finding that outranks passing tests, and the project can't be marked *verified* while it's open. Today the
successor says, accurately: *126 passed — not marked verified: the inheritor's reopened issue is unresolved.* I'd rather
ship a tool that says that than one that shows a green badge.

## Asking the original repository what still matters

An inheritor doesn't only get code. They get the issue tracker too. fn.py still has 24 open issues on GitHub. A Mastra
workflow (`triage-upstream-issues`) reads them and has Gemma compare each one against what the successor changed. As
always, the engine has the last word: "addressed" only survives if it cites a repair or commit that really exists.

In the real run, it found that upstream **#91, "ImportError: cannot import name Iterable from collections on Python
3.10"**, is fixed by the successor, citing the exact repair and commit. The other seven it looked at are feature
requests and questions that still stand. That's the inheritor's to-do list, inherited too.

The Lead Engineer chat runs as a Mastra agent with read-only tools over the engine's findings. The local LM Studio build
couldn't render Gemma 4's chat template with tools, so the companion probes the runtime at startup. When tool calling
isn't available, it uses a Mastra workflow that calls the same tools deterministically and has Gemma answer with
structured output. Same tools, same rules, and it works on today's runtime.

## The engine, not the model, has the last word

Gemma 4 E4B runs comfortably on a 16 GB laptop, and it's genuinely useful. In my runs it wrote correct fixes and caught
a real bug as Reviewer. It also wrote plausible wrong fixes, cited lines that didn't say what it claimed, and
occasionally repeated itself. So the design principle is simple: **the model proposes, the engine verifies.**

- Every citation is **re-read from disk**. In the recorded run, 21 of 22 citations checked out and 1 was flagged as a
  mismatch. A claim the model calls "verified" without a real citation is downgraded to "inferred".
- Edits are exact search/replace operations, applied atomically and only inside the successor. A model can't touch `.git`,
  escape the directory, or create files.
- A repair that makes tests worse, or whose new failure comes from the lines it edited, is reverted.
- **No model ever chooses a shell command.** Test commands come from detection, or from what you type.
- Repository text reaches the model only inside nonce-delimited "untrusted data" blocks. A README that says "ignore
  previous instructions" is just more data.

Every one of those rules exists because a real run broke without it. The repo has a decision log listing which run
taught which lesson.

## Why open innovation matters for this

**Because the whole point is outliving the original maintainer.** A preservation tool that depends on one company's
API staying online and affordable has the same single point of failure as the abandoned projects it rescues. Inheritance
Engine runs Gemma on your own machine. It works offline, costs nothing per run, and the Inheritance Record it produces
is plain JSON you own.

**Because you're feeding it someone else's code.** In `local_only` mode (the default), the engine refuses to send a
single byte of repository content to a non-loopback model endpoint. Inheriting code shouldn't mean uploading it.

**Because I could look inside.** When I needed structured output, I used the runtime's JSON-schema mode. When Ollama's
registry wouldn't serve models to my old client, I pointed the same adapter at LM Studio's OpenAI-compatible server in
an afternoon. When I want a stronger reviewer, one environment variable gives that single role the 12B model. Open
weights and open protocols turned vendor problems into configuration.

**Because it can be inherited too.** Fittingly, the engine is itself designed to be picked up by someone else:
documented decisions, a typed API, and tests for every safety rule.

## How I built it

- **Rust** single binary (axum, SQLite, tokio) as the trusted control plane. It never executes inherited code. It spawns
  `git` and `docker` with argument vectors and no shell.
- **Docker** sandbox: `--network none`, `--read-only`, `--cap-drop ALL`, `no-new-privileges`, memory, CPU and PID limits,
  non-root user, no host environment.
- **Gemma 4 E4B** through LM Studio's OpenAI-compatible API with `json_schema` constrained decoding. Ollama is also supported.
- **Vanilla JS** UI with no build step and no external requests, under a strict Content-Security-Policy.
- **Durable jobs:** every pipeline step is checkpointed. I killed the server mid-investigation and resumed it, and the
  finished steps (clone, inventory, baseline, plan) were skipped, not re-run.
- **45 tests** covering the safety rules: path traversal, symlink escapes, push disabled, original untouched, cleared
  environment, redaction, fabricated citations, the regression rules, and era-runtime selection.

The PRD and the implementation were AI-assisted: I worked with Claude as my engineering partner throughout the build,
and the runtime agents are Gemma. The full development session is `<session link if saved>`.

## Limitations, honestly

Verification currently works for Python and Node.js. It handles public GitHub repositories only. Docker isn't a perfect
security boundary, and native mode barely is one. A 4B model still writes wrong patches. The engine catches many, not
all, and probes only test what the Tester thought to probe. A full run takes 15–25 minutes on a laptop.

## Prize categories

- **Best Use of Gemma**: Gemma 4 runs every agent role, locally.
- **Best Use of Mastra**: the Lead Engineer agent with engine tools, a grounded answer workflow, and the upstream-issue
  triage workflow, all over Gemma, all used by the product.
- *Sentry Agent Tracing and DigitalOcean:* <!-- Claim ONLY after a real trace screenshot / a real Droplet run exists. -->
  The tracing and the deployment are built and tested locally. I'll list these only if real traces and a real Droplet
  run make it into this post.

Everything else was evaluated and not integrated; the repo has a per-category audit (`docs/partners`).

## License

Inheritance Engine is source-available under the PolyForm Noncommercial License 1.0.0. That's *not* an OSI open-source
license, and I want to be upfront about it. Inherited projects keep their own licenses. The engine records attribution
and never relicenses anything.

---

*Software doesn't have to die with its creator. It can be inherited, understood, repaired, and given a future.*
