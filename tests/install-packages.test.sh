#!/bin/zsh
# Check package-scoped installer steps with a private HOME, a fake brew and a fake git. The test installs nothing.
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
test_summary
