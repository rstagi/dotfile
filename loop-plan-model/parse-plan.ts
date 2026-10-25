import type {
  MilestoneArchive,
  Plan,
  PlanMilestone,
  PlanPhase,
  PlanPhaseKind,
  PlanPhaseOutput,
  PlanPhaseStatus,
  LoopConfig,
  PlanProse,
  PlanRepository,
  ReviewConfig,
  ReviewTier,
} from "./types.ts";

const PLAN_PHASE_OUTPUTS: readonly PlanPhaseOutput[] = ["pr", "file", "doc", "output.md"];

const PLAN_STATUSES: readonly PlanPhaseStatus[] = [
  "todo",
  "in-progress",
  "blocked",
  "done",
];

const PHASE_HEADING = /^###\s+Phase\s+(\d+)\s*(.*)$/;
const MILESTONE_HEADING = /^Milestone\s+(\d+)\s*[—–-]\s*(.+)$/;
const REPOSITORY_HEADING = /^###\s+`?([^`\s]+)`?\s*$/;
const REPOSITORY_SLUG = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/;
const LANE_TAG = /\[lane:\s*([^\]]+?)\s*\]/i;
const STATUS_TAG = /\[status:\s*([^\]]+?)\s*\]/i;
const KIND_TAG = /\[kind:\s*([^\]]+?)\s*\]/i;
const REVIEW_TAG = /\[review:\s*([^\]]+?)\s*\]/i;
const ROUNDS_TAG = /\[rounds:\s*([^\]]+?)\s*\]/i;
const INNER_TAG = /\[inner:\s*([^\]]*?)\s*\]/i;
const GATE_TAG = /\[gate:\s*([^\]]+?)\s*\]/i;
const OUTPUT_TAG = /\[output:\s*([^\]]+?)\s*\]/i;
/** Archived milestone stub marker, last on the `## Milestone N — <outcome>` heading. */
const ARCHIVED_TAG = /\s*`?\[archived\]`?\s*$/i;
/** Stub bullet listing the archived milestone's phases, in document order. */
export const ARCHIVED_PHASES_LABEL = "Phases (all done)";
const ARCHIVED_TITLE = "(archived)";
const REVIEW_TIERS: ReviewTier[] = ["standard", "max", "shallow", "medium"];
/** Mirrors LOOP_REVIEW_ROUNDS_CAP in loop-models.conf. */
const REVIEW_ROUNDS_CAP = 5;

interface MilestoneSection {
  number: number;
  title: string;
  explicit: boolean;
  /** `[archived]` stub: phases come from its bullets, not from `### Phase` blocks. */
  archived: boolean;
  lines: string[];
}

/**
 * Parse the canonical multi-phase plan markdown into a normalized `Plan`.
 * Grammar: ~/.claude/skills/multiphase-plan/references/plan-format.md — the inline
 * `[lane:]` / `[status:]` markers and `Depends on:` lines are parsed, not just displayed.
 */
export function parsePlan(md: string): Plan {
  const warnings: string[] = [];
  const lines = md.split(/\r?\n/);
  const loopConfig = parseLoopConfig(sectionLines(lines, "Loop config"));
  const repositorySection = sectionLines(lines, "Repositories");
  const repositories = repositorySection
    ? parseRepositories(repositorySection, loopConfig, warnings)
    : [legacyRepository(loopConfig)];
  const { phases, milestones, explicitMilestones } = parseMilestones(lines, repositories, repositorySection != null, warnings);
  const finalPhase = phases.at(-1);
  if (phases.some((phase) => phase.kind === "pr-review") && finalPhase?.kind !== "pr-review") {
    warnings.push("Executable plans with explicit review phases must end with a PR-review phase");
  }
  if (finalPhase?.kind === "pr-review" && finalPhase.gate) {
    warnings.push(`Phase ${finalPhase.phase}: the final review must not carry [gate: ${finalPhase.gate}] — the loop would park forever`);
  }

  return {
    name: parseName(lines),
    status: parseHeaderValue(lines, "Status"),
    updatedAt: parseHeaderValue(lines, "Last updated"),
    loopConfig,
    repositories,
    milestones,
    explicitMilestones,
    phases,
    prose: parseProse(lines),
    warnings,
  };
}

// --- milestones ----------------------------------------------------------------------

