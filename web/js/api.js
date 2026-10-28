// Thin client for the control-plane API (docs/api/README.md).

export class ApiError extends Error {
  constructor(message, status) { super(message); this.status = status; }
}

async function request(method, path, body) {
  const init = { method, headers: { Accept: 'application/json' } };
  if (body !== undefined) {
    init.headers['Content-Type'] = 'application/json';
    init.body = JSON.stringify(body);
  }
  let res;
  try { res = await fetch(path, init); }
  catch (e) { throw new ApiError('The Inheritance Engine server is not reachable. Is it still running?', 0); }
  const text = await res.text();
  let data = null;
  if (text) { try { data = JSON.parse(text); } catch { data = { error: text.slice(0, 500) }; } }
  if (!res.ok) throw new ApiError((data && data.error) || `${res.status} ${res.statusText}`, res.status);
  return data;
}

const enc = encodeURIComponent;

export const api = {
  status: () => request('GET', '/api/status'),

  hunt: (q, mode, lang) => {
    const p = new URLSearchParams({ q, mode });
    if (lang) p.set('lang', lang);
    return request('GET', `/api/hunt?${p}`);
  },
  explain: (fullName) => request('POST', '/api/hunt/explain', { full_name: fullName }),

  list: () => request('GET', '/api/inheritances'),
  create: (fullName, executionMode) => request('POST', '/api/inheritances', { full_name: fullName, execution_mode: executionMode }),
  get: (id) => request('GET', `/api/inheritances/${enc(id)}`),

  investigate: (id) => request('POST', `/api/inheritances/${enc(id)}/investigate`),
  restore: (id, mode, issueIds) => request('POST', `/api/inheritances/${enc(id)}/restore`, issueIds ? { mode, issue_ids: issueIds } : { mode }),
  verify: (id, command) => request('POST', `/api/inheritances/${enc(id)}/verify`, command ? { command } : {}),
  setExecutionMode: (id, mode) => request('POST', `/api/inheritances/${enc(id)}/execution-mode`, { mode }),
  renameSuccessor: (id, name) => request('PATCH', `/api/inheritances/${enc(id)}/successor`, { name }),

  triage: (id, maxIssues) => request('POST', `/api/inheritances/${enc(id)}/triage`, maxIssues ? { max_issues: maxIssues } : {}),
  reopenIssue: (id, issueId, note) => request('POST', `/api/inheritances/${enc(id)}/issues/${enc(issueId)}/reopen`, { note }),

  jobAction: (jobId, action) => request('POST', `/api/jobs/${enc(jobId)}/${action}`),

  recordUrl: (id) => `/api/inheritances/${enc(id)}/record`,
  file: (id, path, side = 'successor') => request('GET', `/api/inheritances/${enc(id)}/file?${new URLSearchParams({ path, side })}`),
  diff: (id) => request('GET', `/api/inheritances/${enc(id)}/diff`),

  chat: (id) => request('GET', `/api/inheritances/${enc(id)}/chat`),
  sendChat: (id, message) => request('POST', `/api/inheritances/${enc(id)}/chat`, { message }),

  calls: (id) => request('GET', `/api/inheritances/${enc(id)}/calls`),
  call: (callId) => request('GET', `/api/calls/${enc(callId)}`),
};

/** Run a suggested lead action. Returns the started Job (or null). */
export function runSuggestedAction(id, action) {
  switch (action) {
    case 'investigate': return api.investigate(id);
    case 'restore_essential': return api.restore(id, 'essential');
    case 'restore_all': return api.restore(id, 'fix_all');
    case 'verify': return api.verify(id);
    default: return Promise.resolve(null);
  }
}

export const ACTION_LABELS = {
  investigate: 'Run investigation',
  restore_essential: 'Fix essential issues',
  restore_all: 'Fix everything',
  verify: 'Run verification',
};

/** Repeatedly call `fn`; `interval()` decides the next delay. */
export class Poller {
  constructor(fn, interval) { this.fn = fn; this.interval = interval; this.timer = null; this.stopped = false; this.inFlight = false; this.again = false; }
  start() { this.stopped = false; this.tick(); return this; }
  async tick() {
    if (this.stopped) return;
    if (this.inFlight) { this.again = true; return; }
    clearTimeout(this.timer);
    this.inFlight = true;
    try { await this.fn(); } catch (e) { console.error('Refresh failed:', e); }
    this.inFlight = false;
    if (this.stopped) return;
    if (this.again) { this.again = false; this.tick(); return; }
    this.timer = setTimeout(() => this.tick(), this.interval());
  }
  /** Refresh immediately (coalesces with an in-flight refresh). */
  now() { this.tick(); }
  stop() { this.stopped = true; clearTimeout(this.timer); }
}

export const ACTIVE_JOB = new Set(['queued', 'running']);
