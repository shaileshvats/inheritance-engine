// Start (or follow) an inheritance workflow.
//   npm run inherit -- owner/name [--mode docker] [--restore essential|fix_all|none] [--no-research] [--no-triage] [--wait]
//   npm run inherit -- --inheritance inh_… [--wait]
import { Client, Connection } from '@temporalio/client';
import { pathToFileURL } from 'node:url';
import { config, TASK_QUEUE } from './config.ts';
import type { InheritInput } from './workflows.ts';

export function parseArgs(argv: string[]): { input: InheritInput; wait: boolean } {
  const input: InheritInput = {};
  let wait = false;
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--inheritance') input.inheritanceId = argv[++i];
    else if (a === '--mode') input.executionMode = argv[++i] as InheritInput['executionMode'];
    else if (a === '--restore') {
      const v = argv[++i];
      input.restore = v === 'none' ? false : (v as 'essential' | 'fix_all');
    } else if (a === '--no-research') input.research = false;
    else if (a === '--no-triage') input.triage = false;
    else if (a === '--attempts') input.maxJobAttempts = Number(argv[++i]);
    else if (a === '--wait') wait = true;
    else if (/^[\w.-]+\/[\w.-]+$/.test(a)) input.repository = a;
    else throw new Error(`unknown argument ${a}`);
  }
  if (!input.repository && !input.inheritanceId) throw new Error('give owner/name or --inheritance <id>');
  return { input, wait };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const { input, wait } = parseArgs(process.argv.slice(2));
  const cfg = config();
  const client = new Client({ connection: await Connection.connect({ address: cfg.temporalAddress }), namespace: cfg.namespace });
  const slug = (input.repository ?? input.inheritanceId!).replace(/[^\w.-]+/g, '-');
  const handle = await client.workflow.start('inheritAndRestore', {
    taskQueue: TASK_QUEUE,
    workflowId: `inherit-${slug}-${Date.now()}`,
    args: [input],
  });
  console.log(`started ${handle.workflowId}`);
  console.log(`Temporal UI: http://127.0.0.1:8233/namespaces/${cfg.namespace}/workflows/${handle.workflowId}`);
  if (wait) console.log(JSON.stringify(await handle.result(), null, 2));
}
