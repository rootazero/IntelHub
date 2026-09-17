//! hub-core: IntelHub Hub Core library (SP2A data/tool plane + SP2B governance/write plane + SP3 console API).

pub mod access;
pub mod admin;
pub mod alerts;
pub mod auth;
pub mod cache;
pub mod components;
pub mod config;
pub mod console;
pub mod cost;
pub mod embed;
pub mod entity_seeder;
pub mod error;
pub mod events;
pub mod gev_traffic;
pub mod graph;
pub mod graph_queries;
pub mod graph_v2;
pub mod graphw;
pub mod ingest;
pub mod mcp;
pub mod monitor;
pub mod neo4j_init;
pub mod policy;
pub mod rerank;
pub mod investigate;
pub mod planner_llm;
pub mod sensors;
pub mod series;
pub mod server;
pub mod spiderfoot;
pub mod state;
pub mod store;
pub mod types;
pub mod vector;

pub mod api;

pub use config::Config;
pub use error::{HubError, Result};
pub use state::AppState;
