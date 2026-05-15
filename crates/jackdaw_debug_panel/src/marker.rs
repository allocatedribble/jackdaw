use bevy::{prelude::*, reflect::TypeRegistration};
use serde::{Deserialize, Serialize};

/// Type-data attached to a reflected `Resource` (or other reflected type) to
/// mark it as eligible for the editor's Debug Settings panel.
#[derive(Clone, Debug)]
pub struct ReflectDebugPanel {
    pub label: &'static str,
    pub order: i32,
    pub read_only: bool,
}

impl ReflectDebugPanel {
    pub const fn new(label: &'static str) -> Self {
        Self { label, order: 0, read_only: false }
    }
}

/// Wire DTO returned by the `jackdaw/debug_panel_list` BRP method.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DebugPanelEntry {
    pub type_path: String,
    pub label: String,
    pub order: i32,
    pub read_only: bool,
}

/// Attach a `ReflectDebugPanel` to type `T`'s registration. Panics if `T` was
/// not registered first (callers should call `app.register_type::<T>()` before
/// this — the `debug_panel!` macro does so automatically).
pub fn register_marker<T: bevy::reflect::TypePath + 'static>(
    app: &mut App,
    marker: ReflectDebugPanel,
) {
    let registry = app.world().resource::<AppTypeRegistry>().clone();
    let mut registry = registry.write();
    let registration: &mut TypeRegistration = registry
        .get_mut(std::any::TypeId::of::<T>())
        .unwrap_or_else(|| {
            panic!(
                "register_marker::<{}> called before register_type::<{}>",
                std::any::type_name::<T>(),
                std::any::type_name::<T>(),
            )
        });
    registration.insert(marker);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Resource, Reflect, Default, Debug, Clone)]
    #[reflect(Resource)]
    struct DummySetting {
        value: i32,
    }

    #[test]
    fn register_marker_attaches_type_data() {
        let mut app = App::new();
        app.init_resource::<AppTypeRegistry>();
        app.register_type::<DummySetting>();
        register_marker::<DummySetting>(
            &mut app,
            ReflectDebugPanel::new("Dummy"),
        );

        let registry = app.world().resource::<AppTypeRegistry>().read();
        let registration = registry
            .get(std::any::TypeId::of::<DummySetting>())
            .expect("DummySetting registered");
        let marker = registration
            .data::<ReflectDebugPanel>()
            .expect("marker attached");
        assert_eq!(marker.label, "Dummy");
        assert_eq!(marker.order, 0);
        assert!(!marker.read_only);
    }
}
