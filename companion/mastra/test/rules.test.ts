import { describe, expect, it } from 'vitest';
import { proposedAction, requestAllowed } from '../src/server.ts';
import { allowedKey, ContentStripper, scrubSentryEvent } from '../src/telemetry.ts';
import { summarizeDetail, summarizeIssues } from '../src/tools.ts';
import { mentionedFiles } from '../src/workflows/lead-answer.ts';
import { validateClassification } from '../src/workflows/triage.ts';
import { isLoopback } from '../src/config.ts';

const repairs = [{ id: 'rep_45f882c30058485b', issue: 'getargspec', commit: 'cd83239f22aa11bb22cc33dd44ee55ff66778899', files: ['fn/func.py'] }];

describe('triage citation rule (engine has the last word)', () => {
  it('keeps "addressed" only when an existing repair or commit is cited, even loosely', () => {
    const c = validateClassification({ verdict: 'addressed_by_successor', reason: 'fixed', cited_repairs: ['see rep_45f882c30058485b'], cited_commits: ['commit cd83239f22'], confidence: 'high' }, repairs);
    expect(c.verdict).toBe('addressed_by_successor');
    expect(c.cited_repairs).toEqual(['rep_45f882c30058485b']);
    expect(c.cited_commits).toEqual(['cd83239f22']);
  });
  it('accepts an id mentioned only in the reason', () => {
    const c = validateClassification({ verdict: 'addressed_by_successor', reason: 'Repair rep_45f882c30058485b changed fn/func.py', cited_repairs: [], cited_commits: [], confidence: 'medium' }, repairs);
    expect(c.verdict).toBe('addressed_by_successor');
  });
  it('downgrades "addressed" with no real citation and drops invented ids', () => {
    const c = validateClassification({ verdict: 'addressed_by_successor', reason: 'the fix in commit deadbeef1 solved it', cited_repairs: ['rep_ffffffffffff'], cited_commits: [], confidence: 'high' }, repairs);
    expect(c.verdict).toBe('needs_human');
    expect(c.cited_repairs).toEqual([]);
    expect(c.cited_commits).toEqual([]);
    expect(c.engine_check).toMatch(/downgraded/);
  });
  it('leaves other verdicts alone', () => {
    expect(validateClassification({ verdict: 'still_relevant', reason: 'feature request', cited_repairs: [], cited_commits: [], confidence: 'high' }, repairs).verdict).toBe('still_relevant');
  });
});

describe('telemetry never carries content', () => {
  it('allowlists only metadata keys', () => {
    for (const k of ['gen_ai.usage.input_tokens', 'gen_ai.request.model', 'gen_ai.agent.name', 'sentry.op', 'ie.inheritance_id']) expect(allowedKey(k)).toBe(true);
    for (const k of ['input', 'output', 'gen_ai.agent.prompt', 'gen_ai.response.text', 'gen_ai.response.tool_calls', 'gen_ai.request.messages', 'gen_ai.tool.input']) expect(allowedKey(k)).toBe(false);
  });
  it('scrubs Sentry transactions down to metadata', () => {
    const secretSource = 'def curried(func):\n    spec = getargspec(f)  # AKIAABCDEFGHIJKLMNOP';
    const event = scrubSentryEvent({
      contexts: { trace: { data: { 'gen_ai.agent.name': 'Lead Engineer', input: secretSource } } },
      spans: [{ data: { 'gen_ai.usage.output_tokens': 268, 'gen_ai.response.text': secretSource, output: secretSource, 'gen_ai.request.model': 'google/gemma-4-e4b' } }],
      extra: { file: secretSource },
      breadcrumbs: [{ message: secretSource }],
      request: { data: secretSource },
    });
    const json = JSON.stringify(event);
    expect(json).not.toContain('curried');
    expect(json).not.toContain('AKIA');
    expect(json).toContain('gen_ai.usage.output_tokens');
    expect(json).toContain('google/gemma-4-e4b');
  });
  it('strips inputs, outputs and prompts from Mastra spans before export', () => {
    const span: any = { input: 'source', output: 'reply', inputPreview: 'x', outputPreview: 'y', attributes: { prompt: 'p', instructions: 'i', model: 'm', usage: { inputTokens: 3 } }, errorInfo: { message: 'e'.repeat(500) } };
    new ContentStripper().process(span);
    expect(span.input).toBeUndefined();
    expect(span.output).toBeUndefined();
    expect(span.attributes.prompt).toBeUndefined();
    expect(span.attributes.instructions).toBeUndefined();
    expect(span.attributes.model).toBe('m');
    expect(span.attributes.usage.inputTokens).toBe(3);
    expect(span.errorInfo.message.length).toBe(160);
  });
});

describe('request guards', () => {
  it('accepts loopback, same-origin requests only', () => {
    expect(requestAllowed({ host: '127.0.0.1:7880' })).toBe(true);
    expect(requestAllowed({ host: 'localhost:7880' })).toBe(true);
    expect(requestAllowed({ host: 'evil.com' })).toBe(false);
    expect(requestAllowed({ host: '127.0.0.1:7880', origin: 'https://evil.com' })).toBe(false);
  });
  it('treats only loopback model endpoints as local', () => {
    expect(isLoopback('http://127.0.0.1:1234/v1')).toBe(true);
    expect(isLoopback('http://localhost:11434')).toBe(true);
    expect(isLoopback('https://api.example.com/v1')).toBe(false);
    expect(isLoopback('http://127.0.0.1.evil.com/v1')).toBe(false);
  });
  it('only surfaces known actions proposed through the tool', () => {
    expect(proposedAction([{ payload: { toolName: 'proposeAction', result: { proposed: 'verify' } } }])).toBe('verify');
    expect(proposedAction([{ payload: { toolName: 'proposeAction', result: { proposed: 'rm -rf /' } } }])).toBeNull();
    expect(proposedAction([{ payload: { toolName: 'listIssues', result: { proposed: 'verify' } } }])).toBeNull();
  });
});

describe('context shaping', () => {
  it('finds file mentions with line ranges', () => {
    expect(mentionedFiles('show me fn/func.py lines 70-90 and README.rst')).toEqual([
      { path: 'fn/func.py', start: 70, end: 90 },
      { path: 'README.rst', start: 1, end: 120 },
    ]);
    expect(mentionedFiles('what is broken?')).toEqual([]);
  });
  it('flags human-reopened issues and reviewer disagreements', () => {
    const d = { full_name: 'o/r', stage: 'restoring', report: { reviewer_challenges: [{ target: 'c1', verdict: 'disagree', reason: 'r' }, { target: 'c2', verdict: 'agree', reason: 'ok' }] }, issues: [{ id: 'i1', title: 't', status: 'open', reopened_note: 'human saw a bug' }] };
    expect(summarizeDetail(d).reviewer_disagreements).toHaveLength(1);
    expect(summarizeIssues(d)[0].reopened_by_human).toBe(true);
    expect(summarizeDetail(d).upstream_issues).toBeNull();
    const withTriage = { ...d, upstream_triage: { total_open_upstream: 24, counts: { addressed_by_successor: 1 }, triaged: [{ number: 91, title: 'Iterable', verdict: 'addressed_by_successor', cited_repairs: ['rep_1'], cited_commits: [] }] } };
    expect(summarizeDetail(withTriage).upstream_issues?.triaged[0]).toEqual({ number: 91, title: 'Iterable', verdict: 'addressed_by_successor', cites: ['rep_1'] });
  });
});
