# ADR 0003: Exit-code contract (0 / 1 / 2, Unix/clap conventions)

- **Status:** Accepted (2026-11-16)
- **Context:** rustow had three overlapping sources of exit codes: `main_entry` mapped `Conflicts` → 1 and `Aborted` → 2; the user-facing spec documented "2 = user aborted" without a section anyone verified; clap exits `2` by itself for usage errors (`--welp`). Consequences: scripts could not distinguish anything useful, and the spec's claim was unverifiable. The user directed: "Adopt known exit codes in the Unix ecosystem."
- **Decision:** The contract, documented in `docs/spec.md` and pinned by tests:

  | Code | Meaning                                                                                  |
  | ---- | ---------------------------------------------------------------------------------------- |
  | `0`  | Success (`status`: every inspected package fully linked and conflict-free)             |
  | `1`  | General failure: unresolved conflicts, runtime errors, and the user declining an interactive prompt |
  | `2`  | Usage error: unknown/malformed CLI arguments (clap's built-in behavior)                 |

  No separate meanings are invented. In particular, an interactive "abort" is not distinct from any other runtime failure; anyone who wants to distinguish them watches stderr, which always names the reason.
- **Consequences:** Positive — identical to bash/GNU/clap muscle memory; one meaning per code; no drift possible once tests pin all three. Negative — users/scripts lose the (never implemented) 1-vs-2 distinction between "machine couldn't proceed" and "the human chose to stop"; accepted, since neither existed in practice and neither is useful in a linker.
