# Security model

Inheritance Engine exists to open code written by strangers. Security is therefore part of the product, not a layer
on top. This document states what is enforced, what is not, and what has not been tested. It is not a guarantee.

## Trust boundaries

| Zone | Trusted? | Notes |
| --- | --- | --- |
| Control plane (Rust binary, `src/`) | trusted | never executes repository code; no shell; spawns `git` and `docker` with argument vectors |
| Model runtime (LM Studio / Ollama / other) | trusted to compute, not to decide | output is parsed as constrained JSON and validated by the engine |
| Successor workspace | untrusted content | read by the engine with path containment; written only through validated edits |
| Docker container | untrusted execution | see limits below |
| Native execution | untrusted execution, weakly contained | explicit opt-in per inheritance |
| GitHub API, READMEs | untrusted content | delimited as data in prompts |

## Enforced controls

**Intake.** Only `https://github.com/<owner>/<repo>` URLs (validated name). `git` runs with `core.hooksPath=/dev/null`,
`core.symlinks=false`, `core.fsmonitor=false`, empty `credential.helper`, `protocol.ext.allow=never`,
`GIT_TERMINAL_PROMPT=0`, no system config, no submodule recursion. The original is a bare clone with write permission
removed; integrity (branch still at the recorded commit, still read-only) is checked on every detail request. The
successor's push URLs are set to an invalid value, so `git push` fails even if attempted.

**Docker execution.** `--network none` (dependency install is the only networked step, and it does not run project
code beyond what pip/npm do — npm uses `--ignore-scripts`; `setup.py` is not executed by the engine), `--read-only` root
filesystem with a size-limited `/tmp`, `--cap-drop ALL`, `no-new-privileges`, `--memory 1g`, `--cpus 2`,
`--pids-limit 256`, non-root user for syntax and test runs, only the successor and a per-inheritance dependency volume
mounted, no host environment variables, per-run timeout with `docker kill`.

**Native execution.** Environment cleared (`PATH`, a throwaway `HOME`/`TMPDIR`, a short allowlist only), working
directory inside the successor, process group killed on timeout. Network and filesystem access are *not* restricted.
The UI and this document say so.

**Model boundary / prompt injection.** Repository text is only placed inside `<<<UNTRUSTED <nonce> …>>>` blocks with a
per-call random nonce; system prompts state that such content is data. Model output cannot run commands: verification
commands come from deterministic detection or from text the user typed. File edits proposed by models must name an
existing file inside the successor (`safe_join` rejects `..`, absolute paths, `.git`, and symlink escapes) and must match
existing text exactly; all edits in a batch apply or none do.

**Secrets.** A regex scanner flags likely secrets (cloud keys, tokens, private keys, hard-coded passwords) in the
trust passport (masked). The same patterns redact prompts, responses, test output, file views, and errors before they
are stored or displayed. `GITHUB_TOKEN` and `IE_MODEL_API_KEY` are never logged, stored, or passed to inherited code.
Detected secrets are not rotated, used, or transmitted.

**Privacy policy.** `IE_PRIVACY_MODE=local_only` (default) refuses any non-loopback model endpoint before a request is
made. `cloud_with_permission` additionally requires `IE_ALLOW_CLOUD=1`.

**HTTP API.** Binds to `127.0.0.1` by default. Requests whose `Host` is not a loopback name are rejected (DNS
rebinding); state-changing requests with a foreign `Origin` are rejected (CSRF). Responses carry a strict CSP,
`nosniff`, `X-Frame-Options: DENY`, and `no-referrer`. The UI renders all repository- and model-derived text as text.

**External side effects.** The engine never pushes, publishes, opens pull requests, or contacts the original author.
There is no code path for it in this version.

## Known limitations

- Docker is not a perfect security boundary; kernel or Docker Desktop vulnerabilities could allow escape.
- The dependency-install step has network access; a malicious dependency could exfiltrate anything inside that
  container (which contains no host secrets) or poison the dependency volume used by later runs.
- Native mode offers little isolation beyond a cleared environment.
- The secret scanner is pattern-based and will miss secrets; it can also produce false positives.
- Prompt-injection defences reduce, not eliminate, the chance that repository text steers a model. The impact is bounded
  by the engine-side validation above, but a manipulated model could still propose a plausible-looking malicious edit;
  the independent reviewer and the visible diff are the mitigations, and nothing is pushed anywhere.
- There are no user accounts; anyone who can reach the loopback port can use the API.
- Not yet done: SAST/dependency/container scanning in CI, fuzzing of the parsers, a malicious-repository fixture suite,
  and an independent penetration test (OWASP ASVS used as a reference only).

## Tests that cover these controls

`cargo test` includes: path traversal, `.git` and symlink escapes (`security`), atomic edits and refusal to create or
escape files (`pipeline`), locked-down Docker arguments and absence of host environment (`sandbox`), cleared native
environment and timeout kill (`sandbox`), push being disabled and the original untouched after successor commits
(`gitops`), non-GitHub/`ext::` URL rejection (`gitops`), privacy policy enforcement (`models`), secret redaction
(`security`), and citation fabrication detection (`evidence`).
