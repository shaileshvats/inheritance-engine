// Temporal activities: every side effect of the workflow, all of them calls to the engine's loopback HTTP API.
// Failures that should be waited out (engine restarting, model or companion down) are thrown as retryable, so
// Temporal retries them with backoff; the workflow decides what to do with jobs that finished but failed.
import { Context } from '@temporalio/activity';
import { ApplicationFailure } from '@temporalio/common';
import { activeJobOfKind, engineHttp, existingInheritance, jobOutcome, readiness, summarizeRecord, type JobOutcome, type Need, type RecordSummary } from './engine.ts';

const POLL_MS = 3000;

export function createActivities(engineUrl: string, fetchImpl: typeof fetch = fetch) {
  const engine = engineHttp(engineUrl, fetchImpl);

  return {
    /** Wait (by failing and being retried) until every configured service in `needs` is reachable. Returns the
     * optional services that are not configured at all, so the workflow can skip the steps that need them. */
    async requireReady(needs: Need[]): Promise<Need[]> {
      const r = readiness(await engine.status(), needs);
      if (r.waiting.length) throw ApplicationFailure.retryable(`waiting for ${r.waiting.join(', ')}`, 'NotReady');
      return r.absent;
    },

    /** Inherit `fullName` (the engine starts the investigation itself). Idempotent across retries. */
    async createInheritance(fullName: string, executionMode: string, notBefore: string): Promise<{ inheritanceId: string; jobId: string | null }> {
      let id = existingInheritance(await engine.list(), fullName, notBefore);
      if (!id) id = (await engine.create(fullName, executionMode)).id as string;
      const detail = await engine.detail(id);
      return { inheritanceId: id, jobId: detail.active_job?.kind === 'investigate' ? detail.active_job.id : null };
    },

    async hasReport(inheritanceId: string): Promise<boolean> {
      const detail = await engine.detail(inheritanceId);
      return Boolean(detail.report);
    },

    /** Start a `kind` job, or follow the one already running (so a retried activity never starts two). */
    async startJob(inheritanceId: string, kind: string, body: unknown): Promise<string> {
      const running = activeJobOfKind(await engine.detail(inheritanceId), kind);
      if (running) return running;
      return (await engine.start(inheritanceId, kind, body)).id as string;
    },

    /** Follow a job until it finishes, heartbeating its current step. If the engine restarts, the poll fails, the
     * activity is retried, and it picks up the same job (the engine resumes its own checkpointed jobs). */
    async awaitJob(inheritanceId: string, jobId: string): Promise<JobOutcome> {
      for (;;) {
        const detail = await engine.detail(inheritanceId);
        const done = jobOutcome(detail, jobId);
        if (done) return done;
        const job = detail.active_job;
        Context.current().heartbeat({ jobId, status: job?.status, step: job?.step ?? null });
        await Context.current().sleep(POLL_MS);
      }
    },

    async recordSummary(inheritanceId: string): Promise<RecordSummary> {
      const [detail, record] = await Promise.all([engine.detail(inheritanceId), engine.record(inheritanceId)]);
      return summarizeRecord(detail, record);
    },
  };
}

export type Activities = ReturnType<typeof createActivities>;
