use bevy::{
    app::AppExit,
    feathers::cursor::EntityCursor,
    math::CompassOctant,
    picking::{events::Press as PointerPress, hover::Hovered, pointer::PointerButton},
    prelude::*,
    ui_widgets::observe,
    window::{PrimaryWindow, SystemCursorIcon},
};
use jackdaw_api::prelude::*;
use jackdaw_feathers::{
    button::{self, ButtonOperatorCall, ButtonSize, ButtonVariant},
    icons::IconFont,
    menu_bar, separator, split_panel, status_bar,
    text_edit::{self, TextEditProps},
    tokens,
    tree_view::tree_container_drop_observers,
};

use crate::{
    EditorEntity,
    brush::{BrushEditMode, EditMode},
    draw_brush::ActivateDrawBrushModalOp,
    edit_mode_ops::{
        EditModeClipOp, EditModeEdgeOp, EditModeFaceOp, EditModeObjectOp, EditModeVertexOp,
    },
    gizmo_ops::{GizmoModeRotateOp, GizmoModeScaleOp, GizmoModeTranslateOp, GizmoSpaceToggleOp},
    gizmos::{GizmoMode, GizmoSpace},
    hierarchy::{HierarchyPanel, HierarchyShowAllButton, HierarchyTreeContainer},
    inspector::Inspector,
    measure_tool::MeasureDistanceOp,
    physics_tool::PhysicsActivateOp,
    remote::ConnectionManager,
    viewport::SceneViewport,
};

/// Discriminator for the header tab kinds the editor knows how to host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum TabKind {
    /// The live scene being edited. There's exactly one Scene tab.
    #[default]
    Scene,
    /// The Schedule Explorer / remote debug view (replaces the old
    /// "Remote Debug" workspace). There's exactly one Schedule Explorer
    /// tab.
    ScheduleExplorer,
}

impl TabKind {
    /// Human-readable label shown on the tab strip.
    pub fn label(self) -> &'static str {
        match self {
            TabKind::Scene => "Main scene",
            TabKind::ScheduleExplorer => "Schedule Explorer",
        }
    }

    /// Colored accent stripe drawn at the left edge of the tab.
    pub fn accent(self) -> Color {
        match self {
            TabKind::Scene => tokens::DOC_TAB_SCENE_ACCENT,
            TabKind::ScheduleExplorer => tokens::DOC_TAB_TOOL_ACCENT,
        }
    }

    /// Icon glyph rendered in the tab header.
    pub fn icon(self) -> Icon {
        match self {
            TabKind::Scene => Icon::File,
            TabKind::ScheduleExplorer => Icon::CalendarSearch,
        }
    }
}

/// Layout preset for the Scene document tab.
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SceneViewPreset {
    #[default]
    Scene,
}

/// The tab the editor is currently showing.
#[derive(Resource, Default, Clone, Copy)]
pub struct ActiveDocument {
    pub kind: TabKind,
}

/// Marker on the tab strip row container so the tab styling system can
/// find its children.
#[derive(Component)]
pub struct DocumentTabStrip;

/// Marker on an individual document tab button, tagged with the
/// `TabKind` it activates when clicked.
#[derive(Component)]
pub struct DocumentTabButton(pub TabKind);

/// Marker on a document content container. The per-frame
/// `update_active_document_display` system toggles `Node::display` on
/// these so only the matching-kind container is visible.
#[derive(Component)]
pub struct DocumentRoot(pub TabKind);

/// Marker on the center column container. Retained as a hook for
/// systems that want to find the main viewport-plus-bottom-panels
/// area. Formerly driven by `SceneViewPreset`; now unconditional.
#[derive(Component)]
pub struct SceneCenter;

/// Marker on the hierarchy filter text input
#[derive(Component)]
pub struct HierarchyFilter;

/// Marker for the toolbar
#[derive(Component)]
pub struct Toolbar;

