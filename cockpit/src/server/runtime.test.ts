import { spawn, execFile } from 'node:child_process';
import { mkdtemp, mkdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import { expect, it } from 'vitest';

it('starts the real daemon and reports its selected URL through the CLI', async () => {
  const root = await mkdtemp(join(tmpdir(), 'cockpit-runtime-'));
  await mkdir(join(root, 'user'));
  const env = { ...process.env, COCKPIT_HOME: join(root, 'store'), COCKPIT_USER_HOME: join(root, 'user'), COCKPIT_PORT: '0', COCKPIT_LOOP_DIRS: '', COCKPIT_RULE_NAMES: '1' };
  const child = spawn(process.execPath, ['--experimental-transform-types', fileURLToPath(new URL('../../server/index.mjs', import.meta.url))], { env, stdio: ['ignore', 'pipe', 'pipe'] });
  const exit = new Promise<number | null>(resolve => { child.on('exit', resolve); });
  try {
    const url = await new Promise<string>((resolve, reject) => {
      let output = '';
      child.on('error', reject);
      child.on('exit', code => reject(new Error(`The daemon exited with code ${code}.`)));
      child.stdout.on('data', chunk => {
        output += String(chunk);
        for (const line of output.split('\n').filter(Boolean)) {
          try { const value = JSON.parse(line); if (value.url) resolve(value.url); } catch { /* Wait for a complete line. */ }
        }
      });
    });
    expect(new URL(url).hostname).toBe('127.0.0.1');
    const { stdout } = await promisify(execFile)(process.execPath, [fileURLToPath(new URL('../../cli.mjs', import.meta.url)), 'status'], { env });
    expect(stdout.trim()).toBe(url);
    expect((await fetch(`${url}/api/state`)).status).toBe(200);
  } finally {
    child.kill('SIGTERM');
    const timer = setTimeout(() => child.kill('SIGKILL'), 5000);
    await exit;
    clearTimeout(timer);
    await rm(root, { recursive: true, force: true });
  }
}, 30_000);
