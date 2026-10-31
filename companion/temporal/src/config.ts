// Settings for the Temporal companion. Both the engine and the Temporal service default to loopback; a remote Temporal
// service is refused unless IE_TEMPORAL_ALLOW_REMOTE=1, because workflow history holds job and step summaries.
export const TASK_QUEUE = 'inheritance-engine';

export function isLoopback(hostOrUrl: string): boolean {
  const host = hostOrUrl.includes('://') ? new URL(hostOrUrl).hostname : hostOrUrl.replace(/:\d+$/, '');
  return ['127.0.0.1', 'localhost', '::1', '[::1]'].includes(host);
}

export function config(env: NodeJS.ProcessEnv = process.env) {
  const engineUrl = (env.IE_ENGINE_URL ?? 'http://127.0.0.1:7878').replace(/\/$/, '');
  const temporalAddress = env.TEMPORAL_ADDRESS ?? '127.0.0.1:7233';
  const namespace = env.TEMPORAL_NAMESPACE ?? 'default';
  if (!isLoopback(engineUrl)) throw new Error(`IE_ENGINE_URL must be a loopback address, got ${engineUrl}`);
  if (!isLoopback(temporalAddress) && env.IE_TEMPORAL_ALLOW_REMOTE !== '1') {
    throw new Error(`TEMPORAL_ADDRESS ${temporalAddress} is not loopback; set IE_TEMPORAL_ALLOW_REMOTE=1 to allow it`);
  }
  return { engineUrl, temporalAddress, namespace };
}
