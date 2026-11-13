# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

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
