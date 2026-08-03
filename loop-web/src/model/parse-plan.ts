import type { Plan, PlanPhase, PlanPhaseStatus, LoopConfig } from "./types.ts";

const PLAN_STATUSES: readonly PlanPhaseStatus[] = [
  "todo",
  "in-progress",
  "blocked",
  "done",
];

const PHASE_HEADING = /^###\s+Phase\s+(\d+)\s*(.*)$/;
const LANE_TAG = /\[lane:\s*([^\]]+?)\s*\]/i;
const STATUS_TAG = /\[status:\s*([^\]]+?)\s*\]/i;

/**
 * Parse the canonical multi-phase plan markdown into a normalized `Plan`.
 * Grammar: ~/.claude/skills/multiphase-plan/references/plan-format.md — the inline
 * `[lane:]` / `[status:]` markers and `Depends on:` lines are parsed, not just displayed.
 */
export function parsePlan(md: string): Plan {
  const warnings: string[] = [];
  const lines = md.split(/\r?\n/);

  return {
    name: parseName(lines),
    status: parseHeaderValue(lines, "Status"),
    updatedAt: parseHeaderValue(lines, "Last updated"),
    loopConfig: parseLoopConfig(sectionLines(lines, "Loop config")),
    phases: parsePhases(sectionLines(lines, "Phases"), warnings),
    warnings,
  };
}

// --- header --------------------------------------------------------------------------

function parseName(lines: string[]): string {
  const h1 = lines.find((l) => /^#\s+/.test(l));
  if (!h1) return "Untitled effort";
  const text = h1.replace(/^#\s+/, "").trim();
  // Canonical form is "<Effort name> — Multi-Phase Plan"; keep the part before the em dash.
  const parts = text.split(/\s+[—–-]\s+/);
  return (parts[0] || text).trim();
}

/** Read a `**Marker:** value` line's value (first match), or null. */
function parseHeaderValue(lines: string[], marker: string): string | null {
  const re = new RegExp(`^\\*\\*${escapeRegExp(marker)}:\\*\\*\\s*(.+?)\\s*$`);
  for (const l of lines) {
    const m = l.match(re);
    if (m) return m[1].trim();
  }
  return null;
}

// --- loop config ---------------------------------------------------------------------

function parseLoopConfig(section: string[] | null): LoopConfig | null {
  if (!section) return null;
  const branch = bulletValue(section, "Integration branch");
  const verify = bulletValue(section, "Verify");
  const prRaw = bulletValue(section, "PR");
  const concurrencyRaw = bulletValue(section, "Concurrency");
  const concurrency = concurrencyRaw ? parseInt(concurrencyRaw, 10) : NaN;
  return {
    integrationBranch: stripBackticks(branch),
    verify: stripBackticks(verify),
    pr: normalizePr(prRaw),
    concurrency: Number.isFinite(concurrency) ? concurrency : null,
  };
}

/** Read a `- **Label:** value` bullet value from a section. */
function bulletValue(section: string[], label: string): string | null {
  const re = new RegExp(
    `^\\s*[-*]\\s*\\*\\*${escapeRegExp(label)}:\\*\\*\\s*(.+?)\\s*$`,
  );
  for (const l of section) {
    const m = l.match(re);
    if (m) return m[1].trim();
  }
  return null;
}

function normalizePr(raw: string | null): string | null {
  if (!raw) return null;
  if (/_none( yet)?_/i.test(raw) || /^(none|tbd|n\/a)$/i.test(raw.trim())) return null;
  return extractUrl(raw);
}

// --- phases --------------------------------------------------------------------------

function parsePhases(section: string[] | null, warnings: string[]): PlanPhase[] {
  if (!section) return [];
  const phases: PlanPhase[] = [];

  // Split the section into per-phase blocks on each "### Phase N" heading.
  const seen = new Set<string>();
  let current: { num: string; heading: string; body: string[] } | null = null;
  const flush = () => {
    if (!current) return;
    if (seen.has(current.num)) {
      // A duplicate `### Phase N` would produce colliding node/edge ids — drop it, loudly.
      warnings.push(`Duplicate Phase ${current.num} heading — ignoring the repeat`);
      return;
    }
    seen.add(current.num);
    phases.push(buildPhase(current, warnings));
  };
  for (const line of section) {
    const m = line.match(PHASE_HEADING);
    if (m) {
      flush();
      current = { num: m[1], heading: m[2], body: [] };
    } else if (current) {
      current.body.push(line);
    }
  }
  flush();
  return phases;
}

function buildPhase(
  raw: { num: string; heading: string; body: string[] },
  warnings: string[],
): PlanPhase {
  const laneMatch = raw.heading.match(LANE_TAG);
  const statusMatch = raw.heading.match(STATUS_TAG);
  const title = raw.heading
    .replace(/`?\[lane:[^\]]*\]`?/i, "")
    .replace(/`?\[status:[^\]]*\]`?/i, "")
    .replace(/^\s*[—–-]\s*/, "")
    .replace(/`/g, "")
    .trim();

  return {
    phase: raw.num,
    title: title || `Phase ${raw.num}`,
    lane: laneMatch ? laneMatch[1].trim() : "A",
    status: normalizeStatus(statusMatch?.[1], raw.num, warnings),
    dependsOn: parseDependsOn(bulletValue(raw.body, "Depends on")),
    suggestedBranch: stripBackticks(bulletValue(raw.body, "Suggested branch")),
    touches: bulletValue(raw.body, "Touches"),
    doneWhen: bulletValue(raw.body, "Done when"),
    verify: stripBackticks(bulletValue(raw.body, "Verify")),
    taskUrl: extractUrl(bulletValue(raw.body, "Task")),
    notes: bulletValue(raw.body, "Notes"),
  };
}

function normalizeStatus(
  raw: string | undefined,
  phaseNum: string,
  warnings: string[],
): PlanPhaseStatus {
  const v = (raw ?? "").trim().toLowerCase();
  if ((PLAN_STATUSES as readonly string[]).includes(v)) return v as PlanPhaseStatus;
  if (v) warnings.push(`Phase ${phaseNum}: unknown status "${v}" — defaulting to todo`);
  return "todo";
}

function parseDependsOn(raw: string | null): string[] {
  if (!raw || /^\s*none\b/i.test(raw)) return [];
  return Array.from(raw.matchAll(/Phase\s+(\d+)/gi)).map((m) => m[1]);
}

// --- section splitting & small helpers -----------------------------------------------

/** Lines under the `## <title>` heading, up to (not including) the next `## ` heading. */
function sectionLines(lines: string[], title: string): string[] | null {
  const start = lines.findIndex((l) =>
    new RegExp(`^##\\s+${escapeRegExp(title)}\\s*$`).test(l),
  );
  if (start === -1) return null;
  const out: string[] = [];
  for (let i = start + 1; i < lines.length; i++) {
    if (/^##\s+/.test(lines[i])) break;
    out.push(lines[i]);
  }
  return out;
}

function stripBackticks(v: string | null): string | null {
  if (v == null) return null;
  const t = v.replace(/`/g, "").trim();
  return t || null;
}

/** Pull a URL out of a markdown link `[text](url)` or a bare URL. */
function extractUrl(v: string | null): string | null {
  if (!v) return null;
  const link = v.match(/\]\((https?:\/\/[^)]+)\)/);
  if (link) return link[1];
  const bare = v.match(/https?:\/\/\S+/);
  return bare ? bare[0] : null;
}

function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
