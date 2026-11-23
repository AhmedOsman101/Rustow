# rustow

A Stow-compatible dotfiles linker written in Rust, with a per-package health report built in.

GNU Stow does the core job well; `rustow` replaces it for one workflow: one directory per package under a single source root, symlinked onto `$HOME`. It keeps Stow's tree-folding model (with the correctness fixes that come with a from-scratch rewrite) and adds a real `status` command, JSON output, interactive conflict resolution, and an `add`-style workflow.

- **Status column, not guesses** — `rustow status` tells you exactly which packages are linked, partial, unlinked, or in conflict, per package and via `--json` for scripting.
- **Two-phase safety** — plans are computed fully before anything touches disk. Conflicts abort the whole run; nothing is half-applied.
- **Interactive conflict resolution** — existing files are negotiated (`merge / overwrite / skip`), never silently clobbered.

## Install

```sh
cargo install rustow --git https://github.com/AhmedOsman101/rustow
```

Prebuilt binaries are attached to each [GitHub release](https://github.com/AhmedOsman101/Rustow/releases):

| Asset                                                 | Platform                     |
| ----------------------------------------------------- | ---------------------------- |
| `rustow-linux-gnu-x64`                                | Linux x86_64 (glibc)         |
| `rustow-linux-gnu-arm64`                              | Linux aarch64 (glibc)        |
| `rustow-linux-musl-x64`                               | Linux x86_64 (static musl)   |
| `rustow-linux-musl-arm64`                             | Linux aarch64 (static musl)  |
| `rustow-macos-x64` / `rustow-macos-arm64`             | macOS x86_64 / Apple silicon |
| `rustow-windows-x64.exe` / `rustow-windows-arm64.exe` | Windows x86_64 / arm64       |

## Platform support

rustow targets Unix-like systems. On Windows, use it under Git Bash / WSL / Cygwin — Windows is not a supported first-class target. Windows binaries still ship because they work in those environments, but native Windows behavior is not supported or tested (see [ADR 0007](./docs/adr/0007-distribution.md)).

## Quickstart

Declare where your packages live (one directory per package) in `~/.config/rustow/config.toml`:

```toml
source = "~/dotfiles/Configs"
target = "~"          # optional, defaults to $HOME
```

Or run `rustow config init` to write the annotated default. Then:

```sh
rustow sync              # link everything in the source directory
rustow status            # see what's linked (exit code reflects health)
rustow link git zsh      # link specific packages
rustow relink zsh        # restow one package
rustow unlink zsh        # remove one package's links
rustow reset             # remove all links
```

## Commands

| Command                                              | Description                                                                                                                                                                                                                                   |
| ---------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `sync`                                               | Link all packages in `source`                                                                                                                                                                                                                 |
| `link <pkg>...`                                      | Link specific packages                                                                                                                                                                                                                        |
| `unlink <pkg>...` (alias `remove`, `rm`)             | Unlink packages                                                                                                                                                                                                                               |
| `reset`                                              | Unlink all packages                                                                                                                                                                                                                           |
| `relink <pkg>...`                                    | Unlink then re-link (restow)                                                                                                                                                                                                                  |
| `status [pkg...]`                                    | Table of `Package / Status / Notes`; `-s` summary only; `--json` machine-readable; `--state <linked\|partial\|unlinked\|conflict>`to filter (repeatable; applies to`--json`too);`-v`per-issue text detail (capped per package);`-vv` uncapped | `create <app>` (alias `add`) | Create a package, optionally adopting an existing target path |
| `config default`                                     | Print the annotated default config                                                                                                                                                                                                            |
| `config init`                                        | Write the default config (use `--prefill` was not added; run interactively via the provenance prompt for a pre-filled one)                                                                                                                    |
| `config migrate [--yes] [--dry-run] [--stowrc PATH]` | Adopt stowrc values into `config.toml` without deleting the stowrc                                                                                                                                                                            |

Global flags: `-n|--dry-run`, `-f|--force`, `--adopt`, `-v|--verbose`, `--dir`, `--target`, `--as-root` (authorize root-mapped steps without prompting).

`unlink` requires at least one package argument; `reset` does everything at once. `relink` takes packages explicitly — there is no `relink` with no arguments (use `reset && sync`).

## Exit codes

rustow follows the Unix/clap convention ([ADR 0003](./docs/adr/0003-exit-codes.md)):

| Code | Meaning                                                                                        |
| ---- | ---------------------------------------------------------------------------------------------- |
| `0`  | Success. For `status`, every inspected package is fully linked and conflict-free.              |
| `1`  | General failure: unresolved conflicts, runtime errors, or you declining an interactive prompt. |
| `2`  | Usage error: unknown or malformed arguments (clap's built-in behavior).                        |

There is no separate code for "user aborted" — declining a prompt is just a runtime failure. stderr always names the reason, so scripts that need to tell cases apart should read it.

## Link semantics

- Package tree mirrors the target root (`$HOME`); `<pkg>/.config/foo/bar` links to `~/.config/foo/bar`.
- **Folding**: a package directory is replaced by a _single_ symlink when the target doesn't exist yet — as long as the subtree contains no ignored paths (e.g. a nested `.git`), in which case it descends per-file.
- **Unfolding**: a folded symlink (owned by rustow/stow) colliding with a real directory from another package is split open and both packages re-linked individually.
- **Refolding** (on unlink): a directory that ends up containing only links into one package is collapsed back into a single symlink.
- **Idempotent**: re-linking is a no-op for correct links; stale links pointing into the package are replaced; correct links are never re-created.

## Conflicts

Conflicts are detected at plan time and resolved **before anything is written**:

| Situation                                          | Default behavior                                                                                                                                                                         |
| -------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Real file at target                                | TTY: prompt `merge / overwrite / skip` (optionally non-interactive via `on_conflict` / `-f` / `--adopt`). Non-TTY without a flag: abort the whole run, report everything, change nothing |
| Symlink pointing to exactly the right package path | silently skipped (idempotent)                                                                                                                                                            |
| Dangling symlink pointing into the package         | repaired                                                                                                                                                                                 |
| Symlink into another package                       | unfolded (dir/dir) or conflict (file/dir)                                                                                                                                                |
| External symlink or real file (`-f`)               | target removed, link created                                                                                                                                                             |
| Existing content (`--adopt`)                       | target's content moved into the package, then linked                                                                                                                                     |

`merge` opens a diff tool (config `diff_tool` template -> PATH-detected registry -> `$EDITOR` for plain editors). Reconcile into the **package file**; `rustow` then links the package version onto the target.

Per-file adoption in bulk: `rustow link --adopt <pkg>` is equivalent to stow's `--adopt`.

## status

```text
╭─────────┬──────────┬────────────────╮
│ Package │ Status   │ Notes          │
├─────────┼──────────┼────────────────┤
│ eza     │ linked   │ ok             │
│ zsh     │ partial  │ 1 not linked   │
│ vim     │ conflict │ 2 conflicts    │
│ ghost   │ unlinked │ empty package  │
╰─────────┴──────────┴────────────────╯
```

Exit code is `0` iff every inspected package is fully linked and conflict-free, otherwise `1`. `rustow status eza` filters to specific packages. `--short` prints a one-line summary; `--json` prints machine-readable output. Packages with root-mapped links carry a `root` note and count those links at their absolute destinations ([ADR 0001](./docs/adr/0001-root-targeting.md)).

## create

`rustow create <app>` replaces the old `mkconf` workflow.

```sh
rustow create vim          # adopt ~/.config/vim into a new package, then link
rustow create zsh          # adopt ~/.zshrc style layouts: create zsh --path .zshrc
rustow create mytool --empty   # create an empty package only
rustow create mytool --source /path/to/files --path .config/mytool
```

- No source found -> prompt: create an empty package, or point at another path, or abort.
- Source found -> show what will move (`-v` for the full list), confirm, move, link.
- Package already exists -> merge the source into it (or, with no source to move, offer to link it). Nothing is ever clobbered without `-f`.
- `-i` prompts before linking (default: link without asking). `-v` shows the move in detail.
- Package gets **only** what you give it — `create vim` does not scaffold `.config/<app>` for empty packages.

## Config

`~/.config/rustow/config.toml`. All keys optional except `source`:

| Key                     | Default                     | Notes                                                                                                                                |
| ----------------------- | --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| `source`                | _(required — no default)_   | Stow dir; direct subdirectories are packages                                                                                         |
| `target`                | `$HOME`                     | Link destination root                                                                                                                |
| `verbose`               | `true`                      |                                                                                                                                      |
| `fold`                  | `true`                      | Fold directories into single symlinks (`false` = GNU Stow's `--no-folding`)                                                          |
| `ignore`                | GNU Stow 2.4.1 default list | Glob patterns matched against package-relative paths, added to the built-in list                                                     |
| `diff_tool`             | unset                       | Command template for the merge diff tool, e.g. `"nvim -d {left} {right}"` or `"hx --new {left} {right}"`; wins over registry/$EDITOR |
| `on_conflict`           | `prompt`                    | `prompt` / `abort` / `skip` / `overwrite` / `adopt` (`prompt` escalates to abort on non-TTY)                                         |
| `elevate_cmd`           | unset (`sudo`, then `doas`) | Helper for root-mapped actions only, probed in that order when unset                                                                 |
| `[package.<name>.root]` | unset                       | Map package-relative paths to absolute destinations outside `target` (see "Root targeting")                                          |

Precedence: CLI flags -> env (`DOTFILES_DIR`, `DOTFILES_TARGET`) -> `config.toml` -> `~/.stowrc` (fallback only when no config file exists; `$VAR`/`~` expansion). `--no-stowrc` or `RUSTOW_NO_STOWRC=1` disables the `~/.stowrc` fallback completely.

Without a config file you'll see a warning naming what's being used instead; on a TTY you can create one pre-filled from the values in use, or silence the notice permanently (`[s]ilence`). `~/.stowrc` is supported for the transition away from GNU Stow.

### Migrating from GNU Stow

`rustow config migrate` adopts the values rustow already reads from `~/.stowrc` (`source`, `target`, `verbose`) into `config.toml`, filling only what the config leaves unset. It is interactive by default; `--yes` adopts without asking and `--dry-run` (the global flag) previews the change. `--stowrc PATH` reads a stowrc from elsewhere. The stowrc is **left untouched** — it is inert once a config file exists, so it can be deleted separately at your leisure ([ADR 0004](./docs/adr/0004-config-migrate.md)).

Schema for editors: [`schema.json`](./schema.json). The full annotated default: [`docs/config.toml`](./docs/config.toml).

## Root targeting

Paths that belong outside your home directory are configured per package — there is no CLI flag to opt a path in:

```toml
[package.snapper.root]
"configs" = "/etc/snapper"              # directory -> one symlink at /etc/snapper/configs
"configs/root" = "/etc/snapper/root"    # file -> the exact destination
```

Keys are package-relative (no `..` escapes); values are absolute after `~` expansion. The source path's filesystem type decides the kind at plan time: a directory becomes a single fold-style symlink at `<value>/<basename(source-dir)>`, a file links to the exact value. A trailing `/` is cosmetic and never decides the kind; a missing mapped source is a plan-time error. Mapped paths never fold or refold. `status` shows them with a `root` note, `unlink` removes exactly those mapped links (leaving user-home links at the same relative paths alone), and `clean` treats them like other owned links.

Applying root-mapped steps needs elevated write access. rustow probes `elevate_cmd`, then `sudo`, then `doas`, and runs **only** those steps through the helper — never the whole command. Parent-directory writability and helper availability are checked at plan time; a failure aborts before anything changes. On a TTY rustow lists the steps and asks `proceed with elevated steps? [y/n]` (`n` aborts everything); on a non-TTY it fails with a hint naming `--as-root`. `--as-root` authorizes the steps non-interactively, while `--dry-run` wins over it and only lists the root actions (`root link …` / `root fold …`). Running as root prints a warning that root's config and home apply. This is a linker that can write system paths — review the plan before authorizing it ([ADR 0001](./docs/adr/0001-root-targeting.md)).

## Safety model

- Every mutation is planned first; conflicts abort before any write.
- Nothing is ever deleted except links rustow (or a compatible stow) created — foreign symlinks and real files are never touched without an explicit `-f`/overwrite choice.
- `--dry-run` previews all of the above, including root actions, and never prompts or elevates.
- Unlink removes only links pointing into the package (then re-folds link-only directories and removes empty directories it created). It never follows foreign symlinks.
- Ignored paths (the built-in default list — e.g. `.git` — plus your `ignore` globs) are never linked, and never appear through folded symlinks either.

## Differences from GNU Stow

rustow is a strict subset of Stow semantics — fold/unfold/refold, two-phase conflicts — plus a `status` report, JSON output, interactive conflict resolution (merge/overwrite/skip), a `create` workflow, and no `.stow-local-ignore` support (use `ignore` in `config.toml` instead).

## Non-goals

- Full GNU Stow CLI/flag compatibility.
- `.stow-local-ignore` / `.stow-global-ignore` (use `ignore` in `config.toml`).
- Owning your dotfiles layout — rustow links what you give it; layout is yours.

## Development

```sh
mask build        # debug binary
mask test         # cargo test
mask check        # lint + tests
mask e2e          # Void Linux podman container end-to-end run
mask schema       # regenerate schema.json
mask release      # cross-build linux release binaries into ./bin (needs `cross`)
mask install      # cargo install --path .
```

Releases are automated with [release-please](https://github.com/googleapis/release-please): merged conventional commits on `main` open a release PR (version bump + changelog); merging it cuts a GitHub release, and the Release workflow attaches cross-built binaries.

License: GPL-2.0-or-later (see [LICENSE](./LICENSE)).
