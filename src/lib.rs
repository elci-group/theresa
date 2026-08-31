//! Theresa's configuration, project inspection, README rendering, and watcher runtime.

pub mod batching;
pub mod config;
pub mod daemon;
pub mod readme;
pub mod scan;

pub use config::Config;
pub use daemon::{run_once, watch};
