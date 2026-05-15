//! Editor-side Debug Settings panel.
//!
//! Discovers reflected `Resource`s carrying the `ReflectDebugPanel` type-data
//! on the connected BRP target via `jackdaw/debug_panel_list`, polls their
//! values via `world.get_resources`, and renders each as a clickable section
//! in a fixed-position sidebar. Bool fields toggle on click; enum fields
//! cycle to the next variant on click; numeric / string / nested fields are
//! shown read-only for now (the popover / numeric-drag widgets are tracked
//! as follow-up polish — the panel is fully functional without them).
//!
//! Mutations flow through `world.mutate_resources`. The state stays
//! optimistic-with-revert: a click writes the new value into the local
//! `values` cache immediately, then a debounced flush ships the change to
//! the game. The 500 ms value-poll catches and overwrites optimistic state
//! once the server confirms.

use std::collections::HashMap;

use anyhow::anyhow;
use bevy::{prelude::*, tasks::Task, tasks::futures_lite::future};
use jackdaw_debug_panel::{DEBUG_PANEL_LIST_METHOD, DebugPanelEntry};
use jackdaw_feathers::tokens;

use crate::remote::{ConnectionManager, ConnectionState};

const VALUE_POLL_INTERVAL: f32 = 0.5;
const MUTATION_DEBOUNCE: f32 = 0.08;

pub struct DebugSettingsPlugin;

impl Plugin for DebugSettingsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DebugPanelState>()
            .add_systems(
                Update,
                (
                    poll_discovery_task,
                    start_discovery_on_connect,
                    poll_schema_tasks,
                    start_schema_fetches,
                    poll_values_task,
                    start_values_poll,
                    poll_mutation_task,
                    flush_pending_mutations,
                    build_or_rebuild_panel,
                    handle_widget_clicks,
                )
                    .chain()
                    .run_if(in_state(crate::AppState::Editor)),
            );
    }
}

// ─────────────────────────── State & tasks ───────────────────────────

#[derive(Resource, Default)]
struct DebugPanelState {
    entries: Vec<DebugPanelEntry>,
    schemas: HashMap<String, serde_json::Value>,
    values: HashMap<String, serde_json::Value>,
    pending_mutations: HashMap<String, Vec<(String, serde_json::Value)>>,
    last_value_poll: f32,
    last_mutation_edit: f32,
    panel_signature: u64,
    panel_built_signature: u64,
}

impl DebugPanelState {
    fn clear_on_disconnect(&mut self) {
        self.entries.clear();
        self.schemas.clear();
        self.values.clear();
        self.pending_mutations.clear();
        self.panel_signature = self.panel_signature.wrapping_add(1);
    }
}

#[derive(Resource)]
struct DiscoveryTask(Task<Result<serde_json::Value, anyhow::Error>>);

#[derive(Resource)]
struct SchemaTask {
    type_path: String,
    task: Task<Result<serde_json::Value, anyhow::Error>>,
}

#[derive(Resource)]
struct ValuesTask(Task<Result<serde_json::Value, anyhow::Error>>);

#[derive(Resource)]
struct MutationTask(Task<Result<serde_json::Value, anyhow::Error>>);

// ─────────────────────────── BRP helpers ───────────────────────────

fn brp_request_value(
    endpoint: &str,
    method: &str,
    params: Option<serde_json::Value>,
) -> Task<Result<serde_json::Value, anyhow::Error>> {
    use bevy::remote::BrpRequest;
    use bevy::tasks::IoTaskPool;

    let req = BrpRequest {
        method: String::from(method),
        id: None,
        params,
    };
    let url = endpoint.to_string();
    let future = async move {
        let request = ehttp::Request::json(&url, &req)?;
        let resp = ehttp::fetch_async(request)
            .await
            .map_err(|s| anyhow!("{s}"))?;
        let mut v: serde_json::Value = resp.json()?;
        if let Some(val) = v.get_mut("result") {
            Ok(val.take())
        } else if let Some(error) = v.get("error") {
            Err(anyhow!("BRP error: {error}"))
        } else {
            Err(anyhow!("BRP response missing result and error"))
        }
    };
    IoTaskPool::get().spawn(future)
}