#[derive(Resource, Default)]
pub(crate) struct WindowDecorationState {
    maximized: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WindowControlAction {
    Minimize,
    ToggleMaximize,
    Close,
}

#[derive(Component)]
pub(crate) struct WindowControlButton(WindowControlAction);

#[derive(Component)]
pub(crate) struct TitlebarDragRegion;

#[derive(Component)]
pub(crate) struct WindowResizeRegion(CompassOctant);

pub fn editor_layout(icon_font: &IconFont) -> impl Bundle {
    (
        EditorEntity,
        // Outer shell: dark background with padding (Figma: 10px padding, bg #171717)
        BackgroundColor(tokens::WINDOW_BG),
        Node {
            width: percent(100),
            height: percent(100),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            padding: UiRect::all(px(0.0)),
            ..Default::default()
        },
        children![
            (
                // Inner container: the editor workspace with rounded corners and border.
                EditorEntity,
                Node {
                    width: percent(100),
                    flex_grow: 1.0,
                    min_height: px(0.0),
                    flex_direction: FlexDirection::Column,
                    border: UiRect::all(px(1.0)),
                    border_radius: BorderRadius::all(px(8.0)),
                    overflow: Overflow::clip(),
                    ..Default::default()
                },
                BackgroundColor(tokens::WINDOW_BG),
                BorderColor::all(tokens::BORDER_SUBTLE),
                children![
                    // Integrated app titlebar: menu bar + scene tabs + controls.
                    window_header(icon_font.0.clone()),
                    // Content container (flex grow). Holds both workspaces.
                    // Figma: Editor (Rows) has padding: 0px 4px
                    (
                        EditorEntity,
                        Node {
                            width: percent(100),
                            flex_grow: 1.0,
                            min_height: px(0.0),
                            flex_direction: FlexDirection::Column,
                            padding: UiRect::horizontal(px(tokens::PANEL_GAP)),
                            row_gap: px(tokens::PANEL_GAP),
                            ..Default::default()
                        },
                        children![
                            // Scene document (visible by default).
                            //
                            // The dock tree is materialised by `jackdaw_panels`'
                            // reconciler under this single host. The default
                            // tree (left | (center over bottom) | right) is
                            // built in `init_layout` from registered windows;
                            // the user can drag panels anywhere within it.
                            (
                                DocumentRoot(TabKind::Scene),
                                EditorEntity,
                                Node {
                                    width: percent(100),
                                    flex_grow: 1.0,
                                    min_height: px(0.0),
                                    display: Display::Flex,
                                    flex_direction: FlexDirection::Row,
                                    ..Default::default()
                                },
                                children![(
                                    jackdaw_panels::reconcile::DockTreeHost::default(),
                                    EditorEntity,
                                    Node {
                                        width: percent(100),
                                        height: percent(100),
                                        flex_direction: FlexDirection::Row,
                                        overflow: Overflow::clip(),
                                        ..Default::default()
                                    },
                                )],
                            ),
                            // Schedule Explorer document (hidden by default).
                            // Formerly the Remote Debug workspace; same content
                            // repackaged as a document tab.
                            (
                                DocumentRoot(TabKind::ScheduleExplorer),
                                EditorEntity,
                                Node {
                                    width: percent(100),
                                    flex_grow: 1.0,
                                    min_height: px(0.0),
                                    flex_direction: FlexDirection::Column,
                                    display: Display::None,
                                    ..Default::default()
                                },
                                split_panel::panel_group(
                                    0.2,
                                    (
                                        Spawn((
                                            split_panel::panel(1),
                                            crate::remote::entity_browser::remote_debug_workspace_content(),
                                        )),
                                        Spawn(split_panel::panel_handle()),
                                        Spawn((
                                            split_panel::panel(1),
                                            crate::remote::remote_inspector::remote_inspector(),
                                        )),
                                    ),
                                ),
                            )
                        ],
                    ),
                    // Status bar (fixed height) with connection indicator
                    editor_status_bar()
                ],
            ),
            window_resize_handles(),
        ],
    )
}

/// Integrated window header. Two groups separated by a flexible spacer:
/// the **left group** owns the menu bar and the document tab strip (so
/// tabs sit right after the `Add` menu, matching the Figma mock), and
/// the **right group** owns the Scene View combobox and the Play/Pause
/// pill. A flex-grow spacer between them absorbs the slack, so resizing
/// the dropdown label (e.g. `Scene View ▾` → `Animation View ▾`) can't
/// shift the tabs.
fn window_header(icon_font: Handle<Font>) -> impl Bundle {
    (
        EditorEntity,
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            width: percent(100),
            height: px(34.0),
            flex_shrink: 0.0,
            border_radius: BorderRadius::top(Val::Px(7.0)),
            ..Default::default()
        },
        BackgroundColor(tokens::WINDOW_BG),
        children![
            // Left: menu bar + tab strip, sitting flush to the left
            // edge. `column_gap` pushes the tabs slightly away from the
            // last menu item ("Add").
            (
                EditorEntity,
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    height: percent(100),
                    column_gap: px(tokens::SPACING_LG),
                    ..Default::default()
                },
                children![
                    menu_bar::menu_bar_shell(),
                    (
                        jackdaw_panels::WorkspaceTabStrip,
                        DocumentTabStrip,
                        EditorEntity,
                        Node {
                            flex_direction: FlexDirection::Row,
                            align_items: AlignItems::Center,
                            height: percent(100),
                            column_gap: px(4.0),
                            ..Default::default()
                        },
                    ),
                ],
            ),
            // Flexible spacer; absorbs leftover horizontal space
            // between the left group and the right group.
            (
                TitlebarDragRegion,
                EditorEntity,
                Node {
                    flex_grow: 1.0,
                    height: percent(100),
                    ..Default::default()
                },
                BackgroundColor(Color::NONE),
            ),
            // Right: Play/Pause transport + window controls.
            (
                EditorEntity,
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    height: percent(100),
                    padding: UiRect::left(px(tokens::SPACING_MD)),
                    column_gap: px(6.0),
                    ..Default::default()
                },
                children![
                    play_pause_controls(icon_font.clone()),
                    window_controls(icon_font),
                ],
            ),
        ],
    )
}

