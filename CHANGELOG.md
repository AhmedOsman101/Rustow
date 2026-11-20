# Changelog

All notable changes to this project will be documented in this file.

## [0.2.0](https://github.com/AhmedOsman101/Rustow/compare/rustow-v0.1.4...rustow-v0.2.0) (2026-11-20)


### Features

* **apply:** elevated root-mapped actions with plan-time preflight ([ee4f27c](https://github.com/AhmedOsman101/Rustow/commit/ee4f27c99b7fab7e193f7f4cbcb63b7cbd637534))
* **config:** migrate stowrc into config.toml ([4fcf7a4](https://github.com/AhmedOsman101/Rustow/commit/4fcf7a4e1a4ed5626e846d268920c7f0cd95d3a1))
* **config:** offer stowrc adoption during config init ([01b892d](https://github.com/AhmedOsman101/Rustow/commit/01b892d4ea3f5fba5965c0f9ccc16e713e928b55))
* **config:** per-package includes key ([cd524c6](https://github.com/AhmedOsman101/Rustow/commit/cd524c61688010a01e42791d3ebefdbcd783a4c8))
* **config:** root mapping per package ([2389fae](https://github.com/AhmedOsman101/Rustow/commit/2389faefe854b9afbfc0cde14a73df6c0d7f037a))
* **linker:** per-package includes drive linking and fold demotion ([25bfa24](https://github.com/AhmedOsman101/Rustow/commit/25bfa2444b6e99ee1beb9fd0892ef834f26bb467))
* **planner:** root-mapped actions ([ed54fa6](https://github.com/AhmedOsman101/Rustow/commit/ed54fa65f39c7a57b73392707a8a4580452b1c91))
* **root:** status/unlink wiring and docs ([598a1ae](https://github.com/AhmedOsman101/Rustow/commit/598a1ae85583b3a6e6a117a6fc74a6f066613726))
* **status:** add top-level version to --json output ([462324c](https://github.com/AhmedOsman101/Rustow/commit/462324cc9fa1328bf3883c965fb4b5508922e906))


### Bug Fixes

* **clean:** never sweep inside the source tree ([09faa28](https://github.com/AhmedOsman101/Rustow/commit/09faa2841417487e9f6099251049049e0b2833ff))
* **cli:** 0/1/2 exit codes per ADR 0003 ([c1500bd](https://github.com/AhmedOsman101/Rustow/commit/c1500bd0403a10e43c210343c4cf358ea30f47f8))
* **config:** correct init adoption for forced writes and stowrc opt-out ([635dfb2](https://github.com/AhmedOsman101/Rustow/commit/635dfb21daf0171bf98599d85bbfd9b90828f600))
* **config:** escape stowrc sequences by code point ([c99535a](https://github.com/AhmedOsman101/Rustow/commit/c99535afcf3b9dc52796258d09edb9a48cde1c05))
* **config:** harden stowrc migration validation and escaping ([0aa5c8f](https://github.com/AhmedOsman101/Rustow/commit/0aa5c8f716e2dd2e92771e82e73901bcdef26a19))
* **create:** apply root mappings for created/linked packages ([32694dc](https://github.com/AhmedOsman101/Rustow/commit/32694dcb9b34c0a58d5ce8770e72760b7cea74bf))
* **create:** guard interactive source adoption against source-tree paths ([0d66493](https://github.com/AhmedOsman101/Rustow/commit/0d66493556294ce7f19257f5c63a38981d35e106))
* **create:** refuse adopting paths already managed by another package ([6194365](https://github.com/AhmedOsman101/Rustow/commit/61943658e5977905be91a250931796afc4ee2961))
* **docs:** point generated config at the canonical repository ([cdeb441](https://github.com/AhmedOsman101/Rustow/commit/cdeb441ac8ba6e96d26b4fcf020a2d1608872509))
* **linker:** refold on unlink respects per-package includes ([f9fab98](https://github.com/AhmedOsman101/Rustow/commit/f9fab985cd35423fba8d8453a4145298d98a636b))
* **relink:** abort policy detects conflicts before unlink ([61b4f63](https://github.com/AhmedOsman101/Rustow/commit/61b4f6361951eeb097a222c6d6cd2d657b9e6c8b))
* **relink:** make pre-unlink conflict detection side-effect free ([131556f](https://github.com/AhmedOsman101/Rustow/commit/131556f7bd5cc468b92406e33b7f40d0bc4350bf))
* **relink:** plan link phase before unlink, abort leaves tree untouched ([e7ab5d9](https://github.com/AhmedOsman101/Rustow/commit/e7ab5d936b2e4cbd81db38b1640c15b60711094d))
* **root:** address senior-review findings on elevated apply ([7927639](https://github.com/AhmedOsman101/Rustow/commit/7927639612b97c2fcf6033aefb9debe176c0327f))
* **root:** elevate broken root clean and harden unlink ([de3c6d5](https://github.com/AhmedOsman101/Rustow/commit/de3c6d56d526401adf464563b276f708910e4c19))
* **status:** report kind mismatches as conflicts ([982f3b0](https://github.com/AhmedOsman101/Rustow/commit/982f3b0e2face05fec9fa79544ffda97a4abdb3a))
* **status:** require fold link to resolve to the exact package subtree ([b19fdf5](https://github.com/AhmedOsman101/Rustow/commit/b19fdf5be94b52b88496a6162f51381747b50e2d))

## [0.1.4](https://github.com/AhmedOsman101/Rustow/compare/rustow-v0.1.3...rustow-v0.1.4) (2026-11-19)


### Bug Fixes

* **config:** never claim config.toml is missing when it exists ([860d86a](https://github.com/AhmedOsman101/Rustow/commit/860d86a2c534fc9a1d56ca111b500a9477957be3))

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