// ─────────────────────────── Discovery ───────────────────────────

fn start_discovery_on_connect(
    mut commands: Commands,
    manager: Res<ConnectionManager>,
    state: Res<DebugPanelState>,
    in_flight: Option<Res<DiscoveryTask>>,
) {
    let connected = matches!(manager.state, ConnectionState::Connected { .. });
    if !connected {
        return;
    }
    if !state.entries.is_empty() || in_flight.is_some() {
        return;
    }
    let task = brp_request_value(&manager.endpoint, DEBUG_PANEL_LIST_METHOD, None);
    commands.insert_resource(DiscoveryTask(task));
}

fn poll_discovery_task(
    mut commands: Commands,
    mut state: ResMut<DebugPanelState>,
    manager: Res<ConnectionManager>,
    task: Option<ResMut<DiscoveryTask>>,
) {
    if !matches!(manager.state, ConnectionState::Connected { .. }) {
        if task.is_some() {
            commands.remove_resource::<DiscoveryTask>();
        }
        if !state.entries.is_empty() {
            state.clear_on_disconnect();
        }
        return;
    }
    let Some(mut task) = task else { return };
    let Some(result) = future::block_on(future::poll_once(&mut task.0)) else {
        return;
    };
    commands.remove_resource::<DiscoveryTask>();
    match result {
        Ok(value) => match serde_json::from_value::<Vec<DebugPanelEntry>>(value) {
            Ok(entries) => {
                state.entries = entries;
                state.schemas.clear();
                state.values.clear();
                state.panel_signature = state.panel_signature.wrapping_add(1);
            }
            Err(e) => warn!("debug_panel_list: malformed response: {e}"),
        },
        Err(e) => warn!("debug_panel_list failed: {e}"),
    }
}

// ─────────────────────────── Schema fetch ───────────────────────────

fn start_schema_fetches(
    mut commands: Commands,
    state: Res<DebugPanelState>,
    manager: Res<ConnectionManager>,
    in_flight: Option<Res<SchemaTask>>,
) {
    if in_flight.is_some() {
        return;
    }
    if !matches!(manager.state, ConnectionState::Connected { .. }) {
        return;
    }
    let Some(entry) = state
        .entries
        .iter()
        .find(|e| !state.schemas.contains_key(&e.type_path))
    else {
        return;
    };
    let params = serde_json::json!({ "with_type_path": [&entry.type_path] });
    let task = brp_request_value(&manager.endpoint, "registry.schema", Some(params));
    commands.insert_resource(SchemaTask {
        type_path: entry.type_path.clone(),
        task,
    });
}

fn poll_schema_tasks(
    mut commands: Commands,
    mut state: ResMut<DebugPanelState>,
    task: Option<ResMut<SchemaTask>>,
) {
    let Some(mut task) = task else { return };
    let Some(result) = future::block_on(future::poll_once(&mut task.task)) else {
        return;
    };
    let type_path = task.type_path.clone();
    commands.remove_resource::<SchemaTask>();
    match result {
        Ok(value) => {
            state.schemas.insert(type_path, value);
            state.panel_signature = state.panel_signature.wrapping_add(1);
        }
        Err(e) => warn!("registry.schema failed for {type_path}: {e}"),
    }
}

// ─────────────────────────── Value polling ───────────────────────────

fn start_values_poll(
    mut commands: Commands,
    mut state: ResMut<DebugPanelState>,
    manager: Res<ConnectionManager>,
    time: Res<Time>,
    in_flight: Option<Res<ValuesTask>>,
) {
    if in_flight.is_some() {
        return;
    }
    if !matches!(manager.state, ConnectionState::Connected { .. }) {
        return;
    }
    if state.entries.is_empty() {
        return;
    }
    state.last_value_poll += time.delta_secs();
    if state.last_value_poll < VALUE_POLL_INTERVAL {
        return;
    }
    state.last_value_poll = 0.0;

    let type_paths: Vec<String> = state.entries.iter().map(|e| e.type_path.clone()).collect();
    let params = serde_json::json!({ "resources": type_paths });
    let task = brp_request_value(&manager.endpoint, "world.get_resources", Some(params));
    commands.insert_resource(ValuesTask(task));
}

