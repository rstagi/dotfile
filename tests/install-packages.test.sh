#!/bin/zsh
# Check installer steps with a private HOME and fake external commands. The test installs nothing.
# Each configure step runs two times. The second run must not change the sandbox HOME.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib.sh"
TMP="$(mktemp -d /tmp/install-packages.XXXXXX)"
TMP="${TMP:A}"
trap 'rm -rf "$TMP"' EXIT

# Only system tools and the fakes are on PATH. A host brew, git or rustup must not replace them.
SYSTEM_PATH=/usr/bin:/bin:/usr/sbin:/sbin
mkdir -p "$TMP/fake" "$TMP/home/dotfile"
print -r -- "# dotfile tmux config" > "$TMP/home/dotfile/.tmux.conf"
# The fake brew reads installed formulae from $HOME/brew.installed and logs each call.
cat > "$TMP/fake/brew" <<'EOF'
#!/bin/sh
printf '%s|%s\n' "$*" "${HOMEBREW_NO_INSTALL_UPGRADE:-}" >> "$HOME/brew.calls"
case "$1" in
  list) cat "$HOME/brew.installed" 2>/dev/null ;;
  --prefix) printf '%s\n' "$HOME/brew/$2" ;;
  install)
    for formula in "$@"; do
      if [ "$formula" = rustup ]; then
        mkdir -p "$HOME/brew/rustup/bin"
        printf '#!/bin/sh\nprintf "%%s\\n" "brew-rustup $*" >> "$HOME/rustup.calls"\n' > "$HOME/brew/rustup/bin/rustup"
        chmod +x "$HOME/brew/rustup/bin/rustup"
      fi
    done ;;
esac
exit 0
EOF
cat > "$TMP/fake/git" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" >> "$HOME/git.calls"
[ "$1" = clone ] && mkdir -p "$3"
exit 0
EOF
chmod +x "$TMP/fake/brew" "$TMP/fake/git"
# Each fake below only logs its arguments. Host launchctl, chsh and curl change the user session or the system.
for name in launchctl chsh curl cargo node npm nvm pyenv tfenv terraform gcloud kubectl docker-buildx gh claude; do
  printf '#!/bin/sh\nprintf "%%s\\n" "$*" >> "$HOME/%s.calls"\n' "$name" > "$TMP/fake/$name"
  chmod +x "$TMP/fake/$name"
done
# Stop the test when a host command comes before its fake on PATH.
for name in launchctl chsh curl; do
  found="$(env -i PATH="$TMP/fake:$SYSTEM_PATH" /bin/bash -c "command -v $name")"
  [[ "$found" == "$TMP/fake/$name" ]] || { print -u2 "The fake $name must be first on PATH. Found: $found"; exit 1; }
done
SHARED_SKILLS=(${(f)"$(sed -n '/^SHARED_SKILLS=(/,/^)/p' "$ROOT/install-agent-skills.sh" | sed '1d;$d' | tr -d ' ')"})
DEFINITIONS="$(sed '$d' "$ROOT/install.sh")"
[[ "$(tail -1 "$ROOT/install.sh")" == 'main "$@"' ]] || { print -u2 "install.sh must end with main \"\$@\"."; exit 1; }

