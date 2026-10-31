// Integration tests: the real Mastra workflows and HTTP server, run against a local OpenAI-compatible test fixture
// (canned model JSON, every request recorded), a stub engine, and a stub GitHub API. Nothing here is demo evidence.
import { createServer, type Server } from 'node:http';
import type { AddressInfo } from 'node:net';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import type { CompanionConfig } from '../src/config.ts';
import type { EngineClient } from '../src/engine.ts';
import { createMastra } from '../src/mastra/index.ts';
import { createCompanionServer, detectToolMode } from '../src/server.ts';

const INH = 'inh_2583373d06584b58';
const SOURCE = 'def curried(func):\n    spec = signature(f)\n    if count == len(signature(f).parameters) - len(args):\n        return func(*args)';
const requests: any[] = [];
let modelServer: Server;
let modelUrl = '';

function reply(content: string) {
  return { id: 'x', object: 'chat.completion', created: 0, model: 'fixture', choices: [{ index: 0, finish_reason: 'stop', message: { role: 'assistant', content } }], usage: { prompt_tokens: 50, completion_tokens: 10, total_tokens: 60 } };
}

beforeAll(async () => {
  modelServer = createServer(async (req, res) => {
    let body = '';
    for await (const c of req) body += c;
    const json = JSON.parse(body || '{}');
    requests.push(json);
    const system = (json.messages ?? []).find((m: any) => m.role === 'system')?.content ?? '';
    const user = JSON.stringify(json.messages ?? []);
    let content: string;
    if (system.includes('You triage')) {
      content = user.includes('UPSTREAM ISSUE #91')
        ? JSON.stringify({ verdict: 'addressed_by_successor', reason: 'Fixed by repair rep_aaaaaaaaaaaa1111.', cited_repairs: ['rep_aaaaaaaaaaaa1111'], cited_commits: ['commit 0dd934f6db'], confidence: 'high' })
        : JSON.stringify({ verdict: 'addressed_by_successor', reason: 'probably fixed', cited_repairs: ['rep_does_not_exist'], cited_commits: [], confidence: 'low' });
    } else {
      content = JSON.stringify({ reply: 'Tests pass, but issue i1 reopened by a human is unresolved (fn/func.py:3).', proposed_action: 'restore_essential' });
    }
    res.writeHead(200, { 'content-type': 'application/json' });
    res.end(JSON.stringify(reply(content)));
  });
  await new Promise<void>((r) => modelServer.listen(0, '127.0.0.1', r));
  modelUrl = `http://127.0.0.1:${(modelServer.address() as AddressInfo).port}/v1`;
});
afterAll(() => modelServer.close());

const engine: EngineClient = {
  list: async () => [],
  detail: async () => ({
    full_name: 'kachayev/fn.py',
    stage: 'restoring',
    original_commit: 'df53f5d6bf0e',
    report: { purpose: { text: 'Functional programming in Python', status: 'verified' }, reviewer_challenges: [] },
    last_verification: { label: 'after restoration · tests', status: 'passed', summary: { passed: 126, failed: 0 }, command: 'python -m pytest', environment: 'docker' },
    counts: { issues: 1 },
    issues: [{ id: 'i1', title: 'getargspec', status: 'rejected', reopened_note: 'variadic functions changed', diagnosis: 'd' }],
    repairs: [{ id: 'rep_aaaaaaaaaaaa1111', issue_id: 'i1', status: 'applied', commit: '0dd934f6db1234567890abcdef', files: ['fn/iters.py'] }],
  }),
  file: async (_id, path) => ({ path, content: SOURCE, lines: 4 }),
  diff: async () => ({ base: 'a', head: 'b', diff: '-from collections import Iterable\n+from collections.abc import Iterable' }),
};

const github = (async () =>
  new Response(
    JSON.stringify([
      { number: 91, title: 'ImportError: cannot import name Iterable from collections', body: 'Python 3.10 breaks the import', html_url: 'https://github.com/kachayev/fn.py/issues/91', created_at: '2021-11-01T00:00:00Z', comments: 3 },
      { number: 37, title: 'Required memoise function', body: 'please add', html_url: 'https://github.com/kachayev/fn.py/issues/37', created_at: '2014-01-01T00:00:00Z', comments: 1 },
      { number: 99, title: 'a pull request', body: '', html_url: 'u', created_at: '2020-01-01T00:00:00Z', comments: 0, pull_request: {} },
    ]),
    { status: 200 },
  )) as typeof fetch;

