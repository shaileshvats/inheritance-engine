// Companion configuration. Mirrors the engine's environment variables so one `.env` drives both.

export interface CompanionConfig {
  engineUrl: string;
  host: string;
  port: number;
  modelEndpoint: string;
  model: string;
  modelApiKey: string;
  githubToken?: string;
  sentryDsn?: string;
  sentryEnvironment: string;
}

const env = (k: string): string | undefined => {
  const v = process.env[k]?.trim();
  return v ? v : undefined;
};

export function loadConfig(): CompanionConfig {
  return {
    engineUrl: (env('IE_ENGINE_URL') ?? 'http://127.0.0.1:7878').replace(/\/$/, ''),
    host: env('IE_MASTRA_HOST') ?? '127.0.0.1',
    port: Number(env('IE_MASTRA_PORT') ?? 7880),
    modelEndpoint: (env('IE_MODEL_ENDPOINT') ?? 'http://127.0.0.1:1234/v1').replace(/\/$/, ''),
    model: env('IE_MASTRA_MODEL') ?? env('IE_MODEL') ?? 'google/gemma-4-e4b',
    // LM Studio ignores the key; OpenAI-compatible clients require a non-empty one.
    modelApiKey: env('IE_MODEL_API_KEY') ?? 'local',
    githubToken: env('GITHUB_TOKEN'),
    sentryDsn: env('SENTRY_DSN'),
    sentryEnvironment: env('SENTRY_ENVIRONMENT') ?? 'local',
  };
}

/** The open-weight model, served locally through an OpenAI-compatible endpoint (LM Studio by default). */
export function gemmaModel(cfg: CompanionConfig) {
  return {
    providerId: 'local',
    modelId: cfg.model,
    url: cfg.modelEndpoint,
    apiKey: cfg.modelApiKey,
    api: 'chat' as const,
  };
}

/** Same rule as the engine's `local_only` default: only loopback model endpoints receive repository content. */
export function isLoopback(url: string): boolean {
  try {
    const h = new URL(url).hostname;
    return h === '127.0.0.1' || h === 'localhost' || h === '[::1]' || h === '::1';
  } catch {
    return false;
  }
}