/// Play / Pause / Stop transport pill. Clicking a button triggers
/// the corresponding `PiePlugin` handler. The plugin installs a
/// click observer on each `PieButton` via an `On<Add, PieButton>`
/// observer, so wiring here is purely presentation.
fn play_pause_controls(icon_font: Handle<Font>) -> impl Bundle {
    (
        EditorEntity,
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            height: px(22.0),
            padding: UiRect::horizontal(px(6.5)),
            column_gap: px(9.0),
            border: UiRect::all(px(1.0)),
            border_radius: BorderRadius::all(px(tokens::BORDER_RADIUS_LG)),
            ..Default::default()
        },
        BackgroundColor(tokens::HEADER_CONTROL_BG),
        BorderColor::all(tokens::HEADER_CONTROL_BORDER),
        children![
            pie_transport_button(crate::pie::PieButton::Play, Icon::Play, icon_font.clone(),),
            pie_transport_button(crate::pie::PieButton::Pause, Icon::Pause, icon_font.clone(),),
            pie_transport_button(crate::pie::PieButton::Stop, Icon::Square, icon_font),
        ],
    )
}

/// Single clickable glyph. The `PieButton` marker is the hook the
/// `PiePlugin` uses to attach the click observer. Lucide glyphs live
/// in the Private Use Area, so the icon font handle must be passed
/// explicitly: without it the default font (`FiraSans`) renders the
/// codepoints as tofu/`?`.
fn pie_transport_button(
    kind: crate::pie::PieButton,
    icon: Icon,
    icon_font: Handle<Font>,
) -> impl Bundle {
    (
        kind,
        EditorEntity,
        Node {
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            padding: UiRect::horizontal(px(2.0)),
            ..Default::default()
        },
        children![(
            Text::new(String::from(icon.unicode())),
            TextFont {
                font: (icon_font).into(),
                font_size: (13.0).into(),
                ..Default::default()
            },
            TextColor(tokens::HEADER_CONTROL_LABEL),
            Pickable::IGNORE,
        )],
    )
}

pub(crate) fn app_window_titlebar(icon_font: Handle<Font>) -> impl Bundle {
    (
        EditorEntity,
        Node {
            position_type: PositionType::Absolute,
            left: px(0.0),
            top: px(0.0),
            width: percent(100),
            height: px(34.0),
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            border: UiRect::bottom(px(1.0)),
            ..Default::default()
        },
        BackgroundColor(tokens::WINDOW_BG),
        BorderColor::all(tokens::BORDER_SUBTLE),
        ZIndex(1000),
        children![
            (
                TitlebarDragRegion,
                EditorEntity,
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    height: percent(100),
                    flex_grow: 1.0,
                    min_width: px(0.0),
                    padding: UiRect::left(px(tokens::SPACING_MD)),
                    column_gap: px(tokens::SPACING_SM),
                    ..Default::default()
                },
                BackgroundColor(Color::NONE),
                children![
                    (
                        Text::new(String::from(Icon::AppWindow.unicode())),
                        TextFont {
                            font: (icon_font.clone()).into(),
                            font_size: (14.0).into(),
                            ..Default::default()
                        },
                        TextColor(tokens::TEXT_SECONDARY),
                        Pickable::IGNORE,
                    ),
                    (
                        Text::new("jackdaw"),
                        TextFont {
                            font_size: (tokens::FONT_MD).into(),
                            ..Default::default()
                        },
                        TextColor(tokens::TEXT_PRIMARY),
                        Pickable::IGNORE,
                    ),
                ],
            ),
            window_controls(icon_font),
        ],
    )
}

fn window_controls(icon_font: Handle<Font>) -> impl Bundle {
    (
        EditorEntity,
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            height: percent(100),
            flex_shrink: 0.0,
            ..Default::default()
        },
        ZIndex(1000),
        children![
            window_control_button(
                WindowControlAction::Minimize,
                Icon::Minus,
                icon_font.clone()
            ),
            window_control_button(
                WindowControlAction::ToggleMaximize,
                Icon::Maximize2,
                icon_font.clone(),
            ),
            window_control_button(WindowControlAction::Close, Icon::X, icon_font),
        ],
    )
}

fn window_control_button(
    action: WindowControlAction,
    icon: Icon,
    icon_font: Handle<Font>,
) -> impl Bundle {
    (
        Button,
        WindowControlButton(action),
        EditorEntity,
        Node {
            width: px(42.0),
            height: percent(100),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..Default::default()
        },
        BackgroundColor(Color::NONE),
        children![(
            Text::new(String::from(icon.unicode())),
            TextFont {
                font: (icon_font).into(),
                font_size: (13.0).into(),
                ..Default::default()
            },
            TextColor(tokens::TEXT_SECONDARY),
            Pickable::IGNORE,
        )],
    )
}

