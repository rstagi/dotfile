import { parsePlan } from "./parse-plan.ts";

export type ParkResult =
  | { status: "parked"; plan: string }
  | { status: "refused"; reason: string };

const MILESTONE_HEADING = /^##\s+Milestone\s+(\d+)\s*[—–-]/;
const SECTION_HEADING = /^##\s+/;
const PHASE_HEADING = /^###\s+Phase\s+(\d+)\b/;
const REPOSITORY_HEADING = /^###\s+`?([^`\s]+)`?\s*$/;
const STATUS_TAG = /\[status:\s*[^\]]*\]/i;
const STATUS_LINE = /^\*\*Status:\*\*.*$/;
const BRANCH_OR_PR_BULLET = /^\s*[-*]\s*\*\*(Integration branch|PR)(\s+\([^)]*\))?:\*\*/;
const BARE_PR_BULLET = /^\s*[-*]\s*\*\*PR:\*\*/;

/**
 * Park milestone `number` at its merge gate: every phase of the milestone gets `[status: done]`,
 * the milestone gets one PR line per repository and the plan Status becomes `awaiting merge`.
 * Pure: the caller checks the loop state and writes the plan. Idempotent: the same input twice
 * gives the same text. `loop-plan.sh archive` needs these lines.
 * Grammar: .claude/skills/multiphase-plan/references/plan-format.md → Milestones.
 */
export function parkMilestone(planText: string, number: number, prs: Record<string, string>): ParkResult {
  const plan = parsePlan(planText);
  const milestone = plan.milestones.find((entry) => entry.number === number);
  if (!milestone) return refused(`plan has no milestone ${number}`);
  if (milestone.archive) return refused(`milestone ${number} is archived`);
  const missing = milestone.participants.filter((slug) => !prs[slug]);
  if (missing.length > 0) return refused(`no PR for ${missing.join(", ")}`);
  const extra = Object.keys(prs).filter((slug) => !milestone.participants.includes(slug));
  if (extra.length > 0) return refused(`milestone ${number} has no repository ${extra.join(", ")}`);

  const phaseIds = new Set(plan.phases.filter((phase) => phase.milestone === number).map((phase) => phase.phase));
  let lines = planText.split("\n").map((line) => {
    const phase = line.match(PHASE_HEADING)?.[1];
    if (phase === undefined || !phaseIds.has(phase)) return line;
    return STATUS_TAG.test(line) ? line.replace(STATUS_TAG, "[status: done]") : `${line.trimEnd()} \`[status: done]\``;
  });
  lines = lines.map((line) => (STATUS_LINE.test(line) ? "**Status:** awaiting merge" : line));
  lines = plan.explicitMilestones
    ? setMilestonePrs(lines, number, milestone.participants, prs)
    : setRepositoryPrs(lines, prs);
  return { status: "parked", plan: lines.join("\n") };
}

// A milestone with one participant uses the bare `PR:` bullet when it has one.
function setMilestonePrs(lines: string[], number: number, participants: string[], prs: Record<string, string>): string[] {
  const start = lines.findIndex((line) => Number(line.match(MILESTONE_HEADING)?.[1]) === number);
  const next = lines.findIndex((line, index) => index > start && SECTION_HEADING.test(line));
  const end = next === -1 ? lines.length : next;
  const firstPhase = lines.findIndex((line, index) => index > start && index < end && PHASE_HEADING.test(line));
  const headerEnd = firstPhase === -1 ? end : firstPhase;
  const out = lines.slice(0, start + 1);
  let header = lines.slice(start + 1, headerEnd);
  for (const slug of participants) {
    const bare = participants.length === 1;
    const label = new RegExp(`^\\s*[-*]\\s*\\*\\*PR \\(${escapeRegExp(slug)}\\):\\*\\*`);
    const line = bare ? `- **PR:** ${prs[slug]}` : `- **PR (${slug}):** ${prs[slug]}`;
    const index = header.findIndex((entry) => label.test(entry) || (bare && BARE_PR_BULLET.test(entry)));
    if (index !== -1) {
      header[index] = line;
      continue;
    }
    let last = -1;
    header.forEach((entry, at) => { if (BRANCH_OR_PR_BULLET.test(entry)) last = at; });
    header = [...header.slice(0, last + 1), line, ...header.slice(last + 1)];
  }
  return [...out, ...header, ...lines.slice(headerEnd)];
}

// A plan without milestone sections keeps its PR on the `### owner/repo` block of `## Repositories`.
function setRepositoryPrs(lines: string[], prs: Record<string, string>): string[] {
  const start = lines.findIndex((line) => /^##\s+Repositories\s*$/.test(line));
  if (start === -1) return lines;
  const next = lines.findIndex((line, index) => index > start && SECTION_HEADING.test(line));
  const end = next === -1 ? lines.length : next;
  const out = lines.slice(0, start + 1);
  let block: string[] = [];
  let slug: string | null = null;
  const flush = () => {
    if (slug !== null && prs[slug]) {
      const line = `- **PR:** ${prs[slug]}`;
      const index = block.findIndex((entry) => BARE_PR_BULLET.test(entry));
      if (index !== -1) block[index] = line;
      else {
        let last = 0;
        block.forEach((entry, at) => { if (/^\s*[-*]\s+\S/.test(entry)) last = at; });
        block.splice(last + 1, 0, line);
      }
    }
    out.push(...block);
    block = [];
  };
  for (const line of lines.slice(start + 1, end)) {
    const heading = line.match(REPOSITORY_HEADING)?.[1];
    if (heading !== undefined) {
      flush();
      slug = heading;
    }
    block.push(line);
  }
  flush();
  return [...out, ...lines.slice(end)];
}

function escapeRegExp(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function refused(reason: string): { status: "refused"; reason: string } {
  return { status: "refused", reason };
}