# run_installer <interactive> <shell code> — run install.sh functions in bash. Sets ERR and RC.
run_installer() {
  local script="$DEFINITIONS"$'\n'"INTERACTIVE_MODE=$1"$'\n'"$2"
  env -i HOME="$TMP/home" PATH="$TMP/fake:$SYSTEM_PATH" /bin/bash -c "$script" \
    > "$TMP/out" 2> "$TMP/err" < /dev/null
  RC=$?
  ERR="$(<"$TMP/err")"
}
calls() { cat "$TMP/home/$1.calls" 2>/dev/null; }
reset_calls() { rm -f "$TMP/home"/*.calls; }

# reset_home — make a new sandbox HOME. Its dotfile folder has each file that a configure step uses.
reset_home() {
  local home="$TMP/home" skill
  local dot="$home/dotfile"
  rm -rf "$home"
  mkdir -p "$dot/nvim" "$dot/loop-web" "$dot/ws" "$dot/wsd/launchd" "$dot/wsd/target/release" \
    "$dot/.claude/agents" "$home/brew/opt/fzf"
  print -r -- "# dotfile tmux config" > "$dot/.tmux.conf"
  print -r -- "# dotfile git config" > "$dot/.gitconfig"
  print -r -- "# dotfile zsh config" > "$dot/.zshrc"
  print -r -- "-- dotfile nvim config" > "$dot/nvim/init.lua"
  print -r -- "# global Claude instructions" > "$dot/CLAUDE_GLOBAL.md"
  print -r -- "# global Codex instructions" > "$dot/AGENTS_GLOBAL.md"
  cp "$ROOT/install-agent-skills.sh" "$dot/install-agent-skills.sh"
  for skill in $SHARED_SKILLS; do
    mkdir -p "$dot/.claude/skills/$skill"
    print -r -- "# $skill" > "$dot/.claude/skills/$skill/SKILL.md"
  done
  cp "$ROOT/wsd/launchd/com.rstagi.wsd.plist" "$dot/wsd/launchd/"
  print -r -- binary > "$dot/wsd/target/release/ws"
  # The installer runs the launcher to build the notifier.
  printf '#!/bin/sh\nexit 0\n' > "$dot/ws/ws.sh"
  printf '#!/bin/sh\nprintf "%%s\\n" "$*" >> "$HOME/fzf.calls"\n' > "$home/brew/opt/fzf/install"
  chmod +x "$dot/ws/ws.sh" "$home/brew/opt/fzf/install"
}

# tree_snapshot — print each path in the sandbox HOME with its type and its link target or checksum.
# The snapshot ignores the *.calls logs of the fakes.
tree_snapshot() {
  local entry
  (cd "$TMP/home" && find . ! -name '*.calls' | LC_ALL=C sort | while IFS= read -r entry; do
    if [[ -L "$entry" ]]; then
      print -r -- "link $entry -> $(readlink "$entry")"
    elif [[ -d "$entry" ]]; then
      print -r -- "folder $entry"
    else
      print -r -- "file $entry $(shasum < "$entry" | cut -c1-40)"
    fi
  done)
}

# duplicate_lines <file>... — print each line that occurs more than one time in a sandbox HOME file.
duplicate_lines() {
  local file
  for file in "$@"; do
    [[ -f "$TMP/home/$file" ]] || continue
    sort "$TMP/home/$file" | uniq -d | sed "s|^|$file: |"
  done
}

# self_links — print each link in the sandbox HOME that points to the folder that holds it.
self_links() {
  local link
  find "$TMP/home" -type l | while IFS= read -r link; do
    if [[ "$link" -ef "${link:h}" ]]; then print -r -- "${link#$TMP/home/}"; fi
  done
}

# backups <snapshot> — print the snapshot lines of backup paths.
backups() { print -r -- "$1" | grep -F '.bak' }

# run_step <shell code> — run install.sh functions in package mode. Sets ERR and RC.
run_step() { run_installer false "$1"; }

# run_full_install — run install.sh with no arguments and answer yes to each question. Sets RC.
run_full_install() {
  yes y | env -i HOME="$TMP/home" PATH="$TMP/fake:$SYSTEM_PATH" /bin/bash "$ROOT/install.sh" > "$TMP/out" 2> "$TMP/err"
  RC=${pipestatus[2]}
}

# assert_idempotent <label> <runner> [<argument>...] — run the runner two times in the current sandbox HOME.
# The runner must set RC. The second run must succeed. It must not add rc lines, backups, links, files or folders.
assert_idempotent() {
  local label="$1" first second
  shift
  "$@"
  assert_exit "$RC" 0 "$label: the first run succeeds"
  first="$(tree_snapshot)"
  "$@"
  assert_exit "$RC" 0 "$label: the second run succeeds"
  second="$(tree_snapshot)"
  assert_eq "$(duplicate_lines .zshrc_ext .zshrc_additional_plugins)" "" "$label: the rc files have no duplicate lines"
  assert_eq "$(self_links)" "" "$label: no link points to the folder that holds it"
  assert_eq "$(backups "$second")" "$(backups "$first")" "$label: the second run makes no backup"
  assert_eq "$(diff <(print -r -- "$first") <(print -r -- "$second"))" "" "$label: the second run does not change the file tree"
}

echo "install: the tmux step is idempotent"
print -r -- tmux > "$TMP/home/brew.installed"
run_installer false install_tmux
assert_exit "$RC" 0 "first tmux configuration succeeds"
assert_eq "$(calls git)" "clone https://github.com/tmux-plugins/tpm $TMP/home/.tmux/plugins/tpm" "first run clones tpm"
assert_eq "$(readlink "$TMP/home/.tmux.conf")" "$TMP/home/dotfile/.tmux.conf" "first run links .tmux.conf"
reset_calls
run_installer false install_tmux
assert_exit "$RC" 0 "second tmux configuration succeeds"
assert_eq "$(calls git)" "" "second run does not clone tpm again"
assert_eq "$(readlink "$TMP/home/.tmux.conf")" "$TMP/home/dotfile/.tmux.conf" "second run keeps the link"
rm "$TMP/home/.tmux.conf"
print -r -- "# user tmux config" > "$TMP/home/.tmux.conf"
run_installer false install_tmux
assert_exit "$RC" 0 "a different .tmux.conf does not stop the install"
assert_eq "$(<"$TMP/home/.tmux.conf")" "# user tmux config" "the installer keeps a different .tmux.conf"
assert_contains "$ERR" "Warning: $TMP/home/.tmux.conf" "the installer warns about a different .tmux.conf"

echo "install: rust uses an existing rustup"
reset_calls
: > "$TMP/home/brew.installed"
mkdir -p "$TMP/home/.cargo/bin"
printf '#!/bin/sh\nprintf "%%s\\n" "cargo-rustup $*" >> "$HOME/rustup.calls"\n' > "$TMP/home/.cargo/bin/rustup"
chmod +x "$TMP/home/.cargo/bin/rustup"
run_installer false 'PATH="$HOME/.cargo/bin:$PATH"; install_rust'
assert_exit "$RC" 0 "rust succeeds with rustup on PATH"
assert_eq "$(calls brew | grep -c 'install rustup')" 0 "rustup on PATH skips brew install rustup"
assert_eq "$(calls rustup)" "cargo-rustup default stable" "rust uses the rustup on PATH"
reset_calls
rm -rf "$TMP/home/.cargo"
run_installer false install_rust
assert_exit "$RC" 0 "rust succeeds without rustup"
assert_eq "$(calls brew | grep -c '^install rustup|')" 1 "rust installs rustup with brew when it is missing"
assert_eq "$(calls rustup)" "brew-rustup default stable" "rust uses the brew rustup"

echo "install: only the full install upgrades every brew formula"
reset_calls
run_installer false install_prerequisites
assert_exit "$RC" 0 "package prerequisites succeed"
assert_eq "$(calls brew | grep -c '^upgrade|')" 0 "a package install does not run brew upgrade"
assert_eq "$(calls brew | grep '^install openssl' | cut -d'|' -f2)" 1 "package prerequisites do not upgrade installed formulae"
reset_calls
run_installer true install_prerequisites
assert_exit "$RC" 0 "full prerequisites succeed"
assert_eq "$(calls brew | grep -c '^upgrade|')" 1 "the full install runs brew upgrade"
reset_calls
env -i HOME="$TMP/home" PATH="$TMP/fake:$SYSTEM_PATH" /bin/bash "$ROOT/install.sh" --dry-run tmux > "$TMP/out" 2>&1
assert_exit "$?" 0 "the package dry run succeeds"
assert_contains "$(<"$TMP/out")" "Skip the global brew upgrade" "the package dry run shows that it skips brew upgrade"
assert_eq "$(calls brew)" "" "the dry run does not call brew"

echo "install: the neovim step is idempotent"
reset_home
assert_idempotent neovim run_step 'install_package neovim'
assert_eq "$(readlink "$TMP/home/.config/nvim")" "$TMP/home/dotfile/nvim" "neovim links ~/.config/nvim to the dotfile nvim folder"
assert_eq "$(grep -cxF "source $TMP/home/dotfile/.zshrc_vim_ext" "$TMP/home/.zshrc_ext")" 1 "neovim adds the vim source line one time"
# The live machine: the link and the line exist. An earlier run moved a link to nvim.bak.
reset_home
mkdir -p "$TMP/home/.config"
ln -s "$TMP/home/dotfile/nvim" "$TMP/home/.config/nvim"
ln -s "$TMP/home/dotfile/nvim" "$TMP/home/.config/nvim.bak"
print -r -- "source $TMP/home/dotfile/.zshrc_vim_ext" > "$TMP/home/.zshrc_ext"
before="$(tree_snapshot)"
run_installer false 'install_package neovim'
assert_exit "$RC" 0 "neovim succeeds when its link exists"
assert_eq "$(self_links)" "" "neovim does not make a link in the dotfile nvim folder"
assert_eq "$(tree_snapshot)" "$before" "neovim changes nothing when its link and its line exist"
# A user nvim folder goes to a new backup. An older nvim.bak folder stays as it is.
reset_home
mkdir -p "$TMP/home/.config/nvim" "$TMP/home/.config/nvim.bak"
print -r -- "-- user nvim config" > "$TMP/home/.config/nvim/init.lua"
print -r -- "-- older backup" > "$TMP/home/.config/nvim.bak/init.lua"
run_installer false 'install_package neovim'
assert_exit "$RC" 0 "neovim succeeds when a user nvim folder exists"
assert_eq "$(readlink "$TMP/home/.config/nvim")" "$TMP/home/dotfile/nvim" "neovim replaces the user nvim folder with the link"
assert_eq "$(ls "$TMP/home/.config/nvim.bak")" "init.lua" "neovim does not move the user folder into nvim.bak"
assert_eq "$(<"$TMP/home/.config/nvim.bak/init.lua")" "-- older backup" "neovim keeps the older nvim.bak"
new_backups=("$TMP/home/.config"/nvim.bak.*/init.lua(N))
assert_eq "${#new_backups}" 1 "neovim makes one new backup"
assert_eq "$(cat $new_backups < /dev/null)" "-- user nvim config" "neovim moves the user folder to the new backup"

