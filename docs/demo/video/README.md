# Demo video

[`inheritance-engine-demo-v2.mp4`](inheritance-engine-demo-v2.mp4) (4 min 49 s, no audio) is the current demo. It keeps
the original 31 October footage below frame for frame and adds a part recorded on 2026-11-01 from the same laptop:

| Time | Part | Badge |
| --- | --- | --- |
| 0:10–3:03 | The original footage: GitHub, Hunter with SerpApi, fn.py, the replayed catch (Reviewer approval, 126 → 126 tests, probe 3 of 3 changed, reverted), Mastra triage, web research, a live Lead question, the agent trace, two Sentry stills | as v1 |
| 3:07–3:24 | **Temporal**: the completed `inheritAndRestore` workflow from 31 October in the local Temporal UI (one restore job id after the worker was killed; the retried `awaitJob` in the timeline) | RECORDED RUN DATA · TEMPORAL UI |
| 3:24–3:53 | **Restoration memory (Tiger Data)**: the query the Lead's `search-restoration-memory` tool made from smokesignal, re-run live with the same code; its terminal output is shown unedited (lines are only highlighted). It retrieves four fn.py repairs (three rejected, one reverted); the kept repair `rep_8dc8ce72de354902` is in the memory but ranks 15th, and the caption says the Lead's answer wrongly said none was kept | LIVE · TERMINAL OUTPUT |
| 3:53–4:07 | The same fn.py records in the app, including the kept repair's card (commit cd83239f) | RECORDED RUN DATA |
| 4:07–4:25 | **GitHub Actions**: a passing CI run on github.com and its `replay-inheritance-record` artifact (logged out, so logs are not shown) | LIVE · GITHUB.COM |
| 4:25–4:49 | The Inheritance Record (v1 footage) and the end card | as v1 |

The new title and end cards say when each part was recorded. Edits to the new part: frames captured while a page was
still loading were cut, and captions were written after checking every repair id and status against the engine.

## v1

[`inheritance-engine-demo.mp4`](inheritance-engine-demo.mp4) (3 min 26 s, no audio) is kept as the original. It was
recorded on 2026-10-31 from the real application running on one laptop at `127.0.0.1:7878`. A script drove a headless
Chrome over the DevTools protocol (real clicks and typing) and captured the page about 8 times a second; the frames were
encoded with macOS AVFoundation. Captions and corner badges were added during encoding. Nothing was edited into the
application's UI.

What the badges mean:

- **LIVE**: happening in the recording (GitHub page, Hunter search with SerpApi, a question answered by Gemma).
- **RECORDED RUN DATA**: browsing results stored by earlier real runs (archaeology, restoration history, triage).
- **REPLAY · CHECKS RUN LIVE**: recorded Gemma output (patch, review, probe code) put through live checks; see
  [the README](../../../README.md#see-it-work-in-a-few-minutes-without-a-model).
- **SCREENSHOT · NOT LIVE**: two Sentry stills from 2026-10-28; the headless browser had no Sentry session.
- **SPED UP 8×**: the wait for Gemma's answer, fast-forwarded.

Edits after capture: frames showing the browser before github.com had loaded were cut, and the last GitHub frame is held
so it can be read; one caption was corrected to match what Gemma actually answered.
