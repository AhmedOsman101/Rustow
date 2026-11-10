#!/usr/bin/env bash
# E2E scenario for rustow in a throwaway environment (container or local).
# Uses /tmp/rustow-e2e as fake $HOME; cleans up on exit.

set -euo pipefail

E2E=/tmp/rustow-e2e
rm -rf "${E2E}"
trap 'rm -rf "${E2E}"' EXIT

export HOME="${E2E}/home"
export DOTFILES_DIR="${E2E}/Configs"
export DOTFILES_TARGET="${E2E}/home"
export XDG_CONFIG_HOME="${E2E}/xdg-config"
export XDG_CACHE_HOME="${E2E}/xdg-cache"
export XDG_DATA_HOME="${E2E}/xdg-data"
export NO_COLOR=1
mkdir -p "${HOME}" "${DOTFILES_DIR}" "${XDG_CONFIG_HOME}" "${XDG_CACHE_HOME}"

RUSTOW="${RUSTOW:-cargo run --quiet --}"
echo "== building rustow =="
cargo build --quiet
RUSTOW="$(pwd)/target/debug/rustow"

pass() { echo "PASS: $*"; }
fail() {
  echo "FAIL: $*" >&2
  exit 1
}
assert_symlink() { [[ -L "$1" ]] || fail "expected symlink: $1"; }
assert_missing() { [[ ! -e "$1" ]] && [[ ! -L "$1" ]] || fail "expected missing: $1"; }

# --- fixture: two packages + one conflict ---
mkdir -p "${DOTFILES_DIR}/zsh" "${DOTFILES_DIR}/eza/.config/eza" "${DOTFILES_DIR}/git"
printf 'export EDITOR=vim\n' >"${DOTFILES_DIR}/zsh/.zshrc"
mkdir -p "${DOTFILES_DIR}/zsh/.config/zsh"
printf '# zsh config\n' >"${DOTFILES_DIR}/zsh/.config/zsh/rc.zsh"
printf 'theme: catppuccin\n' >"${DOTFILES_DIR}/eza/.config/eza/theme.yml"
printf 'icons: true\n' >"${DOTFILES_DIR}/eza/.config/eza/icons.yml"
mkdir -p "${DOTFILES_DIR}/eza/.config/eza/.git"
printf 'ref: refs/heads/main\n' >"${DOTFILES_DIR}/eza/.config/eza/.git/HEAD" # must NOT be linked
printf '[core]\n\teditor = vim\n' >"${DOTFILES_DIR}/git/.gitconfig"
mkdir -p "${HOME}/.config" # pre-existing real .config

# --- sync ---
echo "== rustow sync =="
"${RUSTOW}" sync >/tmp/rustow-e2e/sync.log 2>&1 || {
  cat /tmp/rustow-e2e/sync.log
  fail "sync exited non-zero"
}
assert_symlink "${HOME}/.zshrc"
assert_symlink "${HOME}/.gitconfig"
[[ -f "${HOME}/.config/eza/theme.yml" ]] || fail "eza theme not reachable"
[[ -f "${HOME}/.config/eza/icons.yml" ]] || fail "eza icons not reachable"
assert_missing "${HOME}/.config/eza/.git/HEAD" # stow default ignore
pass "sync links packages and ignores .git"

# --- status is healthy ---
echo "== rustow status =="
"${RUSTOW}" status >/tmp/rustow-e2e/status.log || {
  cat /tmp/rustow-e2e/status.log
  fail "status exited non-zero"
}
grep -q linked /tmp/rustow-e2e/status.log || fail "status output lacks 'linked'"
pass "status exits 0 when all packages linked"

# --- conflict handling: real file blocks link ---
echo "== conflict abort =="
"${RUSTOW}" unlink zsh >/dev/null
printf 'user contents\n' >"${HOME}/.zshrc"
if "${RUSTOW}" link zsh >/tmp/rustow-e2e/conflict.log 2>&1; then
  fail "expected conflict failure"
fi
grep -qi conflict /tmp/rustow-e2e/conflict.log || fail "no conflict report"
[[ "$(cat "${HOME}/.zshrc")" = "user contents" ]] || fail "conflict touch modified the user's file"
pass "conflict aborts, user file untouched"

# --- --force overwrites ---
echo "== force overwrite =="
"${RUSTOW}" -f link zsh >/dev/null
assert_symlink "${HOME}/.zshrc"
[[ "$(cat "${HOME}/.zshrc")" = "export EDITOR=vim" ]] || fail "forced overwrite lost package content"
pass "--force replaces conflict with package version"

# --- status filter ---
"${RUSTOW}" status eza >/tmp/rustow-e2e/filter.log
grep -q eza /tmp/rustow-e2e/filter.log || fail "status filter missing eza"
if grep -q git /tmp/rustow-e2e/filter.log; then fail "status filter leaked git package"; fi
pass "status filters by package"

# --- reset unlinks everything ---
echo "== rustow reset =="
"${RUSTOW}" reset >/dev/null
assert_missing "${HOME}/.zshrc"
assert_missing "${HOME}/.gitconfig"
assert_missing "${HOME}/.config/eza"
pass "reset removes all links"
# pre-existing .config survives (it was real, and empty now → removed or kept, but not an error either way)
[[ -d "${HOME}/.config" ]] && echo "note: .config kept (empty or user-owned)" || true

# --- create from scratch ---
echo "== rustow create =="
"${RUSTOW}" create mytool --empty >/dev/null
[[ -d "${DOTFILES_DIR}/mytool" ]] || fail "create --empty did not create the package"
[[ "$(ls -A "${DOTFILES_DIR}/mytool")" = "" ]] || fail "create --empty populated the package"
pass "create --empty makes an empty package"

# --- create from an existing directory ---
mkdir -p "${HOME}/.config/mytool"
printf 'key = value\n' >"${HOME}/.config/mytool/settings.conf"
"${RUSTOW}" create mytool --yes >/dev/null
assert_symlink "${HOME}/.config/mytool" # moved into the package, then linked (folded dir link)
[[ -f "${DOTFILES_DIR}/mytool/.config/mytool/settings.conf" ]] || fail "settings.conf not moved into package"
grep -q 'key = value' "${DOTFILES_DIR}/mytool/.config/mytool/settings.conf" || fail "moved content mismatch"
pass "create adopts an existing target into a new package"

echo
echo "ALL E2E SCENARIOS PASSED"
