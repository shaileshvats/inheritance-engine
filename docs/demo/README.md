# Demo: inheriting kachayev/fn.py

[kachayev/fn.py](https://github.com/kachayev/fn.py) ("Functional programming in Python", Apache-2.0, ~3.4k stars) last
received a commit on its default branch in October 2014. On Python 3.12 its test suite does not even start.

Everything below came from real runs on a 16 GB Apple M2 laptop, using Gemma 4 E4B in LM Studio (local, offline
inference) and Docker Desktop. Nothing in this document was hand-written to look like tool output. Excerpts of fn.py (Copyright 2013 Alexey
Kachayev, Apache-2.0) appear here and in `evidence/` and `screenshots/`; see [NOTICE](../../NOTICE). The Inheritance
Record from run A is committed at [`evidence/fn-py-run-a-record.json`](evidence/fn-py-run-a-record.json).

## Demo script (5–7 minutes)

1. **Hunter.** Problem-first search for *"I want currying, function composition and lazy infinite streams in Python like in
   Haskell or Clojure."* Gemma turns this into a GitHub query, and fn.py appears among real results with its evidence
   signals. Press *Why is this interesting?*
2. **Inherit.** Choose Docker. The job panel shows the clone (original kept read-only at `df53f5d`), the inventory, and the
   baseline, which fails: `ImportError: cannot import name 'Iterable' from 'collections'`.
3. **Archaeology.** Ask the Lead: *"Understand this project. Tell me what its creator was trying to build, how it works, and
   which components are essential."* Show the claims with their engine-checked citations, including a downgraded
   mismatch, and the Reviewer's verdicts.
4. **Restoration.** Ask: *"Find what is broken and fix the important problems without destroying the original
   behaviour."* Then press *Fix essential*. Walk through the repair timeline: reverted attempts, the review, the probe.
5. **Verification.** Baseline: 0 tests run (collection error). Final: **126 passed** in `python:3.12-slim` with network off.
   Then the behaviour probe: the original on `python:3.9-slim` against the successor on `python:3.12-slim`.
6. **Successor.** Open *Record & lineage*: the original commit, then the successor commits, `INHERITANCE.md`, and the Record
   download.

## What actually happened (run A)

Model: `google/gemma-4-e4b`. There were 19 traced model calls, about 14 minutes of model time in total.

| # | Step | Outcome |
| --- | --- | --- |
| 1 | Investigation | 4 decay hits found by the engine: `collections` ABC import, `inspect.getargspec`, `distutils`, and a version-guarded `string.letters` that the engine labels as probably harmless. 22 citations: **21 verified on disk, 1 mismatch flagged**. |
| 2 | Repair: Iterable, attempt 1 | Approved by the Reviewer, but the **engine reverted it**: the new failure originated in the edited line `fn/iters.py:2`. |
| 3 | Same issue, attempt 2 | **Engine reverted it before review**: `deque` was no longer imported but still used at line 27. |
| 4 | Re-diagnosis, round 1 | The Restorer read the new failure output; the repair was kept. Test counts were unchanged, but *"the failure moved from fn/iters.py:2 to fn/func.py:2"*. |
| 5 | Repair: getargspec, attempt 1 | **Engine reverted it before review**: `getargspec` was still used. |
| 6 | Same issue, attempt 2 | **The Reviewer (Gemma) rejected it** for a real bug: the patch computed `spec.args - len(args)`, a list minus an int. |
| 7 | Re-diagnosis, round 3 | `inspect.getargspec` became `inspect.getfullargspec`. Reviewer approved. **126 passed, 0 failed.** |
| 8 | Behaviour probe | Run A predates the era-runtime fix, so it compared against Python 2.7, where 2 of 3 probes were inconclusive. Fixed: version-guarded lines no longer pick the era. |

A manual check afterwards confirmed `curried()` works on the successor: `add3(1)(2)(3) == 6`.

## What actually happened (run B, final binary)

This was a fresh run on the final engine. During it, the Historian ran out of tokens while listing evidence, so the job
**failed**. The fix was to cap every agent list with `maxItems`. The job was then **resumed**: clone, inventory,
baseline and plan were skipped, and the agent steps re-ran. The Reviewer assessed all 12 findings.

First restoration pass ("Fix essential"):

| Attempt | Outcome |
| --- | --- |
| Iterable, attempt 1 | Reverted by the engine before review: `Iterable` and `deque` were both still used (lines 223 and 27) |
| Iterable, attempt 2 | Applied. The failure moved from `fn/iters.py:2` to `fn/func.py:2` |
| getargspec, rounds 1–3 | Six attempts, all refused: three by the dangling-name guard, two by the Gemma Reviewer (`spec.args - len(args)`), and **one by the behaviour probe** |
| The probe catch | The Reviewer approved the patch. The original on Python 3.9 printed `3`, `6`, `6` for three `curried` probes, and the successor on 3.12 raised `TypeError` on all three. **Reverted.** |

The pass ended with the successor honestly marked *restoring*, not *verified*: 1 error, and the original still
untouched. Without the probe, a broken `curried()` could have been committed.

**Second pass** (the user presses *Fix essential* again). After one dangling-name revert and one Reviewer rejection,
a fix was applied: **126 passed**, and the behaviour probe showed 3/3 preserved on Python 3.9 vs 3.12. Stage: *verified*.

**Then a human looked.** The kept fix replaced `len(getargspec(f).args)` with `len(signature(f).parameters)`. A manual
differential check (same method as the probe, different inputs) showed the difference:

```
                          original (3.9)     successor (3.12)
plain(1)(2)               3                  3
star(1)(2)  [*rest]       (1, 2, ())         still curried, waiting
kw(1)(2)    [**opts]      (1, 2, [])         still curried, waiting
```

`signature().parameters` counts `*args`/`**kwargs`, and `getargspec().args` did not. The Tester's three probes had
all used plain positional functions. In response:

- **Reopen with your finding.** The issue was reopened from the UI with that note. The Restorer and Tester receive it
  as a human finding that takes priority over passing tests, and the static re-scan no longer auto-closes the issue.
- **Human findings block "verified".** With the issue unresolved, the final step reads *"126 passed — not marked
  verified: the inheritor's reopened issue is unresolved"*, and the stage stays *restoring*.
- **Probes must cover distinct input shapes:** defaults, `*args`, `**kwargs`, keyword-only, empty inputs.

On the reopened issue, Gemma 4 E4B as Restorer did not find a correct fix in two passes. The Reviewer rejected a patch
that invented a nonexistent attribute (`f.prflags`). As of this writing the successor is honestly *restoring*: tests
green, and one human-reported behaviour difference open.

**Larger-model attempt.** The plan was to give the Restorer Gemma 4 12B QAT (`IE_MODEL_RESTORER=google/gemma-4-12b-qat`).
The file downloaded and its SHA-256 matches Hugging Face, but LM Studio (llama.cpp engine 2.13.0) refused to load it next
to the E4B (about 9.4 GB needed). On its own it failed with a generic "Failed to load model", with about 17% of RAM free and
heavy swap. It was not run. On a machine with more memory, or a newer LM Studio runtime, this is a one-variable change.

## Why the behaviour probe exists

During development, a Gemma repair replaced `getargspec(f)` with `inspect.signature(f).parameters` and left
`spec.args` in place. **All 126 tests passed**, because no test calls `curried()`. Run directly, it failed:

```
successor (python:3.12): AttributeError: 'mappingproxy' object has no attribute 'args'
original  (python:3.9):  6
```

That is the gap between "the tests pass" and "the software still does what its creator built". Repairs are now
compared against the original running on the runtime of its era.

## Earlier development runs (Qwen3 4B via Ollama, used for smoke testing)

These runs surfaced engine bugs that were then fixed:

- A wrong patch (`from collections.abc import deque, Iterable`) was kept because test counts didn't change. Fix: the
  failure-origin rule.
- A fix that took the suite from "1 error" to "124 passed, 2 failed" was judged a regression. Fix: a passed-dominant
  regression rule.
- A patch silently dropped `deque`. Fix: the dangling-import guard.
- The Historian repeated one claim six times, and the Reviewer flagged them as identical. Fix: claim deduplication.

## Reliability notes for a live demo

- Model latency on an M2 ranges from about 10 to 140 seconds per call, so a full run takes 15–25 minutes. For a live demo,
  start the run beforehand and present the recorded result: the UI, traces, and Record are the real ones. Run one fresh
  agent call live, such as a Lead chat question or *Why is this interesting?*.
- Durable jobs mean that a crash or restart does not lose progress. Resume continues from the last completed step.
