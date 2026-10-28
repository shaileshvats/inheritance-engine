// Narrow loopback API used by the Inheritance Engine (not the full Mastra server surface):
//   GET  /health
//   POST /lead/chat   { inheritanceId, message, history? }  -> Lead Engineer agent with engine tools
//   POST /triage      { inheritanceId, maxIssues? }         -> triage-upstream-issues workflow
import { createServer, type IncomingMessage, type Server, type ServerResponse } from 'node:http';
import { fileURLToPath } from 'node:url';
import type { Mastra } from '@mastra/core';
import { RequestContext } from '@mastra/core/request-context';
import { isLoopback, loadConfig, type CompanionConfig } from './config.ts';
import { ACTIONS, INHERITANCE_KEY } from './tools.ts';

const MAX_BODY = 64 * 1024;

function send(res: ServerResponse, status: number, body: unknown) {
  res.writeHead(status, { 'content-type': 'application/json', 'cache-control': 'no-store' });
  res.end(JSON.stringify(body));
}

async function readJson(req: IncomingMessage): Promise<any> {
  let size = 0;
  const chunks: Buffer[] = [];
  for await (const c of req) {
    size += c.length;
    if (size > MAX_BODY) throw new Error('request body too large');
    chunks.push(c as Buffer);
  }
  return chunks.length ? JSON.parse(Buffer.concat(chunks).toString('utf8')) : {};
}

/** Same guard as the engine: loopback Host only (DNS rebinding) and no cross-origin browser requests (CSRF). */
export function requestAllowed(headers: IncomingMessage['headers']): boolean {
  const host = String(headers.host ?? '').replace(/:\d+$/, '');
  if (!['127.0.0.1', 'localhost', '[::1]'].includes(host)) return false;
  return headers.origin === undefined;
}

/** Extract the single proposed action from tool results; anything else is ignored. */
export function proposedAction(toolResults: any[] | undefined): string | null {
  for (const r of toolResults ?? []) {
    const p = r?.payload ?? r;
    const name = p?.toolName ?? p?.name;
    const out = p?.result ?? p?.output;
    if (name === 'proposeAction' || name === 'propose-action') {
      const a = out?.proposed;
      if ((ACTIONS as readonly string[]).includes(a)) return a;
    }
  }
  return null;
}

function toolSummary(toolCalls: any[] | undefined) {
  return (toolCalls ?? []).map((c) => {
    const p = c?.payload ?? c;
    return { tool: p?.toolName ?? p?.name ?? 'unknown', args: p?.args ?? p?.input ?? {} };
  });
}

function usageOf(u: any) {
  return { input_tokens: u?.inputTokens ?? u?.promptTokens ?? null, output_tokens: u?.outputTokens ?? u?.completionTokens ?? null };
}

export type ToolMode = 'native' | 'grounded';

/**
 * Probe whether the local runtime can render a tool-calling request for this model. LM Studio 0.4.12's
 * Gemma 4 chat template cannot ("Cannot call something that is not a function"), newer runtimes can.
 */
export async function detectToolMode(cfg: CompanionConfig, fetchImpl: typeof fetch = fetch): Promise<{ mode: ToolMode; reason: string }> {
  const forced = process.env.IE_MASTRA_TOOL_MODE;
  if (forced === 'native' || forced === 'grounded') return { mode: forced, reason: 'set by IE_MASTRA_TOOL_MODE' };
  try {
    const r = await fetchImpl(`${cfg.modelEndpoint}/chat/completions`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', authorization: `Bearer ${cfg.modelApiKey}` },
      body: JSON.stringify({
        model: cfg.model,
        max_tokens: 1,
        messages: [{ role: 'user', content: 'ping' }],
        tools: [{ type: 'function', function: { name: 'noop', description: 'noop', parameters: { type: 'object', properties: { x: { type: 'string' } } } } }],
      }),
    });
    if (r.ok) return { mode: 'native', reason: 'runtime accepted a tool-calling request' };
    const body = await r.text();
    return { mode: 'grounded', reason: `runtime rejected tool calling (${r.status}): ${body.slice(0, 160)}` };
  } catch (e: any) {
    return { mode: 'grounded', reason: `probe failed: ${String(e?.message ?? e).slice(0, 160)}` };
  }
}

