# ADR 0002: Per-package `includes` (biome-style merged key)

- **Status:** Accepted (2026-11-16)
- **Context:** rustow has a global `ignore` list in `config.toml` (plus Stow's built-in default list) suppressing paths from linking and from folding. There was no per-package equivalent — a package that wanted to hide one build artifact had to grow the global list. The user pointed at Biome's `files.includes` (a single array: `**` plus positive globs plus `!`-prefixed negations) as the shape to copy, merged into one key.
- **Decision:** Each package may declare `includes` in `config.toml` under a per-package table:

  ```toml
  [package.eza]
  includes = ["**", "!**/*.local"]
  ```

  Semantics (biome-style list, layered over the global `ignore`):
  - Global defaults and the global `ignore` apply first, machine-wide.
  - The package's `includes` then applies on top, **for that package only**. Biome-style entries: a positive glob re-includes a path the global list hid (e.g. `**` restores it); a `!`-prefixed glob hides it again. The package list has the final word for its own package.
  - Paths the effective list hides are suppressed from linking AND block folding of the containing subtree — identical behavior to paths hidden by the global list.
  - `fold_ignore` remains a separate, untouched key: "this package never folds at all" is a different axis from "which paths this package exposes".
  - Malformed `includes` (wrong type, bad glob) is a config error, not a silent skip.
- **Consequences:** Positive — one config file, one model; per-package tuning without touching global rules; biome users already know the syntax. Negative — `includes` vs `fold_ignore` vs global `ignore` are three keys doing adjacent jobs; docs must say plainly which one to reach for.
