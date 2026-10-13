//! Control APIs shared by the offline CLI and native desktop application.
//!
//! The shared session belongs to UI and MCP control threads. Audio callbacks
//! must consume compiled engines separately and must never lock this session.

pub mod control;
pub mod mcp;
pub mod presets;
pub mod visualizer;
