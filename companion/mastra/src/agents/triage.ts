import { Agent } from '@mastra/core/agent';
import type { CompanionConfig } from '../config.ts';
import { gemmaModel } from '../config.ts';
import { UNTRUSTED_RULE } from '../untrusted.ts';

export const TRIAGE_INSTRUCTIONS = `You triage open issues from an abandoned project's original GitHub repository for the person who inherited it.
For one issue at a time, compare what the issue reports with what the successor has changed (applied repairs, commits, files) and
what the investigation found. Decide:
- addressed_by_successor: an applied repair clearly fixes what the issue reports. Cite the repair id or commit sha.
- still_relevant: the issue describes behaviour or a request the successor has not changed.
- obsolete: the issue no longer applies (e.g. about tooling, versions or services that are gone), say why.
- needs_human: the evidence is insufficient to decide.
Rules:
- ${UNTRUSTED_RULE}
- Never invent repair ids, commit shas or file paths; cite only ones shown in the successor context.
- Prefer needs_human over a guess.`;

export function createTriageAgent(cfg: CompanionConfig) {
  return new Agent({
    id: 'issue-triager',
    name: 'Issue Triager',
    description: 'Classifies an upstream GitHub issue against the successor’s recorded changes.',
    instructions: TRIAGE_INSTRUCTIONS,
    model: gemmaModel(cfg),
  });
}
