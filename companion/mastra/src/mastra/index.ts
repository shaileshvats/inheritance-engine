// The Mastra instance: Lead Engineer agent, Issue Triager agent and the upstream-issue triage workflow,
// all over a local Gemma model. `npm run dev` serves it in Mastra Studio; `src/server.ts` exposes a narrow API to the engine.
import { Mastra } from '@mastra/core';
import { Observability } from '@mastra/observability';
import { SentryExporter } from '@mastra/sentry';
import { makeNodeTransport } from '@sentry/node';
import { createGroundedLeadAgent, createLeadAgent } from '../agents/lead.ts';
import { createTriageAgent } from '../agents/triage.ts';
import { loadConfig } from '../config.ts';
import { engineClient } from '../engine.ts';
import { createMemory, memoryConfig, type Memory } from '../memory.ts';
import { ContentStripper, scrubEnvelope, scrubSentryError, scrubSentryEvent, scrubSentrySpan } from '../telemetry.ts';
import { buildLeadTools } from '../tools.ts';
import { createLeadAnswerWorkflow } from '../workflows/lead-answer.ts';
import { createTriageWorkflow } from '../workflows/triage.ts';

export function createMastra(cfg = loadConfig(), engine = engineClient(cfg.engineUrl), githubFetch: typeof fetch = fetch, memory: Memory | undefined = defaultMemory()) {
  const observability = cfg.sentryDsn
    ? new Observability({
        sensitiveDataFilter: true,
        configs: {
          sentry: {
            serviceName: 'inheritance-engine-mastra',
            spanOutputProcessors: [new ContentStripper()],
            exporters: [
              new SentryExporter({
                dsn: cfg.sentryDsn,
                environment: cfg.sentryEnvironment,
                tracesSampleRate: 1.0,
                options: {
                  sendDefaultPii: false,
                  // No automatic HTTP/console instrumentation: their spans and breadcrumbs carry URLs such as
                  // /file?path=… (repository file paths). Only Mastra's own agent/model/tool spans are sent.
                  defaultIntegrations: false,
                  beforeSendTransaction: (e) => scrubSentryEvent(e),
                  beforeSendSpan: (s) => scrubSentrySpan(s),
                  beforeSend: (e) => scrubSentryError(e),
                  // Every envelope passes the allowlist in the transport, whatever its item type.
                  transport: (o) => {
                    const t = makeNodeTransport(o);
                    return { ...t, send: (envelope) => t.send(scrubEnvelope(envelope)) };
                  },
                },
              }),
            ],
          },
        },
      })
    : undefined;
  return new Mastra({
    agents: {
      leadEngineer: createLeadAgent(cfg, engine, memory),
      leadEngineerGrounded: createGroundedLeadAgent(cfg),
      issueTriager: createTriageAgent(cfg),
    },
    workflows: {
      answerLeadQuestion: createLeadAnswerWorkflow(buildLeadTools(engine, memory)),
      triageUpstreamIssues: createTriageWorkflow(engine, cfg.githubToken, githubFetch),
    },
    ...(observability ? { observability } : {}),
  });
}

/** The restoration memory when IE_MEMORY_DATABASE_URL is set (Tiger Data / Postgres with pgvector); otherwise none. */
function defaultMemory(): Memory | undefined {
  const mc = memoryConfig();
  return mc ? createMemory(mc) : undefined;
}

export const mastra = createMastra();
