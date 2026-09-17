//! mint.run: transaction runtime for consequential AI-agent side effects.
//! Used by: the `mint` binary, TypeScript client tests, and integration tests.

pub mod api;
pub mod bench;
pub mod canonical;
pub mod config;
pub mod credentials;
pub mod domain;
pub mod error;
pub mod events;
pub mod execution;
pub mod failpoints;
pub mod identity;
pub mod keys;
pub mod packs;
pub mod policy;
pub mod receipt;
pub mod server;
pub mod storage;