export function createCompanionServer(mastra: Mastra, cfg: CompanionConfig, tool: { mode: ToolMode; reason: string } = { mode: 'grounded', reason: 'default' }): Server {
  return createServer(async (req, res) => {
    try {
      if (!requestAllowed(req.headers)) return send(res, 403, { error: 'loopback, same-origin requests only' });
      const url = new URL(req.url ?? '/', 'http://127.0.0.1');
      if (req.method === 'GET' && url.pathname === '/health') {
        return send(res, 200, { ok: true, framework: 'mastra', model: cfg.model, model_endpoint: cfg.modelEndpoint, sentry: Boolean(cfg.sentryDsn), tool_mode: tool.mode, tool_mode_reason: tool.reason, agents: ['leadEngineer', 'leadEngineerGrounded', 'issueTriager'], workflows: ['answerLeadQuestion', 'triageUpstreamIssues'] });
      }
      if (req.method !== 'POST') return send(res, 404, { error: 'not found' });
      const body = await readJson(req);
      const inheritanceId = typeof body.inheritanceId === 'string' ? body.inheritanceId : '';
      if (!/^inh_[a-z0-9]{8,32}$/.test(inheritanceId)) return send(res, 400, { error: 'inheritanceId is required' });

      if (url.pathname === '/lead/chat') {
        const message = String(body.message ?? '').slice(0, 4000);
        if (!message.trim()) return send(res, 400, { error: 'message is empty' });
        const history = Array.isArray(body.history) ? body.history.slice(-8) : [];
        const messages = [
          ...history
            .filter((m: any) => m && (m.role === 'user' || m.role === 'assistant') && typeof m.content === 'string')
            .map((m: any) => ({ role: m.role, content: m.content.slice(0, 2000) })),
          { role: 'user' as const, content: message },
        ];
        const t0 = Date.now();
        if (tool.mode === 'grounded') {
          const run = await mastra.getWorkflow('answerLeadQuestion').createRun();
          const result: any = await run.start({ inputData: { inheritanceId, question: message, history: messages.slice(0, -1) } });
          if (result.status !== 'success') {
            return send(res, 502, { error: `workflow ${result.status}: ${String(result.error?.message ?? result.error ?? '')}`.slice(0, 500), run_id: run.runId });
          }
          const out = result.result;
          return send(res, 200, {
            reply: out.reply,
            proposed_action: out.proposed_action === 'none' ? null : out.proposed_action,
            tool_calls: out.toolsUsed.map((t: string) => ({ tool: t, args: {} })),
            mode: 'grounded',
            workflow: 'answer-lead-question',
            run_id: run.runId,
            usage: usageOf(out.usage),
            duration_ms: Date.now() - t0,
            model: cfg.model,
          });
        }
        const requestContext = new RequestContext();
        requestContext.set(INHERITANCE_KEY, inheritanceId);
        const r: any = await mastra.getAgent('leadEngineer').generate(messages as any, { requestContext, maxSteps: 6 });
        return send(res, 200, {
          reply: r.text ?? '',
          proposed_action: proposedAction(r.toolResults),
          tool_calls: toolSummary(r.toolCalls),
          mode: 'native',
          steps: Array.isArray(r.steps) ? r.steps.length : null,
          usage: usageOf(r.usage ?? r.totalUsage),
          duration_ms: Date.now() - t0,
          model: cfg.model,
          trace_id: r.traceId ?? null,
        });
      }

      if (url.pathname === '/triage') {
        const maxIssues = Math.max(1, Math.min(30, Number(body.maxIssues ?? 10) || 10));
        const t0 = Date.now();
        const run = await mastra.getWorkflow('triageUpstreamIssues').createRun();
        const result: any = await run.start({ inputData: { inheritanceId, maxIssues } });
        if (result.status !== 'success') {
          const err = result.error?.message ?? String(result.error ?? result.status);
          return send(res, 502, { error: `workflow ${result.status}: ${err}`.slice(0, 500), run_id: run.runId });
        }
        return send(res, 200, { ...result.result, run_id: run.runId, duration_ms: Date.now() - t0, model: cfg.model });
      }
      return send(res, 404, { error: 'not found' });
    } catch (e: any) {
      return send(res, 500, { error: String(e?.message ?? e).slice(0, 500) });
    }
  });
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const cfg = loadConfig();
  if (!isLoopback(cfg.modelEndpoint)) {
    console.error(`Refusing to start: model endpoint ${cfg.modelEndpoint} is not local (repository content would leave this machine).`);
    process.exit(1);
  }
  const { mastra } = await import('./mastra/index.ts');
  const tool = await detectToolMode(cfg);
  createCompanionServer(mastra, cfg, tool).listen(cfg.port, cfg.host, () => {
    console.log(`Inheritance Engine Mastra companion on http://${cfg.host}:${cfg.port} (model ${cfg.model} at ${cfg.modelEndpoint}; tool mode ${tool.mode}: ${tool.reason}; Sentry ${cfg.sentryDsn ? 'on' : 'off'})`);
  });
}
