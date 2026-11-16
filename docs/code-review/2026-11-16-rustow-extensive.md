# Code Review: rustow (whole tree, `main` @ 36d098b)

- **Date:** 2026-11-16
- **Reviewer:** SeniorReviewer
- **Scope:** Whole working tree — `src/` (4,524 lines, 10 modules), `tests/` (12 suites), `Cargo.toml`, `docs/spec.md`, `README.md`, `mskfile`-adjacent config. Fixed point is `HEAD` = `36d098b` on `main`; worktree clean, so this is the tree as published in v0.1.3.
- **Commits:** 60 commits on `main`, reviewed as a snapshot rather than a range.
- **Spec:** `docs/spec.md` (confirmed 2026-11-09), plus `docs/reports/2026-11-11-v1-report.md` used as the _exclusion list_ for known Stow/Tuckr gaps (§5.1, §7) — none of those gaps are re-reported here.
- **Standards:** `AGENTS.md` (repo root), `docs/spec.md`, `README.md` (user-facing claims treated as contract), `rustfmt.toml`.

## Consent

| Command                                                                                                                                                     | Status                                                                                 |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------- |
| `cargo build --bin rustow`                                                                                                                                  | approved, run                                                                          |
| `cargo test` (full suite, 25 unit + 124 integration)                                                                                                        | approved, run — all green                                                              |
| `target/debug/rustow` against isolated tempdir rigs (`HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`, `--dir`, `--target` all pointed at `/tmp/opencode/rev-*`) | approved, run                                                                          |
| `mask e2e` (podman container)                                                                                                                               | approved in principle, not run — the container scenario was not needed for any finding |
| Anything against the real `$HOME` / `~/.config/rustow`                                                                                                      | refused by the user and by this reviewer; not attempted                                |

Every command below ran with `HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`, `DOTFILES_DIR`, and `DOTFILES_TARGET` overridden into `/tmp/opencode/rev-*`, and `--dir`/`--target` passed explicitly. The live machine was never the target of a mutation; `rustow` in `PATH` was never invoked (only `target/debug/rustow`).

## Summary

