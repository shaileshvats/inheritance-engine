// Loopback client for the Inheritance Engine HTTP API, plus the pure rules the activities apply to its answers.
// Only ids, job states and step summaries cross into Temporal's history; never repository content.

export type Need = 'engine' | 'docker' | 'model' | 'mastra' | 'web';

export interface Readiness {
  /** Services this workflow step waits for: configured but not reachable right now. */
  waiting: string[];
  /** Optional services that are not configured at all, so the step that needs them is skipped. */
  absent: Need[];
}

export interface JobOutcome {
  jobId: string;
  kind: string;
  status: 'completed' | 'failed' | 'cancelled';
  error: string | null;
  steps: { name: string; status: string; detail: string | null }[];
}

export interface RecordSummary {
  recordVersion: string | number | null;
  stage: string;
  attempts: { id: string; status: string }[];
  successorCommits: number;
}

/** Which of `needs` block or skip a step, judged from GET /api/status. */
export function readiness(status: any, needs: Need[]): Readiness {
  const waiting: string[] = [];
  const absent: Need[] = [];
  for (const need of needs) {
    switch (need) {
      case 'engine':
        break; // a status answer means the engine is up
      case 'docker':
        if (!status?.docker?.available) waiting.push(`Docker (${status?.docker?.detail ?? 'not available'})`);
        break;
      case 'model':
        if (!status?.model?.reachable) waiting.push(`model ${status?.model?.model ?? ''} at ${status?.model?.endpoint ?? '?'}`.replace('  ', ' '));
        break;
      case 'mastra':
        if (!status?.mastra?.configured) absent.push('mastra');
        else if (!status.mastra.reachable) waiting.push(`Mastra companion at ${status.mastra.url}`);
        break;
      case 'web':
        if (!status?.web_search?.enabled) absent.push('web');
        break;
    }
  }
  return { waiting, absent };
}

/** The job of `kind` this workflow should follow: the running one if any, else none (start a new one). */
export function activeJobOfKind(detail: any, kind: string): string | null {
  const active = detail?.active_job;
  if (!active) return null;
  if (active.kind !== kind) throw new Error(`a ${active.kind} job is ${active.status} for this inheritance`);
  return active.id;
}

/** Finished outcome of `jobId`, or null while it is still queued, running or paused by a person. */
export function jobOutcome(detail: any, jobId: string): JobOutcome | null {
  const job = (detail?.jobs ?? []).find((j: any) => j.id === jobId);
  if (!job) throw new Error(`job ${jobId} not found on inheritance ${detail?.id}`);
  if (!['completed', 'failed', 'cancelled'].includes(job.status)) return null;
  return {
    jobId: job.id,
    kind: job.kind,
    status: job.status,
    error: job.error ?? null,
    steps: (job.steps ?? []).map((s: any) => ({ name: s.name, status: s.status, detail: s.detail ?? null })),
  };
}

/**
 * An inheritance of `fullName` this workflow already created (on a retry after the response was lost), so a retried
 * activity never inherits the same repository twice.
 */
export function existingInheritance(list: any[], fullName: string, notBefore: string): string | null {
  const mine = list
    .filter((i) => i.full_name?.toLowerCase() === fullName.toLowerCase())
    .filter((i) => !String(i.successor_name ?? '').includes('(replay)'))
    .filter((i) => Date.parse(i.created_at) >= Date.parse(notBefore) - 1000);
  return mine.length ? mine[0].id : null;
}

export function summarizeRecord(detail: any, record: any): RecordSummary {
  const attempts = record?.restoration?.attempts ?? [];
  return {
    recordVersion: record?.record_version ?? null,
    stage: detail?.stage ?? 'unknown',
    attempts: attempts.map((a: any) => ({ id: a.id, status: a.status })),
    successorCommits: (record?.successor?.lineage ?? []).filter((c: any) => c.kind === 'successor').length,
  };
}

export function engineHttp(baseUrl: string, fetchImpl: typeof fetch = fetch) {
  const call = async (method: string, path: string, body?: unknown) => {
    const r = await fetchImpl(`${baseUrl}${path}`, {
      method,
      headers: { accept: 'application/json', ...(body === undefined ? {} : { 'content-type': 'application/json' }) },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const json = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(`engine ${method} ${path} -> ${r.status}: ${json?.error ?? 'error'}`);
    return json;
  };
  const enc = encodeURIComponent;
  return {
    status: () => call('GET', '/api/status'),
    list: () => call('GET', '/api/inheritances'),
    detail: (id: string) => call('GET', `/api/inheritances/${enc(id)}`),
    record: (id: string) => call('GET', `/api/inheritances/${enc(id)}/record`),
    create: (fullName: string, executionMode: string) => call('POST', '/api/inheritances', { full_name: fullName, execution_mode: executionMode }),
    start: (id: string, kind: string, body: unknown) => call('POST', `/api/inheritances/${enc(id)}/${kind}`, body),
  };
}
