#!/bin/zsh
# Notifications: event emission, adapters (terminal-notifier, osascript, boringnotch, custom) and notifier build. Sourced by ws/ws.sh.

# emit_notify_event <state> <prev> <payload> <seq> [target] — send one v1 JSON event on stdin
# to the configured adapter. target (window/pane) defaults to the hook's pane. `closed` marks
# the tab's notification dismissed; any other state marks it open (see cmd_dismiss).
emit_notify_event() {
  (( WS_NOTIFY )) || return 0
  local target="${5:-$TMUX_PANE}" focused
  focused="$(tmux_ display -p -t "$target" '#{&&:#{window_active},#{session_attached}}' 2>/dev/null)"
  [[ "$focused" == 1 ]] && focused=true || focused=false
  local win sess click wt agent branch detail event sound="" name
  win="$(tmux_ display -p -t "$target" '#{window_id}')"
  sess="$(tmux_ display -p -t "$target" '#{session_id}')"
  wt="$(tmux_ display -p -t "$target" '#{@ws_path}')"
  agent="$(tmux_ display -p -t "$target" '#{@ws_agent}')"
  name="${WS_WORKSPACE:-$(workspace_name "$wt")}"
  local msg="$name is $1"
  [[ "$1" == waiting ]] && msg="$name needs input"
  [[ "$1" == closed ]] && msg="$name dismissed"
  tmux_ set -w -t "$win" @ws_notify_open "$([[ "$1" == closed ]] && print 0 || print 1)" 2>/dev/null
  branch="$(git -C "$wt" branch --show-current 2>/dev/null)"
  detail="$(hook_detail_json "${3:-}")"
  [[ -n "$detail" ]] || detail='""'
  click="${(q)$(command -v tmux)}${WS_TMUX_SOCKET:+ -L ${(q)WS_TMUX_SOCKET}} select-window -t ${(q)win} \\; switch-client -t ${(q)sess}"
  local bundle="${WS_TERMINAL_BUNDLE:-$(tmux_ show-environment -g __CFBundleIdentifier 2>/dev/null | cut -d= -f2)}"
  [[ -n "$bundle" ]] && click+=" ; open -b ${(q)bundle}"
  [[ "$1" == idle ]] && sound=done
  [[ "$1" == waiting ]] && sound=waiting
  event="$(jq -nc --arg id "ws:$win" --arg state "$1" --arg prev "$2" \
    --arg message "$msg" --arg agent "$agent" --arg repo "${name%%/*}" \
    --arg branch "$branch" --argjson detail "$detail" --arg sound "$sound" --arg command "$click" \
    --argjson focused "$focused" --argjson seq "${4:-0}" --argjson ts "$(date +%s)" \
    '{v:1, id:$id, source:"ws", state:$state, prev:$prev, title:"ws", message:$message,
      detail:$detail, agent:$agent, repo:$repo, branch:$branch, focused:$focused, sound:$sound,
      actions:[{id:"focus", label:"Focus tab", command:$command}], seq:$seq, ts:$ts}')" || return 0
  { print -r -- "$event" | notify_adapter; } >/dev/null 2>&1 &!
}

# Keep detail JSON-encoded so command substitution preserves trailing newlines in the text.
hook_detail_json() {
  print -r -- "$1" | jq -cs '
    (if length == 1 then .[0] else {} end)
    | if type == "object" then
        [.message, .last_assistant_message, .lastAssistantMessage, .prompt]
        | map(select(type == "string") | select(length > 0))
        | (first // "") | .[:1024]
      else "" end
  ' 2>/dev/null
}

notify_adapter() {
  case "$WS_NOTIFIER" in
  boringnotch)
    local event="$(cat)"
    # The notch app owns sound (its Settings); only the fallback plays ws cues.
    print -r -- "$event" | nc -U -w 1 "$WS_BORINGNOTCH_SOCKET" >/dev/null 2>&1 \
      || print -r -- "$event" | WS_NOTIFIER=auto notify_adapter
    ;;
  auto | terminal-notifier | osascript)
    local event="$(cat)" lockfd
    notify_claim "$event" || return 0
    if [[ "$(print -r -- "$event" | jq -r '.state')" == closed ]]; then
      [[ "$WS_NOTIFIER" == osascript ]] || print -r -- "$event" | terminal_notifier_remove
      return 0
    fi
    print -r -- "$event" | jq -e '
      (.state == "idle" or .state == "waiting") and .focused == false
    ' >/dev/null 2>&1 || return 0
    notify_sound "$(print -r -- "$event" | jq -r '.sound')"
    tmux_ display-message "ws: $(print -r -- "$event" | jq -r '.message')" 2>/dev/null
    if [[ "$WS_NOTIFIER" == osascript ]]; then
      print -r -- "$event" | osascript_notify
    else
      print -r -- "$event" | terminal_notifier_notify
    fi
    ;;
  *)
    if [[ "$WS_NOTIFIER" == */* && -x "$WS_NOTIFIER" ]]; then
      "$WS_NOTIFIER"
    else
      tmux_ display-message "ws: unknown WS_NOTIFIER '$WS_NOTIFIER', using auto" 2>/dev/null
      WS_NOTIFIER=auto notify_adapter
    fi
    ;;
  esac
}