function parseMilestones(
  lines: string[],
  repositories: PlanRepository[],
  multiRepository: boolean,
  warnings: string[],
): { phases: PlanPhase[]; milestones: PlanMilestone[]; explicitMilestones: boolean } {
  const sections: MilestoneSection[] = [];
  let current: MilestoneSection | null = null;
  for (const line of lines) {
    const heading = line.match(/^##\s+(.+?)\s*$/);
    if (heading) {
      const milestone = heading[1].match(MILESTONE_HEADING);
      current = milestone
        ? {
          number: Number(milestone[1]),
          title: milestone[2].replace(ARCHIVED_TAG, "").trim(),
          explicit: true,
          archived: ARCHIVED_TAG.test(milestone[2]),
          lines: [],
        }
        : heading[1] === "Phases"
          ? { number: 1, title: "Phases", explicit: false, archived: false, lines: [] }
          : null;
      if (current) sections.push(current);
      else if (/^Milestone\b/i.test(heading[1])) {
        warnings.push(`Ignored heading "${heading[1]}": milestone headings must read "## Milestone N — <outcome>"; its phases are dropped`);
      }
    } else if (current) {
      current.lines.push(line);
    }
  }

  const phases: PlanPhase[] = [];
  const milestones: PlanMilestone[] = [];
  const seenPhases = new Set<string>();
  const milestonePlan = sections.some((section) => section.explicit);
  for (const section of validateMilestoneNumbers(sections, warnings)) {
    if (section.archived) {
      const participants = archivedParticipants(section, repositories, warnings);
      const previousGate = milestones.at(-1)?.gatePhase ?? null;
      const milestonePhases = archivedPhases(section, participants, previousGate, warnings, seenPhases);
      phases.push(...milestonePhases);
      milestones.push(buildMilestone(section, milestonePhases, participants, repositories, milestonePlan, warnings));
      continue;
    }
    const milestonePhases = parsePhases(section.lines, repositories, multiRepository, warnings, section.number, seenPhases);
    phases.push(...milestonePhases);
    milestones.push(buildMilestone(section, milestonePhases, workParticipants(milestonePhases), repositories, milestonePlan, warnings));
  }
  validateMilestones(milestones, phases, milestonePlan, warnings);
  return { phases, milestones, explicitMilestones: milestonePlan };
}

/** Repositories of a milestone's work phases, in document order. */
function workParticipants(phases: PlanPhase[]): string[] {
  return [...new Set(phases
    .filter((phase) => phase.kind === "work" && phase.output === "pr" && phase.repository)
    .map((phase) => phase.repository))];
}

/** An archived stub names its participants: `- **Repositories:** \`owner/a\`, \`owner/b\``. */
function archivedParticipants(section: MilestoneSection, repositories: PlanRepository[], warnings: string[]): string[] {
  const raw = bulletValue(section.lines, "Repositories");
  const listed = raw ? [...new Set(raw.split(",").map((slug) => stripBackticks(slug)).filter((slug): slug is string => !!slug))] : [];
  if (listed.length === 0) warnings.push(`Milestone ${section.number} is archived but its stub names no Repositories`);
  for (const slug of listed) {
    if (!repositories.some((repository) => repository.slug === slug)) {
      warnings.push(`Milestone ${section.number}: archived stub names unknown repository ${slug}`);
    }
  }
  return listed;
}

/**
 * Phases of an archived stub: each listed number is an existing, `done` phase. The gate is the
 * merge-gate review; the rest are work phases. Bodies live in the archive file, so `dependsOn`
 * is a display-only chain in stub order that starts at the previous milestone's gate.
 */
function archivedPhases(
  section: MilestoneSection,
  participants: string[],
  previousGate: string | null,
  warnings: string[],
  seen: Set<string>,
): PlanPhase[] {
  const listed: string[] = bulletValue(section.lines, ARCHIVED_PHASES_LABEL)?.match(/\d+/g) ?? [];
  if (listed.length === 0) {
    warnings.push(`Milestone ${section.number} is archived but its stub has no "${ARCHIVED_PHASES_LABEL}" list`);
  }
  const gate = bulletValue(section.lines, "Gate")?.match(/\d+/)?.[0] ?? null;
  if (!gate) warnings.push(`Milestone ${section.number} is archived but its stub names no Gate phase`);
  else if (!listed.includes(gate)) {
    warnings.push(`Milestone ${section.number}: archived gate Phase ${gate} is not listed in "${ARCHIVED_PHASES_LABEL}"`);
  }

  const phases: PlanPhase[] = [];
  let previous = previousGate;
  for (const phase of listed) {
    if (seen.has(phase)) {
      warnings.push(`Milestone ${section.number}: archived stub repeats Phase ${phase} — ignoring the repeat`);
      continue;
    }
    seen.add(phase);
    const isGate = phase === gate;
    phases.push({
      phase,
      milestone: section.number,
      title: ARCHIVED_TITLE,
      kind: isGate ? "pr-review" : "work",
      gate: isGate ? "merge" : null,
      output: isGate ? null : "pr",
      lane: isGate ? "review" : "archived",
      status: "done",
      repository: isGate ? "all" : participants.length === 1 ? participants[0] : "",
      dependsOn: previous ? [previous] : [],
      suggestedBranch: null,
      touches: null,
      doneWhen: null,
      verify: null,
      notes: null,
      review: isGate ? { tier: "standard", rounds: null, inner: null } : null,
      archived: true,
    });
    previous = phase;
  }
  return phases;
}

function buildMilestone(
  section: MilestoneSection,
  phases: PlanPhase[],
  participants: string[],
  repositories: PlanRepository[],
  milestonePlan: boolean,
  warnings: string[],
): PlanMilestone {
  const firstPhase = section.lines.findIndex((line) => PHASE_HEADING.test(line));
  const header = firstPhase === -1 ? section.lines : section.lines.slice(0, firstPhase);
  const sharedBranch = stripBackticks(bulletValue(header, "Integration branch"));
  const barePr = bulletValue(header, "PR");
  const sharedPr = participants.length === 1 ? barePr : null;
  if (barePr != null && participants.length >= 2) {
    warnings.push(`Milestone ${section.number}: bare PR bullet ignored with ${participants.length} repositories — use "PR (owner/repo)"`);
  }

  return {
    number: section.number,
    title: section.title,
    gatePhase: phases.find((phase) => phase.kind === "pr-review" && phase.gate)?.phase ?? null,
    integrationBranches: Object.fromEntries(participants.map((slug) => {
      const fallback = section.number === 1
        ? repositories.find((repo) => repo.slug === slug)?.integrationBranch ?? null
        : null;
      return [slug, stripBackticks(bulletValue(header, `Integration branch (${slug})`)) ?? sharedBranch ?? fallback];
    })),
    prs: Object.fromEntries(participants.map((slug) => {
      const raw = bulletValue(header, `PR (${slug})`) ?? sharedPr;
      const fallback = milestonePlan ? null : repositories.find((repo) => repo.slug === slug)?.pr ?? null;
      return [slug, raw != null ? normalizePr(raw) : fallback];
    })),
    participants,
    ...(section.archived ? { archive: archiveRef(header) } : {}),
  };
}

/** The stub's `- **Archive:** [milestones/mN.md](milestones/mN.md)` link and one-line outcome. */
function archiveRef(header: string[]): MilestoneArchive {
  const raw = bulletValue(header, "Archive");
  const file = raw ? raw.match(/\]\(([^)]+)\)/)?.[1] ?? stripBackticks(raw) : null;
  return { file, outcome: bulletValue(header, "Outcome") };
}