function cfg(): CompanionConfig {
  return { engineUrl: 'http://127.0.0.1:1', host: '127.0.0.1', port: 0, modelEndpoint: modelUrl, model: 'fixture-model', modelApiKey: 'k', sentryEnvironment: 'test' };
}

describe('triage-upstream-issues workflow', () => {
  it('classifies real-shaped issues, skips PRs, and enforces citations', async () => {
    const mastra = createMastra(cfg(), engine, github);
    const run = await mastra.getWorkflow('triageUpstreamIssues').createRun();
    const result: any = await run.start({ inputData: { inheritanceId: INH, maxIssues: 5 } });
    expect(result.status).toBe('success');
    const out = result.result;
    expect(out.total_open_upstream).toBe(2);
    const byNum = Object.fromEntries(out.triaged.map((t: any) => [t.number, t]));
    expect(byNum[91].verdict).toBe('addressed_by_successor');
    expect(byNum[91].cited_repairs).toEqual(['rep_aaaaaaaaaaaa1111']);
    expect(byNum[91].cited_commits).toEqual(['0dd934f6db']);
    expect(byNum[37].verdict).toBe('needs_human');
    expect(byNum[37].engine_check).toMatch(/downgraded/);
    expect(out.counts.addressed_by_successor).toBe(1);
    // Upstream issue text reached the model only inside UNTRUSTED blocks.
    const sent = JSON.stringify(requests.filter((r) => JSON.stringify(r).includes('You triage')));
    expect(sent).toMatch(/<<<UNTRUSTED [0-9a-f]+ UPSTREAM ISSUE #91>>>/);
  });
});

describe('answer-lead-question workflow (grounded mode)', () => {
  it('calls the engine tools, wraps source as untrusted, and sends no tools to the model', async () => {
    requests.length = 0;
    const mastra = createMastra(cfg(), engine, github);
    const run = await mastra.getWorkflow('answerLeadQuestion').createRun();
    const result: any = await run.start({ inputData: { inheritanceId: INH, question: 'what is unresolved in fn/func.py lines 1-4?', history: [] } });
    expect(result.status).toBe('success');
    expect(result.result.toolsUsed).toEqual(['get-project-overview', 'list-issues', 'list-repairs', 'read-source-file']);
    expect(result.result.proposed_action).toBe('restore_essential');
    const req = requests.at(-1);
    expect(req.tools).toBeUndefined();
    expect(JSON.stringify(req)).toMatch(/<<<UNTRUSTED [0-9a-f]+ FILE fn\/func.py \(successor\) lines 1-4>>>/);
    expect(JSON.stringify(req)).toContain('reopened_by_human');
  });
});

describe('companion HTTP server', () => {
  let server: Server;
  let base = '';
  beforeAll(async () => {
    server = createCompanionServer(createMastra(cfg(), engine, github), cfg(), { mode: 'grounded', reason: 'test' });
    await new Promise<void>((r) => server.listen(0, '127.0.0.1', r));
    base = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
  });
  afterAll(() => server.close());

  it('answers through the grounded workflow and reports tools used', async () => {
    const r = await fetch(`${base}/lead/chat`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ inheritanceId: INH, message: 'status?' }) });
    const body: any = await r.json();
    expect(r.status).toBe(200);
    expect(body.mode).toBe('grounded');
    expect(body.proposed_action).toBe('restore_essential');
    expect(body.tool_calls.map((t: any) => t.tool)).toContain('get-project-overview');
    expect(body.usage.input_tokens).toBe(50);
  });

  it('rejects malformed ids and cross-origin requests', async () => {
    const bad = await fetch(`${base}/lead/chat`, { method: 'POST', body: JSON.stringify({ inheritanceId: '../../etc', message: 'x' }) });
    expect(bad.status).toBe(400);
    const cross = await fetch(`${base}/triage`, { method: 'POST', headers: { origin: 'https://evil.com' }, body: JSON.stringify({ inheritanceId: INH }) });
    expect(cross.status).toBe(403);
  });
});

describe('tool-mode detection', () => {
  it('falls back to grounded mode when the runtime rejects tools', async () => {
    const rejecting = (async () => new Response('{"error":"Error rendering prompt with jinja template"}', { status: 400 })) as typeof fetch;
    expect((await detectToolMode(cfg(), rejecting)).mode).toBe('grounded');
    const accepting = (async () => new Response('{}', { status: 200 })) as typeof fetch;
    expect((await detectToolMode(cfg(), accepting)).mode).toBe('native');
  });
});