# notify_claim <event> — per-tab delivery watermark for the built-in notifiers: under the tab's
# notify lock (left open in the caller's lockfd, so posts serialize until the adapter exits),
# fail if a same-or-newer seq was already claimed, else record this one — every state, so a
# newer `working` still supersedes an older banner. No seq (custom/legacy) counts as newest.
notify_claim() {
  local id seq win lock last
  print -r -- "$1" | jq -r '.id // "", .seq // 0' | { read -r id; read -r seq; }
  win="${id#ws:}"
  [[ "$id" == ws:?* && "$seq" == <1-> ]] || return 0
  zmodload zsh/system 2>/dev/null
  lock="${TMPDIR:-/tmp}/ws-notify-${WS_TMUX_SOCKET:-default}-$win.lock"
  { : >>"$lock" && zsystem flock -t "$(lock_wait 2)" -i 0.01 -f lockfd "$lock"; } 2>/dev/null || return 1
  last="$(tmux_ display -p -t "$win" '#{@ws_notify_seq}' 2>/dev/null)"
  [[ "$last" == <-> ]] && (( seq <= last )) && return 1
  tmux_ set -w -t "$win" @ws_notify_seq "$seq" 2>/dev/null
  return 0
}

# Play once outside notification transport so a fallback cannot duplicate the sound.
notify_sound() {
  local sound
  case "$1" in
  done) sound="$WS_NOTIFY_SOUND_DONE" ;;
  waiting) sound="$WS_NOTIFY_SOUND_WAITING" ;;
  *) return 0 ;;
  esac
  if [[ ! -f "$sound" && "$sound" != */* ]]; then
    local -a found=({$HOME/Library/Sounds,/Library/Sounds,/System/Library/Sounds}/$sound.*(N.))
    sound="${found[1]:-}"
  fi
  # Detached: a long cue must not delay (and reorder) the notification.
  [[ -n "$sound" && -f "$sound" ]] && afplay "$sound" >/dev/null 2>&1 &!
  return 0
}

# Preserve macOS icon/group/click behavior, preferring the branded ws.app.
# Fall back to osascript if terminal-notifier is missing or macOS rejects it.
terminal_notifier_notify() {
  local event="$(cat)"
  local notifier="$WS_NOTIFIER_APP/Contents/MacOS/terminal-notifier"
  [[ -x "$notifier" ]] || notifier="$(command -v terminal-notifier)"
  if [[ -n "$notifier" ]]; then
    local msg title id bundle click image="$WS_NOTIFY_ICON_DONE"
    local -a icon=()
    [[ "$(print -r -- "$event" | jq -r '.sound')" == waiting ]] \
      && image="$WS_NOTIFY_ICON_WAITING"
    [[ -f "$image" ]] && icon=(-contentImage "$image") # app icon can't be overridden on modern macOS
    msg="$(print -r -- "$event" | jq -r '.message')"
    title="$(print -r -- "$event" | jq -r '.title')"
    id="$(print -r -- "$event" | jq -r '.id')"
    click="$(print -r -- "$event" | jq -r '.actions[] | select(.id == "focus") | .command')"
    bundle="${WS_TERMINAL_BUNDLE:-$(tmux_ show-environment -g __CFBundleIdentifier 2>/dev/null | cut -d= -f2)}"
    "$notifier" -title "$title" -message "$msg" -group "${id/ws:/ws-}" "${icon[@]}" \
      ${bundle:+-activate} ${bundle:+$bundle} -execute "$click" >/dev/null 2>&1 && return 0
  fi
  print -r -- "$event" | osascript_notify
}

# Remove the tab's banner (osascript notifications can't be withdrawn).
terminal_notifier_remove() {
  local notifier="$WS_NOTIFIER_APP/Contents/MacOS/terminal-notifier" id
  [[ -x "$notifier" ]] || notifier="$(command -v terminal-notifier)"
  [[ -n "$notifier" ]] || return 0
  id="$(jq -r '.id')"
  "$notifier" -remove "${id/ws:/ws-}" >/dev/null 2>&1
  return 0
}

osascript_notify() {
  local event="$(cat)"
  local msg="$(print -r -- "$event" | jq -r '.message')" title="$(print -r -- "$event" | jq -r '.title')"
  osascript -e 'on run argv' -e 'display notification (item 1 of argv) with title (item 2 of argv)' \
    -e 'end run' "$msg" "$title" >/dev/null 2>&1
}

# The hook's tab is on screen: current in its session, and that session attached.
pane_focused() {
  [[ "$(tmux_ display -p -t "$TMUX_PANE" '#{&&:#{window_active},#{session_attached}}' 2>/dev/null)" == 1 ]]
}

# Copies terminal-notifier.app to $WS_HOME/ws.app with its own name, bundle id and icon, so
# notifications come from "ws" (own entry in System Settings → Notifications: allow it there,
# pick Alerts to keep them on screen). Re-run after `brew upgrade terminal-notifier`.
build_notifier() {
  [[ -d "$WS_NOTIFIER_SOURCE" ]] || die "terminal-notifier.app not found: $WS_NOTIFIER_SOURCE (brew install terminal-notifier)"
  local app="$WS_NOTIFIER_APP" plist="$WS_NOTIFIER_APP/Contents/Info.plist" iconset size
  rm -rf "$app"
  mkdir -p "$WS_HOME"
  cp -R "$WS_NOTIFIER_SOURCE" "$app" || die "copy failed"
  chmod -R u+w "$app"
  /usr/libexec/PlistBuddy -c "Set :CFBundleIdentifier sh.ratel.ws.notifier" \
    -c "Set :CFBundleName ws" "$plist" || die "cannot edit $plist"
  iconset="$(mktemp -d)/ws.iconset"
  mkdir -p "$iconset"
  for size in 16 32 128 256 512; do
    sips -z $size $size "$WS_APP_ICON" --out "$iconset/icon_${size}x${size}.png" >/dev/null
    sips -z $((size * 2)) $((size * 2)) "$WS_APP_ICON" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
  done
  iconutil -c icns "$iconset" -o "$app/Contents/Resources/Terminal.icns" || die "iconutil failed"
  rm -rf "${iconset:h}"
  codesign --force --deep --sign - "$app" >/dev/null 2>&1 || print -u2 "ws: codesign failed (notifications may be blocked)"
  touch "$app" # nudge LaunchServices to pick up the new icon
  print -r -- "built $app"
}

# Tab selected (tmux hook) or agent exited: emits one `closed` event when the tab's last
# notification is still open, so receivers drop it. The tab state itself is untouched.
cmd_dismiss() {
  zmodload zsh/datetime 2>/dev/null
  local seq="$(realtime_us "$EPOCHREALTIME")" target="${1:-}" open state
  (( WS_NOTIFY )) && [[ -n "$target" ]] || return 0
  IFS='|' read -r open state <<<"$(tmux_ display -p -t "$target" '#{@ws_notify_open}|#{@ws_state}' 2>/dev/null)"
  [[ "$open" == 1 ]] || return 0
  emit_notify_event closed "$state" "" "$seq" "$target"
  return 0
}
