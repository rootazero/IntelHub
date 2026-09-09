//! hub-core: IntelHub data & tool plane library (SP2A).

pub mod admin;
pub mod auth;
pub mod config;
pub mod error;
pub mod events;
pub mod graph;
pub mod ingest;
pub mod mcp;
pub mod sensors;
pub mod server;
pub mod state;
pub mod store;
pub mod types;
pub mod vector;

pub mod api;

pub use config::Config;
pub use error::{HubError, Result};
pub use state::AppState;