The linker core (plan/apply, fold/unfold/refold, conflict resolution) holds up under adversarial fixtures, and all 149 tests pass on this tree. The defects are at the edges of that core: `create` has no guard against adopting a path that already lives _inside_ the source tree (it silently moves one package's files into another), `relink` is not atomic across its unlink/link boundary, and `status` grades two classes of broken package as "linked" or "unlinked" instead of "conflict". The single most important thing is F1: a plain `rustow create <app>` on the documented layout (`~/dotfiles` under `$HOME`, a folded `~/.config`) moves the existing owner package's files into the new package with exit 0 and no warning.

## Findings

### Critical

#### F1 · [correctness] · Confirmed

**Where:** `src/create.rs:L336-L349` in `plan_create` (guard missing also at `L293-L303`)

**What:** `create` moves whatever `ctx.target.join(relpath)` resolves to into the new package, with no check that the resolved source is not already inside `ctx.source_dir`. Because `~/.config` is normally a _fold link_ into some package under `$HOME/dotfiles`, the "target path" is that package's file tree, and it gets relocated.

**Context:** `src_for` (`L51-L56`) defaults `src` to `ctx.target.join(relpath)`, then `L336` only asks `src.exists()`, which follows symlinks. The one containment guard in the function (`L293-L303`) applies _only_ to the explicit `--source` flag, and even there it only rejects the package itself, its ancestors, and its own subtree — not "anywhere inside the source root". The plausible flow is exactly the advertised one: `rustow link base` folds `~/.config -> dotfiles/base/.config`; later the user splits nvim out with `rustow create nvim --path .config/nvim` (or bare `rustow create nvim`, whose default path _is_ `.config/<app>`). `collect_move` then walks `dotfiles/base/.config/nvim`, `move_path` renames it to `dotfiles/nvim/.config/nvim`, and the run exits 0.

**Impact:** Silent cross-package file relocation. `base` loses the subtree, `git status` in the dotfiles repo shows the deletion/rename, and any other machine that links `base` loses that config. Nothing is printed that names `base`. The content is not destroyed — it lands in the new package — which is why a user is likely to notice only when another host or another package goes missing.

**Evidence:**

```
$ rustow --dir $R/dotfiles --target $R/home link base
fold  $R/home/.config -> $R/dotfiles/base/.config
$ rustow --dir $R/dotfiles --target $R/home create nvim2 --path .config/nvim --yes
move $R/home/.config/nvim into $R/dotfiles/nvim2? [--yes]
moved $R/home/.config/nvim -> $R/dotfiles/nvim2/.config/nvim
linked nvim2
   exit=0
$ ls $R/dotfiles/base/.config/nvim
ls: cannot access '.../base/.config/nvim': No such file or directory
```

Same result with per-file links instead of a fold (`tests`-style fixture: `base/.config/nvim/{init.vim,init.lua}` linked under a folded `~/.config`, then `create nvim --path .config/nvim --yes` moved both files out of `base` and left `base/.config/nvim` gone; subsequent `unlink base` reported `unlinked 0 link(s)`).

**Fix:** Resolve `src` (lexically _and_ through the symlink, i.e. `fs::canonicalize` when it exists) and refuse when it is inside `ctx.source_dir`, with a message naming the owning package: "`~/.config/nvim` is already managed by package `base`; unlinking/`--path` or `--source` is required". Extend the `L293-L303` `--source` guard from "package and its ancestors" to the whole `source_dir`, since `--source ~/dotfiles/otherpkg/x` reaches the same relocation by a second route.

### Important

#### F2 · [correctness] · Confirmed

**Where:** `src/cli.rs:L484-L490` in `Commands::Relink`, with the message from `src/link.rs:L821-L824`

**What:** `relink` deletes every link of the named packages first and only then plans+applies the link phase. When that link phase aborts on a conflict, the target keeps _zero_ links and the error text asserts `conflicts detected — nothing was changed:`.

**Context:** The comment (`L486`) states the assumption — "unlink first (never conflicts), then link" — but `apply_plans` treats an unresolved conflict as fatal for the whole run (`link.rs:L821-L824`), and the unlink already happened. Cross-package collisions are the cheap trigger: two packages that both ship `~/.bashrc`. The user's files survive inside the packages; what disappears is the entire linked state of the target.

**Impact:** A failed `relink` leaves the machine in a state the tool's own documentation says it cannot produce ("nothing is half-applied", README L8). The user reads "nothing was changed", believes it, and logs off with no `.bashrc`, `.zshrc`, or `.config` links — on a dotfiles tool, that is a broken login shell away from a bad afternoon. `rustow status` afterwards reports everything unlinked, with nothing recording that `relink` caused it.

**Evidence:**

```
$ rustow --dir $R/dotfiles --target $R/home link alpha
link  $R/home/.bashrc -> .../alpha/.bashrc
link  $R/home/.sharedrc -> .../alpha/.sharedrc
$ rustow --dir $R/dotfiles --target $R/home relink alpha beta
unlink $R/home/.bashrc
unlink $R/home/.sharedrc
unlinked 2 link(s), 1 foreign entr(ies) left alone
rustow: conflicts detected — nothing was changed:
  $R/home/.bashrc (claimed by multiple packages)
   exit=1
$ find $R/home -maxdepth 2
$R/home
```

**Fix:** Compute the whole link plan for the requested packages _before_ unlinking; abort if that plan contains unresolved conflicts. The cheap version: run `plan_link_all` first, and if it yields conflicts, fail without unlinking. (Alternatively add `--no-unlink`-style two-command guidance, but the plan-first version preserves the advertised contract.)

#### F3 · [correctness] · Confirmed

**Where:** `src/status.rs:L118-L122` with `is_pkg_link` at `L76-L80`

**What:** A directory entry is graded `linked` whenever the target is a symlink resolving to _anywhere inside the package root_. A fold link pointing at the wrong directory of the same package passes, sets `covered_prefixes`, and marks the whole subtree as covered.

**Context:** `is_pkg_link(dst, pkg_root)` is `resolve_link(dst).starts_with(pkg_root)` — no comparison against `pkg_root.join(entry.pkg_rel)`. The planner, by contrast, distinguishes the three cases (`link.rs:L332-L350`): link to the same directory is `SkipIdempotent`, link to another directory _inside this package_ is a stale fold to be repaired. So `status` and `link` disagree about the same on-disk state, and `status` is the one the user is told to trust ("Status column, not guesses", README L7).

**Impact:** False-positive health with exit code 0. `rustow status` in CI or a shell prompt says the machine is fine while `~/.config` exposes the wrong directory — i.e. the app silently reads another app's config tree. Exit code 0 also breaks the scripting contract the README advertises ("0 iff every inspected package is fully linked").

**Evidence:**

```
$ ln -s .../dotfiles/pkg/.config/b $R/home/.config   # pkg also has .config/a
$ rustow --dir ... --target ... status pkg
│ pkg │ linked │ ok │        status exit=0
$ rustow --dir ... --target ... link pkg -n
fold  $R/home/.config -> $R/dotfiles/pkg/.config      # planner disagrees: would repair
```

**Fix:** In the `EntryKind::Dir` branch, require `resolve_link(dst) == lexical_normalize(pkg_root.join(&entry.pkg_rel))` for the "folded, covers subtree" verdict; anything else inside `source` is a `conflict` with a reason like `fold points at <other dir>`.

#### F4 · [correctness] · Confirmed

**Where:** `src/status.rs:L118-L133` (the `EntryKind::Dir` branch) and `L135-L175` (the leaf match)

**What:** A real file sitting where the package needs a directory is not reported as a conflict. The dir entry falls through the `is_symlink`/`is_metadata` checks to `// real dir (or missing) — children are checked individually` and `continue`s without counting anything; the children then fail with `ENOTDIR` and are counted as `missing`.

**Context:** The planner reports this exact state as `ConflictReason::DirFileMismatch` (`link.rs:L378-L385` for dirs, `L505-L510` for leaves) and refuses the run. `status` has no counterpart branch for "target exists but is the wrong kind", so the two commands describe the same filesystem differently. `README.md` L48 and `docs/reports/…v1-report.md` §4.5 both promise per-package `conflict` detection.

**Impact:** The status table's whole purpose is telling a user _why_ a package is not linked. Here it says `unlinked · 1 not linked`, which reads as "you never ran `link`", and the user's next action (`rustow link pkg`) produces a conflict report instead. Exit code is 1, so scripts are not fooled; the human is.

**Evidence:**

```
# ~/.config/x is a regular file; pkg wants .config/x/ as a directory
$ rustow status pkg
│ pkg │ unlinked │ 1 not linked │     status exit=1
$ rustow link pkg
rustow: conflicts detected — nothing was changed:
  $R/home/.config/x (file/directory mismatch)
  link exit=1
```

**Fix:** Add the missing arms: dir entry whose target exists and is not a directory → `conflicts += 1` with reason `file where a directory is expected`; leaf entry whose target is a directory or a foreign/external symlink → same treatment. The planner already computes these verdicts; `status` should reuse `Planner` (or its probe) instead of maintaining a parallel, coarser classifier.

### Minor

#### F5 · [correctness] · Confirmed

**Where:** `src/link.rs:L1052` in `plan_clean`

**What:** `plan_clean` walks the entire target tree unconditionally, so when `source` is inside `target` (the layout the spec prescribes: `~/dotfiles/Configs/<pkg>/…` under `$HOME`), it deletes broken symlinks _inside the source repo_, and it ignores the `ignore` list while doing it (parameter is `_ignore`).

**Context:** `plan_clean(source, target, _ignore)` pushes `target` on a stack and descends every real directory, with no check against `source`. Any broken link inside a package that points into the source tree matches and is removed. `docs/reports/…v1-report.md` §4.2 documents clean as "deletes only broken rustow-owned links", which is literally true and practically surprising: it edits the user's version-controlled dotfiles, and `rustow link` would never create such a link.

**Impact:** `rustow clean` produces deletions inside `~/dotfiles` (`git status` noise, potential loss of intentionally-broken links used as "disabled config" markers). Cost is low and recoverable from git, but the command's own help text says it sweeps the _target_.

**Evidence:**

```
$ rustow --dir ~/dotfiles --target ~ clean -n     # source under target
would delete /tmp/opencode/rev-exp/e4/home/dotfiles/pkg2/broken
$ rustow … clean
deleted /tmp/opencode/rev-exp/e4/home/dotfiles/pkg2/broken
```

**Fix:** Skip descending into any directory equal to (or containing) `source`, or filter candidate links whose path is under `source`. Also either honour `ignore` in the walk or delete the parameter so the signature stops implying it.

#### F6 · [correctness] · Confirmed

**Where:** `src/config.rs:L154`, mirrored in `docs/config.toml:L2`

**What:** The generated/default configuration advertises `# Docs: https://github.com/othman-moussa/rustow`, while the project's own `repository` (`Cargo.toml:L7`), README, and CHANGELOG all use `https://github.com/AhmedOsman101/Rustow`.

**Context:** This string is what `rustow config init`, `rustow config default`, and the provenance prompt's "create config now" all write into `~/.config/rustow/config.toml`, and it is the first line a new user reads. `tests/schema_fresh.rs` only pins `docs/config.toml` against `DEFAULT_CONFIG_TOML`, so both copies of the wrong URL agree and the drift is invisible to CI.

**Impact:** Every generated config points at a repository that is not this project. Low blast radius (a comment), but it is the most-read line in the file and it is wrong.

**Evidence:** `grep -rn "othman-moussa" src docs` → `src/config.rs:154`, `docs/config.toml:2`; `cargo` manifest and README use the `AhmedOsman101/Rustow` URL.

**Fix:** Change both to the canonical repository URL. (Rooting the constant in `env!("CARGO_PKG_REPOSITORY")` would make the two impossible to diverge.)

#### F7 · [spec] · Confirmed

**Where:** `src/cli.rs:L582-L586` in `main_entry`; claim in `docs/reports/2026-11-11-v1-report.md` §4.6

**What:** Exit code 2 is documented as "user aborted", but a conflict that the user refuses (interactive abort, or a non-TTY abort) returns `Error::Conflicts` and exits 1. Code 2 is only reachable from `Error::Aborted`, which comes exclusively from the provenance prompt and `create`'s confirmation. Separately, the report states the codes are "documented in the spec now" — `docs/spec.md` contains no exit-code section at all (`grep -n exit docs/spec.md` → only L142/L161, neither about codes).

**Context:** `apply_plans` (`link.rs:L821-L824`) maps "some outcome was Abort" onto `Error::Conflicts`, whose only handler is `main_entry`'s `1`. So the documented distinction between "the tool declined" (1) and "you declined" (2) does not exist where it matters, and the spec is cited as the source of truth for something it does not contain.

**Impact:** Scripts cannot distinguish "conflicts, fix your tree" from "I hit Ctrl-C/aborted the prompt". Documentation-shaped debt: the next reader trusts a spec section that is not there.

**Evidence:** `rustow link b` (non-TTY, conflicting package) → `rustow: conflicts detected — nothing was changed: …` then `exit=1`.

**Fix:** Either route an aborted conflict resolution to `Error::Aborted` (exit 2) and keep `Conflicts` for "aborted without a decision", or drop code 2 from the documentation; and put the exit-code table into `docs/spec.md` so the report's claim becomes true.

#### F8 · [correctness] · Confirmed

**Where:** `src/config.rs:L557-L559` in `resolve` (compare `L553-L555`)

**What:** `--dir` sets `provenance = Provenance::Cli`; `--target` sets the target and leaves provenance alone. The provenance notice then misattributes the value it just accepted from the CLI to an environment variable that was never set.

**Context:** `notice_description` (`L640-L645`) prints `DOTFILES_TARGET={resolved.target}`, and `resolved.target` is post-CLI-override. `notice_kind` (`L627`) shows no notice at all when provenance is `Cli`, so the `--dir` path is silent and the `--target` path is not — same class of override, two behaviours.

**Impact:** The one line of output whose job is to explain where settings came from is false, and a CLI-only override still triggers the "no config.toml" prompt on a TTY, where `[s]ilence` writes a permanent stamp file and `[c]reate` writes a config file the user may not have wanted.

**Evidence:**

```
$ DOTFILES_DIR=$R/src rustow --target $R/home link p     # DOTFILES_TARGET never set
rustow: warning: no config.toml found — using env vars
  (DOTFILES_DIR=/tmp/opencode/rev-e15/src, DOTFILES_TARGET=/tmp/opencode/rev-e15/home)
```

**Fix:** Set `provenance = Provenance::Cli` in the `cli.target` branch (and make `notice_description` build its text from the layer that actually supplied each value, not from the merged result).

#### F9 · [correctness] · Confirmed

**Where:** `src/config.rs:L311` in `expand_tilde_and_vars`

**What:** A backslash escape pushes the escaped byte as a `char` (`out.push(bytes[i + 1] as char)`), which is a Latin-1 reinterpretation, not the original UTF-8 code point. Escaping any non-ASCII character produces mojibake; for a multi-byte character the remaining continuation bytes are then copied verbatim by the `_` arm.

**Context:** `$VAR`/`~` expansion in stowrc values is a supported, tested feature (`config::task3_tests::*`), and the surrounding `_` arm is careful to copy whole code points — so the escape arm is the one inconsistent branch. Trigger is narrow (a stowrc path containing `\` before a non-ASCII character), but it is silent when it happens.

**Impact:** A path silently becomes a different, wrong path; `--dir`/`--target` then point somewhere unintended. Very low likelihood, non-zero.

**Evidence:** Inspection of `L302-L320`; `u8 as char` is a documented Latin-1 cast, so `\é` (0xC3 0xA9) emits `Ã` followed by a stray byte.

**Fix:** Decode the escape by taking the next _code point_ (`s[i+1..].chars().next()`) instead of a byte, or restrict escapes to the documented set (`$`, `~`, `\`).

#### F10 · [smell] · Confirmed

**Where:** `src/link.rs:L1121` (`walk_unlink(_source, …)`) and `src/link.rs:L1052` (`plan_clean(…, _ignore)`)

**What:** Two `pub`/internal signatures carry parameters that are never read; `plan_clean`'s `_ignore` in particular implies the walk honours ignore globs when it does not (see F5).

**Context:** `walk_unlink` takes `_source` at `L1121` and threads it through the recursive call at `L1105` while `plan_unlink` already has `source`. Callers pass real values, so the parameters look load-bearing at every call site.

**Impact:** Readability only, but it is the kind of dead parameter that the next patch will "start using" without noticing the semantics were never defined.

**Evidence:** `#[allow(clippy::too_many_arguments)]` on `walk_unlink` plus `_`-prefixed parameters; no read of either inside the bodies.

**Fix:** Drop `_source`; give `plan_clean` an `Option<&Ignore>` that it actually applies, or drop the parameter.

### Question

#### F11 · [smell] · Confirmed

**Where:** `Cargo.toml:L25`

**What:** `husky-rs = "0.4.0"` is declared as a normal runtime dependency and is referenced nowhere in `src/` or `tests/`; there is no `build.rs`. It compiles into every build and does nothing at runtime.

**Context:** `AGENTS.md` says: "`husky-rs` (regular dependency) is the user's git-hooks tooling — theirs, leave it alone." Taken literally, the declaration is intentional. Taken literally, it is also inert: `.husky/pre-commit` is a plain bash script invoked by git, and nothing in the crate installs or runs `husky-rs`. Both readings are plausible, which is why this is a question and not a defect.

**Impact:** Extra build time and supply-chain surface for zero behaviour. If the intent was for `husky-rs` to install the hook, that installation does not happen.

**Evidence:** `grep -rn husky src/ tests/` → no matches; `.husky/pre-commit` is self-contained bash; `Cargo.toml:L25` is the only mention.

**Fix:** Decide: either call it (e.g. from a small `main`-time installer or a build script) or move it to `[build-dependencies]`/drop it. If the answer is "leave it", add one line to `AGENTS.md` saying why, so the next reviewer does not re-open it.

## Unverified, and why

None. Every finding above was either reproduced with `target/debug/rustow` in an isolated fixture (F1–F5, F7, F8) or is a statically decidable fact about the tree (F6, F9, F10, F11).

## Declined to judge

- Property-based planner tests, TTY interaction tests, `status --json` schema versioning, per-test exit-code assertions, coverage measurement, performance benchmarks — all already listed as missing/partial in `docs/reports/2026-11-11-v1-report.md` §5.1 and §5.3; re-reporting them would be padding.
- Stow/Tuckr feature gaps (`--defer`, `--override`, `--compat`, `--dotfiles`, `.stow-local-ignore`, `.stow-global-ignore`, profiles, hooks, secrets, bulk migration) — explicitly decided out of scope in the same report §6.2–§6.3 and §7.
- Windows symlink semantics and cross-platform behaviour — no Windows runner was available; `platform.rs` (`L15-L26`) reads correctly for the existing/non-existing cases, but the "package contains a symlink to a directory" case would create a _file_ symlink on Windows. Unproven, so not a finding.
- `husky-rs` as a design choice — see F11, asked as a question because `AGENTS.md` claims it as intentional.
- Interactive merge UX (diff-tool choice, `--wait` assumptions for GUI editors) — needs a TTY harness that does not exist yet; read only, no verdict.
- `-f`/`--adopt` semantics versus GNU Stow's — behaviour matches the documented subset; no divergence found to report.

## Roll-up

- correctness · 1 Critical, 3 Important, 4 Minor
- spec · 1 Minor (F7)
- smell · 1 Minor (F10), 1 Question (F11)
- axis coverage: correctness (link, unlink, clean, create, status, config), spec (config layering, exit codes, documented claims), smell (dead params, unused dependency, duplicated status/planner logic)
- 11 findings, 11 confirmed; 0 unverified
