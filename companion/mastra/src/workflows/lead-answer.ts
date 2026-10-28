// Workflow used when the local runtime cannot do native tool calling (e.g. LM Studio 0.4.12 + Gemma 4):
// gather-context calls the Lead's tools deterministically, then answer asks the grounded Lead agent (no tools
// sent to the model) for a structured reply. Same tools, same rules, no dependence on runtime tool support.
import { createStep, createWorkflow } from '@mastra/core/workflows';
import { RequestContext } from '@mastra/core/request-context';
import { z } from 'zod';
import { ACTIONS, INHERITANCE_KEY, type buildLeadTools } from '../tools.ts';

type LeadTools = ReturnType<typeof buildLeadTools>;

/** Repository-relative file paths mentioned in a question, with an optional "lines A-B" range. Pure; unit-tested. */
export function mentionedFiles(question: string): { path: string; start: number; end: number }[] {
  const out: { path: string; start: number; end: number }[] = [];
  const re = /([\w.\-]+(?:\/[\w.\-]+)*\.(?:py|js|ts|tsx|jsx|rs|go|rb|java|c|h|cpp|md|rst|txt|cfg|toml|ini|json|ya?ml)\b)(?:\s*(?:lines?|:)\s*(\d+)(?:\s*[-–]\s*(\d+))?)?/gi;
  for (const m of question.matchAll(re)) {
    const start = m[2] ? Number(m[2]) : 1;
    const end = m[3] ? Number(m[3]) : m[2] ? start + 40 : 120;
    if (!out.some((o) => o.path === m[1])) out.push({ path: m[1], start, end });
  }
  return out.slice(0, 3);
}

export const leadReply = z.object({
  reply: z.string(),
  proposed_action: z.enum(['none', ...ACTIONS]),
});

export function createLeadAnswerWorkflow(tools: LeadTools) {
  const input = z.object({ inheritanceId: z.string(), question: z.string(), history: z.array(z.object({ role: z.string(), content: z.string() })).default([]) });
  const gathered = input.extend({ context: z.string(), toolsUsed: z.array(z.string()) });

  const gatherContext = createStep({
    id: 'gather-context',
    description: 'Call the Lead’s engine tools: overview, issues, repairs, and any files the question mentions.',
    inputSchema: input,
    outputSchema: gathered,
    execute: async ({ inputData }) => {
      const requestContext = new RequestContext();
      requestContext.set(INHERITANCE_KEY, inputData.inheritanceId);
      const ctx = { requestContext } as any;
      const used: string[] = [];
      const call = async (name: keyof LeadTools, args: any) => {
        used.push(String(tools[name].id));
        return (tools[name] as any).execute(args, ctx);
      };
      const overview = await call('getProjectOverview', {});
      const issues = await call('listIssues', {});
      const repairs = await call('listRepairs', {});
      const files = [];
      for (const f of mentionedFiles(inputData.question)) {
        try {
          files.push(await call('readSourceFile', { path: f.path, side: 'successor', startLine: f.start, endLine: f.end }));
        } catch (e: any) {
          files.push({ path: f.path, error: String(e?.message ?? e).slice(0, 200) });
        }
      }
      const context = [
        `OVERVIEW\n${JSON.stringify(overview, null, 1)}`,
        `ISSUES\n${JSON.stringify(issues, null, 1)}`,
        `REPAIRS\n${JSON.stringify(repairs, null, 1)}`,
        ...files.map((f: any) => (f.content ? f.content : `FILE ${f.path}: ${f.error}`)),
      ].join('\n\n');
      return { ...inputData, context, toolsUsed: used };
    },
  });

  const answer = createStep({
    id: 'answer',
    description: 'Ask the Lead Engineer (Gemma) for a grounded, structured reply.',
    inputSchema: gathered,
    outputSchema: leadReply.extend({ toolsUsed: z.array(z.string()), usage: z.any() }),
    execute: async ({ inputData, mastra }) => {
      const agent = mastra!.getAgent('leadEngineerGrounded');
      const convo = inputData.history.slice(-6).map((m) => `${m.role}: ${m.content.slice(0, 800)}`).join('\n');
      const prompt = `Tool results (recorded by the engine):\n${inputData.context}\n\nConversation so far:\n${convo || '(none)'}\n\nUser's question: ${inputData.question}\n\nAnswer the question. Set proposed_action to one action the user could approve, or "none".`;
      const r: any = await agent.generate(prompt, { structuredOutput: { schema: leadReply } });
      return { ...(r.object as z.infer<typeof leadReply>), toolsUsed: inputData.toolsUsed, usage: r.usage ?? null };
    },
  });

  return createWorkflow({
    id: 'answer-lead-question',
    description: 'Answer the inheritor’s question with the Lead’s tools called deterministically (for runtimes without tool calling).',
    inputSchema: input,
    outputSchema: leadReply.extend({ toolsUsed: z.array(z.string()), usage: z.any() }),
  })
    .then(gatherContext)
    .then(answer)
    .commit();
}
