# Deploying Inheritance Engine on DigitalOcean

> **Status: prepared, not deployed.** These files were built and checked locally (see *Validation*), but no Droplet has
> been created. Creating one is billable. That needs the owner's DigitalOcean account and either Hacktoberfest credits
> or explicit approval to spend.

## Why a Droplet

Inheritance Engine runs untrusted code in Docker sandboxes, so it needs a host where it can use a Docker daemon. A
Droplet is a plain Linux VM, so the whole system runs there unchanged: the engine, the Mastra companion, and an
open-weight model served by Ollama. That turns the PRD's "optional cloud worker" into something you can actually run,
for when the laptop is busy or underpowered.

```
Droplet (Ubuntu, Docker)
 ├─ ollama      127.0.0.1:11434  gemma4:e4b (open weights, CPU)
 ├─ engine      127.0.0.1:7878   control plane + UI; drives the host Docker daemon for sandboxes
 ├─ mastra      127.0.0.1:7880   Lead Engineer agent + upstream-issue triage workflow
 └─ sandboxes   python:3.12-slim / node:22-slim, network off, read-only rootfs, caps dropped
Internet → SSH only (ufw). The UI is reached through an SSH tunnel.
```

## Sizing and cost

- Gemma 4 E4B needs about 7 GB of RAM in Ollama. Add the engine, the companion and a 1 GB sandbox, and that means
  **at least 16 GB RAM**, for example a General Purpose or Basic Droplet with 4+ vCPUs.
- CPU inference is slow, a few tokens per second, so a full investigate-and-restore run can take an hour or more. A GPU
  Droplet is much faster and much more expensive.
- Check current prices at <https://www.digitalocean.com/pricing/droplets> before creating anything. Droplets bill
  hourly while they exist, **including when powered off**. Destroy the Droplet when you are done.

## Steps (owner only)

1. Check credits: sign in at <https://hacktoberfest.com/my/promos/> and redeem any DigitalOcean offer in your own
   account.
2. Create a Droplet (Ubuntu 24.04, 16 GB+ RAM) and paste [`cloud-init.yaml`](cloud-init.yaml) as *User data*. Add
   your SSH key. Do not add a public IP firewall rule beyond SSH.
3. Wait for cloud-init to finish (`cloud-init status --wait` on the Droplet). The first start downloads the model, about
   7 GB.
4. Open a tunnel from your machine, then browse to http://127.0.0.1:7878:

   ```bash
   ssh -L 7878:127.0.0.1:7878 root@DROPLET_IP
   ```

5. Optional: put `SENTRY_DSN=...` and `GITHUB_TOKEN=...` (a token with no scopes) in
   `/opt/inheritance-engine/deploy/digitalocean/.env`, then `docker compose up -d`.
6. When finished: destroy the Droplet in the control panel, or with `doctl compute droplet delete <name>`.

## Security notes

- The engine container mounts `/var/run/docker.sock`. That grants control of the host's Docker daemon, which is
  equivalent to root on the Droplet. It is required for sandboxing, and acceptable only on a single-purpose Droplet.
  Don't run other workloads on it.
- No port other than SSH is open. The API has no user accounts. Don't expose 7878 publicly.
- `IE_PRIVACY_MODE=local_only` holds on the Droplet too: the model runs on the same machine.

## Validation

Done on 2026-10-28 with Docker Desktop on macOS (arm64):

- Both images built from these Dockerfiles (engine 359 MB, companion 848 MB).
- **Companion container:** started, served `/health` (grounded mode, since no model was reachable inside it), and
  refused to start when pointed at a non-local model endpoint.
- **Engine container:** started with `/var/run/docker.sock` and a same-path data mount. It reported Docker 29.5.3 through
  the socket. It then inherited `shaunduncan/smokesignal`: clone, inventory, and a **baseline run in a sandbox started
  from inside the container** (pytest collection error, the repository's real state on Python 3.12). The first agent
  call was then **refused by `local_only`**, because the host's LM Studio isn't loopback from inside the container.
  That is the intended behaviour. On a Droplet the model is Ollama on the same loopback.

`network_mode: host` behaves differently on Docker Desktop than on Linux, so the full compose stack was **not** run
end to end locally. It is meant for a Linux Droplet.
