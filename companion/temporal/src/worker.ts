// Runs the Temporal worker for the inheritance workflow. Start the engine and a Temporal service first
// (`temporal server start-dev`, or the docker command in README.md).
import { NativeConnection, Worker } from '@temporalio/worker';
import { fileURLToPath } from 'node:url';
import { createActivities } from './activities.ts';
import { config, TASK_QUEUE } from './config.ts';

const cfg = config();
const connection = await NativeConnection.connect({ address: cfg.temporalAddress });
const worker = await Worker.create({
  connection,
  namespace: cfg.namespace,
  taskQueue: TASK_QUEUE,
  workflowsPath: fileURLToPath(new URL('./workflows.ts', import.meta.url)),
  activities: createActivities(cfg.engineUrl),
});
console.log(`worker on ${cfg.temporalAddress} (task queue ${TASK_QUEUE}), engine ${cfg.engineUrl}`);
await worker.run();
