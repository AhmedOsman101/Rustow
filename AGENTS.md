# AGENTS.md — rustow

`rustow` — Stow-compatible dotfiles linker in Rust (edition 2024, rustc 1.99+). Single crate, lib + thin bin, links a package tree from a stow-style `source` dir onto a `target` root (usually `$HOME`).

## Hard rules for agents (learned the hard way)

- **Never run rustow, stow, or any filesystem mutation against the real `$HOME`.** Every test runs in a tempdir. The e2e harness uses a fake `$HOME` under `/tmp` inside a podman container. If you need to observe behavior on real dotfiles, ask first — never "just try it", never via a test, never via a script.
- **Never kill, signal, or reparent processes on this machine.** If a process looks confusing or relevant, ask the user first — they run things interactively and pipeline into pagers; it is usually theirs, not yours.
- **Read full command output.** No `| tail`, `| head`, or `| grep` to hide what a command printed. Run `mask check` plain and read it. A passing-looking exit code is not proof.
- **Do not leave stray sessions, hooks, or tests behind** — if you open a pty/server/container, close it, or hand it back cleanly.
- **Match docs to reality.** Only write what you verified. Never leak session-only coordination (agent-vs-user notes, task assignments) into README, docs/, commit messages, or spec files.
- **No user-specific examples presented as the norm.** Fixture names go in tests only; docs stay generic. If a user ran a command with their real config, that's their setup, not the product's.
- **Don't namedrop tools you don't build on.** Mention a third-party project only if you genuinely drew from it, and say precisely what. Otherwise it's noise.
- **When a re-plan is small, amend don't tack on.** Style-only follow-ups go in with `--amend --no-edit` (check push state first; never rewrite shared history).
- **Verify before claiming.** "Done" requires the full output read: lint + all tests green. If something looks off mid-run, stop and say so instead of burying it.

## Commands (mask is the task runner — use it, not raw cargo)

- `mask check` — lint (`cargo fmt --check` + `cargo clippy --all-targets`) + `cargo test`
- `mask format [file...]` — `cargo fmt`, or rustfmt the given files (accepts a space-separated string via eval)
- `mask test` — `cargo test`
- `mask lint` — fmt check + clippy
- `mask schema` — regenerate `schema.json` from `Config` (schemars); `mask schema check` verifies freshness (test enforces this)
- `mask e2e` — builds `rustow-e2e` Void Linux container (`./Dockerfile`) and runs `tests/e2e.sh` (fake `$HOME`)
- `mask install`, `mask build`, `mask clean`
- Single test: `cargo test --test link_unlink unfold_then_unlink_refolds`

E2E notes: image is `ghcr.io/void-linux/void-glibc-full`; container must run with `--userns=keep-id` (rootless podman maps uids), else UID mismatch makes `/app/target` unwritable. Image build takes a few minutes; run `mask e2e` rather than the podman bits by hand.

Always `mask format` before committing — otherwise the pre-commit hook (`.husky/pre-commit`) rewrites your files and leaves the worktree dirty. Hook also runs `cargo check` and prompts via `/dev/tty`; non-interactive it auto-proceeds.

## Layout & invariants

- `src/` — `link.rs` (two-phase plan/apply; planner decides everything before any fs op touches disk), `config.rs` (model + precedence + provenance prompt), `cli.rs`, `status.rs`, `create.rs`, `ignore.rs`, `diff.rs`, `prompt.rs`, `error.rs`
- `references/` — read-only behavior references (GNU Stow source); not imported
- `docs/spec.md` — the confirmed design (the law); `docs/plans/2026-11-09-rustow.md` — execution history; keep spec in sync with behavior, not the reverse
- `docs/config.toml` must equal `rustow config default` output exactly (test enforces this)
- `schema.json` at repo root is generated, not hand-written

Behavior to know:

- Fold = whole package dir → one symlink, but only over subtrees with no ignored paths (nested `.git` stays unlinked); unfold = split open a rustow/stow-owned fold colliding with another package; refold = collapse link-only dirs on unlink.
- Two-phase conflicts: nothing mutates until every conflict resolves (`merge`/`overwrite`/`skip`, or `-f`/`--adopt`); non-TTY without a flag aborts the entire run.
- Config precedence: CLI flags > env (`DOTFILES_DIR`, `DOTFILES_TARGET`) > `config.toml` > `~/.stowrc` (stowrc only when no config.toml). `source` has no default — error out until set.
- tabled is used with default-features off + `features = ["ansi"]` (derive dropped to avoid `proc-macro-error2`; color width is ANSI-aware).
- `status` exit code: 0 iff all inspected packages fully linked (unlinked/partial/conflict → 1).

## Tests

- `tests/` — integration tests drive tempdir fixtures only; set `HOME`, `DOTFILES_DIR`/`DOTFILES_TARGET` (or `XDG_CONFIG_HOME`) per fixture; never the real environment. A prior fixture leaked `$HOME` and ran the binary against the real stowrc — don't reintroduce that.
- `tests/regression.rs` pins review findings; `tests/e2e.sh` is the container scenario.
- Commits use Conventional Commits. `husky-rs` (regular dependency) is the user's git-hooks tooling — theirs, leave it alone.
