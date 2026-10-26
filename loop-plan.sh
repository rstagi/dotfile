#!/bin/zsh
set -u -o pipefail

# Loop Engineering — plan helper: register/push a multi-phase plan on the central Loop
# Observatory daemon (the sole backend for EVERY plan) and post progress notes to it.
#
# Usage:
#   loop-plan.sh register [--dir .loop] [--plan-file <f>] [--plan-id <id>] [--effort <name>]
#     mkdir the loop dir, resolve/mint the planId, ensure the daemon is up, POST /register
#     (the server reads planText from the plan file). Prints the planId. Idempotent.
#   loop-plan.sh push [--dir .loop] [--plan-file <f>] [--plan-id <id>] [--effort <name>]
#     alias of register — re-POSTing /register re-reads the plan file, so the daemon copy
#     is overwritten with the current markdown.
#   loop-plan.sh get --plan-id <id> [--out <f>]
#     GET /api/loops/<id>/plan and print (or write) its planText — how a fresh worktree
#     fetches the plan with no local copy.
#   loop-plan.sh list
#     GET /api/loops → a compact `runId · status · effort` table.
#   loop-plan.sh note --plan-id <id> [--phase <n>] --body <text>
#     POST a progress.note event — a phase progress comment,
#     visible cross-worktree on the loop timeline (jq-built, injection-safe).
#   loop-plan.sh archive --milestone <N> [--dir .loop] [--plan-file <f>] [--outcome <text>]
#     Move shipped milestone N out of plan.md: its section (branch/PR lines + phase blocks) and
#     its progress-log entries go unchanged into <plan dir>/milestones/m<N>.md; plan.md keeps a
#     stub (grammar: multiphase-plan/references/plan-format.md → Archived milestones). Then push.
#     Refuses unless every phase of N is done and every gate PR in the plan is merged (the
#     loop-milestone.sh check rule). Atomic: plan.md changes by one rename, last; any failure
#     leaves it unchanged. Idempotent: an archived milestone is a no-op (no write, no push).
#     A pending <plan dir>/milestones/m<N>.retro.md goes first in the archive file, before
#     `## Raw log`. Its Outcome bullet becomes the stub Outcome unless --outcome is given. The
#     command removes the retro file after the write. For an archived milestone, a pending retro
#     is attached to the existing archive file (not a no-op).
#     Exit 0 archived, attached or already archived; 3 refused (phase not done, no gate, no PR
#     link, PR open); 4 a gate PR closed unmerged; 1 usage, gh or write error. Needs Node >= 22.18.

DIR=".loop"
CMD="${1:-}"; [[ -n "$CMD" ]] && shift

PLAN_FILE="" PLAN_ID="" EFFORT="" OUT="" PHASE="" BODY="" MILESTONE="" OUTCOME=""
while [[ $# -gt 0 ]]; do
  case "$1" in
  --dir) DIR="$2"; shift 2 ;;
  --plan-file) PLAN_FILE="$2"; shift 2 ;;
  --plan-id) PLAN_ID="$2"; shift 2 ;;
  --effort) EFFORT="$2"; shift 2 ;;
  --out) OUT="$2"; shift 2 ;;
  --phase) PHASE="$2"; shift 2 ;;
  --body) BODY="$2"; shift 2 ;;
  --milestone) MILESTONE="$2"; shift 2 ;;
  --outcome) OUTCOME="$2"; shift 2 ;;
  *) echo "loop-plan: unknown arg $1" >&2; exit 1 ;;
  esac
done

die() { echo "loop-plan: $1" >&2; exit "${2:-1}"; }

# Best-effort daemon emission (sourced; LOOP_EMIT_SH is a test seam like loop-orchestrator.sh).
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
EMIT_SH="${LOOP_EMIT_SH:-$SCRIPT_DIR/loop-emit.sh}"
if [[ -r "$EMIT_SH" ]]; then source "$EMIT_SH"; else loop_emit() { :; }; loop_ensure_daemon() { :; }; fi
# Always resolvable for the read paths (get/list) even when the fake emit stub omits it.
: "${LOOP_DAEMON_URL:=http://localhost:7717}"

