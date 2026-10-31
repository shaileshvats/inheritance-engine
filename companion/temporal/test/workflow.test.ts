import { TestWorkflowEnvironment } from '@temporalio/testing';
import { Worker } from '@temporalio/worker';
import { fileURLToPath } from 'node:url';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { createActivities } from '../src/activities.ts';
import { config, isLoopback } from '../src/config.ts';
import { activeJobOfKind, existingInheritance, jobOutcome, readiness } from '../src/engine.ts';
import { parseArgs } from '../src/start.ts';
import type { InheritResult } from '../src/workflows.ts';

const up = {
  docker: { available: true },
  model: { reachable: true, model: 'google/gemma-4-e4b', endpoint: 'http://127.0.0.1:1234/v1' },
  mastra: { configured: true, reachable: true, url: 'http://127.0.0.1:7880' },
  web_search: { enabled: true },
};

describe('readiness rules', () => {
  it('waits for configured services that are down and skips ones that are not configured', () => {
    const r = readiness({ ...up, model: { ...up.model, reachable: false }, mastra: { configured: false }, web_search: { enabled: false } }, ['model', 'mastra', 'web', 'docker']);
    expect(r.waiting).toEqual(['model google/gemma-4-e4b at http://127.0.0.1:1234/v1']);
    expect(r.absent).toEqual(['mastra', 'web']);
  });
  it('waits for a configured companion that is unreachable', () => {
    expect(readiness({ ...up, mastra: { ...up.mastra, reachable: false } }, ['mastra']).waiting).toEqual(['Mastra companion at http://127.0.0.1:7880']);
  });
});

describe('job and inheritance rules', () => {
  const detail = {
    id: 'inh_1',
    active_job: { id: 'job_2', kind: 'restore', status: 'running' },
    jobs: [
      { id: 'job_1', kind: 'investigate', status: 'completed', error: null, steps: [{ name: 'plan', status: 'done', detail: 'ok' }] },
      { id: 'job_2', kind: 'restore', status: 'running', steps: [] },
    ],
  };
  it('follows a running job of the same kind instead of starting another', () => {
    expect(activeJobOfKind(detail, 'restore')).toBe('job_2');
    expect(() => activeJobOfKind(detail, 'research')).toThrow(/restore job is running/);
    expect(activeJobOfKind({ active_job: null }, 'restore')).toBeNull();
  });
  it('reports a job only once it has finished', () => {
    expect(jobOutcome(detail, 'job_2')).toBeNull();
    expect(jobOutcome(detail, 'job_1')).toMatchObject({ status: 'completed', steps: [{ name: 'plan', detail: 'ok' }] });
    expect(() => jobOutcome(detail, 'job_9')).toThrow(/not found/);
  });
  it('reuses an inheritance this workflow already created, but never a replay or an older one', () => {
    const list = [
      { id: 'inh_old', full_name: 'a/b', successor_name: 'b-successor', created_at: '2026-10-30T10:00:00Z' },
      { id: 'inh_rep', full_name: 'a/b', successor_name: 'b-successor (replay)', created_at: '2026-10-31T12:00:05Z' },
      { id: 'inh_new', full_name: 'A/B', successor_name: 'b-successor', created_at: '2026-10-31T12:00:03Z' },
    ];
    expect(existingInheritance(list, 'a/b', '2026-10-31T12:00:00Z')).toBe('inh_new');
    expect(existingInheritance(list.slice(0, 2), 'a/b', '2026-10-31T12:00:00Z')).toBeNull();
  });
});

describe('configuration and CLI', () => {
  it('keeps the engine and Temporal on loopback unless remote Temporal is explicitly allowed', () => {
    expect(isLoopback('http://127.0.0.1:7878')).toBe(true);
    expect(isLoopback('temporal.example.com:7233')).toBe(false);
    expect(() => config({ IE_ENGINE_URL: 'http://10.0.0.5:7878' })).toThrow(/loopback/);
    expect(() => config({ TEMPORAL_ADDRESS: 'x.tmprl.cloud:7233' })).toThrow(/not loopback/);
    expect(config({ TEMPORAL_ADDRESS: 'x.tmprl.cloud:7233', IE_TEMPORAL_ALLOW_REMOTE: '1' }).temporalAddress).toBe('x.tmprl.cloud:7233');
  });
  it('parses the inherit command', () => {
    expect(parseArgs(['kachayev/fn.py', '--restore', 'none', '--no-triage', '--wait'])).toEqual({ input: { repository: 'kachayev/fn.py', restore: false, triage: false }, wait: true });
    expect(() => parseArgs(['--wait'])).toThrow(/owner\/name/);
  });
});

