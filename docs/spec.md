# rustow — Design Spec

**Status:** Confirmed 2026-11-09 (grilling session). This is the shared understanding between othman and the implementing agent.

## Purpose

A Stow-compatible dotfiles linker in Rust. GNU Stow 2.4.1 does the core job; rustow replaces it for one user's workflow, dropping full Stow compatibility in favor of a focused subset plus a per-package status report. The tool links a package tree onto `$HOME` as symlinks.

The `status` command's design was informed by reading the status implementation of [tuckr](https://github.com/RaphGL/Tuckr); all link semantics come from GNU Stow.

- Source layout: one directory per package under `source` (e.g. `~/dotfiles/Configs/<pkg>/.config/<app>/...`). The package tree mirrors the target root (`$HOME`).
- `rustow` operates on `$HOME` only when the user explicitly invokes a command. There is no background behavior, no implicit scan-and-fix, and no "live home" guard beyond the normal conflict semantics — that _is_ the safety model.
- No scripts are modified as part of this work (cutover is the user's manual decision).

## Commands

| Command                                     | Behavior                                                                                                                                                                                                                                                                                                                      |
| ------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `sync`                                      | Link all packages in `source`.                                                                                                                                                                                                                                                                                                |
| `link <pkg>...`                             | Link packages. Args required; no-args is failure.                                                                                                                                                                                                                                                                             |
| `unlink <pkg>...` (aliases: `remove`, `rm`) | Unlink packages. Args required; no-args is failure.                                                                                                                                                                                                                                                                           |
| `reset`                                     | Unlink all packages.                                                                                                                                                                                                                                                                                                          |
| `relink <pkg>...`                           | Unlink then link (restow). No-args is failure. Relink-all = `reset && sync`.                                                                                                                                                                                                                                                  |
| `status [pkg...]`                           | Per-package table; filters by name. `-s\|--short` prints only the summary line; `--json` machine-readable output. `-v` prints per-issue text detail (20 issues per package max); `-vv` prints everything. Exit 0 iff all inspected packages are fully linked and conflict-free, else 1. `-s` and `-v` are mutually exclusive. |
| `create <app>` (alias: `add`)               | See "create workflow" below.                                                                                                                                                                                                                                                                                                  |
| `config default`                            | Print the annotated default config to stdout.                                                                                                                                                                                                                                                                                 |
| `config init`                               | Write the default config to `$XDG_CONFIG_HOME/rustow/config.toml` (default `~/.config/rustow/config.toml`). Can pre-fill from an existing `.stowrc`/env when run interactively via the provenance prompt.                                                                                                                     |

Global flags: `-n|--dry-run`, `-f|--force`, `--adopt`, `-v|--verbose`.

## Link semantics

GNU Stow-compatible subset, not a full clone:

- Package tree is mirrored onto `target` as symlinks.
- **Fold**: when the target path for a package directory does not exist, the whole directory becomes a single symlink into the package (stow behavior) — except when the subtree contains ignored paths (e.g. a nested `.git`), in which case the tool descends and links per-file so ignored data is never exposed through a symlink.
- **Unfold**: when a target directory (real dir, or symlink owned by rustow/stow from another package) collides with a folded dir from another package, the folded link is replaced by a real directory and both package subtrees are stowed individually into it.
- **Refold** (on `unlink`): a now-empty/link-only target directory whose links all point into one package subtree is collapsed back into a single symlink.
- Ownership: a symlink is "owned" if its resolved target lies inside the configured `source` dir (any package), or was created by rustow. GNU stow creates absolute links into the stow dir, so this is compatible with existing setups.
- Never modifies anything not owned by rustow/stow, except per an explicit user decision at a conflict prompt.
- **Idempotent**: re-running `link` on an already-linked package is a silent no-op. Dangling links pointing into the package are repaired.

## Conflict handling

- Resolution is **two-phase** (plan everything, then apply): any unresolved conflict aborts the whole run; nothing is changed; all conflicts are reported.
- Existing **real file** at a target path, on a TTY -> prompt `[m]erge / [o]verwrite / [s]kip`:
  - **Merge**: open both files in a diff tool (see "diff tools" below). The user reconciles the two files starting from the _package_ copy (they edit the package file until it is the desired merged state). On Enter, the package file is left as-is and the target becomes a symlink to it.
  - **Overwrite**: target is deleted; symlink to the package's file created.
  - **Skip**: target untouched; package file reported as not linked.
- `-f|--force`: skip prompting; package wins (target deleted, symlink created). (Stow `--override`.)
- `--adopt`: skip prompting; target's content is moved _into the package_ (package copy replaced by the target's content), target becomes a symlink. (Stow `--adopt`.)
- Non-TTY with neither flag, or unresolved after prompting: abort the whole run two-phase; report; change nothing.
- Existing **symlink** at target path:
  - Points at exactly the wanted package path -> silently skip (idempotent).
  - Dangling, points into this package -> replace (repair).
  - Points into another package: directory-vs-directory -> unfold both; otherwise treat as conflict (prompt `[o]verwrite / [s]kip`; no merge).
  - Points anywhere else (external/absolute) -> same prompt as files minus merge.

