# Demo video: shot list (about 2.5 minutes)

Everything shown is the real recorded state of the fn.py inheritance plus **one live Gemma call** (the Lead chat).
Nothing is staged or edited into the UI.

## Before recording

- LM Studio running with `google/gemma-4-e4b` loaded, Docker running, the engine on http://127.0.0.1:7878 and the
  Mastra companion on :7880 (status chips at the top must all be green).
- Browser window about 1440×900, zoom 100%, dark theme. Close other tabs.
- Never show `.env`, a terminal with environment variables, or Sentry's project settings / DSN page.
- Record with macOS **Cmd+Shift+5** (free). Upload to YouTube as **Unlisted** (free), then embed the link in the article.

## Shots

| Time | Screen | What to say (roughly) |
| --- | --- | --- |
| 0:00–0:15 | github.com/kachayev/fn.py: the commit list, last commit October 2014 | "fn.py: 3.4k stars, last touched in 2014. On Python 3.12 its tests don't even start." |
| 0:15–0:30 | Dashboard → fn.py header: *Original untouched*, successor branch, stage tracker | "Inheritance Engine keeps the original read-only and works in a successor with pushing disabled." |
| 0:30–0:55 | **Archaeology** tab: Reviewer challenges, the engine's citation checks | "A team of Gemma agents, running on this laptop, investigates. Every quote is re-read from disk by the engine, not trusted from the model." |
| 0:55–1:30 | **Restoration** tab: the attempts in order (reverted by engine, dangling import, Reviewer reject, behaviour probe catch), then 126 passed | "The model proposes, the engine verifies. This probe ran the original on Python 3.9 and the successor on 3.12. Original said 3, 6, 6; the repair said TypeError. Reverted." |
| 1:30–1:50 | The reopened issue and the final step: *126 passed — not marked verified* | "A human found one more behaviour change with *args. Human findings outrank green tests, so it is not marked verified." |
| 1:50–2:10 | **Overview**: Mastra triage card (#91 addressed, with its verified citation). Then ask the Lead live: *"What is still unresolved, and why?"* | "A Mastra workflow triages the original's open issues. The Lead Engineer is a Mastra agent over the same local Gemma." |
| 2:10–2:25 | Sentry: AI Agents overview → engine trace → a model call showing *No input for this span* | "Sentry shows every agent call, tokens and latency, and none of the code." |
| 2:25–2:40 | **Record & lineage**: `INHERITANCE.md` naming Alexey Kachayev and the other authors | "The first thing the successor gets is the names of the people who built it." |

## If the live chat is slow

Gemma calls take 10–140 s on this laptop. Ask the question before you start recording that segment, or cut the wait
in editing and say so on screen ("sped up").
