# ADR 0004: Stow → rustow migration via `rustow config migrate`

- **Status:** Accepted (2026-11-16)
- **Context:** A user moving from GNU Stow has a `~/.stowrc` (and rustow-friendly links created by stow, which are indistinguishable from rustow-created ones). rustow already *uses* stowrc as a fallback, but there was no way to graduate to a real config file without hand-copying values. The user scoped this as: a `rustow config migrate` subcommand, plus an offer during `rustow config init` when a stowrc is detected.
- **Decision:**
  - `rustow config migrate`: reads `~/.stowrc` (or a given path), applies it to the user's `config.toml`, and writes only what the config leaves unset — **existing config.toml values always win**. Values: `source`, `target`, `verbose` (the same set stowrc can supply today).
  - Interactive by default: prints the adoption diff and asks `[a]dopt / [s]kip`. `--yes` adopts without asking; `--dry-run` prints what would happen and writes nothing.
  - After adoption, `~/.stowrc` is **left untouched**; rustow prints a note that it can be deleted (it is inert once a config file exists).
  - `rustow config init` hooks the same code path: on detecting `~/.stowrc` it offers adoption inline (`[a]dopt / [k]eep defaults`) before writing.
  - Shared implementation: both entry points call one function (e.g. `config::migrate_stowrc`); `config init` passes its destination config path, `config migrate` uses the resolved one.
  - Malformed/unreadable stowrc → clean error, config untouched, non-zero exit.
  - No `.stowrc.bak` copy is made; the file stays exactly where stow left it.
- **Consequences:** Positive — the stow→rustow path is one command, no hand-translation, safe against clobbering a hand-written config. Negative — users expecting stowrc deletion get a note instead; deliberate, since destroying inputs of a half-finished migration is worse than leaving them.