pub(crate) fn window_resize_handles() -> impl Bundle {
    const EDGE: f32 = 6.0;
    const CORNER: f32 = 14.0;

    (
        EditorEntity,
        Node {
            position_type: PositionType::Absolute,
            left: px(0.0),
            top: px(0.0),
            width: percent(100),
            height: percent(100),
            ..Default::default()
        },
        Pickable::IGNORE,
        ZIndex(900),
        children![
            window_resize_handle(
                CompassOctant::North,
                SystemCursorIcon::NResize,
                Node {
                    position_type: PositionType::Absolute,
                    left: px(CORNER),
                    right: px(CORNER),
                    top: px(0.0),
                    height: px(EDGE),
                    ..Default::default()
                },
            ),
            window_resize_handle(
                CompassOctant::South,
                SystemCursorIcon::SResize,
                Node {
                    position_type: PositionType::Absolute,
                    left: px(CORNER),
                    right: px(CORNER),
                    bottom: px(0.0),
                    height: px(EDGE),
                    ..Default::default()
                },
            ),
            window_resize_handle(
                CompassOctant::East,
                SystemCursorIcon::EResize,
                Node {
                    position_type: PositionType::Absolute,
                    right: px(0.0),
                    top: px(CORNER),
                    bottom: px(CORNER),
                    width: px(EDGE),
                    ..Default::default()
                },
            ),
            window_resize_handle(
                CompassOctant::West,
                SystemCursorIcon::WResize,
                Node {
                    position_type: PositionType::Absolute,
                    left: px(0.0),
                    top: px(CORNER),
                    bottom: px(CORNER),
                    width: px(EDGE),
                    ..Default::default()
                },
            ),
            window_resize_handle(
                CompassOctant::NorthEast,
                SystemCursorIcon::NeResize,
                Node {
                    position_type: PositionType::Absolute,
                    right: px(0.0),
                    top: px(0.0),
                    width: px(CORNER),
                    height: px(CORNER),
                    ..Default::default()
                },
            ),
            window_resize_handle(
                CompassOctant::NorthWest,
                SystemCursorIcon::NwResize,
                Node {
                    position_type: PositionType::Absolute,
                    left: px(0.0),
                    top: px(0.0),
                    width: px(CORNER),
                    height: px(CORNER),
                    ..Default::default()
                },
            ),
            window_resize_handle(
                CompassOctant::SouthEast,
                SystemCursorIcon::SeResize,
                Node {
                    position_type: PositionType::Absolute,
                    right: px(0.0),
                    bottom: px(0.0),
                    width: px(CORNER),
                    height: px(CORNER),
                    ..Default::default()
                },
            ),
            window_resize_handle(
                CompassOctant::SouthWest,
                SystemCursorIcon::SwResize,
                Node {
                    position_type: PositionType::Absolute,
                    left: px(0.0),
                    bottom: px(0.0),
                    width: px(CORNER),
                    height: px(CORNER),
                    ..Default::default()
                },
            ),
        ],
    )
}

fn window_resize_handle(
    direction: CompassOctant,
    cursor: SystemCursorIcon,
    node: Node,
) -> impl Bundle {
    (
        WindowResizeRegion(direction),
        EditorEntity,
        node,
        BackgroundColor(Color::NONE),
        EntityCursor::System(cursor),
    )
}

pub(crate) fn handle_window_control_click(
    mut click: On<Pointer<Click>>,
    controls: Query<&WindowControlButton>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut decoration: ResMut<WindowDecorationState>,
    mut exit: MessageWriter<AppExit>,
) {
    let Ok(control) = controls.get(click.event_target()) else {
        return;
    };
    if click.button != PointerButton::Primary {
        return;
    }

    click.propagate(false);
    match control.0 {
        WindowControlAction::Minimize => {
            if let Ok(mut window) = windows.single_mut() {
                window.set_minimized(true);
            }
        }
        WindowControlAction::ToggleMaximize => {
            if let Ok(mut window) = windows.single_mut() {
                decoration.maximized = !decoration.maximized;
                window.set_maximized(decoration.maximized);
            }
        }
        WindowControlAction::Close => {
            exit.write(AppExit::Success);
        }
    }
}

pub(crate) fn handle_window_control_over(
    over: On<Pointer<Over>>,
    controls: Query<&WindowControlButton>,
    mut backgrounds: Query<&mut BackgroundColor>,
) {
    let Ok(control) = controls.get(over.event_target()) else {
        return;
    };
    let Ok(mut background) = backgrounds.get_mut(over.event_target()) else {
        return;
    };

    background.0 = match control.0 {
        WindowControlAction::Close => Color::srgb(0.78, 0.16, 0.18),
        _ => tokens::HOVER_BG,
    };
}

pub(crate) fn handle_window_control_out(
    out: On<Pointer<Out>>,
    controls: Query<&WindowControlButton>,
    mut backgrounds: Query<&mut BackgroundColor>,
) {
    if controls.get(out.event_target()).is_ok()
        && let Ok(mut background) = backgrounds.get_mut(out.event_target())
    {
        background.0 = Color::NONE;
    }
}

pub(crate) fn handle_titlebar_drag_press(
    mut press: On<Pointer<PointerPress>>,
    regions: Query<(), With<TitlebarDragRegion>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    if press.button != PointerButton::Primary || !regions.contains(press.event_target()) {
        return;
    }

    press.propagate(false);
    if let Ok(mut window) = windows.single_mut() {
        window.start_drag_move();
    }
}

pub(crate) fn handle_titlebar_double_click(
    mut click: On<Pointer<Click>>,
    regions: Query<(), With<TitlebarDragRegion>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut decoration: ResMut<WindowDecorationState>,
) {
    if click.button != PointerButton::Primary
        || click.count < 2
        || !regions.contains(click.event_target())
    {
        return;
    }

    click.propagate(false);
    if let Ok(mut window) = windows.single_mut() {
        decoration.maximized = !decoration.maximized;
        window.set_maximized(decoration.maximized);
    }
}

