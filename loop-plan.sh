#!/bin/zsh
set -u -o pipefail

# Loop Engineering — plan helper: register/push a multi-phase plan on the central Loop
# Observatory daemon (the base backend for EVERY plan) and post progress notes to it. The
# daemon is Kestral-free; Kestral is an opt-in LINKED backend layered on top by the skills.
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
#     POST a progress.note event — the unlinked substitute for a Kestral progress comment,
#     visible cross-worktree on the loop timeline (jq-built, injection-safe).

DIR=".loop"
CMD="${1:-}"; [[ -n "$CMD" ]] && shift

PLAN_FILE="" PLAN_ID="" EFFORT="" OUT="" PHASE="" BODY=""
while [[ $# -gt 0 ]]; do
  case "$1" in
  --dir) DIR="$2"; shift 2 ;;
  --plan-file) PLAN_FILE="$2"; shift 2 ;;
  --plan-id) PLAN_ID="$2"; shift 2 ;;
  --effort) EFFORT="$2"; shift 2 ;;
  --out) OUT="$2"; shift 2 ;;
  --phase) PHASE="$2"; shift 2 ;;
  --body) BODY="$2"; shift 2 ;;
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
  loop_ensure_daemon
  jq -cn --arg loopDir "$loop_abs" --arg planFile "$plan_abs" --arg effort "$EFFORT" \
    --arg started "$(now_iso)" \
    '{loopDir:$loopDir, planFile:$planFile,
      effort:(if ($effort|length)>0 then $effort else null end), startedAt:$started}' \
    | loop_emit "$pid" register
  printf '%s\n' "$pid"
}

case "$CMD" in
register|push)
  do_register
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
  die "unknown command '${CMD}' (register|push|get|list|note)"
  ;;
esac