echo "install: each configure step is idempotent"
for package in tmux python node terraform gcloud kubectl docker ws loop-web claude-config gh fzf zsh; do
  reset_home
  assert_idempotent "$package" run_step "install_package $package"
done
reset_home
assert_idempotent "core setup" run_step setup_core
assert_eq "$(readlink "$TMP/home/.gitconfig")" "$TMP/home/dotfile/.gitconfig" "core setup links ~/.gitconfig"
# A user file is in the way: the first run moves it to a backup. The second run makes no backup.
reset_home
print -r -- "# user zshrc" > "$TMP/home/.zshrc"
assert_idempotent "zsh with a user .zshrc" run_step 'install_package zsh'
assert_eq "$(readlink "$TMP/home/.zshrc")" "$TMP/home/dotfile/.zshrc" "zsh links ~/.zshrc"
assert_eq "$(cat "$TMP/home"/.zshrc.bak.*(N) < /dev/null)" "# user zshrc" "zsh moves the user .zshrc to a backup"
reset_home
mkdir -p "$TMP/home/.claude/agents"
print -r -- "# user agent" > "$TMP/home/.claude/agents/mine.md"
assert_idempotent "claude-config with a user agents folder" run_step 'install_package claude-config'
assert_eq "$(readlink "$TMP/home/.claude/agents")" "$TMP/home/dotfile/.claude/agents" "claude-config links ~/.claude/agents"
assert_eq "$(readlink "$TMP/home/.claude/skills")" "$TMP/home/dotfile/.claude/skills" "claude-config links ~/.claude/skills"
assert_eq "$(readlink "$TMP/home/.claude/CLAUDE.md")" "$TMP/home/dotfile/CLAUDE_GLOBAL.md" "claude-config links ~/.claude/CLAUDE.md"
assert_eq "$(readlink "$TMP/home/.codex/AGENTS.md")" "$TMP/home/dotfile/AGENTS_GLOBAL.md" "claude-config links ~/.codex/AGENTS.md"
assert_eq "$(cat "$TMP/home/.claude"/agents.bak.*/mine.md(N) < /dev/null)" "# user agent" "claude-config moves the user agents folder to a backup"
# The installer starts its line on a new line when the last line of ~/.zshrc_ext has no newline.
reset_home
printf 'export EDITOR=nvim' > "$TMP/home/.zshrc_ext"
run_installer false 'install_package terraform'
assert_eq "$(<"$TMP/home/.zshrc_ext")" "export EDITOR=nvim"$'\n'"source $TMP/home/dotfile/.zshrc_terraform_ext" "terraform adds its line after a last line with no newline"
# Only the exact line counts. A line in a comment does not stop the installer.
for package ext in ws ws loop-web loop_web claude-config claude; do
  reset_home
  print -r -- "# source $TMP/home/dotfile/.zshrc_${ext}_ext" > "$TMP/home/.zshrc_ext"
  run_installer false "install_package $package"
  assert_eq "$(grep -cxF "source $TMP/home/dotfile/.zshrc_${ext}_ext" "$TMP/home/.zshrc_ext")" 1 "$package adds its source line when only a comment names it"
