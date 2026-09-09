//! hub-core: IntelHub Hub Core library (SP2A data/tool plane + SP2B governance/write plane + SP3 console API).

pub mod admin;
pub mod alerts;
pub mod auth;
pub mod components;
pub mod config;
pub mod console;
pub mod cost;
pub mod embed;
pub mod error;
pub mod events;
pub mod graph;
pub mod graphw;
pub mod ingest;
pub mod mcp;
pub mod policy;
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
