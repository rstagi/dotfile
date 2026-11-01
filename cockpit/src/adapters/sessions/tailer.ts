import { mkdir, open, readFile, rename, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";

export interface JsonlLine {
  offset: number;
  text: string;
}

export interface TailRead {
  lines: JsonlLine[];
  modifiedAt: number;
  reset: boolean;
}

interface Cursor {
  offset: number;
  inode: number;
  device: number;
}

type Offsets = Record<string, Cursor>;

export class JsonlTailer {
  private constructor(private readonly home: string, private readonly offsets: Offsets) {}

  static async open(home: string): Promise<JsonlTailer> {
    let offsets: Offsets = {};
    try {
      offsets = JSON.parse(await readFile(join(home, "offsets.json"), "utf8"));
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    }
    return new JsonlTailer(home, offsets);
  }

  async read(path: string): Promise<TailRead | undefined> {
    const absolute = resolve(path);
    let file;
    try {
      file = await open(absolute, "r");
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return undefined;
      throw error;
    }
    try {
      const info = await file.stat();
      const previous = this.offsets[absolute];
      const reset = !!previous && (previous.inode !== info.ino || previous.device !== info.dev || previous.offset > info.size);
      const start = !previous || reset ? 0 : previous.offset;
      const buffer = Buffer.alloc(info.size - start);
      let count = 0;
      while (count < buffer.length) {
        const { bytesRead } = await file.read(buffer, count, buffer.length - count, start + count);
        if (!bytesRead) break;
        count += bytesRead;
      }
      const complete = buffer.subarray(0, count).lastIndexOf(10) + 1;
      const lines: JsonlLine[] = [];
      let offset = 0;
      while (offset < complete) {
        const end = buffer.indexOf(10, offset);
        lines.push({ offset: start + offset, text: buffer.subarray(offset, end).toString("utf8").replace(/\r$/, "") });
        offset = end + 1;
      }
      this.offsets[absolute] = { offset: start + complete, inode: info.ino, device: info.dev };
      return { lines, modifiedAt: info.mtimeMs, reset };
    } finally {
      await file.close();
    }
  }

  // Save the offsets after the caller stores the emitted events.
  async save(): Promise<void> {
    await mkdir(this.home, { recursive: true, mode: 0o700 });
    const path = join(this.home, "offsets.json");
    await writeFile(`${path}.tmp`, JSON.stringify(this.offsets), { mode: 0o600 });
    await rename(`${path}.tmp`, path);
  }
}