pub(crate) fn handle_window_resize_press(
    mut press: On<Pointer<PointerPress>>,
    regions: Query<&WindowResizeRegion>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    let Ok(region) = regions.get(press.event_target()) else {
        return;
    };
    if press.button != PointerButton::Primary {
        return;
    }

    press.propagate(false);
    if let Ok(mut window) = windows.single_mut() {
        window.start_drag_resize(region.0);
    }
}

/// Project Files panel. File tree browser.
pub fn project_files_panel_content() -> impl Bundle {
    (
        EditorEntity,
        Node {
            flex_direction: FlexDirection::Column,
            width: percent(100),
            height: percent(100),
            ..Default::default()
        },
        children![
            // Search input
            (
                Node {
                    flex_direction: FlexDirection::Column,
                    width: percent(100),
                    padding: UiRect::all(px(tokens::SPACING_SM)),
                    flex_shrink: 0.0,
                    ..Default::default()
                },
                children![(text_edit::text_edit(
                    TextEditProps::default()
                        .with_placeholder("Search...")
                        .allow_empty()
                ),)],
            ),
            // File tree content, populated by ProjectFilesPlugin.
            (
                crate::project_files::ProjectFilesTree,
                EditorEntity,
                Node {
                    flex_direction: FlexDirection::Column,
                    width: percent(100),
                    flex_grow: 1.0,
                    min_height: px(0.0),
                    overflow: Overflow::scroll_y(),
                    padding: UiRect::all(px(tokens::SPACING_SM)),
                    ..Default::default()
                },
            ),
        ],
    )
}

/// Bundle the editor toolbar and the `SceneViewport` node together so
/// `setup_viewport` can mount the whole thing inside the dock tree's
/// "center" leaf in one go. Public to the crate because it's spawned
/// by the viewport plugin, not by `editor_layout` directly.
pub(crate) fn viewport_with_toolbar() -> impl Bundle {
    (
        EditorEntity,
        Node {
            width: percent(100),
            height: percent(100),
            flex_direction: FlexDirection::Column,
            overflow: Overflow::clip(),
            border_radius: BorderRadius::all(px(tokens::BORDER_RADIUS_LG)),
            ..Default::default()
        },
        BackgroundColor(tokens::PANEL_BG),
        children![
            toolbar(),
            crate::navmesh::toolbar::navmesh_toolbar(),
            crate::terrain::toolbar::terrain_toolbar(),
            scene_view(),
        ],
    )
}

fn toolbar() -> impl Bundle {
    // Every toolbar entry below goes through `feathers::button(...)`,
    // the same constructor extensions use. Active-state highlighting
    // is driven by [`update_toolbar_button_variants`] flipping
    // `ButtonVariant::Active` on the owning entity, so we never
    // mutate `BackgroundColor` directly and `handle_hover` stays the
    // sole bg writer.
    //
    // Sizing matches the Figma viewport-toolbar spec: 30px tall, 1px
    // border, top corners rounded against the panel below.
    (
        Toolbar,
        EditorEntity,
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            padding: UiRect {
                left: px(tokens::TOOLBAR_PADDING_LEFT),
                right: px(tokens::TOOLBAR_PADDING_RIGHT),
                top: px(0.0),
                bottom: px(0.0),
            },
            column_gap: px(tokens::TOOLBAR_GAP),
            width: percent(100),
            height: px(tokens::TOOLBAR_HEIGHT),
            border: UiRect::all(px(1.0)),
            border_radius: BorderRadius {
                top_left: px(tokens::TOOLBAR_RADIUS),
                top_right: px(tokens::TOOLBAR_RADIUS),
                bottom_left: px(0.0),
                bottom_right: px(0.0),
            },
            flex_shrink: 0.0,
            ..Default::default()
        },
        BackgroundColor(tokens::PANEL_HEADER_BG),
        BorderColor::all(tokens::TOOLBAR_BORDER),
        children![
            toolbar_op_button::<GizmoModeTranslateOp>(Icon::Move3d),
            toolbar_op_button::<GizmoModeRotateOp>(Icon::Rotate3d),
            toolbar_op_button::<GizmoModeScaleOp>(Icon::Scale3d),
            separator::separator(separator::SeparatorProps::vertical()),
            // Gizmo space toggle. Active highlight = `Local`; default
            // = `World`. Tooltip is the discoverability path.
            toolbar_op_button::<GizmoSpaceToggleOp>(Icon::Globe),
            separator::separator(separator::SeparatorProps::vertical()),
            toolbar_op_button::<EditModeObjectOp>(Icon::MousePointer2),
            toolbar_op_button::<ActivateDrawBrushModalOp>(Icon::Box),
            toolbar_op_button::<MeasureDistanceOp>(Icon::RulerDimensionLine),
            toolbar_op_button::<EditModeVertexOp>(Icon::CircleDot),
            toolbar_op_button::<EditModeEdgeOp>(Icon::GitCommitHorizontal),
            toolbar_op_button::<EditModeFaceOp>(Icon::Hexagon),
            toolbar_op_button::<EditModeClipOp>(Icon::ScissorsLineDashed),
            separator::separator(separator::SeparatorProps::vertical()),
            toolbar_op_button::<PhysicsActivateOp>(Icon::Zap),
        ],
    )
}

