# Submission-day readiness pass (2026-10-31)

A record of what was audited, what was changed, what was tested and what is still broken. Times are PDT.

## Ranked gaps found (07:30)

| # | Gap | Impact | Plan |
| --- | --- | --- | --- |
| 1 | **Deadline ambiguity.** DEV's challenge record has `ends_at` = 2026-10-31 06:59 UTC (Oct 30, 11:59 PM PDT), while its text says "October 31: Submissions due at 11:59 PM PDT". Every earlier week has the same one-day offset, so the text is probably right, but it is a risk. | Could void the entry | Publish as early as the owner approves; the video can be added by editing the post. |
| 2 | **The signature demo is not repeatable.** The "Reviewer approved, tests green, probe caught it" moment came from one Gemma run. Judges cannot reproduce it, and without a 6 GB local model they can't see anything at all. | Technical execution, judge trust | A replay that feeds the *recorded* patch, review and probe code through the engine's *live* checks (Docker tests, probe on Python 3.9 vs 3.12, revert rule, Inheritance Record). No model needed. Clearly labelled as a replay. |
| 3 | **Clean-start bugs.** README says Rust 1.80+, but dependencies need 1.89. The companion says Node ≥22, but `node src/server.ts` needs ≥22.18 (type stripping). `IE_PROMPT_BUDGET_CHARS`, `IE_TELEMETRY_AUDIT` and `IE_WEB_DIR` are missing from `.env.example`. | First impression for judges | Fix the docs and manifests; test a fresh clone. |
| 4 | **Article structure.** Writing quality is weighted most. The opening is good but the demo story is spread over three sections, and the post never separates what runs offline from what needs the internet. | Writing quality | Tighten the opening, add a "what runs where" table, and a short "try it" path. |
| 5 | **The Inheritance Record is peripheral to the story.** It is the project's answer to the theme (provenance, authors, intent, evidence) but appears only at the end, with no excerpt or screenshot. | Theme relevance | Make the record the spine of the theme section, with a real excerpt and screenshot. |
| 6 | **No single submission-day regression record.** Results are spread across docs. | Honesty, reproducibility | Run the focused checks below and record actual results here. |

Not pursued: new integrations (none is both free and meaningful at this point), the agent-session embed (owner's decision).

## Known broken (before this pass)

- fn.py: the reopened `*args`/`**kwargs` issue is unresolved after four Gemma patches. The successor is *restoring*, 126 passed.
- smokesignal: 7 passed, 28 failed. The bound-method failures are not solved.

## What was changed

| Commit | Change |
| --- | --- |
| `1e781ba` | Probe output normalises memory addresses (`<function f at 0x…>`), which differ on every run and could have produced a false "changed". Probe execution is split from probe generation so recorded probes can run through the same code. |
| `3aa16cc` | **Replays.** Recorded model output (patch, Reviewer verdict, probe code) goes through the live checks: clone, inventory, Docker tests before and after, probes on Python 3.9 vs 3.12, the revert rule, and the Inheritance Record. No model is called. Dashboard entry, a banner on the inheritance, and a badge on the repair. Never commits recorded output. |
| `83c3e73` | Clean start: Rust 1.89+ (`rust-version` set), Node.js 22.18+ for the companion, every setting in `.env.example`, a new `IE_MODEL_TESTER` override, a README replay quick start and an offline/online table. The README and decision log no longer say Mastra is unused. |
| `8a95e39` | Replay bug found by the fresh-clone test: with the default relative data directory, `git apply` could not find the patch. |
| `2908e89` | Web research: personal social profiles are never results, the docs query no longer uses the owner's name, and "successor"/"fork"/"original" claims the engine can't support are downgraded to "mention" with a visible note. |

## What was tested (actual results)

| Check | How | Result |
| --- | --- | --- |
| Rust tests | `cargo test --release` | **61 passed, 0 failed** (was 57; new: fixture completeness, address normalisation, personal-profile filter, lineage-claim rule). `cargo clippy --all-targets`: no warnings. |
| Mastra tests | `npm test` in `companion/mastra` | **19 passed** |
| Fresh clone, README path | `git clone`, `cargo run --release` (no `.env`), then the README's `curl` replay | **First attempt failed** (relative patch path, fixed in `8a95e39`). After the fix: build 90 s cold, replay 13 s, completed, `record-v1.json` written. |
| Signature demo (replay) | 3 replays, model endpoint unreachable on 2 of them | **3 of 3 reproduced the catch**: 126 → 126 passed, Reviewer approval (recorded), 3 of 3 probes changed (original `(1, 2)` then `TypeError`; successor `NameError`), reverted. |
| fn.py successor | Docker `verify` job | 126 passed, stage *restoring* (reopened human finding still open). Matches the article. |
| smokesignal successor | Docker `verify` job | 7 passed, 28 failed (AttributeError and TypeError from Python 3 bound methods). Matches the article. Repair history: two attempts reverted for the dangling `Mock` import, the third kept; later a truncated Restorer call, a revert because the failure came from the edited line, and a Reviewer rejection. |
| Web research + citation checks | 4 live runs on smokesignal (12 SerpApi searches, free plan), one before and three during the fixes | **Weak result, honestly.** Run 1 surfaced the author's personal social profiles and called CPython's `signal` module a "successor". Run 2 (profile fix): profiles gone, "successor" still accepted. Runs 3 and 4 (lineage rule): "successor" downgraded both times; run 3 also claimed CPython was the "original", which led to the last rule. The docs Gemma picks are still about Python's built-in `signal` module, not smokesignal: the engine can verify that a source exists, not whether it is relevant. |
| Mastra Lead reads research | Question to fn.py's Lead, answered via Mastra in 96 s | Cited `kachayev.github.io/fn.py/` from the web research, #91 as `needs_human`, and the unresolved `getargspec` issue. It called the open issue a "test failure"; the tests pass and the open item is the human finding. |
| Sentry scrubbing | Unit tests, plus a scan of all 26 payloads in the local telemetry audit files (engine and companion) | No inherited source lines, no keys, none of the 9 content attributes. Only metadata (`gen_ai.usage.*`, model, operation and tool names). |

## Still broken or limited

- fn.py's reopened `*args`/`**kwargs` issue is unresolved; the successor is *restoring*.
- smokesignal: 28 failures remain.
- Web research for obscure projects is often irrelevant; citation checks do not catch irrelevance.
- The Mastra Lead occasionally mislabels things (see above).
- Replays exist for one recorded case only.
