//! Control APIs shared by the offline CLI and native desktop application.
//!
//! The shared session belongs to UI and MCP control threads. Audio callbacks
//! must consume compiled engines separately and must never lock this session.

pub mod control;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod local_mcp;
pub mod mcp;
pub mod presets;
pub mod project_document;
pub mod vcv_archive;
pub mod visualizer;

pub mod vcv_import;
