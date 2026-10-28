// Agent tracing for Sentry, with privacy enforced twice:
// 1. `ContentStripper` (a Mastra span output processor) removes inputs, outputs, prompts and previews from every span.
// 2. `scrubSentryEvent` (Sentry beforeSendTransaction) keeps only an allowlist of metadata keys on every span.
// Repository source, prompts, model responses and tool arguments therefore never leave the machine;
// latency, token counts, model ids, agent/tool names and failures do.
import { appendFileSync } from 'node:fs';
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

/** Optional local audit: append exactly what is handed to the Sentry transport (after scrubbing) as JSON lines. */
export function audit(kind: string, payload: unknown) {
  const file = process.env.IE_TELEMETRY_AUDIT;
  if (!file) return;
  try {
    appendFileSync(file, JSON.stringify({ process: 'mastra-companion', kind, at: new Date().toISOString(), payload }) + '\n');
  } catch {
    // auditing must never break tracing
  }
}

/** Span descriptions are free text; keep them only for Mastra's own agent/tool/workflow spans. */
export function safeDescription(op: string | undefined, description: string | undefined): string | undefined {
  if (!description) return description;
  if (op && (op.startsWith('http') || op.startsWith('db') || op.startsWith('fs'))) return op;
  return description.replace(/[?#].*$/, '').slice(0, 120);
}

/** Sentry `beforeSendSpan`: same allowlist for spans sent outside a transaction payload. */
export function scrubSentrySpan<T extends Record<string, any>>(span: T): T {
  const s: any = span;
  scrubData(s.data);
  s.description = safeDescription(s.op, s.description);
  return span;
}

/** Sentry `beforeSend` (error events): keep the exception type and stack frames, drop messages and context. */
export function scrubSentryError<T extends Record<string, any>>(event: T): T {
  const e: any = event;
  delete e.message;
  delete e.logentry;
  for (const ex of e.exception?.values ?? []) ex.value = '[redacted]';
  delete e.extra;
  delete e.request;
  delete e.breadcrumbs;
  delete e.user;
  delete e.contexts?.trace?.data;
  return event;
}

/** Streamed span (envelope `span` container item): attributes are `{ key: { type, value } }`. */
function scrubStreamedSpan(s: any) {
  if (s?.attributes) for (const k of Object.keys(s.attributes)) if (!allowedKey(k)) delete s.attributes[k];
  const op = s?.attributes?.['sentry.op']?.value;
  if (typeof s?.name === 'string') s.name = safeDescription(op, s.name);
  return s;
}

const PASS_THROUGH = new Set(['client_report', 'session', 'sessions']);

/**
 * Last line of defence, applied in the transport to every envelope before it leaves the process (the Mastra
 * exporter streams agent/model spans as `span` items that bypass `beforeSendTransaction`). Allowlist by item type:
 * transactions, error events and spans are scrubbed; sessions/client reports pass; everything else is dropped.
 * The envelope header (which carries the DSN) is not audited.
 */
export function scrubEnvelope(envelope: any): any {
  const [headers, items] = envelope;
  const kept: any[] = [];
  for (const [itemHeaders, payload] of items ?? []) {
    const type = itemHeaders?.type;
    if (type === 'transaction') kept.push([itemHeaders, scrubSentryEvent(payload)]);
    else if (type === 'event') kept.push([itemHeaders, scrubSentryError(payload)]);
    else if (type === 'span') {
      if (Array.isArray(payload?.items)) payload.items.forEach(scrubStreamedSpan);
      else scrubSentrySpan(payload);
      kept.push([itemHeaders, payload]);
    } else if (PASS_THROUGH.has(type)) kept.push([itemHeaders, payload]);
  }
  audit('envelope', kept.map(([h, p]) => ({ type: h?.type, payload: p })));
  return [headers, kept];
}

/** Sentry `beforeSendTransaction`: allowlist span data, drop request bodies, breadcrumbs and extras. */
export function scrubSentryEvent<T extends Record<string, any>>(event: T): T {
  const e: any = event;
  if (e.contexts?.trace?.data) scrubData(e.contexts.trace.data);
  e.transaction = safeDescription(e.contexts?.trace?.op, e.transaction);
  for (const s of e.spans ?? []) {
    scrubData(s.data);
    s.description = safeDescription(s.op, s.description);
  }
  delete e.extra;
  delete e.request;
  delete e.breadcrumbs;
  delete e.user;
  return event;
}
