// Tools the Lead Engineer agent can call. They read the engine's recorded findings; none of them changes state.
// `proposeAction` only records a suggestion, which the engine shows as a button the user must press.
import { createTool } from '@mastra/core/tools';
import type { RequestContext } from '@mastra/core/request-context';
import { z } from 'zod';
import type { EngineClient } from './engine.ts';
import { repairDocuments, type Memory } from './memory.ts';
import { nonce, numberLines, truncate, untrusted } from './untrusted.ts';

export const INHERITANCE_KEY = 'inheritanceId';
export const ACTIONS = ['investigate', 'restore_essential', 'restore_all', 'verify'] as const;

function inheritanceId(ctx: { requestContext?: RequestContext } | undefined): string {
  const id = ctx?.requestContext?.get(INHERITANCE_KEY);
  if (typeof id !== 'string' || !id) throw new Error('no inheritance bound to this conversation');
  return id;
}

/** Shapes the engine's detail document into a compact, model-friendly overview. Pure, so it is unit-tested. */
export function summarizeDetail(d: any) {
  const v = d.last_verification;
  return {
    repository: d.full_name,
    stage: d.stage,
    original_commit: d.original_commit,
    original_untouched: d.original?.integrity?.ok ?? null,
    successor_commits_ahead: d.successor?.commits_ahead ?? 0,
    execution_mode: d.execution_mode,
    purpose: d.report?.purpose ? { text: d.report.purpose.text, status: d.report.purpose.status } : null,
    lead_summary: d.report?.lead_summary ? truncate(d.report.lead_summary, 600) : null,
    reviewer_disagreements: (d.report?.reviewer_challenges ?? [])
      .filter((c: any) => c.verdict !== 'agree')
      .slice(0, 8)
      .map((c: any) => ({ target: c.target, verdict: c.verdict, reason: truncate(c.reason ?? '', 240) })),
    latest_verification: v
      ? { label: v.label, status: v.status, summary: v.summary, command: v.command, environment: v.environment }
      : null,
    counts: d.counts,
    upstream_issues: d.upstream_triage
      ? {
          open_upstream: d.upstream_triage.total_open_upstream,
          counts: d.upstream_triage.counts,
          triaged: (d.upstream_triage.triaged ?? []).slice(0, 12).map((t: any) => ({
            number: t.number,
            title: t.title,
            verdict: t.verdict,
            cites: [...(t.cited_repairs ?? []), ...(t.cited_commits ?? [])],
          })),
        }
      : null,
    web_research: d.web_research
      ? {
          summary: truncate(d.web_research.summary ?? '', 400),
          related: (d.web_research.related ?? []).slice(0, 6).map((r: any) => ({ name: r.name, relation: r.relation, url: r.url })),
          documentation: (d.web_research.documentation ?? []).slice(0, 4).map((x: any) => ({ title: x.title, url: x.url })),
        }
      : null,
  };
}

export function summarizeIssues(d: any, status?: string) {
  return (d.issues ?? [])
    .filter((i: any) => !status || i.status === status)
    .slice(0, 25)
    .map((i: any) => ({
      id: i.id,
      title: i.title,
      category: i.category,
      severity: i.severity,
      status: i.status,
      source: i.source,
      reopened_by_human: Boolean(i.reopened_note),
      diagnosis: truncate(i.diagnosis ?? '', 400),
    }));
}

export function summarizeRepairs(d: any) {
  return (d.repairs ?? []).slice(-15).map((r: any) => ({
    id: r.id,
    issue_id: r.issue_id,
    status: r.status,
    commit: r.commit ?? null,
    files: r.files,
    review: r.review ? { verdict: r.review.verdict, concerns: (r.review.concerns ?? []).slice(0, 3) } : null,
    behaviour_probe: r.probe ? { verdict: r.probe.verdict, original_runtime: r.probe.legacy_image, note: r.probe.note } : null,
    note: r.note ? truncate(r.note, 300) : null,
  }));
}

