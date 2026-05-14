//! Project Files panel: a file tree view with live filesystem watching.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, mpsc};

use bevy::prelude::*;
use jackdaw_feathers::{
    file_browser,
    icons::{Icon, IconFont},
    tokens,
};
use jackdaw_widgets::tree_view::{
    TreeChildrenPopulated, TreeNodeExpandToggle, TreeNodeExpanded, TreeRowChildren, TreeRowContent,
    TreeRowLabel,
};

// EditorEntity not needed for project file nodes

pub struct ProjectFilesPlugin;

impl Plugin for ProjectFilesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ProjectFilesState>()
            .add_systems(OnEnter(crate::AppState::Editor), setup_project_files)
            .add_systems(
                Update,
                (check_project_watcher, refresh_project_tree)
                    .run_if(in_state(crate::AppState::Editor)),
            )
            .add_observer(handle_directory_expand);
    }
}

/// State for the project files panel.
#[derive(Resource, Default)]
pub struct ProjectFilesState {
    pub root_directory: PathBuf,
    pub needs_refresh: bool,
    pub initialized: bool,
}

/// Marker on the project files tree container.
#[derive(Component)]
pub struct ProjectFilesTree;

/// Component on tree nodes representing a filesystem path.
#[derive(Component)]
pub struct ProjectFileNode(pub PathBuf);

/// Marker for directory nodes (have expandable children).
#[derive(Component)]
pub struct ProjectFileIsDir;

/// File watcher resource for the project root.
#[derive(Resource)]
struct ProjectFileWatcher {
    _watcher: notify::RecommendedWatcher,
    receiver: Mutex<mpsc::Receiver<()>>,
}

/// Initial setup: read project root and set up file watcher.
fn setup_project_files(
    project_root: Option<Res<crate::project::ProjectRoot>>,
    mut state: ResMut<ProjectFilesState>,
    mut commands: Commands,
) {
    let root = project_root
        .map(|p| p.root.clone())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    state.root_directory = root.clone();
    state.needs_refresh = true;
    state.initialized = false;
    debug!("ProjectFiles: setup root {}", root.display());

    // Set up file watcher
    let (tx, rx) = mpsc::channel();
    let watcher = notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
        if let Ok(event) = res {
            use notify::EventKind;
            if matches!(
                event.kind,
                EventKind::Create(_)
                    | EventKind::Remove(_)
                    | EventKind::Modify(notify::event::ModifyKind::Name(_))
            ) {
                let _ = tx.send(());
            }
        }
    });
    if let Ok(mut w) = watcher {
        use notify::Watcher;
        if w.watch(&root, notify::RecursiveMode::Recursive).is_ok() {
            commands.insert_resource(ProjectFileWatcher {
                _watcher: w,
                receiver: Mutex::new(rx),
            });
        }
    }
}

/// Poll the file watcher for changes.
fn check_project_watcher(
    watcher: Option<Res<ProjectFileWatcher>>,
    mut state: ResMut<ProjectFilesState>,
) {
    let Some(watcher) = watcher else { return };
    let Ok(rx) = watcher.receiver.lock() else {
        return;
    };
    if rx.try_recv().is_ok() {
        // Drain any additional pending events
        while rx.try_recv().is_ok() {}
        state.needs_refresh = true;
    }
}

/// Rebuild the root-level tree when `needs_refresh` is set.
fn refresh_project_tree(
    mut state: ResMut<ProjectFilesState>,
    tree_query: Query<(Entity, Option<&Children>), With<ProjectFilesTree>>,
    mut commands: Commands,
    icon_font: Option<Res<IconFont>>,
) {
    if !state.needs_refresh {
        return;
    }

    let tree_containers = tree_query
        .iter()
        .map(|(entity, children)| (entity, children.map(|children| children.iter().collect())))
        .collect::<Vec<(Entity, Option<Vec<Entity>>)>>();
    if tree_containers.is_empty() {
        debug!("ProjectFiles: refresh pending, no tree containers mounted");
        return;
    }

    let Some(icon_font) = icon_font else {
        debug!("ProjectFiles: refresh pending, icon font unavailable");
        return;
    };

    // Scan root directory
    let root = state.root_directory.clone();
    if !root.is_dir() {
        warn!("ProjectFiles: root is not a directory: {}", root.display());
        return;
    }
    state.needs_refresh = false;

    let mut entries = scan_directory(&root);
    entries.sort_by(|a, b| {
        // Directories first, then alphabetical
        b.1.cmp(&a.1).then_with(|| {
            a.0.file_name()
                .unwrap_or_default()
                .to_ascii_lowercase()
                .cmp(&b.0.file_name().unwrap_or_default().to_ascii_lowercase())
        })
    });
    debug!(
        "ProjectFiles: populating {} tree container(s) from {} with {} visible entrie(s)",
        tree_containers.len(),
        root.display(),
        entries.len()
    );

    for (tree_entity, existing_children) in tree_containers {
        if let Some(children) = existing_children {
            for child in children {
                commands.entity(child).despawn();
            }
        }

        for (path, is_dir) in &entries {
            spawn_file_tree_row(&mut commands, tree_entity, path, *is_dir, &icon_font.0);
        }
    }

    state.initialized = true;
}

