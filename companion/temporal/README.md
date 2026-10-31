# Temporal companion (optional)

One durable workflow, `inheritAndRestore`, that takes a repository from "inherit" to an exported Inheritance Record
without anyone watching:

1. inherit and **investigate** (or continue an existing inheritance),
2. **restore** (`essential` or `fix_all`),
3. **web research** (SerpApi) and **upstream-issue triage** (Mastra), each skipped if not configured,
4. read back the **Inheritance Record**.

The engine already checkpoints each of its own jobs. What it doesn't have is anything that chains them, waits out a
service that is down, or re-runs a job that failed. That's what this workflow adds:

- **Waits instead of failing.** Before each stage it checks `GET /api/status`. If the model server, Docker or the
  Mastra companion is down, the check fails and Temporal retries it with backoff for up to six hours.
- **Survives restarts.** `awaitJob` follows the engine's job and heartbeats its current step. If the worker dies, the
  heartbeat times out and another worker resumes following the same job. If the engine restarts, polling fails and is
  retried until the engine is back and has resumed its job.
- **Re-runs failed jobs a bounded number of times** (`--attempts`, default 2). A model call that timed out or got cut
  off gets one more try, and both job ids are recorded.
- **Never starts work twice.** Retried activities attach to the job that is already running, and a retried "inherit"
  reuses the inheritance it already created.
- **Respects people.** If a job is paused in the engine UI, the workflow waits. If it's cancelled, the workflow stops
  that stage.

Only ids, job states and step summaries go into Temporal's history. Repository content never does. Both the engine
URL and the Temporal address must be loopback. To use a remote Temporal service you have to set
`IE_TEMPORAL_ALLOW_REMOTE=1` explicitly.

## Run it

```bash
# 1. A local Temporal service (the open-source dev server, persisted to a Docker volume)
docker run -d --name ie-temporal --user 0 -p 127.0.0.1:7233:7233 -p 127.0.0.1:8233:8233 \
  -v ie-temporal-data:/data temporalio/temporal:latest server start-dev --ip 0.0.0.0 --db-filename /data/temporal.db

# 2. The worker (with the engine running on 127.0.0.1:7878)
cd companion/temporal && npm ci && npm run worker

# 3. Start a workflow from another terminal
npm run inherit -- kachayev/fn.py --wait                      # inherit, investigate, restore, research, triage
npm run inherit -- --inheritance inh_… --restore fix_all      # continue an existing inheritance
```

Watch it in the Temporal UI at http://127.0.0.1:8233. Use the `progress` query to see the stage list.

Node.js 22.18+ is required. Nothing in this package is needed to use Inheritance Engine. If you stop the worker,
the engine works exactly as before.

## Tests

`npm test` runs the rule tests, plus the real workflow on Temporal's time-skipping test server against a scripted fake
engine. The workflow tests cover waiting for the model to return, re-running a failed restore, stopping at the attempt
limit, and skipping partners that aren't configured. The first run downloads Temporal's test server binary.