/** Bring the restoration memory up to date with every inheritance the engine holds; unchanged repairs are skipped. */
export async function refreshMemory(engine: EngineClient, memory: Memory): Promise<number> {
  const docs = [];
  for (const i of await engine.list()) docs.push(...repairDocuments(await engine.detail(i.id)));
  return memory.index(docs);
}

export function buildLeadTools(engine: EngineClient, memory?: Memory) {
  const getProjectOverview = createTool({
    id: 'get-project-overview',
    description:
      'Get the inherited project overview: stage, purpose, the Lead summary, reviewer disagreements, the latest verification result and counts. Call this first.',
    inputSchema: z.object({}),
    execute: async (_input, ctx) => summarizeDetail(await engine.detail(inheritanceId(ctx))),
  });

  const listIssues = createTool({
    id: 'list-issues',
    description: 'List diagnosed issues (id, title, category, severity, status, diagnosis). Optionally filter by status: open, repaired, failed, rejected.',
    inputSchema: z.object({ status: z.enum(['open', 'repaired', 'failed', 'rejected']).optional() }),
    execute: async ({ status }, ctx) => ({ issues: summarizeIssues(await engine.detail(inheritanceId(ctx)), status) }),
  });

  const listRepairs = createTool({
    id: 'list-repairs',
    description: 'List repair attempts with their status, commit, independent review verdict and behaviour-probe verdict.',
    inputSchema: z.object({}),
    execute: async (_input, ctx) => ({ repairs: summarizeRepairs(await engine.detail(inheritanceId(ctx))) }),
  });

  const readSourceFile = createTool({
    id: 'read-source-file',
    description:
      'Read lines of a file from the successor (current state) or the original (as inherited). Use exact repository-relative paths, e.g. fn/func.py. Returns numbered lines.',
    inputSchema: z.object({
      path: z.string(),
      side: z.enum(['successor', 'original']).default('successor'),
      startLine: z.number().int().min(1).default(1),
      endLine: z.number().int().min(1).default(120),
    }),
    execute: async ({ path, side, startLine, endLine }, ctx) => {
      const f = await engine.file(inheritanceId(ctx), path, side);
      const end = Math.min(endLine, startLine + 199);
      return { path: f.path, side, total_lines: f.lines, content: untrusted(nonce(), `FILE ${f.path} (${side}) lines ${startLine}-${end}`, numberLines(f.content, startLine, end)) };
    },
  });

  const proposeAction = createTool({
    id: 'propose-action',
    description:
      'Propose ONE next action for the user to approve: investigate, restore_essential, restore_all or verify. This does not run anything; the user decides.',
    inputSchema: z.object({ action: z.enum(ACTIONS), reason: z.string() }),
    execute: async ({ action, reason }) => ({ proposed: action, reason, executed: false }),
  });

  // Only present when IE_MEMORY_DATABASE_URL is set; without it the Lead works exactly as before.
  const searchRestorationMemory = memory
    ? createTool({
        id: 'search-restoration-memory',
        description:
          'Search repair attempts recorded in OTHER inherited projects (kept, reverted or rejected) for a failure or fix, using hybrid keyword and vector search. Use it to find precedents and known bad fixes; each hit has a repair id.',
        inputSchema: z.object({ query: z.string(), limit: z.number().int().min(1).max(8).default(4) }),
        execute: async ({ query, limit }, ctx) => {
          await refreshMemory(engine, memory);
          const hits = await memory.search(query, limit, inheritanceId(ctx));
          return { hits: hits.map((h) => ({ ...h, excerpt: untrusted(nonce(), `REPAIR ${h.id} from ${h.repository}`, h.excerpt) })) };
        },
      })
    : undefined;

  return { getProjectOverview, listIssues, listRepairs, readSourceFile, proposeAction, ...(searchRestorationMemory ? { searchRestorationMemory } : {}) };
}