/// Spawn a square icon-only toolbar button bound to operator `Op`.
/// Identical to what an extension would write. The icon is the only
/// visible glyph; `ButtonSize::Icon` suppresses the content text
/// label, and the operator's label and description show in the rich
/// operator tooltip on hover via [`OperatorTooltipPlugin`].
///
/// Initial variant is `Ghost` so idle buttons render transparent
/// against the toolbar's `#1F1F24` panel; the
/// [`update_toolbar_button_variants`] system flips them to `Active`
/// when the matching mode/modal is current. Without this, every
/// button would sit on the muted `Default` grey and the toolbar
/// would lose the "one currently-active tool" reading.
///
/// [`OperatorTooltipPlugin`]: crate::operator_tooltip::OperatorTooltipPlugin
fn toolbar_op_button<Op: Operator>(icon: Icon) -> impl Bundle {
    button::button(
        ButtonProps::from_operator::<Op>()
            .with_variant(ButtonVariant::Ghost)
            .icon(icon)
            .with_size(ButtonSize::Icon),
    )
}

pub fn hierarchy_content(icon_font: Handle<Font>) -> impl Bundle {
    let add_entity_icon_font = icon_font.clone();
    (
        HierarchyPanel,
        Node {
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            min_height: px(0.0),
            padding: UiRect::all(px(tokens::SPACING_SM)),
            ..Default::default()
        },
        children![
            (
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(tokens::SPACING_XS),
                    width: percent(100),
                    ..Default::default()
                },
                children![
                    (
                        Node {
                            flex_grow: 1.0,
                            ..Default::default()
                        },
                        children![(
                            HierarchyFilter,
                            text_edit::text_edit(
                                TextEditProps::default()
                                    .with_placeholder("Filter...")
                                    .allow_empty()
                            ),
                        )],
                    ),
                    (
                        HierarchyShowAllButton,
                        Interaction::default(),
                        Hovered::default(),
                        jackdaw_feathers::tooltip::Tooltip::title("Show All Entities")
                            .with_description(
                                "Toggle visibility of editor-internal entities and \
                                 hidden objects in the hierarchy.",
                            ),
                        Node {
                            width: px(24.0),
                            height: px(24.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            border_radius: BorderRadius::all(px(tokens::BORDER_RADIUS_SM)),
                            ..Default::default()
                        },
                        children![(
                            Text::new(String::from(Icon::Eye.unicode())),
                            TextFont {
                                font: (icon_font).into(),
                                font_size: (14.0).into(),
                                ..Default::default()
                            },
                            TextColor(tokens::TEXT_SECONDARY),
                        )],
                    ),
                ],
            ),
            (
                crate::add_entity_picker::AddEntityButton,
                Interaction::default(),
                Hovered::default(),
                Node {
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    width: percent(100),
                    height: px(tokens::ROW_HEIGHT),
                    column_gap: px(tokens::SPACING_SM),
                    border_radius: BorderRadius::all(px(tokens::BORDER_RADIUS_MD)),
                    margin: UiRect::vertical(px(tokens::SPACING_XS)),
                    flex_shrink: 0.0,
                    ..Default::default()
                },
                BackgroundColor(tokens::ELEVATED_BG),
                observe(
                    |hover: On<Pointer<Over>>, mut bg: Query<&mut BackgroundColor>| {
                        if let Ok(mut bg) = bg.get_mut(hover.event_target()) {
                            bg.0 = tokens::TOOLBAR_ACTIVE_BG;
                        }
                    },
                ),
                observe(
                    |out: On<Pointer<Out>>, mut bg: Query<&mut BackgroundColor>| {
                        if let Ok(mut bg) = bg.get_mut(out.event_target()) {
                            bg.0 = tokens::ELEVATED_BG;
                        }
                    },
                ),
                observe(|mut click: On<Pointer<Click>>, mut commands: Commands| {
                    click.propagate(false);
                    commands.queue(|world: &mut World| {
                        world.run_system_cached(crate::add_entity_picker::open_add_entity_picker)
                    });
                },),
                children![
                    (
                        Text::new(String::from(Icon::PackagePlus.unicode())),
                        TextFont {
                            font: (add_entity_icon_font).into(),
                            font_size: (tokens::ICON_SM).into(),
                            ..Default::default()
                        },
                        TextColor(tokens::TEXT_PRIMARY),
                    ),
                    (
                        Text::new("Add Entity"),
                        TextFont {
                            font_size: (tokens::TEXT_SIZE).into(),
                            weight: FontWeight::MEDIUM,
                            ..Default::default()
                        },
                        TextColor(tokens::TEXT_PRIMARY),
                    ),
                ],
            ),
            (
                HierarchyTreeContainer,
                Node {
                    flex_direction: FlexDirection::Column,
                    width: percent(100),
                    flex_grow: 1.0,
                    min_height: px(0.0),
                    overflow: Overflow::scroll_y(),
                    margin: UiRect::top(px(tokens::SPACING_SM)),
                    ..Default::default()
                },
                BackgroundColor(Color::NONE),
                tree_container_drop_observers(),
            ),
            (
                crate::status_bar::SceneStatsText,
                Text::new(""),
                TextFont {
                    font_size: (tokens::FONT_SM).into(),
                    ..Default::default()
                },
                TextColor(tokens::TEXT_SECONDARY),
                TextLayout::justify(Justify::Center),
                Node {
                    padding: UiRect::all(px(tokens::SPACING_XS)),
                    flex_shrink: 0.0,
                    width: percent(100),
                    ..Default::default()
                },
            )
        ],
    )
}

