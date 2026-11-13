//! Shared seam for every capability: settings, clock, errors, database access, app wiring.

pub mod app;
pub mod clock;
pub mod config;
pub mod db;
pub mod error;
pub mod scrub;
