//! Editor-side Debug Settings panel hook (skeleton).
//!
//! Discovers reflected `Resource`s carrying the `ReflectDebugPanel` type-data
//! on the connected BRP target and renders them as dropdowns / inputs in a
//! sidebar panel. The plugin is in place; the discovery, schema-fetch,
//! value-poll, mutation, and widget systems are intentionally stubbed —
//! they're scheduled as a follow-up commit so this PR doesn't blob into
//! UI work that benefits from interactive iteration.
//!
//! Game side already exposes the `jackdaw/debug_panel_list` BRP method and
//! the `JackdawDebugPanelPlugin` from `jackdaw_debug_panel`; this editor
//! plugin's job is only to consume that surface.

use bevy::prelude::*;

/// Editor plugin that will host the Debug Settings panel. Currently a no-op
/// scaffold — see module doc-comment.
pub struct DebugSettingsPlugin;

impl Plugin for DebugSettingsPlugin {
    fn build(&self, _app: &mut App) {
        // Intentionally empty. The panel UI, BRP polling, and widget set
        // land in a follow-up commit.
    }
}
