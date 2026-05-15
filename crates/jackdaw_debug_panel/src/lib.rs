//! Shared types and Bevy plugin for the Jackdaw editor's Debug Settings panel.
//!
//! Game crates depend on this crate to opt their reflected `Resource`s into
//! the editor panel via the [`debug_panel!`] macro. The editor depends on it
//! for the wire DTO and method name constants.

mod macros;
mod marker;
mod methods;
mod plugin;

pub use marker::{DebugPanelEntry, ReflectDebugPanel, register_marker};
pub use methods::DEBUG_PANEL_LIST_METHOD;
pub use plugin::JackdawDebugPanelPlugin;