fn scene_view() -> impl Bundle {
    (
        EditorEntity,
        SceneViewport,
        Node {
            width: percent(100),
            flex_grow: 1.0,
            ..Default::default()
        },
    )
}

/// Flip every toolbar button's [`ButtonVariant`] between `Default`
/// and `Active` based on the matching editor state. The feathers
/// `handle_hover` system reads the variant to compute the
/// background, so this is the only place toolbar active-state lives;
/// `BackgroundColor` is never mutated directly. New toolbar buttons
/// just need to register their operator id below to opt in.
///
/// Runs every frame: `ActiveModalOperator` is added/removed via
/// observers that don't trigger `Res::is_changed()` on any of the
/// scalar resources, so a change-detection short-circuit would miss
/// the start of a Draw Brush / Measure Distance / etc. session. The
/// loop is O(toolbar buttons), trivially cheap.
pub fn update_toolbar_button_variants(
    edit_mode: Res<EditMode>,
    gizmo_mode: Res<GizmoMode>,
    gizmo_space: Res<GizmoSpace>,
    active_modal: ActiveModalQuery,
    mut buttons: Query<(&ButtonOperatorCall, &mut ButtonVariant)>,
) {
    let modal_running = active_modal.is_modal_running();
    for (call, mut variant) in &mut buttons {
        // While any modal is running only the modal's own button
        // highlights. Gizmo / mode buttons go quiet so the user sees
        // a single active tool at a time, matching how Blender
        // surfaces the current mode. New extension modal operators
        // pick this up automatically through the fall-through arm.
        let active = if modal_running {
            active_modal.is_operator(&call.id)
        } else if call.id == GizmoModeTranslateOp::ID {
            *gizmo_mode == GizmoMode::Translate
        } else if call.id == GizmoModeRotateOp::ID {
            *gizmo_mode == GizmoMode::Rotate
        } else if call.id == GizmoModeScaleOp::ID {
            *gizmo_mode == GizmoMode::Scale
        } else if call.id == GizmoSpaceToggleOp::ID {
            *gizmo_space == GizmoSpace::Local
        } else if call.id == EditModeObjectOp::ID {
            *edit_mode == EditMode::Object
        } else if call.id == EditModeVertexOp::ID {
            *edit_mode == EditMode::BrushEdit(BrushEditMode::Vertex)
        } else if call.id == EditModeEdgeOp::ID {
            *edit_mode == EditMode::BrushEdit(BrushEditMode::Edge)
        } else if call.id == EditModeFaceOp::ID {
            *edit_mode == EditMode::BrushEdit(BrushEditMode::Face)
        } else if call.id == EditModeClipOp::ID {
            *edit_mode == EditMode::BrushEdit(BrushEditMode::Clip)
        } else if call.id == PhysicsActivateOp::ID {
            *edit_mode == EditMode::Physics
        } else {
            false
        };
        // Inactive toolbar buttons fall back to `Ghost` (transparent)
        // so only the active one stands out as solid grey. Using
        // `Default` here would tint every idle button with the muted
        // ZINC_700 fill at ~50% alpha and they'd all read as
        // "highlighted" against the toolbar's dark panel.
        let target = if active {
            ButtonVariant::Active
        } else {
            ButtonVariant::Ghost
        };
        if *variant != target {
            *variant = target;
        }
    }
}

/// Toggle document-root visibility when the active tab changes.
pub fn update_active_document_display(
    active: Res<ActiveDocument>,
    mut roots: Query<(&DocumentRoot, &mut Node)>,
) {
    if !active.is_changed() {
        return;
    }
    for (root, mut node) in &mut roots {
        node.display = if root.0 == active.kind {
            Display::Flex
        } else {
            Display::None
        };
    }
}

/// Refresh tab-strip styling. Active tab gets its bg + border; inactive
/// tabs go transparent. Schedule Explorer dims when Remote is
/// disconnected.
pub fn update_tab_strip_highlights(
    active: Res<ActiveDocument>,
    manager: Res<ConnectionManager>,
    mut tabs: Query<(
        &DocumentTabButton,
        &mut BackgroundColor,
        &mut BorderColor,
        &Children,
    )>,
    mut texts: Query<&mut TextColor>,
) {
    if !active.is_changed() && !manager.is_changed() {
        return;
    }
    let connected = manager.is_connected();
    for (tab, mut tab_bg, mut tab_border, children) in &mut tabs {
        let is_active = tab.0 == active.kind;
        let is_disabled = tab.0 == TabKind::ScheduleExplorer && !connected;

        tab_bg.0 = if is_active {
            tokens::DOC_TAB_ACTIVE_BG
        } else {
            Color::NONE
        };
        *tab_border = BorderColor::all(if is_active {
            tokens::DOC_TAB_ACTIVE_BORDER
        } else {
            Color::NONE
        });

        let label_color = if is_disabled {
            Color::srgba(0.4, 0.4, 0.4, 0.5)
        } else if is_active {
            tokens::DOC_TAB_ACTIVE_LABEL
        } else {
            tokens::DOC_TAB_INACTIVE_LABEL
        };

        // First child is the accent strip; skip it (its color is
        // type-fixed). Second and third children are the icon and
        // label text; refresh their colors.
        for child in children.iter().skip(1) {
            if let Ok(mut tc) = texts.get_mut(child) {
                tc.0 = label_color;
            }
        }
    }
}

