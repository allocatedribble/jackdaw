/// Opts a set of reflected `Resource` types into the editor Debug Settings
/// panel.
///
/// Each entry attaches a `ReflectDebugPanel` to the type's registration.
/// The `label:` field is required; `order:` defaults to 0; `read_only:` defaults
/// to false. Types are `register_type::<T>()`'d automatically.
///
/// ```ignore
/// debug_panel!(app, {
///     CloudGlobalSettings    => { label: "Clouds",         order:  0 },
///     SolariDenoiserSettings => { label: "Solari Denoiser", order: 20 },
///     MirageRenderDiagnostics => { label: "Diagnostics",   order: 90, read_only: true },
/// });
/// ```
#[macro_export]
macro_rules! debug_panel {
    ($app:expr, {
        $(
            $ty:ty => { label: $label:expr
                       $(, order: $order:expr )?
                       $(, read_only: $ro:expr )?
                       $(,)? }
        ),* $(,)?
    }) => {{
        let __app: &mut ::bevy::prelude::App = &mut $app;
        $(
            __app.register_type::<$ty>();
            $crate::register_marker::<$ty>(
                __app,
                $crate::ReflectDebugPanel {
                    label: $label,
                    order: $crate::__debug_panel_or!(0_i32 $(, $order)?),
                    read_only: $crate::__debug_panel_or!(false $(, $ro)?),
                },
            );
        )*
    }};
}

/// Internal: returns the override expression if present, otherwise the default.
#[doc(hidden)]
#[macro_export]
macro_rules! __debug_panel_or {
    ($default:expr) => { $default };
    ($default:expr, $value:expr) => { $value };
}

#[cfg(test)]
mod tests {
    use crate::{ReflectDebugPanel, register_marker};
    use bevy::prelude::*;

    #[derive(Resource, Reflect, Default, Debug, Clone)]
    #[reflect(Resource)]
    struct AlphaSetting {
        x: f32,
    }

    #[derive(Resource, Reflect, Default, Debug, Clone)]
    #[reflect(Resource)]
    struct BetaSetting {
        y: bool,
    }

    #[test]
    fn macro_attaches_defaults_and_overrides() {
        let mut app = App::new();
        app.init_resource::<AppTypeRegistry>();

        crate::debug_panel!(app, {
            AlphaSetting => { label: "Alpha" },
            BetaSetting  => { label: "Beta", order: 7, read_only: true },
        });

        let registry = app.world().resource::<AppTypeRegistry>().read();

        let alpha = registry
            .get(std::any::TypeId::of::<AlphaSetting>())
            .and_then(|r| r.data::<ReflectDebugPanel>())
            .expect("Alpha marker present");
        assert_eq!(alpha.label, "Alpha");
        assert_eq!(alpha.order, 0);
        assert!(!alpha.read_only);

        let beta = registry
            .get(std::any::TypeId::of::<BetaSetting>())
            .and_then(|r| r.data::<ReflectDebugPanel>())
            .expect("Beta marker present");
        assert_eq!(beta.label, "Beta");
        assert_eq!(beta.order, 7);
        assert!(beta.read_only);
    }
}