/** Milestones (an implicit `## Phases` counts as 1) must be numbered 1, 2, 3… with no gaps; duplicates are dropped. */
function validateMilestoneNumbers(sections: MilestoneSection[], warnings: string[]): MilestoneSection[] {
  const kept: MilestoneSection[] = [];
  const seen = new Set<number>();
  let expected = 1;
  for (const section of sections) {
    if (seen.has(section.number)) {
      // Same policy as duplicate phases: a second entry would collide on milestone number.
      warnings.push(`Milestone ${section.number} is a duplicate milestone number — ignoring the repeat and its phases`);
      continue;
    }
    if (section.number !== expected) warnings.push(`Milestone ${section.number} is out of sequence: expected Milestone ${expected}`);
    seen.add(section.number);
    kept.push(section);
    expected = section.number + 1;
  }
  return kept;
}

function validateMilestones(
  milestones: PlanMilestone[],
  phases: PlanPhase[],
  milestonePlan: boolean,
  warnings: string[],
): void {
  const branches = new Map<string, Set<string>>();
  for (const [index, milestone] of milestones.entries()) {
    const milestonePhases = phases.filter((phase) => phase.milestone === milestone.number);
    // An archived stub already warned about its own participants; its phases carry no edges.
    if (milestonePlan && !milestone.archive && milestone.participants.length === 0
      && !milestonePhases.some((phase) => phase.kind === "work" && phase.output !== "pr")) {
      warnings.push(`Milestone ${milestone.number} has no work phases — its review has no repositories to review`);
    }
    const lastPhase = milestonePhases.at(-1);
    if (milestone !== milestones.at(-1) && (lastPhase?.kind !== "pr-review" || !lastPhase.gate)) {
      warnings.push(`Milestone ${milestone.number} must end with a PR-review phase tagged [gate: merge] or [gate: accept]`);
    }
    const previousGate = milestones[index - 1]?.gatePhase;
    if (previousGate && !milestone.archive) {
      const localPhases = new Set(milestonePhases.map((phase) => phase.phase));
      for (const phase of milestonePhases) {
        const root = !phase.dependsOn.some((dependency) => localPhases.has(dependency));
        if (root && !phase.dependsOn.includes(previousGate)) {
          warnings.push(`Milestone ${milestone.number}: root Phase ${phase.phase} must explicitly depend on gate Phase ${previousGate}`);
        }
      }
    }
    for (const slug of milestone.participants) {
      const branch = milestone.integrationBranches[slug];
      if (milestone.number >= 2 && !branch) {
        warnings.push(`Milestone ${milestone.number}: repository ${slug} must declare an Integration branch`);
      }
      if (branch) {
        const seen = branches.get(slug) ?? new Set<string>();
        if (seen.has(branch)) warnings.push(`Milestone ${milestone.number}: repository ${slug} branch "${branch}" is reused across milestones`);
        seen.add(branch);
        branches.set(slug, seen);
      }
    }
  }
}

