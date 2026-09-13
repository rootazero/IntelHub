//! SP9 Knowledge Graph Memory Layer (graph_v2).
//!
//! Submodules are populated across SP9 Tasks 2-5:
//! - `resolve` / `resolve_async` — Task 2 (entity identity: sync + async worker)
//! - `compiler` — Task 3 (claim-to-graph projection + Neo4j MERGE)
//! - `evidence` — Task 3 (evidence binding)
//! - `contradiction` — Task 4 (claim contradiction detection + review)
//! - `temporal` — Task 5 (valid_from/valid_until semantics, point-in-time reads)

pub mod compiler;
pub mod contradiction;
pub mod evidence;
pub mod resolve;
pub mod resolve_async;
pub mod temporal;