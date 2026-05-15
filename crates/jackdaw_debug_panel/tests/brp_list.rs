//! End-to-end test for `jackdaw/debug_panel_list`: spin up a minimal Bevy app
//! with the plugin and three opted-in resources, invoke the handler directly,
//! and assert ordering + content.

use bevy::{
    prelude::*,
    remote::{RemoteMethodSystemId, RemoteMethods},
};
use jackdaw_debug_panel::{
    DEBUG_PANEL_LIST_METHOD, DebugPanelEntry, JackdawDebugPanelPlugin, debug_panel,
};

#[derive(Resource, Reflect, Default, Debug, Clone)]
#[reflect(Resource)]
struct SettingsA {
    v: f32,
}

#[derive(Resource, Reflect, Default, Debug, Clone)]
#[reflect(Resource)]
struct SettingsB {
    v: bool,
}

#[derive(Resource, Reflect, Default, Debug, Clone)]
#[reflect(Resource)]
struct Diagnostics {
    ran: bool,
}

#[test]
fn list_returns_sorted_opted_in_resources() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(JackdawDebugPanelPlugin);
    app.init_resource::<AppTypeRegistry>();
    app.init_resource::<SettingsA>();
    app.init_resource::<SettingsB>();
    app.init_resource::<Diagnostics>();

    debug_panel!(app, {
        SettingsB   => { label: "Beta",  order: 10 },
        SettingsA   => { label: "Alpha", order:  0 },
        Diagnostics => { label: "Diag",  order: 90, read_only: true },
    });

    app.finish();
    app.cleanup();

    let methods = app.world().resource::<RemoteMethods>();
    let RemoteMethodSystemId::Instant(system_id) = *methods
        .get(DEBUG_PANEL_LIST_METHOD)
        .expect("method registered")
    else {
        panic!("expected Instant system");
    };

    let result = app
        .world_mut()
        .run_system_with(system_id, None)
        .expect("system runs")
        .expect("handler returns Ok");

    let entries: Vec<DebugPanelEntry> = serde_json::from_value(result).expect("entries parse");
    assert_eq!(entries.len(), 3, "exactly the three opted-in resources");

    assert_eq!(entries[0].label, "Alpha");
    assert_eq!(entries[0].order, 0);
    assert_eq!(entries[1].label, "Beta");
    assert_eq!(entries[1].order, 10);
    assert_eq!(entries[2].label, "Diag");
    assert!(entries[2].read_only);
}