// A fake engine behind the real activities and the real workflow, on Temporal's time-skipping test server.
function fakeEngine(script: { statusDown?: number; failRestoreTimes?: number; web?: boolean; mastra?: boolean }) {
  let statusCalls = 0;
  let restoreFailures = script.failRestoreTimes ?? 0;
  let n = 0;
  const inh: any = { id: 'inh_t', full_name: 'a/b', successor_name: 'b-successor', created_at: new Date(0).toISOString(), stage: 'investigated', report: null, active_job: null, jobs: [] };
  const started: string[] = [];
  const start = (kind: string) => {
    const job: any = { id: `job_${++n}`, kind, status: 'running', error: null, steps: [{ name: kind, status: 'running', detail: null }] };
    inh.jobs.push(job);
    inh.active_job = job;
    started.push(kind);
    // Jobs finish on the next poll.
    setTimeout(() => {
      const fail = kind === 'restore' && restoreFailures-- > 0;
      job.status = fail ? 'failed' : 'completed';
      job.error = fail ? 'model call timed out' : null;
      job.steps[0] = { name: kind, status: fail ? 'failed' : 'done', detail: fail ? null : `${kind} ok` };
      if (kind === 'investigate') inh.report = {};
      inh.active_job = null;
    }, 10);
    return job;
  };
  const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
  const fetchImpl = (async (url: string, init?: RequestInit) => {
    const path = new URL(url).pathname;
    const method = init?.method ?? 'GET';
    if (path === '/api/status') {
      statusCalls++;
      const down = statusCalls <= (script.statusDown ?? 0);
      return json({ ...up, model: { ...up.model, reachable: !down }, web_search: { enabled: script.web ?? true }, mastra: script.mastra === false ? { configured: false } : up.mastra });
    }
    if (path === '/api/inheritances' && method === 'GET') return json(started.length ? [inh] : []);
    if (path === '/api/inheritances' && method === 'POST') {
      inh.created_at = new Date().toISOString();
      start('investigate');
      return json(inh);
    }
    if (path === '/api/inheritances/inh_t') return json(inh);
    if (path === '/api/inheritances/inh_t/record') return json({ record_version: 2, restoration: { attempts: [{ id: 'rep_1', status: 'kept' }] }, successor: { lineage: [{ kind: 'original' }, { kind: 'successor' }] } });
    const m = path.match(/^\/api\/inheritances\/inh_t\/(restore|research|triage|investigate)$/);
    if (m && method === 'POST') return json(start(m[1]));
    return json({ error: `unexpected ${method} ${path}` }, 404);
  }) as typeof fetch;
  return { fetchImpl, started, statusCalls: () => statusCalls };
}

describe('inheritAndRestore workflow (Temporal time-skipping test server)', () => {
  let env: TestWorkflowEnvironment;
  beforeAll(async () => {
    env = await TestWorkflowEnvironment.createTimeSkipping();
  }, 120_000);
  afterAll(async () => {
    await env?.teardown();
  });

  async function run(engine: ReturnType<typeof fakeEngine>, input: object): Promise<InheritResult> {
    const taskQueue = `test-${Math.random()}`;
    const worker = await Worker.create({
      connection: env.nativeConnection,
      taskQueue,
      workflowsPath: fileURLToPath(new URL('../src/workflows.ts', import.meta.url)),
      activities: createActivities('http://127.0.0.1:7878', engine.fetchImpl),
    });
    return worker.runUntil(env.client.workflow.execute('inheritAndRestore', { taskQueue, workflowId: taskQueue, args: [input] }));
  }

  it('waits for the model to come back, then runs every stage in order', async () => {
    const engine = fakeEngine({ statusDown: 2 });
    const result = await run(engine, { repository: 'a/b' });
    expect(engine.started).toEqual(['investigate', 'restore', 'research', 'triage']);
    expect(engine.statusCalls()).toBeGreaterThan(2);
    expect(result.stages.map((s) => [s.stage, s.status])).toEqual([
      ['investigate', 'done'], ['restore', 'done'], ['web research', 'done'], ['upstream triage', 'done'], ['inheritance record', 'done'],
    ]);
    expect(result.record).toEqual({ recordVersion: 2, stage: 'investigated', attempts: [{ id: 'rep_1', status: 'kept' }], successorCommits: 1 });
  }, 120_000);

  it('re-runs a restore job that failed, and records both attempts', async () => {
    const engine = fakeEngine({ failRestoreTimes: 1 });
    const result = await run(engine, { repository: 'a/b' });
    const restore = result.stages.find((s) => s.stage === 'restore')!;
    expect(restore).toMatchObject({ status: 'done', attempts: 2 });
    expect(restore.jobIds).toHaveLength(2);
  }, 120_000);

  it('gives up after the attempt limit and still exports the record', async () => {
    const engine = fakeEngine({ failRestoreTimes: 5 });
    const result = await run(engine, { repository: 'a/b', maxJobAttempts: 2 });
    expect(result.stages.find((s) => s.stage === 'restore')).toMatchObject({ status: 'failed', attempts: 2, detail: 'model call timed out' });
    expect(result.stages.at(-1)).toMatchObject({ stage: 'inheritance record', status: 'done' });
  }, 120_000);

  it('skips optional partners that are not configured instead of failing', async () => {
    const engine = fakeEngine({ web: false, mastra: false });
    const result = await run(engine, { repository: 'a/b', restore: false });
    expect(engine.started).toEqual(['investigate']);
    expect(result.stages.map((s) => s.status)).toEqual(['done', 'skipped', 'skipped', 'skipped', 'done']);
  }, 120_000);
});
