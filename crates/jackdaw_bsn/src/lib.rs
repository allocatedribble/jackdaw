mod loader;

pub use loader::{BsnAssetLoader, BsnLoadError};

use bevy::prelude::*;

pub struct JackdawBsnPlugin {
    /// Whether to run the built-in runtime mesh rebuild for brushes.
    /// Defaults to `true`. Set to `false` if the editor is doing its own
    /// authoring-time mesh work.
    pub runtime_mesh_rebuild: bool,
}

impl Default for JackdawBsnPlugin {
    fn default() -> Self {
        Self {
            runtime_mesh_rebuild: true,
        }
    }
}

impl Plugin for JackdawBsnPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(jackdaw_jsn::JsnPlugin {
            runtime_mesh_rebuild: self.runtime_mesh_rebuild,
        })
        .init_asset_loader::<BsnAssetLoader>();
    }
}