/// Handle directory expansion: lazily populate children.
fn handle_directory_expand(
    mut event: On<bevy::picking::events::Pointer<bevy::picking::events::Click>>,
    parent_query: Query<&ChildOf>,
    file_nodes: Query<(), With<ProjectFileNode>>,
    mut tree_nodes: Query<(
        &mut TreeNodeExpanded,
        &mut TreeChildrenPopulated,
        &Children,
        &ProjectFileNode,
    )>,
    children_containers: Query<Entity, With<TreeRowChildren>>,
    mut commands: Commands,
    icon_font: Option<Res<IconFont>>,
    file_dirs: Query<(), With<ProjectFileIsDir>>,
    mut node_query: Query<&mut Node>,
    children_query: Query<&Children>,
    content_query: Query<Entity, With<TreeRowContent>>,
    toggle_markers: Query<Entity, With<TreeNodeExpandToggle>>,
    mut text_query: Query<&mut Text>,
) {
    if event.event.button != PointerButton::Primary {
        return;
    }

    let clicked = event.event_target();

    // Walk up from whatever was clicked inside the row until we hit the
    // project file tree node. Text/icon children are common click targets.
    let mut current = clicked;
    let mut tree_node_entity = None;
    for _ in 0..8 {
        if file_nodes.get(current).is_ok() {
            tree_node_entity = Some(current);
            break;
        }
        let Ok(parent) = parent_query.get(current) else {
            break;
        };
        current = parent.parent();
    }
    let Some(tree_node_entity) = tree_node_entity else {
        return;
    };

    // Only handle directory nodes
    if file_dirs.get(tree_node_entity).is_err() {
        return;
    }
    event.propagate(false);

    let Ok((mut expanded, mut populated, children, file_node)) =
        tree_nodes.get_mut(tree_node_entity)
    else {
        return;
    };

    // Toggle expanded state
    expanded.0 = !expanded.0;

    // Find the TreeRowChildren container
    let Some(children_entity) = children
        .iter()
        .find(|c| children_containers.get(*c).is_ok())
    else {
        return;
    };

    if let Ok(mut node) = node_query.get_mut(children_entity) {
        node.display = if expanded.0 {
            Display::Flex
        } else {
            Display::None
        };
    }

    if expanded.0 && !populated.0 {
        // First expansion: scan and populate children
        populated.0 = true;

        let Some(icon_font) = icon_font else { return };
        let dir_path = &file_node.0;

        let mut entries = scan_directory(dir_path);
        entries.sort_by(|a, b| {
            b.1.cmp(&a.1).then_with(|| {
                a.0.file_name()
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .cmp(&b.0.file_name().unwrap_or_default().to_ascii_lowercase())
            })
        });

        for (path, is_dir) in entries {
            spawn_file_tree_row(&mut commands, children_entity, &path, is_dir, &icon_font.0);
        }
    }

    if let Some(content_entity) = children.iter().find(|c| content_query.get(*c).is_ok())
        && let Ok(content_children) = children_query.get(content_entity)
    {
        for content_child in content_children.iter() {
            if toggle_markers.get(content_child).is_ok()
                && let Ok(mut text) = text_query.get_mut(content_child)
            {
                text.0 = String::from(if expanded.0 {
                    Icon::ChevronDown.unicode()
                } else {
                    Icon::ChevronRight.unicode()
                });
            }
        }
    }
}

/// Scan a directory and return (path, `is_directory`) entries.
fn scan_directory(dir: &Path) -> Vec<(PathBuf, bool)> {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    read_dir
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            let is_dir = path.is_dir();
            // Skip hidden files/directories (starting with .)
            let name = path.file_name()?.to_string_lossy().to_string();
            if name.starts_with('.') {
                return None;
            }
            // Skip target directory
            if name == "target" {
                return None;
            }
            Some((path, is_dir))
        })
        .collect()
}

