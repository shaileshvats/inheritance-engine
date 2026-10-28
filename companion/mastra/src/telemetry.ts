// Agent tracing for Sentry, with privacy enforced twice:
// 1. `ContentStripper` (a Mastra span output processor) removes inputs, outputs, prompts and previews from every span.
// 2. `scrubSentryEvent` (Sentry beforeSendTransaction) keeps only an allowlist of metadata keys on every span.
// Repository source, prompts, model responses and tool arguments therefore never leave the machine;
// latency, token counts, model ids, agent/tool names and failures do.
import type { SpanOutputProcessor } from '@mastra/core/observability';

export class ContentStripper implements SpanOutputProcessor {
  name = 'inheritance-engine-content-stripper';

  process(span?: any) {
    if (!span) return span;
    span.input = undefined;
    span.output = undefined;
    span.inputPreview = null;
    span.outputPreview = null;
    span.errorPreview = null;
    span.requestContext = undefined;
    if (span.attributes) {
      for (const k of ['prompt', 'instructions', 'tools', 'availableTools', 'toolDescription']) delete span.attributes[k];
    }
    if (span.errorInfo?.message) span.errorInfo.message = String(span.errorInfo.message).slice(0, 160);
    return span;
  }

  async shutdown() {}
}

const EXACT = new Set([
  'gen_ai.operation.name',
  'gen_ai.system',
  'gen_ai.request.model',
  'gen_ai.response.model',
  'gen_ai.agent.name',
  'gen_ai.tool.name',
  'gen_ai.tool.call.id',
  'gen_ai.pipeline.name',
  'gen_ai.conversation.id',
  'gen_ai.request.stream',
  'gen_ai.response.streaming',
  'gen_ai.completion_start_time',
  'gen_ai.response.finish_reasons',
]);

export function allowedKey(k: string): boolean {
  return EXACT.has(k) || k.startsWith('gen_ai.usage.') || k.startsWith('sentry.') || k.startsWith('otel.') || k.startsWith('ie.');
}

function scrubData(data: Record<string, unknown> | undefined) {
  if (!data) return data;
  for (const k of Object.keys(data)) if (!allowedKey(k)) delete data[k];
  return data;
}

/** Sentry `beforeSendTransaction`: allowlist span data, drop request bodies, breadcrumbs and extras. */
export function scrubSentryEvent<T extends Record<string, any>>(event: T): T {
  const e: any = event;
  if (e.contexts?.trace?.data) scrubData(e.contexts.trace.data);
  for (const s of e.spans ?? []) scrubData(s.data);
  delete e.extra;
  delete e.request;
  delete e.breadcrumbs;
  delete e.user;
  return event;
}