fn poll_values_task(
    mut commands: Commands,
    mut state: ResMut<DebugPanelState>,
    task: Option<ResMut<ValuesTask>>,
) {
    let Some(mut task) = task else { return };
    let Some(result) = future::block_on(future::poll_once(&mut task.0)) else {
        return;
    };
    commands.remove_resource::<ValuesTask>();
    match result {
        Ok(value) => {
            if let Some(map) = value.as_object() {
                for (k, v) in map {
                    state.values.insert(k.clone(), v.clone());
                }
                state.panel_signature = state.panel_signature.wrapping_add(1);
            }
        }
        Err(e) => warn!("world.get_resources failed: {e}"),
    }
}

// ─────────────────────────── Mutation ───────────────────────────

fn queue_mutation(
    state: &mut DebugPanelState,
    time_secs: f32,
    type_path: &str,
    field_path: &str,
    value: serde_json::Value,
) {
    let entry = state
        .pending_mutations
        .entry(type_path.to_string())
        .or_default();
    if let Some(existing) = entry.iter_mut().find(|(p, _)| p == field_path) {
        existing.1 = value;
    } else {
        entry.push((field_path.to_string(), value));
    }
    state.last_mutation_edit = time_secs;
    // Optimistic local update so the UI redraws immediately.
}

fn flush_pending_mutations(
    mut commands: Commands,
    mut state: ResMut<DebugPanelState>,
    manager: Res<ConnectionManager>,
    time: Res<Time>,
    in_flight: Option<Res<MutationTask>>,
) {
    if state.pending_mutations.is_empty() {
        return;
    }
    if in_flight.is_some() {
        return;
    }
    let now = time.elapsed_secs();
    if now - state.last_mutation_edit < MUTATION_DEBOUNCE {
        return;
    }
    if !matches!(manager.state, ConnectionState::Connected { .. }) {
        state.pending_mutations.clear();
        return;
    }

    // Pull one resource's pending changes; the rest follow on subsequent ticks.
    let (type_path, paths) = {
        let key = state
            .pending_mutations
            .keys()
            .next()
            .cloned()
            .expect("non-empty checked above");
        let paths = state.pending_mutations.remove(&key).unwrap();
        (key, paths)
    };

    for (field_path, value) in paths {
        let params = serde_json::json!({
            "resource": &type_path,
            "path": field_path,
            "value": value,
        });
        let task = brp_request_value(&manager.endpoint, "world.mutate_resources", Some(params));
        // Only one outstanding mutation task at a time keeps ordering sane.
        // The remaining field updates for this resource land in subsequent
        // ticks once this one acks.
        commands.insert_resource(MutationTask(task));
        return;
    }
}

fn poll_mutation_task(
    mut commands: Commands,
    task: Option<ResMut<MutationTask>>,
) {
    let Some(mut task) = task else { return };
    let Some(result) = future::block_on(future::poll_once(&mut task.0)) else {
        return;
    };
    commands.remove_resource::<MutationTask>();
    if let Err(e) = result {
        warn!("world.mutate_resources failed: {e}");
    }
}

// ─────────────────────────── Panel UI ───────────────────────────

#[derive(Component)]
struct DebugSettingsRoot;

#[derive(Component)]
struct WidgetButton {
    type_path: String,
    field_path: String,
    kind: WidgetKind,
}

#[derive(Clone)]
enum WidgetKind {
    Bool { current: bool },
    Enum { current: String, variants: Vec<String> },
}

