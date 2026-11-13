//! Accounts (L1-accounts): registration, sign-in, profiles and the identity API.
//!
//! DERIVED design (adr-002 2.1). Other capabilities depend on [`identity`] only (STATED, CF-004,
//! baseline gate). `store` and `service` are `pub` only so the integration tests under `tests/`
//! can reach them: they are not an API for other capabilities.

pub mod extract;
pub mod identity;
pub mod model;
pub mod password;
pub mod routes;
pub mod service;
pub mod store;
pub mod token;
pub mod validate;
