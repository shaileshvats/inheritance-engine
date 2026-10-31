// Restoration memory from the command line (needs IE_MEMORY_DATABASE_URL and the engine running):
//   npm run memory -- index                     index every repair attempt the engine has recorded
//   npm run memory -- search "<failure text>"   hybrid keyword + vector search over them
import { loadConfig } from './config.ts';
import { engineClient } from './engine.ts';
import { createMemory, memoryConfig } from './memory.ts';
import { refreshMemory } from './tools.ts';

const [cmd, ...rest] = process.argv.slice(2);
const mc = memoryConfig();
if (!mc) throw new Error('set IE_MEMORY_DATABASE_URL (see README: restoration memory)');
const memory = createMemory(mc);
try {
  await memory.migrate();
  if (cmd === 'index') {
    const changed = await refreshMemory(engineClient(loadConfig().engineUrl), memory);
    console.log(`embedded ${changed} new or changed repair(s); ${await memory.count()} in memory`);
  } else if (cmd === 'search' && rest.length) {
    for (const h of await memory.search(rest.join(' '), 5)) {
      console.log(`${h.score.toFixed(4)}  [${h.matched_by.join('+')}]  ${h.repository}  ${h.id}  ${h.status}  ${h.title}`);
    }
  } else {
    console.log('usage: npm run memory -- index | search "<text>"');
  }
} finally {
  await memory.close();
}