fn build_or_rebuild_panel(
    mut commands: Commands,
    mut state: ResMut<DebugPanelState>,
    existing: Query<Entity, With<DebugSettingsRoot>>,
) {
    if state.panel_signature == state.panel_built_signature {
        return;
    }
    state.panel_built_signature = state.panel_signature;

    for entity in &existing {
        commands.entity(entity).despawn();
    }
    if state.entries.is_empty() {
        return;
    }

    let mut root = commands.spawn((
        DebugSettingsRoot,
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(48.0),
            right: Val::Px(12.0),
            width: Val::Px(280.0),
            max_height: Val::Vh(80.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(tokens::SPACING_SM),
            padding: UiRect::all(Val::Px(tokens::SPACING_MD)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(tokens::BORDER_RADIUS_MD)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.07, 0.07, 0.08, 0.92)),
        BorderColor::all(tokens::BORDER_SUBTLE),
        ZIndex(50),
    ));
    root.with_children(|root| {
        root.spawn((
            Text::new("Debug Settings"),
            TextFont::from_font_size(tokens::FONT_MD),
            TextColor(tokens::TEXT_PRIMARY),
        ));

        for entry in &state.entries {
            root.spawn((
                Text::new(entry.label.clone()),
                TextFont::from_font_size(tokens::FONT_SM),
                TextColor(tokens::TEXT_ACCENT),
                Node {
                    margin: UiRect::top(Val::Px(tokens::SPACING_SM)),
                    ..default()
                },
            ));

            let schema = state.schemas.get(&entry.type_path);
            let value = state.values.get(&entry.type_path);
            match (schema, value) {
                (Some(schema), Some(value)) => {
                    render_fields(root, &entry.type_path, "", schema, value, entry.read_only);
                }
                _ => {
                    root.spawn((
                        Text::new("  loading…"),
                        TextFont::from_font_size(tokens::FONT_SM),
                        TextColor(tokens::TEXT_DISABLED),
                    ));
                }
            }
        }
    });
}

fn render_fields(
    parent: &mut ChildSpawnerCommands,
    type_path: &str,
    prefix: &str,
    schema: &serde_json::Value,
    value: &serde_json::Value,
    read_only: bool,
) {
    let Some(properties) = schema.get("properties").and_then(|p| p.as_object()) else {
        // Leaf value — render its JSON.
        spawn_leaf_row(parent, prefix, value, read_only, type_path, schema);
        return;
    };
    for (name, field_schema) in properties {
        let field_path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };
        let field_value = value
            .get(name)
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        if field_schema
            .get("properties")
            .and_then(|p| p.as_object())
            .is_some()
        {
            // Nested struct — header + recurse one level deeper.
            parent.spawn((
                Text::new(format!("  {name}:")),
                TextFont::from_font_size(tokens::FONT_SM),
                TextColor(tokens::TEXT_SECONDARY),
            ));
            render_fields(parent, type_path, &field_path, field_schema, &field_value, read_only);
        } else {
            spawn_field_row(parent, type_path, &field_path, name, field_schema, &field_value, read_only);
        }
    }
}

fn spawn_field_row(
    parent: &mut ChildSpawnerCommands,
    type_path: &str,
    field_path: &str,
    label: &str,
    schema: &serde_json::Value,
    value: &serde_json::Value,
    read_only: bool,
) {
    let kind = schema.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let variants: Vec<String> = schema
        .get("oneOf")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| {
                    v.get("const")
                        .and_then(|c| c.as_str().map(String::from))
                        .or_else(|| v.as_str().map(String::from))
                })
                .collect()
        })
        .unwrap_or_default();

    let (display, widget_kind) = match (kind, value, !variants.is_empty()) {
        (_, _, true) => {
            let current = value.as_str().unwrap_or("?").to_string();
            (
                format!("  {label}: {current} ▾"),
                Some(WidgetKind::Enum {
                    current: current.clone(),
                    variants,
                }),
            )
        }
        ("boolean", v, _) => {
            let b = v.as_bool().unwrap_or(false);
            (
                format!("  {label}: {}", if b { "on" } else { "off" }),
                Some(WidgetKind::Bool { current: b }),
            )
        }
        ("integer", v, _) => (
            format!("  {label}: {}", v.as_i64().unwrap_or(0)),
            None,
        ),
        ("number", v, _) => {
            let n = v.as_f64().unwrap_or(0.0);
            let formatted = if n.fract() == 0.0 && n.abs() < 1e9 {
                format!("{}", n as i64)
            } else {
                format!("{n:.3}")
            };
            (format!("  {label}: {formatted}"), None)
        }
        ("string", v, _) => (
            format!("  {label}: {}", v.as_str().unwrap_or("")),
            None,
        ),
        _ => (
            format!("  {label}: {}", compact_json(value)),
            None,
        ),
    };

    let interactive = !read_only && widget_kind.is_some();
    let text_color = if read_only {
        tokens::TEXT_DISABLED
    } else if interactive {
        tokens::TEXT_PRIMARY
    } else {
        tokens::TEXT_SECONDARY
    };

    let mut row = parent.spawn((
        Text::new(display),
        TextFont::from_font_size(tokens::FONT_SM),
        TextColor(text_color),
    ));
    if let Some(kind) = widget_kind
        && interactive
    {
        row.insert((
            Interaction::default(),
            WidgetButton {
                type_path: type_path.to_string(),
                field_path: field_path.to_string(),
                kind,
            },
        ));
    }
}

