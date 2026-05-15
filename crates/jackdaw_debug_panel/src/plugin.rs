use bevy::{
    prelude::*,
    remote::{RemoteMethodSystemId, RemoteMethods, RemotePlugin},
};

use crate::methods::{DEBUG_PANEL_LIST_METHOD, debug_panel_list_handler};

/// Game-side plugin that registers the `jackdaw/debug_panel_list` BRP method.
///
/// Add this to your `App` alongside `JackdawRemotePlugin` (or any other
/// plugin that brings up `RemotePlugin`). If `RemotePlugin` hasn't been added
/// yet, this plugin adds it.
#[derive(Default)]
pub struct JackdawDebugPanelPlugin;

impl Plugin for JackdawDebugPanelPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<RemotePlugin>() {
            app.add_plugins(
                RemotePlugin::default()
                    .with_method_main(DEBUG_PANEL_LIST_METHOD, debug_panel_list_handler),
            );
        }
    }

    fn finish(&self, app: &mut App) {
        let world = app.world_mut();
        let already_registered = world
            .get_resource::<RemoteMethods>()
            .is_some_and(|methods| methods.get(DEBUG_PANEL_LIST_METHOD).is_some());
        if already_registered {
            return;
        }
        if world.get_resource::<RemoteMethods>().is_none() {
            return;
        }
        let system_id = world.register_system(debug_panel_list_handler);
        world
            .resource_mut::<RemoteMethods>()
            .insert(DEBUG_PANEL_LIST_METHOD, RemoteMethodSystemId::Instant(system_id));
    }
}
