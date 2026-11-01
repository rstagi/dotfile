import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import type { CockpitEvent } from "../../core/events.ts";
import type { SessionSnapshot } from "./session.ts";
import { adaptClaude } from "./claude.ts";
import { adaptCodex } from "./codex.ts";
import { regularFiles, sessionMedia, writtenMediaPaths } from "./media.ts";
import { JsonlTailer } from "./tailer.ts";

export interface SessionReaderOptions {
  home?: string;
  userHome?: string;
  now?: () => number;
}

interface ObservedSession {
  session: SessionSnapshot;
  media: Record<string, string>;
  writtenPaths: string[];
}

type Sessions = Record<string, ObservedSession>;

export class SessionReader {
  private constructor(
    private readonly home: string,
    private readonly userHome: string,
    private readonly now: () => number,
    private readonly tailer: JsonlTailer,
    private readonly sessions: Sessions,
  ) {}

  static async open(options: SessionReaderOptions = {}): Promise<SessionReader> {
    const home = options.home ?? process.env.COCKPIT_HOME ?? join(homedir(), ".cockpit");
    let sessions: Sessions = {};
    try {
      sessions = JSON.parse(await readFile(join(home, "sessions.json"), "utf8"));
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    }
    return new SessionReader(home, options.userHome ?? homedir(), options.now ?? Date.now, await JsonlTailer.open(home), sessions);
  }

  snapshot(): SessionSnapshot[] {
    return structuredClone(Object.values(this.sessions).map(item => item.session));
  }

  async poll(): Promise<CockpitEvent[]> {
    const events: CockpitEvent[] = [];
    for (const source of ["claude", "codex"] as const) {
      const root = source === "claude" ? join(this.userHome, ".claude/projects") : join(this.userHome, ".codex/sessions");
      for (const path of await regularFiles(root)) {
        if (!path.endsWith(".jsonl")) continue;
        const read = await this.tailer.read(path);
        if (!read) continue;
        const previous = this.sessions[path];
        const context = { path, userHome: this.userHome, modifiedAt: read.modifiedAt, now: this.now() };
        const adapt = source === "claude" ? adaptClaude : adaptCodex;
        const result = adapt(read.lines, context, read.reset ? undefined : previous?.session);
        const writtenPaths = [...new Set([...(previous?.writtenPaths ?? []), ...writtenMediaPaths(result.events, result.state.meta.cwd)])];
        const media = await sessionMedia(result.state, this.userHome, writtenPaths, previous?.media ?? {});
        this.sessions[path] = { session: result.state, media: media.signatures, writtenPaths };
        events.push(...result.events, ...media.events);
      }
    }
    return events;
  }

  // Call this method after the caller appends all poll events to the event store.
  async save(): Promise<void> {
    await mkdir(this.home, { recursive: true, mode: 0o700 });
    const path = join(this.home, "sessions.json");
    await writeFile(`${path}.tmp`, JSON.stringify(this.sessions), { mode: 0o600 });
    await rename(`${path}.tmp`, path);
    await this.tailer.save();
  }
}