## status output

Table with columns `Package | Status | Notes`:

- `Status`: `linked` | `partial` | `unlinked` | `conflict` (folded links count all subtree leaves as linked).
- `Notes`: e.g. `3 not linked`, `2 conflicts`, `empty package`, or a short reason.
- No positional args -> all packages; package names -> filter.
- `-s|--short`: print only the summary line.
- `--json`: `{ "packages": [...], "summary": {...} }`.
- Exit 0 iff every inspected package is `linked` and conflict-free; else 1.

## Config file

`~/.config/rustow/config.toml` (i.e. `$XDG_CONFIG_HOME/rustow/config.toml`). Keys (all optional except `source`):

| Key           | Type                      | Default                                                | Notes                                                                                                                                                                       |
| ------------- | ------------------------- | ------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `source`      | string                    | **none — error out until set** (Q1 answer: no default) | The stow dir; packages are its direct subdirectories.                                                                                                                       |
| `target`      | string                    | `$HOME`                                                | Link destination root.                                                                                                                                                      |
| `verbose`     | bool                      | `true`                                                 |                                                                                                                                                                             |
| `fold`        | bool                      | `true`                                                 | Clearer name for stow's `--no-folding` toggle.                                                                                                                              |
| `ignore`      | list of glob              | GNU Stow 2.4.1's default ignore list                   | Added on top of the built-in list; matched against package-relative paths. The complete built-in default is present as the default value in the printed/initialized config. |
| `diff_tool`   | string (command template) | unset                                                  | `{left}` / `{right}` placeholders, shlex-parsed. **Wins** over the registry and `$EDITOR`.                                                                                  |
| `on_conflict` | string                    | `prompt`                                               | `prompt` (TTY) / `abort` / `skip` / `overwrite` / `adopt`. `prompt` escalates to `abort` when not a TTY.                                                                    |

**Precedence**: CLI flags -> env vars (`DOTFILES_DIR`, `DOTFILES_TARGET`) -> `config.toml` -> `~/.stowrc` (fallback **only** when no config.toml exists; supports `$VAR`/`${VAR}` expansion and `--dir`/`--target`/`--verbose`; `--ignore` not supported from stowrc).

**Provenance prompt** (when no `config.toml`):

- TTY: warning naming what's used instead (`~/.stowrc` fallback, or env vars) + prompt: `[c]reate config.toml now (pre-filled from current values) / [k]eep fallback for this run / [s]ilence permanently / [a]bort?`
- Non-TTY: stderr warning; proceed.
- Config exists -> completely silent.
- `[s]ilence permanently` writes a stamp file under `$XDG_CACHE_HOME/rustow/` (or `~/.cache/rustow/`); suppresses the prompt until a `config.toml` exists.

**Default config & schema**: `config default` prints the annotated default (including the complete stow default ignore list as its `ignore` value). `schema.json` is auto-generated at the repo root from the config struct (schemars); regenerated via mask task, checked in.

**Env vars**: `DOTFILES_DIR` (source), `DOTFILES_TARGET` (target).

## create workflow

`create <app> [options]` — replaces `mkconf`.

- `<relpath>`: path relative to `target`; the package mirrors it 1:1 (`<source>/<app>/<relpath>`). Default: `.config/<app>`. Set with `--path`.
- **New package, no source found** (nothing at `target/<relpath>`): prompt — `[e]mpty package` (create only `<source>/<app>`, completely empty, and stop) / supply a source path to move in / abort.
- **Source exists** at `target/<relpath>`: show the source -> destination mapping (with `--verbose`: full file list + sizes), confirm, move it into the package, then link. Uses Rust prompts (dialoguer-style), no gum.
- **Package dir already exists**: never clobber; report what's there; offer to link it or abort.
- After moving, link the package by default; `-i|--interactive` asks before linking.
- Non-interactive: requires `--empty` or `--source <path>` (for the move path), plus `--yes` to skip confirmations; `--path` for custom layouts; `--verbose` for the file list.
- Layout example: `create vim` -> `<source>/vim/.config/vim/...`; `create zsh --path .zshrc` -> `<source>/zsh/.zshrc`.

## Ignore handling

