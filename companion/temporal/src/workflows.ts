// The durable inheritance workflow. It chains the engine's own jobs (investigate, restore, web research, Mastra
// triage), waits out services that are down instead of failing, re-runs a job that failed (a timed-out model call, a
// truncated answer) a bounded number of times, and records what happened at every stage. Workflow code is
// deterministic: all I/O is in activities.
import { defineQuery, proxyActivities, setHandler, sleep, workflowInfo } from '@temporalio/workflow';
import type { Activities } from './activities.ts';
import type { JobOutcome, Need, RecordSummary } from './engine.ts';

export interface InheritInput {
  /** owner/name of a public GitHub repository to inherit, or */
  repository?: string;
  /** an existing inheritance to continue. */
  inheritanceId?: string;
  executionMode?: 'docker' | 'native' | 'none';
  /** Restoration mode, or false to stop after the investigation. */
  restore?: 'essential' | 'fix_all' | false;
  research?: boolean;
  triage?: boolean;
  /** How many times a job that finished as failed is run in total. */
  maxJobAttempts?: number;
}

export interface Stage {
  stage: string;
  status: 'running' | 'waiting' | 'done' | 'failed' | 'skipped';
  attempts: number;
  jobIds: string[];
  detail: string;
}

export interface InheritResult {
  inheritanceId: string;
  stages: Stage[];
  record: RecordSummary | null;
}

export const progressQuery = defineQuery<{ inheritanceId: string | null; stages: Stage[] }>('progress');

// Waiting for a service: retried with backoff for up to six hours, then the workflow gives up on that stage.
const waits = proxyActivities<Activities>({
  startToCloseTimeout: '30 seconds',
  scheduleToCloseTimeout: '6 hours',
  retry: { initialInterval: '5 seconds', backoffCoefficient: 2, maximumInterval: '2 minutes' },
});

// Calls that change or follow engine state. awaitJob heartbeats; a missed heartbeat (worker or engine gone) means
// another worker picks it up and resumes following the same job.
const engine = proxyActivities<Activities>({
  startToCloseTimeout: '3 hours',
  heartbeatTimeout: '1 minute',
  retry: { initialInterval: '5 seconds', backoffCoefficient: 2, maximumInterval: '1 minute', maximumAttempts: 50 },
});

export async function inheritAndRestore(input: InheritInput): Promise<InheritResult> {
  const stages: Stage[] = [];
  let inheritanceId: string | null = input.inheritanceId ?? null;
  setHandler(progressQuery, () => ({ inheritanceId, stages }));

  const mode = input.executionMode ?? 'docker';
  const maxAttempts = Math.max(1, input.maxJobAttempts ?? 2);
  const execNeeds: Need[] = mode === 'docker' ? ['docker'] : [];

  const begin = (stage: string): Stage => {
    const s: Stage = { stage, status: 'running', attempts: 0, jobIds: [], detail: '' };
    stages.push(s);
    return s;
  };

  // Run one engine job to completion, re-running it if it finishes as failed.
  const runJob = async (stage: Stage, kind: string, body: unknown, needs: Need[], firstJobId?: string | null): Promise<JobOutcome | null> => {
    let outcome: JobOutcome | null = null;
    for (let attempt = 1; attempt <= maxAttempts; attempt++) {
      stage.status = 'waiting';
      await waits.requireReady(needs);
      stage.status = 'running';
      stage.attempts = attempt;
      const jobId = attempt === 1 && firstJobId ? firstJobId : await engine.startJob(inheritanceId!, kind, body);
      stage.jobIds.push(jobId);
      outcome = await engine.awaitJob(inheritanceId!, jobId);
      const last = outcome.steps.filter((s) => s.status !== 'pending').at(-1);
      stage.detail = outcome.error ?? last?.detail ?? outcome.status;
      if (outcome.status === 'completed') {
        stage.status = 'done';
        return outcome;
      }
      if (outcome.status === 'cancelled') break; // a person cancelled it in the engine: respect that
      if (attempt < maxAttempts) await sleep('30 seconds');
    }
    stage.status = 'failed';
    return outcome;
  };

  // 1. Inherit and investigate.
  const investigate = begin('investigate');
  if (!inheritanceId) {
    if (!input.repository) throw new Error('give a repository (owner/name) or an existing inheritanceId');
    await waits.requireReady(['engine', 'model', ...execNeeds]);
    const created = await engine.createInheritance(input.repository, mode, new Date(workflowInfo().startTime).toISOString());
    inheritanceId = created.inheritanceId;
    await runJob(investigate, 'investigate', {}, ['model', ...execNeeds], created.jobId);
  } else if (await engine.hasReport(inheritanceId)) {
    investigate.status = 'skipped';
    investigate.detail = 'already investigated';
  } else {
    await runJob(investigate, 'investigate', {}, ['model', ...execNeeds]);
  }
  if (investigate.status === 'failed') return { inheritanceId: inheritanceId!, stages, record: null };

  // 2. Restore.
  const restore = begin('restore');
  if (input.restore === false) {
    restore.status = 'skipped';
    restore.detail = 'not requested';
  } else {
    await runJob(restore, 'restore', { mode: input.restore ?? 'essential' }, ['model', ...execNeeds]);
  }

  // 3. Optional partners: skipped, not failed, when they are not configured.
  for (const [name, kind, need, wanted, body] of [
    ['web research', 'research', 'web', input.research ?? true, {}],
    ['upstream triage', 'triage', 'mastra', input.triage ?? true, { max_issues: 8 }],
  ] as const) {
    const s = begin(name);
    if (!wanted) {
      s.status = 'skipped';
      s.detail = 'not requested';
      continue;
    }
    const absent = await waits.requireReady([need]);
    if (absent.includes(need)) {
      s.status = 'skipped';
      s.detail = need === 'web' ? 'web search is not configured (SERPAPI_API_KEY)' : 'the Mastra companion is not configured (IE_MASTRA_URL)';
      continue;
    }
    await runJob(s, kind, body, ['model', need]);
  }

  // 4. The Inheritance Record, as the engine now exports it.
  const rec = begin('inheritance record');
  const record = await engine.recordSummary(inheritanceId!);
  rec.status = 'done';
  rec.detail = `record v${record.recordVersion}; stage ${record.stage}; ${record.attempts.length} repair attempt(s) on record`;
  return { inheritanceId: inheritanceId!, stages, record };
}