now_iso() { date -u +%Y-%m-%dT%H:%M:%SZ; }

# Absolute path of a file whose parent dir exists (parent is cd-able); prints as-is otherwise.
abs_path() {
  local d b
  d="$(dirname "$1")"; b="$(basename "$1")"
  if d="$(cd "$d" 2>/dev/null && pwd)"; then printf '%s/%s' "$d" "$b"; else printf '%s' "$1"; fi
}

# planId from the `<!-- loop-plan ... -->` header of a plan file (first match), or empty.
header_plan_id() { [[ -f "$1" ]] && sed -n 's/^planId:[[:space:]]*//p' "$1" | head -1 || true; }

# Read the engine only from the canonical header.
header_engine() {
  [[ -f "$1" ]] || return 0
  awk 'NR == 1 && $0 != "<!-- loop-plan" {exit}
    /^-->$/ {exit}
    /^engine:/ {sub(/^engine:[[:space:]]*/, ""); sub(/[[:space:]]*$/, ""); print; exit}' "$1"
}

# Append the exact file text. The SHA includes every final newline.
append_ws_plan() {
  local target="$1" plan="$2" sha body
  sha="$(shasum -a 256 < "$plan" | cut -d' ' -f1)" || return 3
  body="$(jq -n --rawfile text "$plan" --arg sha "$sha" '{text: $text, sha: $sha}')" || return 3
  "${WS_BIN:-ws}" event loop.plan --target "$target" --source tool --body "$body" >/dev/null || return 3
}

# Mint a runId-scheme planId: loop-<effort-slug>-<date>-<HHMMSS>.
mint_plan_id() {
  local slug
  slug="$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]' | sed -E 's/[^a-z0-9]+/-/g; s/^-+//; s/-+$//')"
  [[ -n "$slug" ]] || slug="loop"
  printf 'loop-%s-%s-%s' "$slug" "$(date -u +%Y-%m-%d)" "$(date -u +%H%M%S)"
}

# Resolve the planId: explicit flag > header > mint. When minted against a plan file that has a
# `<!-- loop-plan` header but no planId line, write it back so push stays on the same record.
resolve_plan_id() {
  local pid
  if [[ -n "$PLAN_ID" ]]; then printf '%s' "$PLAN_ID"; return; fi
  pid="$(header_plan_id "$PLAN_FILE")"
  if [[ -n "$pid" ]]; then printf '%s' "$pid"; return; fi
  pid="$(mint_plan_id "$EFFORT")"
  if [[ -f "$PLAN_FILE" ]] && grep -q '^<!-- loop-plan' "$PLAN_FILE" && ! grep -q '^planId:' "$PLAN_FILE"; then
    local tmp; tmp="$(mktemp)"
    awk -v id="$pid" 'BEGIN{d=0} /^<!-- loop-plan[[:space:]]*$/ && !d {print; print "planId: " id; d=1; next} {print}' \
      "$PLAN_FILE" > "$tmp" && mv "$tmp" "$PLAN_FILE" || rm -f "$tmp"
  fi
  printf '%s' "$pid"
}

