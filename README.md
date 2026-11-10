# rustow

A Stow-compatible dotfiles linker written in Rust, with a per-package health report built in.

GNU Stow does the core job well; `rustow` replaces it for one workflow: one directory per package under a single source root, symlinked onto `$HOME`. It keeps Stow's tree-folding model (with the correctness fixes that come with a from-scratch rewrite) and adds a real `status` command, JSON output, interactive conflict resolution, and an `add`-style workflow.

- **Status column, not guesses** — `rustow status` tells you exactly which packages are linked, partial, unlinked, or in conflict, per package and via `--json` for scripting.
- **Two-phase safety** — plans are computed fully before anything touches disk. Conflicts abort the whole run; nothing is half-applied.
- **Interactive conflict resolution** — existing files are negotiated (`merge / overwrite / skip`), never silently clobbered.

## Install

```sh
cargo install --path .
```

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

| Command | Description |
|---|---|
| `sync` | Link all packages in `source` |
| `link <pkg>...` | Link specific packages |
| `unlink <pkg>...` (alias `remove`, `rm`) | Unlink packages |
| `reset` | Unlink all packages |
| `relink <pkg>...` | Unlink then re-link (restow) |
| `status [pkg...]` | Table of `Package / Status / Notes`; `-s` summary only; `--json` machine-readable; `--state linked|partial|unlinked|conflict` to filter (repeatable; applies to `--json` too) |
| `create <app>` (alias `add`) | Create a package, optionally adopting an existing target path |
| `config default` | Print the annotated default config |
| `config init` | Write the default config (use `--prefill` was not added; run interactively via the provenance prompt for a pre-filled one) |

Global flags: `-n|--dry-run`, `-f|--force`, `--adopt`, `-v|--verbose`, `--dir`, `--target`.

`unlink` requires at least one package argument; `reset` does everything at once. `relink` takes packages explicitly — there is no `relink` with no arguments (use `reset && sync`).

## Link semantics

- Package tree mirrors the target root (`$HOME`); `<pkg>/.config/foo/bar` links to `~/.config/foo/bar`.
- **Folding**: a package directory is replaced by a *single* symlink when the target doesn't exist yet — as long as the subtree contains no ignored paths (e.g. a nested `.git`), in which case it descends per-file.
- **Unfolding**: a folded symlink (owned by rustow/stow) colliding with a real directory from another package is split open and both packages re-linked individually.
- **Refolding** (on unlink): a directory that ends up containing only links into one package is collapsed back into a single symlink.
- **Idempotent**: re-linking is a no-op for correct links; stale links pointing into the package are replaced; correct links are never re-created.

## Conflicts

Conflicts are detected at plan time and resolved **before anything is written**:

| Situation | Default behavior |
|---|---|
| Real file at target | TTY: prompt `merge / overwrite / skip` (optionally non-interactive via `on_conflict` / `-f` / `--adopt`). Non-TTY without a flag: abort the whole run, report everything, change nothing |
| Symlink pointing to exactly the right package path | silently skipped (idempotent) |
| Dangling symlink pointing into the package | repaired |
| Symlink into another package | unfolded (dir/dir) or conflict (file/dir) |
| External symlink or real file (`-f`) | target removed, link created |
| Existing content (`--adopt`) | target's content moved into the package, then linked |

`merge` opens a diff tool (config `diff_tool` template → PATH-detected registry → `$EDITOR` for plain editors). Reconcile into the **package file**; `rustow` then links the package version onto the target.

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

Exit code is `0` iff every inspected package is fully linked and conflict-free, otherwise `1`. `rustow status eza` filters to specific packages. `--short` prints a one-line summary; `--json` prints machine-readable output.

## create

`rustow create <app>` replaces the old `mkconf` workflow.

```sh
rustow create vim          # adopt ~/.config/vim into a new package, then link
rustow create zsh          # adopt ~/.zshrc style layouts: create zsh --path .zshrc
rustow create mytool --empty   # create an empty package only
rustow create mytool --source /path/to/files --path .config/mytool
```

- No source found → prompt: create an empty package, or point at another path, or abort.
- Source found → show what will move (`-v` for the full list), confirm, move, link.
- Package already exists → merge the source into it (or, with no source to move, offer to link it). Nothing is ever clobbered without `-f`.
- `-i` prompts before linking (default: link without asking). `-v` shows the move in detail.
- Package gets **only** what you give it — `create vim` does not scaffold `.config/<app>` for empty packages.

## Config

`~/.config/rustow/config.toml`. All keys optional except `source`:

| Key | Default | Notes |
|---|---|---|
| `source` | *(required — no default)* | Stow dir; direct subdirectories are packages |
| `target` | `$HOME` | Link destination root |
| `verbose` | `true` | |
| `fold` | `true` | Fold directories into single symlinks (`false` = GNU Stow's `--no-folding`) |
| `ignore` | GNU Stow 2.4.1 default list | Glob patterns matched against package-relative paths, added to the built-in list |
| `diff_tool` | unset | Command template for the merge diff tool, e.g. `"nvim -d {left} {right}"` or `"hx --new {left} {right}"`; wins over registry/$EDITOR |
| `on_conflict` | `prompt` | `prompt` / `abort` / `skip` / `overwrite` / `adopt` (`prompt` escalates to abort on non-TTY) |

Precedence: CLI flags → env (`DOTFILES_DIR`, `DOTFILES_TARGET`) → `config.toml` → `~/.stowrc` (fallback only when no config file exists; `$VAR`/`~` expansion).

Without a config file you'll see a warning naming what's being used instead; on a TTY you can create one pre-filled from the values in use, or silence the notice permanently (`[s]ilence`). `~/.stowrc` is supported for the transition away from GNU Stow.

Schema for editors: [`schema.json`](./schema.json). The full annotated default: [`docs/config.toml`](./docs/config.toml).

## Safety model

- Every mutation is planned first; conflicts abort before any write.
- Nothing is ever deleted except links rustow (or a compatible stow) created — foreign symlinks and real files are never touched without an explicit `-f`/overwrite choice.
- `--dry-run` previews all of the above.
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
mask install      # cargo install --path .
```

License: GPL-2.0-or-later (see [LICENSE](./LICENSE)).