- Built-in **GNU Stow 2.4.1 default ignore list** (from `references/stow/default-ignore-list`), segment-match for slashless regexps, path-match for slashed ones — exact strings copied into the implementation.
- Plus user `ignore` globs in `config.toml` (globset; matched against package-relative paths).
- Nothing else is ignored. Runtime junk (e.g. `pi` sessions, `vscode` globalStorage) is the user's responsibility — either don't stow those packages or use `ignore`.

## Diff tools (merge flow)

Resolution order (first wins):

1. `diff_tool` in `config.toml` — full command template over `{left}`/`{right}`, shlex-parsed.
2. Built-in registry, filtered by PATH availability, interactive pick on TTY, cached in `$XDG_CACHE_HOME/rustow/diff-tool` (override with `XDG_CACHE_HOME`). Persisted selection reused while it still exists on PATH.
3. `$EDITOR file{left} file{right}` — opens both as buffers; user resolves manually (for editors without diff). No parsing of $EDITOR flags beyond shlex splitting... (note: `$EDITOR` may contain args; shlex-split `$EDITOR; append both files`).
4. None of the above -> error telling the user to set `diff_tool` or `$EDITOR`.

Registry (name, command template, blocks-until-done):

- VS Code family (`--wait`): `code --diff --wait {left} {right}`, `codium`, `cursor`, `vscode`
- `zed --diff --wait {left} {right}`
- `meld {left} {right}`
- `kdiff3 {left} {right}`
- `diffuse {left} {right}`
- `kompare {left} {right}`
- `diffmerge {left} {right}`
- `tkdiff {left} {right}`
- `xxdiff {left} {right}`
- `guiffy {left} {right}` (GUI)
- `bcompare {left} {right}` (Beyond Compare)
- `nvim -d {left} {right}`, `vimdiff {left} {right}`, `gvimdiff {left} {right}` (gvim blocking when `-f`), fallback `vi -d`
- `emacs --eval '(ediff-files "{left}" "{right}")'` (terminal; GUI emacs does not block — treated as non-blocking)
- Pager-like, non-blocking-into-less semantics: `colordiff -u {left} {right}`, `delta {left} {right}`, `diff -u --color=always {left} {right}`
- Helix deliberately absent (no two-file diff mode) — use `diff_tool = "hx --new {left} {right}"` if wanted.

Merge UX (real-file conflict, user picks `m`): message explaining they should edit the **package** copy to the merged state; open diff tool on (target, package file); on tool exit or Enter, symlink target -> package file. If the user cancels/backs out of the prompt without choosing, that file is skipped (conflict remains).

## Config key -> flag mapping (useful defaults promoted)

`verbose`/`fold`/`ignore`/`target`/`source`/`diff_tool`/`on_conflict` — that is the complete key list; no other flags are promoted.

## Implementation layout

Single crate, lib + thin bin:

- `src/lib.rs` — module wiring for integration tests
- `src/main.rs` — arg parsing -> dispatch -> exit codes
- `src/cli.rs` — clap types
- `src/config.rs` — config model, precedence, stowrc fallback, provenance prompt
- `src/ignore.rs` — built-in stow list + glob user ignores
- `src/link.rs` — plan/apply for link (fold/unfold/conflicts), unlink (+refold), relink
- `src/status.rs` — status computation/render (table, short, json)
- `src/create.rs` — create workflow
- `src/diff.rs` — registry, selection, $EDITOR fallback, merge flow
- `src/prompt.rs` — dialoguer wrappers (confirm, select, provenance)
- `src/error.rs` — thiserror errors

`tests/` — integration tests (tempfile fake `$HOME`s, assert_cmd). `tests/e2e.sh` — container scenario run by `mask e2e`.

## Non-goals (V1)

- Full Stow CLI compatibility (`--no-folding` flag exists as `fold=false`; other stow flags out of scope)
- `.stow-local-ignore` / `.stow-global-ignore` (deliberately unsupported — use `ignore` in config)
- Packages with `.git` handling beyond the default ignore list (e.g. no `.git`-specific logic; nested repos are fine, they're just not stowed)
- No script/cutover changes; installation is `cargo install --path .` (user-run)
- No JSON schema for anything except `config.toml`

## Constraints

- Rust 1.99.0, edition 2024
- Binary name `rustow`, crate name `rustow`
- GNU Stow 2.4.1 is the behavior reference (`references/stow/`); the status UX alone was informed by tuckr (`references/tuckr/`)
- `mask` is the task runner (`maskfile.md` at repo root; `$MASK` for chaining)
- Do not run any destructive operation against the real `$HOME`; all test rigs use tempdirs or the Void podman container
- Conventional commits