# register + push share this: resolve id, POST /register (server reads planText from planFile).
do_register() {
  mkdir -p "$DIR"
  local loop_abs plan_abs pid
  loop_abs="$(cd "$DIR" && pwd)" || die "cannot resolve --dir $DIR"
  if [[ -n "$PLAN_FILE" ]]; then plan_abs="$(abs_path "$PLAN_FILE")"; else plan_abs="$loop_abs/plan.md"; PLAN_FILE="$plan_abs"; fi
  pid="$(resolve_plan_id)"
  [[ -n "$pid" ]] || die "could not resolve a planId"
  if [[ "$(header_engine "$plan_abs")" == skills ]]; then
    local session
    session="$("${WS_BIN:-ws}" loop create --dir "$loop_abs" --plan-id "$pid")" || return 3
    append_ws_plan "$session" "$plan_abs" || return 3
    printf '%s\n' "$pid"
    return 0
  fi
  loop_ensure_daemon
  jq -cn --arg loopDir "$loop_abs" --arg planFile "$plan_abs" --arg effort "$EFFORT" \
    --arg started "$(now_iso)" \
    '{loopDir:$loopDir, planFile:$planFile,
      effort:(if ($effort|length)>0 then $effort else null end), startedAt:$started}' \
    | loop_emit "$pid" register
  printf '%s\n' "$pid"
}

# archive: compute in a temp dir beside plan.md → check the gate PRs → place the archive file →
# rename the new plan.md over the old one (the only change to plan.md) → remove the attached
# retro → push. A pending milestones/m<N>.retro.md goes first in the archive file.
ARCHIVE_TMP=""
do_archive() {
  [[ "$MILESTONE" =~ '^[1-9][0-9]*$' ]] || die "archive requires --milestone <N>"
  local plan="${PLAN_FILE:-$DIR/plan.md}" result plan_sum target retro created=false
  local -a extra=()
  [[ -f "$plan" ]] || die "archive: no plan at $plan"
  plan_sum="$(shasum < "$plan")"
  retro="${plan:h}/milestones/m${MILESTONE}.retro.md"
  [[ -n "$OUTCOME" ]] && extra+=(--outcome "$OUTCOME")
  [[ -f "$retro" ]] && extra+=(--retro "$retro")
  node --version >/dev/null 2>&1 || die "archive: cannot start node (needs Node >= 22.18)"
  ARCHIVE_TMP="$(mktemp -d "${plan:h}/.archive.XXXXXX" 2>/dev/null)" || die "archive: cannot create a temp dir in ${plan:h}"
  result="$(node "$SCRIPT_DIR/loop-plan-archive.mjs" --plan "$plan" --milestone "$MILESTONE" \
    --out "$ARCHIVE_TMP" "${extra[@]}")" \
    || return $?
  case "$(jq -r '.status' <<< "$result")" in
  already)
    print -r -- "loop-plan: milestone $MILESTONE already archived ($(jq -r '.archiveFile // "?"' <<< "$result"))"
    return 0 ;;
  refused) die "archive refused: $(jq -r '.reason' <<< "$result")" 3 ;;
  attached) attach_retro "$plan" "$plan_sum" "${plan:h}/$(jq -r '.archiveFile' <<< "$result")" "$retro"; return ;;
  archived) ;;
  *) die "archive: unexpected result: $result" ;;
  esac
  check_gate_prs "$result"

  target="${plan:h}/$(jq -r '.archiveFile' <<< "$result")"
  mkdir -p "${target:h}" 2>/dev/null || die "archive: cannot create ${target:h}"
  if [[ -e "$target" ]]; then
    cmp -s "$ARCHIVE_TMP/archive.md" "$target" || die "archive: $target exists with other content; move it away first"
  else
    mv "$ARCHIVE_TMP/archive.md" "$target" 2>/dev/null || die "archive: cannot write $target"
    created=true
  fi
  if [[ "$(shasum < "$plan")" != "$plan_sum" ]] || ! mv "$ARCHIVE_TMP/plan.md" "$plan" 2>/dev/null; then
    $created && rm -f "$target"
    die "archive: $plan changed or cannot be replaced; nothing archived"
  fi
  rm -f "$retro"
  print -r -- "loop-plan: archived milestone $MILESTONE → $target"
  PLAN_FILE="$plan"
  do_register >/dev/null
}