/// Custom status bar that wraps the feathers status bar sections and adds
/// a connection indicator on the far right.
fn editor_status_bar() -> impl Bundle {
    (
        status_bar::StatusBar,
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::SpaceBetween,
            width: Val::Percent(100.0),
            height: Val::Px(tokens::STATUS_BAR_HEIGHT),
            padding: UiRect::horizontal(Val::Px(tokens::SPACING_MD)),
            flex_shrink: 0.0,
            ..Default::default()
        },
        BackgroundColor(tokens::WINDOW_BG),
        children![
            (
                status_bar::StatusBarLeft,
                Text::new("Ready"),
                TextFont {
                    font_size: (tokens::FONT_SM).into(),
                    ..Default::default()
                },
                bevy::feathers::theme::ThemedText,
            ),
            (
                status_bar::StatusBarCenter,
                Text::new(""),
                TextFont {
                    font_size: (tokens::FONT_SM).into(),
                    ..Default::default()
                },
                TextColor(tokens::TEXT_SECONDARY),
            ),
            // Right side: gizmo info + connection indicator
            (
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(tokens::SPACING_LG),
                    ..Default::default()
                },
                children![
                    (
                        status_bar::StatusBarRight,
                        Text::new(""),
                        TextFont {
                            font_size: (tokens::FONT_SM).into(),
                            ..Default::default()
                        },
                        TextColor(tokens::TEXT_SECONDARY),
                    ),
                    // Connection indicator
                    crate::remote::panel::connection_indicator()
                ],
            )
        ],
    )
}

pub fn inspector_components_content(icon_font: Handle<Font>) -> impl Bundle {
    (
        Node {
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            min_height: px(0.0),
            ..Default::default()
        },
        children![
            (
                Node {
                    flex_direction: FlexDirection::Column,
                    width: percent(100),
                    padding: UiRect::all(px(tokens::SPACING_SM)),
                    row_gap: px(tokens::SPACING_XS),
                    flex_shrink: 0.0,
                    ..Default::default()
                },
                children![
                    (
                        crate::inspector::InspectorSearch,
                        text_edit::text_edit(
                            TextEditProps::default()
                                .with_placeholder("Filter...")
                                .allow_empty()
                        ),
                    ),
                    (
                        crate::inspector::AddComponentButton,
                        Interaction::default(),
                        Node {
                            flex_direction: FlexDirection::Row,
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            width: percent(100),
                            height: px(tokens::ROW_HEIGHT),
                            column_gap: px(tokens::SPACING_SM),
                            border_radius: BorderRadius::all(px(tokens::BORDER_RADIUS_MD)),
                            flex_shrink: 0.0,
                            ..Default::default()
                        },
                        BackgroundColor(tokens::ELEVATED_BG),
                        observe(
                            |hover: On<Pointer<Over>>, mut bg: Query<&mut BackgroundColor>| {
                                if let Ok(mut bg) = bg.get_mut(hover.event_target()) {
                                    bg.0 = tokens::TOOLBAR_ACTIVE_BG;
                                }
                            },
                        ),
                        observe(
                            |out: On<Pointer<Out>>, mut bg: Query<&mut BackgroundColor>| {
                                if let Ok(mut bg) = bg.get_mut(out.event_target()) {
                                    bg.0 = tokens::ELEVATED_BG;
                                }
                            },
                        ),
                        children![
                            (
                                Text::new(String::from(Icon::PackagePlus.unicode())),
                                TextFont {
                                    font: (icon_font).into(),
                                    font_size: (tokens::ICON_SM).into(),
                                    ..Default::default()
                                },
                                TextColor(tokens::TEXT_PRIMARY),
                            ),
                            (
                                Text::new("Add Component"),
                                TextFont {
                                    font_size: (tokens::TEXT_SIZE).into(),
                                    weight: FontWeight::MEDIUM,
                                    ..Default::default()
                                },
                                TextColor(tokens::TEXT_PRIMARY),
                            ),
                        ],
                        observe(|click: On<Pointer<Click>>, mut commands: Commands| {
                            commands.trigger(jackdaw_feathers::button::ButtonClickEvent {
                                entity: click.event_target(),
                            });
                        },),
                    ),
                ],
            ),
            (
                Inspector,
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: px(tokens::SPACING_SM),
                    overflow: Overflow::scroll_y(),
                    flex_grow: 1.0,
                    min_height: px(0.0),
                    padding: UiRect::all(px(tokens::SPACING_SM)),
                    ..Default::default()
                }
            ),
        ],
    )
}
