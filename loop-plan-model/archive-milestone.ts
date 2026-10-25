import { ARCHIVED_PHASES_LABEL, parsePlan } from "./parse-plan.ts";
import type { PlanMilestone, PlanPhase } from "./types.ts";

export type ArchiveResult =
  | {
    status: "archived";
    /** The new plan.md text: the milestone section replaced by its stub, its log entries gone. */
    plan: string;
    /** The archive file text: the milestone section and its log entries, moved without changes. */
    archive: string;
    /** Archive path relative to plan.md. */
    archiveFile: string;
    /** Gate PR per participant — the caller checks that each one is merged before it writes. */
    prs: Record<string, string>;
  }
  | { status: "already"; archiveFile: string | null }
  | { status: "refused"; reason: string };

export type AttachResult =
  | {
    status: "attached";
    /** The plan text with the stub Outcome taken from the retro (or `outcome`). */
    plan: string;
    /** The archive text: header, the retro, then `## Raw log` with the archived content unchanged. */
    archive: string;
    archiveFile: string;
  }
  | { status: "refused"; reason: string };

export interface ArchiveOptions {
  /** One-line outcome for the stub. Default: the retro Outcome bullet, else the milestone title. */
  outcome?: string;
  /** The pending `milestones/m<N>.retro.md` text. It goes first in the archive, before `## Raw log`. */
  retro?: string;
}

const MILESTONE_HEADING = /^##\s+Milestone\s+(\d+)\s*[—–-]/;
const SECTION_HEADING = /^##\s+/;
const PHASE_HEADING = /^###\s+Phase\s+\d+/;
const BRANCH_OR_PR_BULLET = /^\s*[-*]\s*\*\*(Integration branch|PR)(\s+\([^)]*\))?:\*\*/;
const LOG_ENTRY = /^[-*]\s+/;
/** A log entry is about one milestone when its subject (after the date) is that milestone or one of its phases. */
const LOG_SUBJECT = /^[-*]\s+(?:\S+\s+[—–-]\s+)?(?:Phase\s+(\d+)\b|Milestone\s+(\d+)\b|M(\d+)\b)/;
const RETRO_HEADING = "## Analysis and recap";
const RAW_LOG_HEADING = "## Raw log";
const OUTCOME_BULLET = /^\s*[-*]\s+\*\*Outcome:\*\*\s*(.*)$/;
const STUB_OUTCOME = /^- \*\*Outcome:\*\*.*$/;

/**
 * Archive shipped milestone `number`: move its section (heading, branch/PR lines, phase blocks)
 * and its progress-log entries, unchanged, into the archive text, and leave a stub section in
 * the plan. Pure: the caller checks the gate PRs and writes both files atomically.
 * Grammar: .claude/skills/multiphase-plan/references/plan-format.md → Milestones → Archived milestones.
 */
export function archiveMilestone(planText: string, number: number, opts: ArchiveOptions = {}): ArchiveResult {
  const plan = parsePlan(planText);
  const milestone = plan.explicitMilestones ? plan.milestones.find((entry) => entry.number === number) : undefined;
  if (!milestone) return refused(`plan has no milestone ${number}`);
  if (milestone.archive) return { status: "already", archiveFile: milestone.archive.file };
  const phases = plan.phases.filter((phase) => phase.milestone === number);
  const open = phases.filter((phase) => phase.status !== "done");
  if (open.length > 0) {
    return refused(`milestone ${number} has phases that are not done: ${open.map((phase) => `Phase ${phase.phase} (${phase.status})`).join(", ")}`);
  }
  if (!milestone.gatePhase) return refused(`milestone ${number} has no [gate: merge] phase`);
  const missing = milestone.participants.filter((slug) => !milestone.prs[slug]);
  if (missing.length > 0) return refused(`milestone ${number} has no PR link for ${missing.join(", ")}`);

  const lines = planText.split("\n");
  const start = lines.findIndex((line) => Number(line.match(MILESTONE_HEADING)?.[1]) === number);
  const next = lines.findIndex((line, index) => index > start && SECTION_HEADING.test(line));
  const end = next === -1 ? lines.length : next;
  let bodyEnd = end;
  while (bodyEnd > start + 1 && lines[bodyEnd - 1].trim() === "") bodyEnd--;
  const section = lines.slice(start, bodyEnd);
  const archiveFile = `milestones/m${number}.md`;
  const moved = milestoneLogEntries(lines, number, new Set(phases.map((phase) => phase.phase)));

  const outcome = opts.outcome ?? (opts.retro != null ? retroOutcome(opts.retro) : undefined);
  const out: string[] = [];
  for (let index = 0; index < lines.length; index++) {
    if (index === start) {
      out.push(...stub(section, milestone, phases, archiveFile, outcome));
      index = bodyEnd - 1;
    } else if (!moved.has(index)) {
      out.push(lines[index]);
    }
  }

  const raw = archiveText(planText, plan.name, number, section, [...moved].map((index) => lines[index]));
  return {
    status: "archived",
    plan: out.join("\n"),
    archive: opts.retro != null ? withRetro(raw, opts.retro) : raw,
    archiveFile,
    prs: milestone.prs as Record<string, string>,
  };
}

/**
 * Attach a pending retro to an archived milestone: the archive gets the retro before `## Raw log`
 * (a newer retro replaces an attached one), and the stub Outcome takes `outcome` or the retro
 * Outcome bullet. Pure: the caller writes both files.
 */