// --- repositories --------------------------------------------------------------------

function legacyRepository(config: LoopConfig | null): PlanRepository {
  return {
    slug: "primary",
    verify: config?.verify ?? null,
    integrationBranch: config?.integrationBranch ?? null,
    pr: config?.pr ?? null,
  };
}

function parseRepositories(
  section: string[],
  config: LoopConfig | null,
  warnings: string[],
): PlanRepository[] {
  const repositories: PlanRepository[] = [];
  const seen = new Set<string>();
  let current: { slug: string; body: string[] } | null = null;
  const flush = () => {
    if (!current) return;
    const slug = current.slug;
    if (!REPOSITORY_SLUG.test(slug)) {
      warnings.push(`Invalid repository slug "${slug}" — expected owner/repo`);
      return;
    }
    if (seen.has(slug)) {
      warnings.push(`Duplicate repository ${slug} — ignoring the repeat`);
      return;
    }
    seen.add(slug);
    const verify = stripBackticks(bulletValue(current.body, "Verify"));
    if (!verify) warnings.push(`Repository ${slug}: missing Verify command`);
    repositories.push({
      slug,
      verify,
      integrationBranch:
        stripBackticks(bulletValue(current.body, "Integration branch")) ??
        config?.integrationBranch ??
        null,
      pr: normalizePr(bulletValue(current.body, "PR")),
    });
  };
  for (const line of section) {
    const match = line.match(REPOSITORY_HEADING);
    if (match) {
      flush();
      current = { slug: match[1], body: [] };
    } else if (current) {
      current.body.push(line);
    }
  }
  flush();
  if (repositories.length === 0) warnings.push("Repositories section contains no valid repositories");
  return repositories;
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

function parsePhases(
  section: string[] | null,
  repositories: PlanRepository[],
  multiRepository: boolean,
  warnings: string[],
  milestone: number,
  seen: Set<string>,
): PlanPhase[] {
  if (!section) return [];
  const phases: PlanPhase[] = [];

  // Split the section into per-phase blocks on each "### Phase N" heading.
  let current: { num: string; heading: string; body: string[] } | null = null;
  const flush = () => {
    if (!current) return;
    if (seen.has(current.num)) {
      // A duplicate `### Phase N` would produce colliding node/edge ids — drop it, loudly.
      warnings.push(`Duplicate Phase ${current.num} heading — ignoring the repeat`);
      return;
    }
    seen.add(current.num);
    phases.push(buildPhase(current, repositories, multiRepository, warnings, milestone));
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
  repositories: PlanRepository[],
  multiRepository: boolean,
  warnings: string[],
  milestone: number,
): PlanPhase {
  const laneMatch = raw.heading.match(LANE_TAG);
  const statusMatch = raw.heading.match(STATUS_TAG);
  const kind = normalizeKind(raw.heading.match(KIND_TAG)?.[1], raw.num, warnings);
  const rawGate = raw.heading.match(GATE_TAG)?.[1].toLowerCase();
  if (rawGate && kind !== "pr-review") {
    warnings.push(`Phase ${raw.num}: gate tags are only allowed on PR-review phases`);
  } else if (rawGate && rawGate !== "merge" && rawGate !== "accept") {
    warnings.push(`Phase ${raw.num}: unknown gate "${rawGate}" (merge|accept) — ignoring`);
  }
  const rawOutput = raw.heading.match(OUTPUT_TAG)?.[1];
  if (rawOutput && kind !== "work") warnings.push(`Phase ${raw.num}: output tags are only allowed on work phases`);
  const output = kind === "work" ? normalizeOutput(rawOutput, raw.num, warnings) : null;
  const title = raw.heading
    .replace(/`?\[lane:[^\]]*\]`?/i, "")
    .replace(/`?\[status:[^\]]*\]`?/i, "")
    .replace(/`?\[kind:[^\]]*\]`?/i, "")
    .replace(/`?\[review:[^\]]*\]`?/i, "")
    .replace(/`?\[rounds:[^\]]*\]`?/i, "")
    .replace(/`?\[inner:[^\]]*\]`?/i, "")
    .replace(/`?\[gate:[^\]]*\]`?/i, "")
    .replace(/`?\[output:[^\]]*\]`?/i, "")
    .replace(/^\s*[—–-]\s*/, "")
    .replace(/`/g, "")
    .trim();

  // A phase whose output is not a PR may have no repository.
  const repository = kind === "pr-review"
    ? "all"
    : output !== "pr"
      ? stripBackticks(bulletValue(raw.body, "Repository")) ?? ""
      : multiRepository
        ? stripBackticks(bulletValue(raw.body, "Repository")) ?? ""
        : "primary";
  if (output === "pr" && multiRepository && !repository) {
    warnings.push(`Phase ${raw.num}: missing Repository assignment`);
  } else if (kind === "work" && repository && !repositories.some((entry) => entry.slug === repository)) {
    warnings.push(`Phase ${raw.num}: unknown repository ${repository}`);
  }

  return {
    phase: raw.num,
    milestone,
    title: title || `Phase ${raw.num}`,
    kind,
    gate: rawGate === "merge" || rawGate === "accept" ? rawGate : null,
    output,
    lane: laneMatch ? laneMatch[1].trim() : "A",
    status: normalizeStatus(statusMatch?.[1], raw.num, warnings),
    repository,
    dependsOn: parseDependsOn(bulletValue(raw.body, "Depends on")),
    suggestedBranch: stripBackticks(bulletValue(raw.body, "Suggested branch")),
    touches: bulletValue(raw.body, "Touches"),
    doneWhen: bulletValue(raw.body, "Done when"),
    verify: stripBackticks(bulletValue(raw.body, "Verify")),
    notes: bulletValue(raw.body, "Notes"),
    review: kind === "pr-review" ? parseReviewConfig(raw.heading, raw.num, warnings) : null,
  };
}

/** Omitted or invalid limits stay null: loop-models.conf resolves them at execution time. */
function parseReviewConfig(heading: string, phaseNum: string, warnings: string[]): ReviewConfig {
  const rawTier = heading.match(REVIEW_TAG)?.[1]?.toLowerCase();
  let tier: ReviewTier = "standard";
  if (rawTier && (REVIEW_TIERS as string[]).includes(rawTier)) tier = rawTier as ReviewTier;
  else if (rawTier) warnings.push(`Phase ${phaseNum}: unknown review tier "${rawTier}" (standard|max) — using standard`);
  if (tier === "shallow" || tier === "medium") {
    warnings.push(`Phase ${phaseNum}: legacy review tier "${tier}" retained for history — use standard|max for execution`);
  }

  const rawRounds = heading.match(ROUNDS_TAG)?.[1];
  let rounds: number | null = null;
  if (rawRounds != null) {
    const n = Number(rawRounds);
    if (Number.isInteger(n) && n >= 1 && n <= REVIEW_ROUNDS_CAP) rounds = n;
    else warnings.push(`Phase ${phaseNum}: rounds must be 1..${REVIEW_ROUNDS_CAP} — using the tier default`);
  }
  const rawInner = heading.match(INNER_TAG)?.[1];
  let inner: number | null = null;
  if (rawInner != null) {
    const n = Number(rawInner);
    if (Number.isInteger(n) && n >= 1 && n <= REVIEW_ROUNDS_CAP) inner = n;
    else warnings.push(`Phase ${phaseNum}: inner must be 1..${REVIEW_ROUNDS_CAP} — using the runtime default`);
  }
  return { tier, rounds, inner };
}

function normalizeOutput(
  raw: string | undefined,
  phaseNum: string,
  warnings: string[],
): PlanPhaseOutput {
  const value = (raw ?? "pr").trim().toLowerCase();
  if ((PLAN_PHASE_OUTPUTS as readonly string[]).includes(value)) return value as PlanPhaseOutput;
  warnings.push(`Phase ${phaseNum}: unknown output "${value}" (pr|file|doc|output.md) — defaulting to pr`);
  return "pr";
}

function normalizeKind(
  raw: string | undefined,
  phaseNum: string,
  warnings: string[],
): PlanPhaseKind {
  const value = (raw ?? "work").trim().toLowerCase();
  if (value === "work" || value === "pr-review") return value;
  warnings.push(`Phase ${phaseNum}: unknown kind "${value}" — defaulting to work`);
  return "work";
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
  // Anchor on the "Phase(s)" keyword (so prose numbers like a project id are
  // ignored), then read the run of phase numbers after it: singletons ("1"),
  // enumerations ("2, 3"), and ranges ("1–4" / "1—4" / "1-3"). Dedup in order.
  const out: string[] = [];
  const seen = new Set<string>();
  const push = (n: string) => {
    if (!seen.has(n)) {
      seen.add(n);
      out.push(n);
    }
  };
  for (const kw of raw.matchAll(/Phases?\s+([\d\s,–—-]+)/gi)) {
    for (const tok of kw[1].matchAll(/(\d+)\s*[–—-]\s*(\d+)|\d+/g)) {
      if (tok[1] && tok[2]) {
        for (let n = Number(tok[1]); n <= Number(tok[2]); n++) push(String(n));
      } else {
        push(tok[0]);
      }
    }
  }
  return out;
}

// --- prose ---------------------------------------------------------------------------

/** The free-form narrative sections, tolerant of the two common heading variants. */
function parseProse(lines: string[]): PlanProse {
  return {
    goal: sectionText(lines, "Goal"),
    approach: sectionText(lines, "Approach & key decisions") ?? sectionText(lines, "Approach"),
    parallelGuide: joinSection(sectionLinesBy(lines, /^Parallel/i)),
    progressLog: sectionText(lines, "Progress log"),
  };
}

// --- section splitting & small helpers -----------------------------------------------

/** Raw prose under the `## <title>` heading (joined), or null if absent/empty. */
export function sectionText(lines: string[], title: string): string | null {
  return joinSection(sectionLines(lines, title));
}

/** Lines under the `## <title>` heading, up to (not including) the next `## ` heading. */
function sectionLines(lines: string[], title: string): string[] | null {
  return sectionLinesBy(lines, new RegExp(`^${escapeRegExp(title)}$`));
}

/** As `sectionLines`, but the `## ` heading text is matched against a regex (prefix variants). */
function sectionLinesBy(lines: string[], titleRe: RegExp): string[] | null {
  const start = lines.findIndex((l) => {
    const m = l.match(/^##\s+(.+?)\s*$/);
    return m ? titleRe.test(m[1]) : false;
  });
  if (start === -1) return null;
  const out: string[] = [];
  for (let i = start + 1; i < lines.length; i++) {
    if (/^##\s+/.test(lines[i])) break;
    out.push(lines[i]);
  }
  return out;
}

function joinSection(section: string[] | null): string | null {
  if (!section) return null;
  const text = section.join("\n").trim();
  return text || null;
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
