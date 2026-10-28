# Handover — Inheritance Engine v0.1.0 (built 2026-10-28)

## 1. What was built

A working, end-to-end inheritance journey: Hunter (GitHub search) → Inherit (read-only original, push-disabled
successor) → Investigate (deterministic inventory, trust passport, Docker baseline, Gemma agent team with
engine-checked evidence) → Restore (patch → engine guards → independent review → Docker verification → behaviour probe
against the original in its era runtime → commit or revert, with re-diagnosis rounds) → Preserve (`INHERITANCE.md`,
versioned Inheritance Record). It has a web UI, durable jobs, and a full agent trace.

## 2. How to run it

See the README quick start. In short: `lms load google/gemma-4-e4b --context-length 16384 && lms server start`,
start Docker, then `cargo run --release` and open http://127.0.0.1:7878.

## 3. Architecture

See [architecture/README.md](architecture/README.md) and [decisions/README.md](decisions/README.md).

## 4. Configuration

All optional; see [`.env.example`](../.env.example). Nothing requires an account or a paid service.

## 5. Tests and their real results

- `cargo test`: **45 passed, 0 failed**. `cargo clippy --all-targets`: no warnings.
- End-to-end runs on kachayev/fn.py with Gemma 4 E4B: see [demo/README.md](demo/README.md). That document reports
  the failures too.
- Not run: SAST, dependency audit (`cargo audit` is not installed), container scanning, fuzzing, a malicious-repository
  fixture suite, a penetration test.

## 6. Security limitations

See [security/README.md](security/README.md). In short, Docker is not a perfect boundary, the dependency-install
step has network access, native mode barely isolates anything, there are no user accounts, and secret scanning is
pattern-based.

## 7. Sponsor integrations actually implemented

- **Gemma**: Gemma 4 E4B runs every agent role, locally, through LM Studio. Gemma 4 12B QAT was also downloaded and
  can be assigned to any role. This is genuine, central use.
- **None of the others.** Mastra, Temporal, Sentry, Render, DigitalOcean and the rest were evaluated and not integrated
  (see decision log D2, D3, D7). Claiming them would be false. Arduino is excluded because no hardware is available.

## 8. Deployment

Local only, by design: the engine executes untrusted code through the local Docker daemon and binds to loopback.
A hosted deployment would need user accounts, per-user isolation and remote workers. That isn't built.

## 9. Screenshots

[`demo/screenshots/`](demo/screenshots/)

## 10. Demo instructions

[demo/README.md](demo/README.md) has the script, what really happened, and reliability notes for presenting live.

## 11. Known bugs

- The dependency graph view scrolls horizontally to a selected node and can leave much of the canvas empty.
- The UI theme toggle switches only between light and dark (there is no "follow system" option after the first toggle).
- Model-written issue titles are sometimes inconsistent with their category/severity (for example "essential / low").
- If Docker is not running, verification is recorded as `not_run` with the reason. It is not retried automatically.

## 12. Unfinished features (from the PRD)

Private repositories / GitHub OAuth, GitLab/Bitbucket/ZIP intake, multi-language verification beyond Python/Node,
lineage graph across forks, community features, cloud workers, encrypted backups, export/import of whole workspaces,
notifications beyond in-app, accessibility audit with a screen reader, legal documents (privacy policy and terms; there
is no hosted service, so none collect data yet).

## 13. Roadmap

1. More runtimes for verification and probes (Go, Rust, Java), and probe generation guided by coverage.
2. GitHub OAuth (read-only first), then *authorized* branch and PR creation for successors.
3. Lineage across forks and independent restorations, with provenance and confidence for each edge.
4. A malicious-repository fixture suite and gVisor/Firecracker execution adapters.
5. Shared, opt-in restoration recipes ("collections.Iterable → collections.abc.Iterable, verified on N projects").
