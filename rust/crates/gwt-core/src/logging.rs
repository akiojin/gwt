//! Logging module
//!
//! Provides structured JSON Lines logging with tracing spans (Pino compatible).

mod logger;
mod reader;

pub use logger::{init_logger, LogConfig};
pub use reader::LogReader;