done

echo "install: a backup never replaces an older backup"
reset_home
print -r -- "# older backup" > "$TMP/home/.gitconfig.bak"
print -r -- "# user git config 1" > "$TMP/home/.gitconfig"
run_installer false setup_core
assert_exit "$RC" 0 "core setup succeeds with a user .gitconfig"
rm "$TMP/home/.gitconfig"
print -r -- "# user git config 2" > "$TMP/home/.gitconfig"
run_installer false setup_core
assert_exit "$RC" 0 "core setup succeeds with a second user .gitconfig"
assert_eq "$(<"$TMP/home/.gitconfig.bak")" "# older backup" "core setup keeps the older .gitconfig.bak"
assert_eq "$(cat "$TMP/home"/.gitconfig.bak.*(N) < /dev/null)" "# user git config 1"$'\n'"# user git config 2" "core setup puts each user .gitconfig in its own backup"
assert_eq "$(readlink "$TMP/home/.gitconfig")" "$TMP/home/dotfile/.gitconfig" "core setup links ~/.gitconfig after the backup"

echo "install: ws keeps a different file where its links go"
reset_home
mkdir -p "$TMP/home/.local/bin"
print -r -- "# user ws" > "$TMP/home/.local/bin/ws"
run_installer false install_ws
assert_exit "$RC" 0 "a different ~/.local/bin/ws does not stop the ws install"
assert_eq "$(<"$TMP/home/.local/bin/ws")" "# user ws" "the ws install keeps a different ~/.local/bin/ws"
assert_contains "$ERR" "Warning: $TMP/home/.local/bin/ws" "the ws install warns about a different ~/.local/bin/ws"
assert_eq "$(readlink "$TMP/home/.ws/bin/ws")" "$TMP/home/dotfile/ws/ws.sh" "the ws install links ~/.ws/bin/ws"

echo "install: the full interactive install is idempotent"
reset_home
print -r -- "# user zshrc" > "$TMP/home/.zshrc"
assert_idempotent "full install" run_full_install
assert_contains "$(<"$TMP/out")" "Done!" "the second full install completes"
test_summary