# attach_retro <plan> <plan sum> <archive file> <retro>: milestone N is archived and a retro is
# pending. Replace the archive file (keep the old one until plan.md is replaced), replace plan.md,
# remove the retro, push.
attach_retro() {
  local plan="$1" plan_sum="$2" target="$3" retro="$4"
  cp -p "$target" "$ARCHIVE_TMP/archive.old" 2>/dev/null || die "archive: cannot read $target"
  if [[ "$(shasum < "$plan")" != "$plan_sum" ]] || ! mv "$ARCHIVE_TMP/archive.md" "$target" 2>/dev/null; then
    die "archive: $plan changed or $target cannot be replaced; no retro attached"
  fi
  if ! mv "$ARCHIVE_TMP/plan.md" "$plan" 2>/dev/null; then
    mv "$ARCHIVE_TMP/archive.old" "$target"
    die "archive: $plan cannot be replaced; no retro attached"
  fi
  rm -f "$retro"
  print -r -- "loop-plan: attached the milestone $MILESTONE retro → $target"
  PLAN_FILE="$plan"
  do_register >/dev/null
}

# The loop-milestone.sh check rule on the plan's gate PRs: one "slug<TAB>url<TAB>state" row each;
# any closed unmerged → exit 4, any open → exit 3, any unreadable → exit 1.
check_gate_prs() {
  local slug url state open=false closed=false failed=false
  while IFS=$'\t' read -r slug url; do
    [[ -n "$slug" ]] || continue
    if ! state="$("$SCRIPT_DIR/loop-repo.sh" pr-state --pr "$url" </dev/null 2>/dev/null)"; then
      print -u2 "loop-plan: cannot read PR state: $url"
      failed=true
      continue
    fi
    print -r -- "$slug"$'\t'"$url"$'\t'"$state"
    [[ "$state" == open ]] && open=true
    [[ "$state" == closed ]] && closed=true
  done < <(jq -r '.prs | to_entries | sort_by(.key)[] | "\(.key)\t\(.value)"' <<< "$1")
  $failed && die "archive refused: cannot read every gate PR state"
  $closed && die "archive refused: a gate PR is closed unmerged" 4
  $open && die "archive refused: a gate PR is still open" 3
  return 0
}

case "$CMD" in
register|push)
  do_register
  ;;
archive)
  trap '[[ -n "$ARCHIVE_TMP" ]] && rm -rf "$ARCHIVE_TMP"' EXIT
  do_archive
  ;;
get)
  [[ -n "$PLAN_ID" ]] || die "get requires --plan-id <id>"
  loop_ensure_daemon
  body="$(curl -sf -m 5 "${LOOP_DAEMON_URL}/api/loops/${PLAN_ID}/plan")" \
    || die "get: daemon unreachable or unknown plan '$PLAN_ID'" 3
  text="$(printf '%s' "$body" | jq -r '.planText // empty')"
  if [[ -n "$OUT" ]]; then printf '%s' "$text" > "$OUT"; else printf '%s' "$text"; fi
  ;;
list)
  loop_ensure_daemon
  body="$(curl -sf -m 5 "${LOOP_DAEMON_URL}/api/loops")" || die "list: daemon unreachable" 3
  printf '%s' "$body" | jq -r '.[] | "\(.runId)  ·  \(.status)  ·  \(.effort // "-")"'
  ;;
note)
  [[ -n "$PLAN_ID" ]] || die "note requires --plan-id <id>"
  [[ -n "$BODY" ]] || die "note requires --body <text>"
  loop_ensure_daemon
  printf '%s' "$(jq -cn --arg event progress.note --arg phase "$PHASE" --arg detail "$BODY" \
    --arg ts "$(now_iso)" '{event:$event, phase:$phase, detail:$detail, ts:$ts}')" \
    | loop_emit "$PLAN_ID" event
  ;;
*)
  die "unknown command '${CMD}' (register|push|get|list|note|archive)"
  ;;
esac
