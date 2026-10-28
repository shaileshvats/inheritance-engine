// Workflow: triage the original repository's open GitHub issues against what the successor has changed.
// load-inheritance -> fetch-upstream-issues -> classify-issues (Gemma) -> validate-evidence (deterministic).
import { createStep, createWorkflow } from '@mastra/core/workflows';
import { z } from 'zod';
import type { EngineClient } from '../engine.ts';
import { nonce, truncate, untrusted } from '../untrusted.ts';

export const VERDICTS = ['addressed_by_successor', 'still_relevant', 'obsolete', 'needs_human'] as const;

const repairRef = z.object({ id: z.string(), issue: z.string(), commit: z.string(), files: z.array(z.string()) });
const upstreamIssue = z.object({
  number: z.number(),
  title: z.string(),
  body: z.string(),
  url: z.string(),
  created_at: z.string(),
  comments: z.number(),
});
const context = z.object({
  inheritanceId: z.string(),
  repository: z.string(),
  purpose: z.string(),
  verification: z.string(),
  repairs: z.array(repairRef),
  diff: z.string(),
  maxIssues: z.number(),
});
export const classification = z.object({
  verdict: z.enum(VERDICTS),
  reason: z.string(),
  cited_repairs: z.array(z.string()).max(4),
  cited_commits: z.array(z.string()).max(4),
  confidence: z.enum(['high', 'medium', 'low']),
});
const triaged = upstreamIssue.omit({ body: true }).extend({
  verdict: z.enum(VERDICTS),
  reason: z.string(),
  cited_repairs: z.array(z.string()),
  cited_commits: z.array(z.string()),
  confidence: z.enum(['high', 'medium', 'low']),
  engine_check: z.string(),
});
export const triageOutput = z.object({
  repository: z.string(),
  total_open_upstream: z.number(),
  triaged: z.array(triaged),
  counts: z.record(z.string(), z.number()),
});

/**
 * The engine-side rule: a model may only call an issue "addressed" by citing a repair or commit that exists.
 * Unknown citations are dropped and the verdict downgraded to needs_human. Pure, so it is unit-tested.
 */
export function validateClassification(c: z.infer<typeof classification>, repairs: z.infer<typeof repairRef>[]) {
  // Models write citations loosely ("commit ed0b4e499d", "see rep_45f8…"). Extract real identifiers from the
  // citations and the reason, then keep only the ones that exist in the successor.
  const text = [...c.cited_repairs, ...c.cited_commits, c.reason].join(' ');
  const repairIds = new Set(repairs.map((r) => r.id));
  const mentionedRepairs = [...new Set(text.match(/rep_[0-9a-f]{6,32}/g) ?? [])];
  const mentionedShas = [...new Set((text.match(/\b[0-9a-f]{7,40}\b/g) ?? []).filter((h) => !mentionedRepairs.some((r) => r.endsWith(h))))];
  const repairsOk = mentionedRepairs.filter((r) => repairIds.has(r));
  const commitsOk = mentionedShas.filter((h) => repairs.some((r) => r.commit.startsWith(h)));
  const unknown = mentionedRepairs.length - repairsOk.length + (mentionedShas.length - commitsOk.length);
  let verdict = c.verdict;
  let check = unknown ? `ignored ${unknown} identifier(s) that do not exist in the successor` : 'citations verified';
  if (verdict === 'addressed_by_successor' && repairsOk.length + commitsOk.length === 0) {
    verdict = 'needs_human';
    check = 'downgraded: "addressed" without citing an applied repair or commit that exists';
  } else if (verdict === 'addressed_by_successor') {
    check = `verified: cites ${[...repairsOk, ...commitsOk.map((h) => h.slice(0, 10))].join(', ')}`;
  }
  return { ...c, verdict, cited_repairs: repairsOk, cited_commits: commitsOk, engine_check: check };
}

