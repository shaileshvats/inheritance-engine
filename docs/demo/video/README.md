# Demo video

[`inheritance-engine-demo.mp4`](inheritance-engine-demo.mp4) (3 min 26 s, no audio) was recorded on 2026-10-31 from the
real application running on one laptop at `127.0.0.1:7878`. A script drove a headless Chrome over the DevTools protocol
(real clicks and typing) and captured the page about 8 times a second; the frames were encoded with macOS AVFoundation.
Captions and corner badges were added during encoding. Nothing was edited into the application's UI.

What the badges mean:

- **LIVE**: happening in the recording (GitHub page, Hunter search with SerpApi, a question answered by Gemma).
- **RECORDED RUN DATA**: browsing results stored by earlier real runs (archaeology, restoration history, triage).
- **REPLAY · CHECKS RUN LIVE**: recorded Gemma output (patch, review, probe code) put through live checks; see
  [the README](../../../README.md#see-it-work-in-a-few-minutes-without-a-model).
- **SCREENSHOT · NOT LIVE**: two Sentry stills from 2026-10-28; the headless browser had no Sentry session.
- **SPED UP 8×**: the wait for Gemma's answer, fast-forwarded.

Edits after capture: frames showing the browser before github.com had loaded were cut, and the last GitHub frame is held
so it can be read; one caption was corrected to match what Gemma actually answered.
