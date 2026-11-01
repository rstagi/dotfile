import { createServer } from 'node:http';
import { readFile, realpath, stat } from 'node:fs/promises';
import { extname, isAbsolute, relative, resolve, sep } from 'node:path';
import type { ServerResponse } from 'node:http';
import type { createObserver } from '../observer/index.ts';
import { reduceEvent } from '../core/state.ts';
import type { RunState } from '../core/events.ts';

type Observer = Awaited<ReturnType<typeof createObserver>>;
export interface ServerOptions {
  observer: Observer;
  port?: number;
  uiDirectory?: string;
  allowedFolders?: () => string[];
}
const MIME: Record<string, string> = {
  '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8',
  '.png': 'image/png', '.jpg': 'image/jpeg', '.jpeg': 'image/jpeg', '.gif': 'image/gif', '.webp': 'image/webp',
  '.avif': 'image/avif', '.bmp': 'image/bmp', '.webm': 'video/webm', '.mp4': 'video/mp4', '.ico': 'image/x-icon', '.woff2': 'font/woff2',
};
const MEDIA_TYPES = new Set(['.png', '.jpg', '.jpeg', '.gif', '.webp', '.avif', '.bmp', '.webm', '.mp4']);

export async function startServer(options: ServerOptions) {
  const clients = new Set<ServerResponse>();
  const observer = options.observer;
  const unsubscribe = observer.subscribe(event => {
    for (const client of clients) client.write(`event: cockpit\ndata: ${JSON.stringify(event)}\n\n`);
  });
  const heartbeat = setInterval(() => { for (const client of clients) client.write(': heartbeat\n\n'); }, 15_000);
  heartbeat.unref();
  const server = createServer((request, response) => {
    void handle().catch(() => { if (!response.headersSent) response.writeHead(500); response.end('The request failed.'); });
    async function handle() {
      response.setHeader('X-Content-Type-Options', 'nosniff');
      response.setHeader('Cache-Control', 'no-store');
      const address = server.address();
      const expectedHost = address && typeof address !== 'string' ? `127.0.0.1:${address.port}` : '';
      if (request.headers.host !== expectedHost || (request.headers.origin && request.headers.origin !== `http://${expectedHost}`)) {
        response.writeHead(403); response.end('The request origin is not allowed.'); return;
      }
      if (request.method !== 'GET') { response.writeHead(405); response.end(); return; }
      const url = new URL(request.url ?? '/', 'http://127.0.0.1');
      const json = (value: unknown) => { response.setHeader('Content-Type', 'application/json'); response.end(JSON.stringify(value)); };
      if (url.pathname === '/api/state') {
        const current = observer.store.snapshot();
        let runs = current;
        const replayTimes = new Map<string, Record<string, Record<string, string>>>();
        const at = url.searchParams.get('at');
        if (at) {
          const time = Date.parse(at);
          if (!Number.isFinite(time)) { response.writeHead(400); response.end('The replay time is invalid.'); return; }
          const replay: RunState[] = [];
          for (const run of runs) {
            let state: RunState | undefined;
            const times: Record<string, Record<string, string>> = {};
            for (const event of await observer.store.events(run.id)) {
              if (Date.parse(event.timestamp) > time) continue;
              state = reduceEvent(state, event);
              const value = Object.values(event).find(value => value && typeof value === 'object' && 'id' in value);
              if (value?.id) (times[event.type] ??= {})[value.id] = event.timestamp;
            }
            replayTimes.set(run.id, times);
            if (state) replay.push(state);
          }
          runs = replay;
        }
        const sessions = current.flatMap(run => [
          ...(run.meta.sessionId ? [{ id: run.meta.sessionId, chatId: run.meta.chatId, title: run.meta.title }] : []),
          ...(run.meta.sessions ?? []),
        ].map(session => ({ ...session, runId: run.id })));
        if (url.searchParams.get('view') === 'session') {
          const chat = url.searchParams.get('chat');
          const allowed = new Set(current.filter(run => run.meta.chatId === chat || run.meta.sessionId === chat ||
            run.meta.sessions?.some(session => session.id === chat || session.chatId === chat)).map(run => run.id));
          runs = runs.filter(run => allowed.has(run.id));
        }
        json({ runs: runs.map(run => ({ ...run, eventTimes: replayTimes.get(run.id) ?? observer.eventTimes(run.id) })), sessions, observer: observer.status, timestamp: new Date().toISOString() });
        return;
      }
      if (url.pathname === '/api/events') {
        const run = url.searchParams.get('run') ?? '';
        if (!observer.store.getRun(run)) { response.writeHead(404); response.end(); return; }
        json({ events: await observer.store.events(run) }); return;
      }
      if (url.pathname === '/api/stream') {
        response.writeHead(200, { 'Content-Type': 'text/event-stream', Connection: 'keep-alive', 'X-Accel-Buffering': 'no' });
        response.write(': connected\n\n');
        clients.add(response);
        request.on('close', () => { clients.delete(response); });
        return;
      }
      if (url.pathname.startsWith('/media/')) {
        const id = decodeURIComponent(url.pathname.slice('/media/'.length));
        const media = observer.store.snapshot().flatMap(run => run.media).find(media => media.id === id);
        if (!media) { response.writeHead(404); response.end(); return; }
        const roots = options.allowedFolders?.() ?? [];
        let path: string;
        try { path = await realpath(media.path); } catch { response.writeHead(404); response.end(); return; }
        const allowed = await Promise.all(roots.map(async root => {
          try { return inside(await realpath(root), path); } catch { return false; }
        }));
        if (!MEDIA_TYPES.has(extname(path).toLowerCase()) || !allowed.some(Boolean)) {
          response.writeHead(403); response.end('The media file is outside the allowed folders.'); return;
        }
        await file(path); return;
      }
      const root = resolve(options.uiDirectory ?? new URL('../../dist', import.meta.url).pathname);
      const path = resolve(root, `.${decodeURIComponent(url.pathname === '/' ? '/index.html' : url.pathname)}`);
      if (!inside(root, path)) { response.writeHead(403); response.end(); return; }
      try {
        if (!inside(await realpath(root), await realpath(path))) { response.writeHead(403); response.end(); return; }
        await file(path);
      } catch { response.writeHead(404); response.end('The file does not exist.'); }
      async function file(path: string) {
        if (!(await stat(path)).isFile()) { response.writeHead(404); response.end(); return; }
        response.setHeader('Content-Type', MIME[extname(path).toLowerCase()] ?? 'application/octet-stream');
        response.end(await readFile(path));
      }
    }
  });
  let port = options.port ?? 5191;
  try {
    while (true) {
      try {
        await new Promise<void>((resolve, reject) => {
          const error = (error: Error) => { server.off('listening', ready); reject(error); };
          const ready = () => { server.off('error', error); resolve(); };
          server.once('error', error); server.once('listening', ready); server.listen(port, '127.0.0.1');
        });
        break;
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== 'EADDRINUSE' || port === 0 || port >= 65535) throw error;
        port += 1;
      }
    }
  } catch (error) { clearInterval(heartbeat); unsubscribe(); throw error; }
  const address = server.address();
  if (!address || typeof address === 'string') throw new Error('The server address is unavailable.');
  return {
    url: `http://127.0.0.1:${address.port}`,
    async close() {
      unsubscribe(); clearInterval(heartbeat);
      for (const client of clients) client.end();
      server.closeIdleConnections();
      await new Promise<void>((resolve, reject) => { server.close(error => error ? reject(error) : resolve()); });
    },
  };
}

function inside(root: string, path: string): boolean {
  const child = relative(root, path);
  return child !== '..' && !child.startsWith(`..${sep}`) && !isAbsolute(child);
}
