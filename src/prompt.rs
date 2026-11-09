//! dialoguer wrappers — thin, so the interactive glue stays small.

use dialoguer::Select;

use crate::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProvenanceChoice {
  /// Write ~/.config/rustow/config.toml pre-filled from the values in use.
  Create,
  /// Use the fallback for this run only.
  Keep,
  /// Stop showing this notice until a config file exists.
  Silence,
  /// Abort now.
  Abort,
}

/// Ask what to do about the missing config file (TTY only).
pub fn confirm_provenance() -> Result<ProvenanceChoice> {
  let items = [
    "Create config.toml now (pre-filled with the values in use)",
    "Keep using the fallback for this run",
    "Silence this notice permanently",
    "Abort",
  ];
  let selection = Select::new()
    .with_prompt("rustow")
    .items(&items)
    .default(0)
    .interact()
    .map_err(|e| crate::error::Error::Message(format!("prompt failed: {e}")))?;
  Ok(match selection {
    0 => ProvenanceChoice::Create,
    1 => ProvenanceChoice::Keep,
    2 => ProvenanceChoice::Silence,
    _ => ProvenanceChoice::Abort,
  })
}
