// Minimal client for the Inheritance Engine HTTP API (loopback). The companion only reads from the engine;
// every action that changes state stays a button the user presses in the engine's UI.

export interface EngineClient {
  detail(id: string): Promise<any>;
  file(id: string, path: string, side?: 'successor' | 'original'): Promise<{ path: string; content: string; lines: number }>;
  diff(id: string): Promise<{ base: string; head: string; diff: string }>;
}

export function engineClient(baseUrl: string, fetchImpl: typeof fetch = fetch): EngineClient {
  const get = async (path: string) => {
    const r = await fetchImpl(`${baseUrl}${path}`, { headers: { accept: 'application/json' } });
    const body = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(`engine ${path} -> ${r.status}: ${body?.error ?? 'error'}`);
    return body;
  };
  const enc = encodeURIComponent;
  return {
    detail: (id) => get(`/api/inheritances/${enc(id)}`),
    file: (id, path, side = 'successor') => get(`/api/inheritances/${enc(id)}/file?${new URLSearchParams({ path, side })}`),
    diff: (id) => get(`/api/inheritances/${enc(id)}/diff`),
  };
}
