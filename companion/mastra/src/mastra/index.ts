// The Mastra instance: Lead Engineer agent, Issue Triager agent and the upstream-issue triage workflow,
// all over a local Gemma model. `npm run dev` serves it in Mastra Studio; `src/server.ts` exposes a narrow API to the engine.
import { Mastra } from '@mastra/core';
import { Observability } from '@mastra/observability';
import { SentryExporter } from '@mastra/sentry';
import { createGroundedLeadAgent, createLeadAgent } from '../agents/lead.ts';
import { createTriageAgent } from '../agents/triage.ts';
import { loadConfig } from '../config.ts';
import { engineClient } from '../engine.ts';
import { ContentStripper, scrubSentryEvent } from '../telemetry.ts';
import { buildLeadTools } from '../tools.ts';
import { createLeadAnswerWorkflow } from '../workflows/lead-answer.ts';
import { createTriageWorkflow } from '../workflows/triage.ts';

export function createMastra(cfg = loadConfig(), engine = engineClient(cfg.engineUrl), githubFetch: typeof fetch = fetch) {
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
                options: { sendDefaultPii: false, beforeSendTransaction: (e) => scrubSentryEvent(e) },
              }),
            ],
          },
        },
      })
    : undefined;
  return new Mastra({
    agents: {
      leadEngineer: createLeadAgent(cfg, engine),
      leadEngineerGrounded: createGroundedLeadAgent(cfg),
      issueTriager: createTriageAgent(cfg),
    },
    workflows: {
      answerLeadQuestion: createLeadAnswerWorkflow(buildLeadTools(engine)),
      triageUpstreamIssues: createTriageWorkflow(engine, cfg.githubToken, githubFetch),
    },
    ...(observability ? { observability } : {}),
  });
}

export const mastra = createMastra();