/// Spawn a single file/directory tree row.
fn spawn_file_tree_row(
    commands: &mut Commands,
    parent: Entity,
    path: &Path,
    is_dir: bool,
    icon_font: &Handle<Font>,
) {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    // Create the tree node entity
    let node_entity = commands
        .spawn((
            // Use the node entity itself as the "source" since we don't have scene entities
            ProjectFileNode(path.to_path_buf()),
            TreeNodeExpanded(false),
            TreeChildrenPopulated(false),
            Node {
                flex_direction: FlexDirection::Column,
                width: Val::Percent(100.0),
                ..Default::default()
            },
            ChildOf(parent),
        ))
        .id();

    // Note: We intentionally do NOT add TreeNode(self) here. TreeNode is a
    // relationship component that would warn about self-referencing. Project file
    // nodes use ProjectFileNode instead of TreeNode for identification.

    if is_dir {
        commands.entity(node_entity).insert(ProjectFileIsDir);
    }

    // Clickable row content
    let content = commands
        .spawn((
            TreeRowContent,
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                padding: UiRect::axes(Val::Px(tokens::SPACING_SM), Val::Px(tokens::SPACING_XS)),
                column_gap: Val::Px(tokens::SPACING_SM),
                border_radius: BorderRadius::all(Val::Px(tokens::BORDER_RADIUS_MD)),
                width: Val::Percent(100.0),
                ..Default::default()
            },
            ChildOf(node_entity),
        ))
        .id();

    // Hover effects
    commands.entity(content).observe(
        |hover: On<Pointer<Over>>, mut bg: Query<&mut BackgroundColor>| {
            if let Ok(mut bg) = bg.get_mut(hover.event_target()) {
                bg.0 = tokens::HOVER_BG;
            }
        },
    );
    commands.entity(content).observe(
        |out: On<Pointer<Out>>, mut bg: Query<&mut BackgroundColor>| {
            if let Ok(mut bg) = bg.get_mut(out.event_target()) {
                bg.0 = Color::NONE;
            }
        },
    );

    if is_dir {
        // Expand toggle (chevron)
        let _ = commands
            .spawn((
                TreeNodeExpandToggle,
                Text::new(String::from(Icon::ChevronRight.unicode())),
                TextFont {
                    font: (icon_font.clone()).into(),
                    font_size: (tokens::ICON_SM).into(),
                    ..Default::default()
                },
                TextColor(tokens::TEXT_SECONDARY),
                Node {
                    width: Val::Px(15.0),
                    flex_shrink: 0.0,
                    ..Default::default()
                },
                ChildOf(content),
            ))
            .id();

        // Directory label (no icon, just text)
        commands.spawn((
            TreeRowLabel,
            Text::new(file_name),
            TextFont {
                font_size: (tokens::TEXT_SIZE).into(),
                ..Default::default()
            },
            TextColor(tokens::TEXT_PRIMARY),
            ChildOf(content),
        ));

        // Children container (initially hidden)
        commands.spawn((
            TreeRowChildren,
            Node {
                flex_direction: FlexDirection::Column,
                padding: UiRect::left(Val::Px(16.0)),
                margin: UiRect::left(Val::Px(tokens::SPACING_SM)),
                border: UiRect::left(Val::Px(1.0)),
                width: Val::Percent(100.0),
                display: Display::None,
                ..Default::default()
            },
            BorderColor::all(tokens::CONNECTION_LINE),
            ChildOf(node_entity),
        ));
    } else {
        // File icon based on extension
        let icon = file_browser::file_icon(&file_name);

        commands.spawn((
            Text::new(String::from(icon.unicode())),
            TextFont {
                font: (icon_font.clone()).into(),
                font_size: (tokens::ICON_SM).into(),
                ..Default::default()
            },
            TextColor(tokens::FILE_ICON_COLOR),
            Node {
                width: Val::Px(15.0),
                flex_shrink: 0.0,
                ..Default::default()
            },
            ChildOf(content),
        ));

        // File label
        commands.spawn((
            TreeRowLabel,
            Text::new(file_name),
            TextFont {
                font_size: (tokens::TEXT_SIZE).into(),
                ..Default::default()
            },
            TextColor(tokens::TEXT_PRIMARY),
            ChildOf(content),
        ));
    }
}
