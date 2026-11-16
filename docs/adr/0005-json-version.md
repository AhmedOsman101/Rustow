# ADR 0005: `status --json` carries a top-level `version`

- **Status:** Accepted (2026-11-16)
- **Context:** `status --json` emits `{ "packages": [...], "summary": {...} }` with no version marker; scripts cannot pin against it and any schema change would be silent breakage. Flagged in the V1 report and code review.
- **Decision:** Add `"version": 1` as a top-level sibling of `packages` and `summary`. The field is additive for consumers that ignore unknown keys. The schema is frozen for the rest of the v0.1 line; the next version bump of this field accompanies only breaking shape changes and is documented in `docs/spec.md`.
- **Consequences:** Positive — machine consumers can version-stamp; future shape changes become safe and explicit. Negative — consumers written against the unversioned shape see one unknown key, which standard JSON consumers ignore by construction.
