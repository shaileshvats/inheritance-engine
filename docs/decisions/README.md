# Decision log

## D1 — One Rust binary instead of a TypeScript server plus a Rust companion

The machine used for development had Rust but no Node.js. The PRD preferred memory-safe tooling for the part that
touches the filesystem, processes, Git and Docker. Splitting a hackathon MVP across two languages and an IPC boundary
would have added risk without adding capability. So a single Rust binary is both the API server and the "companion",
and it exposes a narrow HTTP API on loopback, not a shell.

## D2 — No agent framework (Mastra not used)

Mastra is a TypeScript framework, so using it would have meant introducing Node.js only for orchestration. The agent
team needs four things, and Rust covers them directly: constrained JSON calls, traced parent/child calls, scoped
context, and engine-side validation. Using Mastra purely to qualify for its prize category would violate the PRD's
"genuine use" rule. The Mastra category is therefore **not claimed**.

## D3 — SQLite checkpoints instead of Temporal

Durable jobs need persistence of step status, pause/resume/cancel, and recovery after restart, without re-running
finished model calls. Every pipeline step checks its checkpoint artifact, which gives that behaviour with no extra
services to run. Temporal would be the right tool once restoration runs on remote workers. The Temporal category is
**not claimed**.

## D4 — LM Studio as the default runtime, Ollama supported

Ollama 0.17.7 was installed on the development machine. Its registry refused to serve any current model to that client
version, so Gemma 4 could not be pulled without upgrading the application. LM Studio, also installed, could download
`google/gemma-4-e4b` and serve it over an OpenAI-compatible API. The engine therefore speaks both protocols, and the
default points at LM Studio. Gemma 4 E4B was chosen over 12B because it leaves headroom for Docker on a 16 GB machine.
The 12B QAT build can be used for any single role through `IE_MODEL_<ROLE>`.

## D5 — Search/replace edits instead of unified diffs

4B-class models produce malformed unified diffs often. Exact search/replace edits are easier for the model to produce
and easy for the engine to validate (exactly one match, or an explicit `all_occurrences`). The engine applies them
atomically.

## D6 — Rules learned from real runs (fn.py)

Each of these was added after a real run went wrong, not in anticipation:

1. **Passed-dominant regression rule.** A collection error hides every test, so going from "1 error" to "124 passed,
   2 failed" is progress. It had been misjudged as a regression.
2. **Failure origin.** A patch that moved `deque` into `collections.abc` left counts unchanged, so it was kept. Now, if
   the innermost traceback frame lands in the edited lines, the patch is reverted.
3. **Dangling imports.** Patches that drop an import the file still uses are reverted before review.
4. **Behaviour probes.** A `getargspec` fix passed all 126 tests and still broke `curried()`, which no test calls.
   Probes now compare the successor on Python 3.12 against the original on Python 3.9.
5. **Bounded lists.** A Historian call ran out of tokens while listing evidence and produced invalid JSON. Every schema
   array now has `maxItems`, which LM Studio's constrained decoding enforces. Truncation is reported as truncation.
6. **Era runtime ignores guarded code.** A version-guarded `string.letters` once selected Python 2.7 as the era
   runtime, which made probes inconclusive.
7. **Humans outrank green tests.** A fix with 126 passing tests and 3/3 preserved probes still changed `curried()` for
   variadic functions. The inheritor can now reopen an issue with their finding. An unresolved reopened issue blocks
   the *verified* stage, and Tester probes must cover distinct input shapes.
8. **Reviewer rejections get one informed retry**, carrying the reviewer's concrete concerns.

## D7 — Prize categories

Only Gemma is used in the product. The other partner technologies were evaluated against the PRD's rule ("genuine use
only, no spending without approval"), and none were integrated in this build. Claiming them would be false.
