# ADR 0001: Root targeting via per-package config mapping

- **Status:** Accepted (2026-11-16)
- **Context:** rustow manages `~/dotfiles` → `$HOME`. Some config files legitimately belong at absolute paths outside `$HOME` (e.g. `/etc/php/php.ini`, `/etc/snapper/configs/root`). Today this is done by hand, or by symlinking into `/etc` manually and re-linking on every machine — brittle and invisible to rustow (no plan, no status, no unlink).
- **Decision:** Root targeting is **config-only** (no CLI flag). A package opts paths in with a `[package.<name>.root]` table in `config.toml`:

  ```toml
  [package.snapper.root]
  "configs/" = "/etc/snapper/"          # directory → directory
  "configs/root" = "/etc/snapper/root"  # file → file
  ```

  - Keys are package-relative paths. A key ending in `/` is a **directory mapping**: value must be an absolute directory (with trailing `/`), and rustow creates ONE symlink at `<value><basename(source-dir)>` (e.g. `/etc/snapper/configs -> <pkg>/configs`) — fold semantics against an absolute parent.
  - A key without a trailing slash is a **file mapping**: the value is the exact absolute destination for that file.
  - Values support `~` expansion; values must be absolute (non-absolute → config error). Keys must not escape the package (`..` rejected; keys are validated the same way as `--path` in `create`).
  - Root-mapped paths never fold, never refold, and are excluded from the package fold decision.
  - Same two-phase conflict machinery: nothing is planned as ok'd until all conflicts resolve.
  - `status` includes root-mapped links with a `root` note in the package's row; `unlink` removes exactly the mapped links (never more); `clean` treats them like any other owned link.
  - Elevation: root-mapped apply needs write access to directories the user usually cannot write. rustow does NOT use `sudo` for the whole run. It performs only the root-mapped actions through an elevation helper, probing `sudo` first then `doas`, overridable with `elevate_cmd` in config.
  - Interactively, when any root-mapped action exists, rustow asks `proceed with elevated steps? [y/n]` before anything changes; answering `n` aborts the whole run (nothing applied). On non-TTY, the run fails loudly with a hint; `--as-root` (global flag, all subcommands) authorises the elevated steps non-interactively.
  - When the process itself runs as root (EUID 0), rustow prints a loud warning that root's config and home apply, then follows the same rules.
  - Resolution of the privileged vs unprivileged parts happens at **plan time**: for each root-mapped action, rustow preflights destination parent writability and elevation availability; a failure aborts before any filesystem mutation.
- **Consequences:** Positive — first-class, auditable `/etc` management; dry-run and status finally cover system paths; no `sudo bash` wrappers. Negative — privilege handling is new code with real blast radius; the planner grows a second mapping axis; documentation must teach caution (this is a linker that can write `/etc`).
