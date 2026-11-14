# Changelog

All notable changes to this project will be documented in this file.

## [0.1.3](https://github.com/AhmedOsman101/Rustow/compare/rustow-v0.1.2...rustow-v0.1.3) (2026-11-14)

### Bug Fixes

- **status:** print a plain message instead of an empty table when nothing matches ([882e937](https://github.com/AhmedOsman101/Rustow/commit/882e937be3d78834e4d3a6063cde9c37206311d1))

## [0.1.2] - 2026-11-13

### Added

- _(release)_ add release-pr and gh release tasks to maskfile
- _(config)_ add --no-stowrc kill switch, RUSTOW_NO_STOWRC env var, and source==target refusal
- _(clean)_ delete broken rustow-owned symlinks
- _(fold-ignore)_ per-package fold opt-out via config fold_ignore key

### Fixed

- _(platform)_ make the crate build on Windows (symlink_file/symlink_dir)

### Other

- update release-plz configuration for automated releases
- _(reports)_ rewrite V1 coverage report
- _(edge-cases)_ non-ASCII paths, fold=false, stowrc opt-out, source==target, clean idempotency
- update 7 files 2026-11-13@07:50pm
- update maskfile.md 2026-11-12@01:59am
- add hard rule banning command-output filtering; add question-tool ban
- add V1 coverage report
- _(release)_ only build on release-plz merge or manual dispatch
- _(actions)_ bump checkout to v7, upload-artifact to v7, download-artifact to v8

## [0.1.1](https://github.com/AhmedOsman101/Rustow/compare/v0.1.0...v0.1.1) - 2026-11-11

### Fixed

- _(ci)_ make schema.json target default machine-independent
- _(lint)_ make schema check environment-independent; fix clippy needless borrows

### Other

- apply pre-commit formatting
- fix schema freshness check to compare JSON trees; attach release binaries via workflow_run/dispatch
- _(deps)_ bump toml from 0.9.12+spec-1.1.0 to 1.2.1+spec-1.1.0 ([#9](https://github.com/AhmedOsman101/Rustow/pull/9))
- _(deps)_ bump dialoguer from 0.11.0 to 0.12.0 ([#8](https://github.com/AhmedOsman101/Rustow/pull/8))
- _(deps)_ bump tabled from 0.17.0 to 0.22.0 ([#10](https://github.com/AhmedOsman101/Rustow/pull/10))
- _(deps)_ bump shlex from 1.3.0 to 2.0.1 ([#7](https://github.com/AhmedOsman101/Rustow/pull/7))
- _(deps)_ bump dirs from 6.0.0 to 7.0.0 ([#6](https://github.com/AhmedOsman101/Rustow/pull/6))
- _(deps)_ bump softprops/action-gh-release from 2 to 3 ([#2](https://github.com/AhmedOsman101/Rustow/pull/2))

## [0.1.0] - 2026-11-10

## Added

- `rustow` — a Stow-compatible dotfiles linker in Rust: fold/unfold/refold semantics, two-phase conflict resolution with interactive `merge`/`overwrite`/`skip`, JSON `--json` status output, `create`/`add` package workflow, `config default`/`config init`, `.stowrc` fallback, and a Void Linux e2e harness.
