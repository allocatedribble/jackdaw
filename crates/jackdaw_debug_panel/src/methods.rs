use bevy::{prelude::*, remote::BrpResult};
use serde_json::Value;

use crate::marker::{DebugPanelEntry, ReflectDebugPanel};

/// Method name for the BRP discovery call.
pub const DEBUG_PANEL_LIST_METHOD: &str = "jackdaw/debug_panel_list";

/// Handler for `jackdaw/debug_panel_list`. Walks the type registry and
/// returns a sorted list of every registration carrying `ReflectDebugPanel`.
pub fn debug_panel_list_handler(
    In(_params): In<Option<Value>>,
    type_registry: Res<AppTypeRegistry>,
) -> BrpResult {
    let registry = type_registry.read();
    let mut entries: Vec<DebugPanelEntry> = registry
        .iter()
        .filter_map(|registration| {
            registration.data::<ReflectDebugPanel>().map(|marker| DebugPanelEntry {
                type_path: registration.type_info().type_path().to_string(),
                label: marker.label.to_string(),
                order: marker.order,
                read_only: marker.read_only,
            })
        })
        .collect();

    entries.sort_by(|a, b| {
        a.order
            .cmp(&b.order)
            .then_with(|| a.label.cmp(&b.label))
            .then_with(|| a.type_path.cmp(&b.type_path))
    });

    Ok(serde_json::to_value(entries).unwrap())
}
