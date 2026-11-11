pub mod cli;
pub mod config;
pub mod create;
pub mod diff;
pub mod error;
pub mod ignore;
pub mod link;
pub mod prompt;
pub mod status;

/// Platform-specific helpers kept cfg-gated so Windows builds compile too.
pub mod platform;
