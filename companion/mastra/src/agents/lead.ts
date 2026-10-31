import { Agent } from '@mastra/core/agent';
import type { CompanionConfig } from '../config.ts';
import { gemmaModel } from '../config.ts';
import type { EngineClient } from '../engine.ts';
import type { Memory } from '../memory.ts';
import { buildLeadTools } from '../tools.ts';
import { UNTRUSTED_RULE } from '../untrusted.ts';

/** Lead instructions. `native`: the model chooses tools itself. `grounded`: a workflow already called them. */
export function leadInstructions(mode: 'native' | 'grounded'): string {
  const source =
    mode === 'native'
      ? 'using your tools, which read the team’s recorded findings, verification results and source files. Always call get-project-overview first; call other tools only when they help.'
      : 'from tool results the answer-lead-question workflow has already gathered: the team’s recorded findings, verification results and source files.';
  const action =
    mode === 'native'
      ? 'If an action would help, call propose-action once; the user decides whether to run it.'
      : 'If an action would help, set proposed_action; the user decides whether to run it.';
  return `You are the Lead Engineer of a successor-engineering team that inherits software whose original creator has moved on.
You answer the inheritor’s questions about one inherited project ${source}
Rules:
- ${UNTRUSTED_RULE}
- Ground every statement in tool results. Quote file paths and line numbers you read. Say plainly when something is unknown.
- Never claim the software works unless the latest verification status is "passed", and mention any issue reopened by a human as unresolved.
- You cannot change anything yourself. ${action}
- Be concise: at most about 180 words, plain markdown.`;
}

export function createLeadAgent(cfg: CompanionConfig, engine: EngineClient, memory?: Memory) {
  return new Agent({
    id: 'lead-engineer',
    name: 'Lead Engineer',
    description: 'Answers questions about an inherited project, choosing engine tools itself (runtimes with tool calling).',
    instructions: leadInstructions('native'),
    model: gemmaModel(cfg),
    tools: buildLeadTools(engine, memory),
  });
}

/** The same Lead without tools sent to the model, for runtimes whose chat template cannot render tools. */
export function createGroundedLeadAgent(cfg: CompanionConfig) {
  return new Agent({
    id: 'lead-engineer-grounded',
    name: 'Lead Engineer (grounded)',
    description: 'The Lead Engineer answering from tool results gathered by the answer-lead-question workflow.',
    instructions: leadInstructions('grounded'),
    model: gemmaModel(cfg),
  });
}