export function attachRetro(planText: string, archive: string, number: number, retro: string, opts: { outcome?: string } = {}): AttachResult {
  const plan = parsePlan(planText);
  const milestone = plan.explicitMilestones ? plan.milestones.find((entry) => entry.number === number) : undefined;
  if (!milestone) return refused(`plan has no milestone ${number}`);
  if (!milestone.archive) return refused(`milestone ${number} is not archived`);
  const outcome = opts.outcome ?? retroOutcome(retro);
  const lines = planText.split("\n");
  const start = lines.findIndex((line) => Number(line.match(MILESTONE_HEADING)?.[1]) === number);
  const next = lines.findIndex((line, index) => index > start && SECTION_HEADING.test(line));
  const end = next === -1 ? lines.length : next;
  const newPlan = lines.map((line, index) => (outcome && index > start && index < end && STUB_OUTCOME.test(line) ? `- **Outcome:** ${oneLine(outcome)}` : line));
  return { status: "attached", plan: newPlan.join("\n"), archive: withRetro(archive, retro), archiveFile: milestone.archive.file ?? `milestones/m${number}.md` };
}

/** The text of the retro's `- **Outcome:**` bullet, continuation lines included, or undefined. */
function retroOutcome(retro: string): string | undefined {
  const lines = retro.split("\n");
  const index = lines.findIndex((line) => OUTCOME_BULLET.test(line));
  if (index === -1) return undefined;
  const parts = [lines[index].match(OUTCOME_BULLET)![1]];
  for (let next = index + 1; next < lines.length && /^\s+\S/.test(lines[next]) && !/^\s*[-*]\s/.test(lines[next]); next++) parts.push(lines[next]);
  return oneLine(parts.join(" ")) || undefined;
}

/**
 * Archive text with the retro first: the file header (everything before the first `## ` line),
 * the retro, `## Raw log`, then the archived content. A retro that is already there is replaced.
 */
function withRetro(archive: string, retro: string): string {
  const lines = archive.split("\n");
  const first = lines.findIndex((line) => SECTION_HEADING.test(line));
  const header = first === -1 ? lines : lines.slice(0, first);
  const rawAt = lines.indexOf(RAW_LOG_HEADING);
  let raw = first === -1 ? [] : lines.slice(rawAt === -1 ? first : rawAt + 1);
  while (raw.length > 0 && raw[0].trim() === "") raw = raw.slice(1);
  let body = retro.trim();
  if (!body.startsWith(RETRO_HEADING)) body = `${RETRO_HEADING}\n${body}`;
  return [...header, body, "", RAW_LOG_HEADING, "", ...raw].join("\n");
}

function oneLine(text: string): string {
  return text.replace(/\s+/g, " ").trim();
}

function stub(section: string[], milestone: PlanMilestone, phases: PlanPhase[], archiveFile: string, outcome?: string): string[] {
  const firstPhase = section.findIndex((line) => PHASE_HEADING.test(line));
  const header = firstPhase === -1 ? section : section.slice(0, firstPhase);
  return [
    `${section[0].trimEnd()} \`[archived]\``,
    ...header.filter((line) => BRANCH_OR_PR_BULLET.test(line)),
    `- **Outcome:** ${(outcome && oneLine(outcome)) || milestone.title}`,
    `- **Repositories:** ${milestone.participants.map((slug) => `\`${slug}\``).join(", ")}`,
    `- **${ARCHIVED_PHASES_LABEL}:** ${phases.map((phase) => phase.phase).join(", ")}`,
    `- **Gate:** Phase ${milestone.gatePhase}`,
    `- **Archive:** [${archiveFile}](${archiveFile})`,
  ];
}

/** Line indexes (in order) of the `## Progress log` entries about this milestone, continuation lines included. */
function milestoneLogEntries(lines: string[], number: number, phaseIds: Set<string>): Set<number> {
  const moved = new Set<number>();
  const start = lines.findIndex((line) => /^##\s+Progress log\s*$/.test(line));
  if (start === -1) return moved;
  let inEntry = false;
  for (let index = start + 1; index < lines.length && !SECTION_HEADING.test(lines[index]); index++) {
    const line = lines[index];
    if (LOG_ENTRY.test(line)) {
      const subject = line.match(LOG_SUBJECT);
      const milestoneRef = subject?.[2] ?? subject?.[3];
      inEntry = subject != null && (subject[1] != null ? phaseIds.has(subject[1]) : Number(milestoneRef) === number);
    } else if (!/^\s+\S/.test(line)) {
      inEntry = false;
    }
    if (inEntry) moved.add(index);
  }
  return moved;
}

function archiveText(planText: string, name: string, number: number, section: string[], entries: string[]): string {
  const planId = planText.match(/^planId:\s*(\S+)/m)?.[1];
  const header = [
    "<!-- loop-milestone-archive",
    ...(planId ? [`planId: ${planId}`] : []),
    `milestone: ${number}`,
    "-->",
    "",
    `# ${name} — Milestone ${number} archive`,
    "",
    `\`loop-plan.sh archive --milestone ${number}\` moved this milestone here from [../plan.md](../plan.md) without changes.`,
    "",
  ];
  const log = entries.length > 0 ? ["", "## Progress log", ...entries] : [];
  return [...header, ...section, ...log].join("\n") + "\n";
}

function refused(reason: string): { status: "refused"; reason: string } {
  return { status: "refused", reason };
}
