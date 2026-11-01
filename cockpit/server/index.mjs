import { homedir } from 'node:os';
import { join } from 'node:path';
import { writeFile, unlink } from 'node:fs/promises';
import { createObserver } from '../src/observer/index.ts';
import { startServer } from '../src/server/index.ts';

const userHome = process.env.COCKPIT_USER_HOME ?? homedir();
const directories = (process.env.COCKPIT_LOOP_DIRS ?? '').split(':').filter(Boolean);
const model = process.env.COCKPIT_RULE_NAMES === '1' ? async () => { throw new Error('Use rule names.'); } : undefined;
const observer = await createObserver({ userHome, loopDirectories: directories, model,
  onError: () => { console.error(JSON.stringify({ feature: 'cockpit', error: 'The observer pass failed.' })); } });
const server = await startServer({ observer, port: Number(process.env.COCKPIT_PORT ?? 5191),
  allowedFolders: () => observer.mediaFolders() });
const runtime = join(observer.store.home, 'runtime.json');
await writeFile(runtime, JSON.stringify({ pid: process.pid, url: server.url }), { mode: 0o600 });
console.log(JSON.stringify({ feature: 'cockpit', url: server.url }));
observer.start();
let closing = false;
async function stop() {
  if (closing) return;
  closing = true;
  await observer.stop().catch(() => {});
  await server.close();
  await unlink(runtime).catch(() => {});
}
process.on('SIGINT', () => { void stop(); });
process.on('SIGTERM', () => { void stop(); });
