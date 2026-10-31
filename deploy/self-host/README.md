# Self-hosting Inheritance Engine on a Linux host

> **Status: validated locally, never deployed to a cloud provider.** The images were built and the containers were run
> on this project's development machine (see *Validation*). The project was built on a zero budget, so no cloud VM was
> created and no cloud provider category is claimed.

The supported way to run Inheritance Engine is still on your own machine (see the main README). This directory is for
running it on a dedicated Linux host instead, for example when your laptop is busy or underpowered.

## Why a whole host

Inheritance Engine runs untrusted code in Docker sandboxes, so it needs a host where it can use a Docker daemon. A plain
Linux VM or spare machine runs the whole system unchanged: the engine, the Mastra companion, and an open-weight model
served by Ollama.

```
Linux host (Ubuntu, Docker)
 ├─ ollama      127.0.0.1:11434  gemma4:e4b (open weights, CPU)
 ├─ engine      127.0.0.1:7878   control plane + UI; drives the host Docker daemon for sandboxes
 ├─ mastra      127.0.0.1:7880   Lead Engineer agent + upstream-issue triage workflow
 └─ sandboxes   python:3.12-slim / node:22-slim, network off, read-only rootfs, caps dropped
Internet → SSH only (ufw). The UI is reached through an SSH tunnel.
```

## Sizing

- Gemma 4 E4B needs about 7 GB of RAM in Ollama. Add the engine, the companion and a 1 GB sandbox, and that means
  **at least 16 GB RAM** and 4+ CPU cores.
- CPU inference is slow, a few tokens per second, so a full investigate-and-restore run can take an hour or more. A GPU
  is much faster.
- If you rent a cloud VM, check the provider's pricing first. Most bill hourly while the VM exists, often including
  when it is powered off. Delete it when you are done.

## Steps

1. Create an Ubuntu 24.04 host with 16 GB+ RAM. If your provider supports cloud-init, paste
   [`cloud-init.yaml`](cloud-init.yaml) as user data. Otherwise run its commands by hand. Open only SSH to the internet.
2. Wait for setup to finish (`cloud-init status --wait`). The first start downloads the model, about 7 GB.
3. Open a tunnel from your machine, then browse to http://127.0.0.1:7878:

   ```bash
   ssh -L 7878:127.0.0.1:7878 user@HOST
   ```

4. Optional: put `SENTRY_DSN=...` and `GITHUB_TOKEN=...` (a token with no scopes) in
   `/opt/inheritance-engine/deploy/self-host/.env`, then `docker compose up -d`.

## Security notes

- The engine container mounts `/var/run/docker.sock`. That grants control of the host's Docker daemon, which is
  equivalent to root on the host. It is required for sandboxing, and acceptable only on a single-purpose host. Don't
  run other workloads on it.
- No port other than SSH is open. The API has no user accounts. Don't expose 7878 publicly.
- `IE_PRIVACY_MODE=local_only` holds here too: the model runs on the same machine.

## Validation

Done on 2026-10-28 with Docker Desktop on macOS (arm64):

- Both images built from these Dockerfiles (engine 359 MB, companion 848 MB).
- **Companion container:** started, served `/health` (grounded mode, since no model was reachable inside it), and
  refused to start when pointed at a non-local model endpoint.
- **Engine container:** started with `/var/run/docker.sock` and a same-path data mount. It reported Docker 29.5.3 through
  the socket. It then inherited `shaunduncan/smokesignal`: clone, inventory, and a **baseline run in a sandbox started
  from inside the container** (pytest collection error, the repository's real state on Python 3.12). The first agent
  call was then **refused by `local_only`**, because the host's LM Studio isn't loopback from inside the container.
  That is the intended behaviour. On a Linux host the model is Ollama on the same loopback.

`network_mode: host` behaves differently on Docker Desktop than on Linux, so the full compose stack has **not** been
run end to end. Treat it as a starting point, not a tested deployment.
