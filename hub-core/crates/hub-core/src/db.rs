//! Data-access layer: SQL CRUD modules over the `hub-core/migrations` schema.
//!
//! Each submodule owns one table (or one cohesive table group) and returns
//! plain `sqlx` rows/types with no HTTP or auth concerns — the `api::*` and
//! `mcp.rs` layers consume these functions.

pub mod annotations;

pub use annotations::*;