export function createTriageWorkflow(engine: EngineClient, githubToken?: string, fetchImpl: typeof fetch = fetch) {
  const loadInheritance = createStep({
    id: 'load-inheritance',
    description: 'Read the successor’s applied repairs, diff, purpose and verification from the engine.',
    inputSchema: z.object({ inheritanceId: z.string(), maxIssues: z.number().int().min(1).max(30).default(10) }),
    outputSchema: context,
    execute: async ({ inputData }) => {
      const d = await engine.detail(inputData.inheritanceId);
      const issues = new Map<string, string>((d.issues ?? []).map((i: any) => [i.id, i.title]));
      const repairs = (d.repairs ?? [])
        .filter((r: any) => r.status === 'applied' && r.commit)
        .map((r: any) => ({ id: r.id, issue: issues.get(r.issue_id) ?? r.issue_id, commit: r.commit, files: r.files ?? [] }));
      const diff = repairs.length ? (await engine.diff(inputData.inheritanceId)).diff : '';
      const v = d.last_verification;
      return {
        inheritanceId: inputData.inheritanceId,
        repository: d.full_name,
        purpose: d.report?.purpose?.text ?? 'unknown',
        verification: v ? `${v.label}: ${v.status} ${JSON.stringify(v.summary ?? {})}` : 'not run',
        repairs,
        diff: truncate(diff, 6000),
        maxIssues: inputData.maxIssues,
      };
    },
  });

  const fetchUpstreamIssues = createStep({
    id: 'fetch-upstream-issues',
    description: 'Fetch open issues (not pull requests) from the original repository on GitHub.',
    inputSchema: context,
    outputSchema: context.extend({ issues: z.array(upstreamIssue), totalOpen: z.number() }),
    execute: async ({ inputData }) => {
      const headers: Record<string, string> = { accept: 'application/vnd.github+json', 'user-agent': 'inheritance-engine-mastra' };
      if (githubToken) headers.authorization = `Bearer ${githubToken}`;
      const url = `https://api.github.com/repos/${inputData.repository}/issues?state=open&per_page=100&sort=comments&direction=desc`;
      const r = await fetchImpl(url, { headers });
      if (!r.ok) throw new Error(`GitHub issues request failed: ${r.status}`);
      const all = ((await r.json()) as any[]).filter((i) => !i.pull_request);
      const issues = all.slice(0, inputData.maxIssues).map((i) => ({
        number: i.number,
        title: String(i.title),
        body: truncate(String(i.body ?? ''), 1500),
        url: i.html_url,
        created_at: i.created_at,
        comments: i.comments,
      }));
      return { ...inputData, issues, totalOpen: all.length };
    },
  });

  const classifyIssues = createStep({
    id: 'classify-issues',
    description: 'Ask the Issue Triager agent (Gemma) to classify each upstream issue against the successor.',
    inputSchema: context.extend({ issues: z.array(upstreamIssue), totalOpen: z.number() }),
    outputSchema: context.extend({
      totalOpen: z.number(),
      results: z.array(z.object({ issue: upstreamIssue, classification })),
    }),
    execute: async ({ inputData, mastra }) => {
      const agent = mastra!.getAgent('issueTriager');
      const n = nonce();
      const successor = [
        `Repository: ${inputData.repository}`,
        `Purpose (from the investigation): ${inputData.purpose}`,
        `Latest verification: ${inputData.verification}`,
        'Applied repairs:',
        ...(inputData.repairs.length
          ? inputData.repairs.map((r) => `- repair id ${r.id}, commit ${r.commit.slice(0, 10)}: ${r.issue}; files ${r.files.join(', ')}`)
          : ['- none']),
      ].join('\n');
      const results = [];
      for (const issue of inputData.issues) {
        const prompt = `Successor context (trusted, recorded by the engine):\n${successor}\n\n${untrusted(n, 'SUCCESSOR DIFF vs original', inputData.diff || '(no changes)')}\n\n${untrusted(n, `UPSTREAM ISSUE #${issue.number}`, `Title: ${issue.title}\nOpened: ${issue.created_at}\n\n${issue.body}`)}\n\nClassify this issue. If an applied repair fixes it, put that repair id (rep_...) in cited_repairs and its commit in cited_commits.`;
        const res = await agent.generate(prompt, { structuredOutput: { schema: classification } });
        results.push({ issue, classification: res.object as z.infer<typeof classification> });
      }
      const { issues: _omit, ...rest } = inputData;
      return { ...rest, results };
    },
  });

  const validateEvidence = createStep({
    id: 'validate-evidence',
    description: 'Deterministically check every citation against the successor’s applied repairs and commits.',
    inputSchema: context.extend({
      totalOpen: z.number(),
      results: z.array(z.object({ issue: upstreamIssue, classification })),
    }),
    outputSchema: triageOutput,
    execute: async ({ inputData }) => {
      const triagedList = inputData.results.map(({ issue, classification: c }) => {
        const { body: _b, ...meta } = issue;
        return { ...meta, ...validateClassification(c, inputData.repairs) };
      });
      const counts: Record<string, number> = Object.fromEntries(VERDICTS.map((v) => [v, 0]));
      for (const t of triagedList) counts[t.verdict] += 1;
      return { repository: inputData.repository, total_open_upstream: inputData.totalOpen, triaged: triagedList, counts };
    },
  });

  return createWorkflow({
    id: 'triage-upstream-issues',
    description: 'Triage the original repository’s open GitHub issues against what the successor has changed.',
    inputSchema: z.object({ inheritanceId: z.string(), maxIssues: z.number().int().min(1).max(30).default(10) }),
    outputSchema: triageOutput,
  })
    .then(loadInheritance)
    .then(fetchUpstreamIssues)
    .then(classifyIssues)
    .then(validateEvidence)
    .commit();
}
