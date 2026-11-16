# ADR 0007: Distribution identity (name collision, Windows positioning)

- **Status:** Accepted (2026-11-14 / 2026-11-16)
- **Context:** Two distribution decisions from recent sessions. (1) The name `rustow` is taken by an unrelated project (Erenoit/rustow on GitLab, published on AUR as `rustow-git`, GPL-3.0+, inactive since 2024-07); `paru rustow` shows it. (2) Windows binaries ship by default, but the user's actual position is that Windows is not a target on its own — rustow is for Unix-like environments, including Git Bash/WSL/Cygwin on Windows.
- **Decision:**
  - The project stays `rustow` / `Rustow` / `AhmedOsman101/Rustow`. No rename. GitHub distribution is unaffected (namespace), and nobody installs this tool from the AUR today.
  - `rustow-git` on the AUR remains that project's package; this project is not published there. If Arch packaging is ever wanted, it ships under a distinct package name, decided then.
  - Docs state plainly: rustow targets Unix-like systems; on Windows it works under Git Bash / WSL / Cygwin and is "not a first-class target". Windows binaries keep shipping (they work in those environments) — no pipeline changes.
- **Consequences:** Positive — no rename, no repackaging churn, no false AUR claims. Negative — the "rustow" search result for Arch users leads to the other project; unavoidable without a rename this project won't pay for.