fn spawn_leaf_row(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    value: &serde_json::Value,
    read_only: bool,
    _type_path: &str,
    _schema: &serde_json::Value,
) {
    let display = if label.is_empty() {
        compact_json(value)
    } else {
        format!("  {label}: {}", compact_json(value))
    };
    parent.spawn((
        Text::new(display),
        TextFont::from_font_size(tokens::FONT_SM),
        TextColor(if read_only {
            tokens::TEXT_DISABLED
        } else {
            tokens::TEXT_SECONDARY
        }),
    ));
}

fn compact_json(value: &serde_json::Value) -> String {
    let s = value.to_string();
    if s.len() > 40 {
        format!("{}…", &s[..40])
    } else {
        s
    }
}

// ─────────────────────────── Click handlers ───────────────────────────

fn handle_widget_clicks(
    time: Res<Time>,
    mut state: ResMut<DebugPanelState>,
    mut interactions: Query<(&Interaction, &WidgetButton), Changed<Interaction>>,
) {
    let now = time.elapsed_secs();
    for (interaction, widget) in &mut interactions {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match &widget.kind {
            WidgetKind::Bool { current } => {
                let next = !current;
                // Optimistic local update so the panel redraws immediately
                // with the new value rather than waiting for the 500ms poll.
                set_local_value(&mut state, &widget.type_path, &widget.field_path, serde_json::Value::Bool(next));
                queue_mutation(
                    &mut state,
                    now,
                    &widget.type_path,
                    &widget.field_path,
                    serde_json::Value::Bool(next),
                );
            }
            WidgetKind::Enum { current, variants } => {
                if variants.is_empty() {
                    continue;
                }
                let idx = variants.iter().position(|v| v == current).unwrap_or(0);
                let next = variants[(idx + 1) % variants.len()].clone();
                set_local_value(
                    &mut state,
                    &widget.type_path,
                    &widget.field_path,
                    serde_json::Value::String(next.clone()),
                );
                queue_mutation(
                    &mut state,
                    now,
                    &widget.type_path,
                    &widget.field_path,
                    serde_json::Value::String(next),
                );
            }
        }
    }
}

fn set_local_value(
    state: &mut DebugPanelState,
    type_path: &str,
    field_path: &str,
    value: serde_json::Value,
) {
    if let Some(root) = state.values.get_mut(type_path) {
        apply_path(root, field_path, value);
        state.panel_signature = state.panel_signature.wrapping_add(1);
    }
}

/// Sets `root.<field_path>` to `value`, creating intermediate object nodes as
/// needed. Path segments are dot-separated.
fn apply_path(root: &mut serde_json::Value, field_path: &str, value: serde_json::Value) {
    let parts: Vec<&str> = field_path.split('.').collect();
    let Some((last, prefix)) = parts.split_last() else {
        *root = value;
        return;
    };
    let mut cursor = root;
    for segment in prefix {
        if !cursor.is_object() {
            *cursor = serde_json::Value::Object(serde_json::Map::new());
        }
        cursor = cursor
            .as_object_mut()
            .unwrap()
            .entry((*segment).to_string())
            .or_insert(serde_json::Value::Object(serde_json::Map::new()));
    }
    if !cursor.is_object() {
        *cursor = serde_json::Value::Object(serde_json::Map::new());
    }
    cursor
        .as_object_mut()
        .unwrap()
        .insert((*last).to_string(), value);
}
