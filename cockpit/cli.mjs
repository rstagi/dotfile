#!/usr/bin/env node
import { execFileSync, spawn } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { join } from 'node:path';

const label = 'com.rstagi.cockpit';
const target = `gui/${process.getuid()}/${label}`;
const home = process.env.COCKPIT_HOME ?? join(homedir(), '.cockpit');
const command = process.argv[2] ?? 'status';
async function status() {
  try {
    const runtime = JSON.parse(await readFile(join(home, 'runtime.json'), 'utf8'));
    process.kill(runtime.pid, 0);
    const url = new URL(runtime.url);
    if (url.hostname !== '127.0.0.1' || url.protocol !== 'http:') throw new Error('The URL is invalid.');
    const response = await fetch(`${url.origin}/api/state`, { signal: AbortSignal.timeout(2000) });
    if (!response.ok) throw new Error('The server is unavailable.');
    return url.origin;
  } catch { return undefined; }
}
try {
  if (command === 'start') {
    const running = await status();
    if (running) console.log(running);
    else {
      execFileSync('launchctl', ['enable', target]);
      execFileSync('launchctl', ['kickstart', target]);
      console.log('The cockpit daemon is starting. Run cockpit status for its URL.');
    }
  } else if (command === 'stop') {
    // Disable KeepAlive before stopping the job.
    execFileSync('launchctl', ['disable', target]);
    execFileSync('launchctl', ['kill', 'SIGTERM', target]);
    console.log('The cockpit daemon stopped.');
  } else if (command === 'status' || command === 'open') {
    const url = await status();
    if (!url) { console.log('The cockpit daemon is stopped. Run cockpit start.'); process.exitCode = 1; }
    else if (command === 'open') spawn('open', [url], { stdio: 'ignore' }).on('error', () => { process.exitCode = 1; });
    else console.log(url);
  } else { console.error('Usage: cockpit start|stop|status|open'); process.exitCode = 2; }
} catch { console.error('The cockpit command failed. Install it with ./install.sh cockpit.'); process.exitCode = 1; }
