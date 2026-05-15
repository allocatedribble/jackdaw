use bevy::{
    asset::{AssetLoader, LoadContext, io::Reader},
    ecs::{
        reflect::{AppTypeRegistry, ReflectComponent},
        world::{FromWorld, World},
    },
    prelude::*,
    reflect::{
        ReflectMut, TypeRegistration, TypeRegistry, TypeRegistryArc,
        enums::{DynamicEnum, DynamicVariant},
        prelude::ReflectDefault,
        serde::TypedReflectDeserializer,
    },
    world_serialization::{DynamicWorld, DynamicWorldBuilder},
};
use serde::de::DeserializeSeed;

#[derive(Debug, TypePath)]
pub struct BsnAssetLoader {
    type_registry: TypeRegistryArc,
}

impl FromWorld for BsnAssetLoader {
    fn from_world(world: &mut World) -> Self {
        let type_registry = world.resource::<AppTypeRegistry>();
        Self {
            type_registry: type_registry.0.clone(),
        }
    }
}

impl AssetLoader for BsnAssetLoader {
    type Asset = DynamicWorld;
    type Settings = ();
    type Error = BsnLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .await
            .map_err(|error| BsnLoadError::Io(error.to_string()))?;

        let text =
            std::str::from_utf8(&bytes).map_err(|error| BsnLoadError::Parse(error.to_string()))?;
        let scene = Parser::new(text).parse_scene()?;

        build_dynamic_world(&scene, &self.type_registry).map_err(BsnLoadError::Scene)
    }

    fn extensions(&self) -> &[&str] {
        &["bsn"]
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BsnLoadError {
    #[error("IO error: {0}")]
    Io(String),
    #[error("Parse error: {0}")]
    Parse(String),
    #[error("Scene deserialization error: {0}")]
    Scene(String),
}

#[derive(Debug, Clone, Default)]
struct BsnScene {
    roots: Vec<BsnEntity>,
}

#[derive(Debug, Clone, Default)]
struct BsnEntity {
    patches: Vec<BsnPatch>,
    children: Vec<BsnEntity>,
}

impl BsnEntity {
    fn is_empty(&self) -> bool {
        self.patches.is_empty() && self.children.is_empty()
    }
}

#[derive(Debug, Clone)]
enum BsnPatch {
    Name(String),
    Bare(String),
    Data {
        type_path: String,
        value: serde_json::Value,
    },
}

#[derive(Debug)]
struct FlatBsnEntity {
    parent: Option<usize>,
    patches: Vec<BsnPatch>,
}

fn build_dynamic_world(
    scene: &BsnScene,
    type_registry: &TypeRegistryArc,
) -> Result<DynamicWorld, String> {
    let mut flat = Vec::new();
    for root in &scene.roots {
        flatten_entity(root, None, &mut flat);
    }

    let mut world = World::new();
    world.insert_resource(AppTypeRegistry(type_registry.clone()));

    let mut spawned = Vec::with_capacity(flat.len());
    for entity in &flat {
        let id = world.spawn_empty().id();
        spawned.push(id);
        if let Some(parent_index) = entity.parent
            && let Some(&parent) = spawned.get(parent_index)
        {
            world.entity_mut(id).insert(ChildOf(parent));
        }
    }

    let registry_guard = type_registry.read();
    for (index, entity) in flat.iter().enumerate() {
        let ecs_entity = spawned[index];
        for patch in &entity.patches {
            match patch {
                BsnPatch::Name(name) => {
                    world.entity_mut(ecs_entity).insert(Name::new(name.clone()));
                }
                BsnPatch::Bare(type_path) => {
                    apply_bare_patch(&mut world, ecs_entity, type_path, &registry_guard);
                }
                BsnPatch::Data { type_path, value } => {
                    apply_data_patch(&mut world, ecs_entity, type_path, value, &registry_guard);
                }
            }
        }
    }
    drop(registry_guard);

    let registry_guard = type_registry.read();
    Ok(DynamicWorldBuilder::from_world(&world, &registry_guard)
        .extract_entities(spawned.into_iter())
        .build())
}

fn flatten_entity(entity: &BsnEntity, parent: Option<usize>, out: &mut Vec<FlatBsnEntity>) {
    let index = out.len();
    out.push(FlatBsnEntity {
        parent,
        patches: entity.patches.clone(),
    });
    for child in &entity.children {
        flatten_entity(child, Some(index), out);
    }
}

fn apply_bare_patch(world: &mut World, entity: Entity, type_path: &str, registry: &TypeRegistry) {
    if let Some(registration) = resolve_registration(registry, type_path) {
        let Some(reflect_default) = registration.data::<ReflectDefault>() else {
            return;
        };
        let Some(reflect_component) = registration.data::<ReflectComponent>() else {
            return;
        };
        let value = reflect_default.default();
        reflect_component.insert(
            &mut world.entity_mut(entity),
            value.as_partial_reflect(),
            registry,
        );
        return;
    }

    let Some((enum_type_path, variant)) = type_path.rsplit_once("::") else {
        return;
    };
    let Some(registration) = resolve_registration(registry, enum_type_path) else {
        return;
    };
    let Some(reflect_default) = registration.data::<ReflectDefault>() else {
        return;
    };
    let Some(reflect_component) = registration.data::<ReflectComponent>() else {
        return;
    };

    let mut value = reflect_default.default();
    if let ReflectMut::Enum(enumeration) = value.reflect_mut() {
        enumeration.apply(&DynamicEnum::new(variant, DynamicVariant::Unit));
    }
    reflect_component.insert(
        &mut world.entity_mut(entity),
        value.as_partial_reflect(),
        registry,
    );
}

fn apply_data_patch(
    world: &mut World,
    entity: Entity,
    type_path: &str,
    value: &serde_json::Value,
    registry: &TypeRegistry,
) {
    let Some(registration) = resolve_registration(registry, type_path) else {
        warn!("Unknown BSN component type '{type_path}', skipping");
        return;
    };
    if registration.data::<ReflectComponent>().is_none() {
        return;
    }

    let deserializer = TypedReflectDeserializer::new(registration, registry);
    match deserializer.deserialize(value) {
        Ok(reflected) => {
            world.entity_mut(entity).insert_reflect(reflected);
        }
        Err(error) => {
            warn!("Failed to deserialize BSN component '{type_path}': {error}");
        }
    }
}

fn resolve_registration<'a>(
    registry: &'a TypeRegistry,
    type_path: &str,
) -> Option<&'a TypeRegistration> {
    if let Some(registration) = registry.get_with_type_path(type_path) {
        return Some(registration);
    }

    let mut candidate = None;
    for registration in registry.iter() {
        let registered_path = registration.type_info().type_path();
        let matches = registered_path == type_path
            || registered_path.ends_with(type_path)
                && registered_path
                    .as_bytes()
                    .get(registered_path.len().saturating_sub(type_path.len() + 2))
                    == Some(&b':')
            || registered_path.rsplit("::").next() == Some(type_path);
        if matches {
            if candidate.is_some() {
                return None;
            }
            candidate = Some(registration);
        }
    }
    candidate
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Ident(String),
    String(String),
    Number(String),
    Symbol(char),
    ColonColon,
    Eof,
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(text: &str) -> Self {
        Self {
            tokens: tokenize(text),
            pos: 0,
        }
    }

    fn parse_scene(mut self) -> Result<BsnScene, BsnLoadError> {
        let mut roots = Vec::new();
        while !self.at_eof() {
            if self.consume_symbol(',') {
                continue;
            }
            let entity = self.parse_entity()?;
            if !entity.is_empty() {
                roots.push(entity);
            }
            if self.consume_symbol(',') {
                continue;
            }
            if !self.at_eof() {
                return Err(self.error("expected comma or end of file"));
            }
        }
        Ok(BsnScene { roots })
    }

    fn parse_entity(&mut self) -> Result<BsnEntity, BsnLoadError> {
        if self.consume_symbol('(') {
            let entity = self.parse_entity()?;
            self.expect_symbol(')')?;
            return Ok(entity);
        }

        let mut entity = BsnEntity::default();
        while !(self.at_eof()
            || self.check_symbol(',')
            || self.check_symbol(']')
            || self.check_symbol(')'))
        {
            if self.consume_symbol('#') {
                let name = match self.next() {
                    Token::Ident(name) | Token::String(name) => name,
                    other => {
                        return Err(
                            self.error(format!("expected entity name after '#', found {other:?}"))
                        );
                    }
                };
                entity.patches.push(BsnPatch::Name(name));
                continue;
            }

            if self.consume_symbol(':') {
                self.skip_inheritance_target()?;
                continue;
            }

            let _template = self.consume_symbol('@');
            let type_path = self.parse_path()?;

            if self.consume_symbol('[') {
                if !is_children_path(&type_path) {
                    return Err(self.error(format!(
                        "only Children relations are supported by the BSN loader, found '{type_path}'"
                    )));
                }
                entity.children.extend(self.parse_children()?);
            } else if self.consume_symbol('{') {
                let fields = self.parse_fields()?;
                entity.patches.push(BsnPatch::Data {
                    type_path,
                    value: serde_json::Value::Object(fields),
                });
            } else if self.consume_symbol('(') {
                let values = self.parse_values_until(')')?;
                entity.patches.push(BsnPatch::Data {
                    type_path,
                    value: serde_json::Value::Array(values),
                });
            } else {
                entity.patches.push(BsnPatch::Bare(type_path));
            }
        }
        Ok(entity)
    }

    fn parse_children(&mut self) -> Result<Vec<BsnEntity>, BsnLoadError> {
        let mut children = Vec::new();
        while !self.consume_symbol(']') {
            if self.at_eof() {
                return Err(self.error("unterminated Children list"));
            }
            if self.consume_symbol(',') {
                continue;
            }
            let child = self.parse_entity()?;
            if !child.is_empty() {
                children.push(child);
            }
            let _ = self.consume_symbol(',');
        }
        Ok(children)
    }

    fn parse_fields(&mut self) -> Result<serde_json::Map<String, serde_json::Value>, BsnLoadError> {
        let mut fields = serde_json::Map::new();
        while !self.consume_symbol('}') {
            if self.at_eof() {
                return Err(self.error("unterminated struct field list"));
            }
            if self.consume_symbol(',') {
                continue;
            }
            let name = match self.next() {
                Token::Ident(name) => name,
                other => {
                    return Err(
                        self.error(format!("expected field name in struct, found {other:?}"))
                    );
                }
            };
            self.expect_symbol(':')?;
            let value = self.parse_value()?;
            fields.insert(name, value);
            let _ = self.consume_symbol(',');
        }
        Ok(fields)
    }

    fn parse_values_until(
        &mut self,
        terminator: char,
    ) -> Result<Vec<serde_json::Value>, BsnLoadError> {
        let mut values = Vec::new();
        while !self.consume_symbol(terminator) {
            if self.at_eof() {
                return Err(self.error(format!("unterminated value list, expected '{terminator}'")));
            }
            if self.consume_symbol(',') {
                continue;
            }
            values.push(self.parse_value()?);
            let _ = self.consume_symbol(',');
        }
        Ok(values)
    }

    fn parse_value(&mut self) -> Result<serde_json::Value, BsnLoadError> {
        match self.next() {
            Token::String(value) => Ok(serde_json::Value::String(value)),
            Token::Number(raw) => parse_json_number(&raw)
                .ok_or_else(|| self.error(format!("invalid number literal '{raw}'"))),
            Token::Ident(value) if value == "true" => Ok(serde_json::Value::Bool(true)),
            Token::Ident(value) if value == "false" => Ok(serde_json::Value::Bool(false)),
            Token::Ident(first) => {
                let path = self.parse_path_tail(first);
                if self.consume_symbol('{') {
                    Ok(serde_json::Value::Object(self.parse_fields()?))
                } else if self.consume_symbol('(') {
                    Ok(serde_json::Value::Array(self.parse_values_until(')')?))
                } else {
                    Ok(serde_json::Value::String(
                        path.rsplit("::").next().unwrap_or(&path).to_string(),
                    ))
                }
            }
            Token::Symbol('[') => Ok(serde_json::Value::Array(self.parse_values_until(']')?)),
            other => Err(self.error(format!("expected BSN value, found {other:?}"))),
        }
    }

    fn parse_path(&mut self) -> Result<String, BsnLoadError> {
        match self.next() {
            Token::Ident(first) => Ok(self.parse_path_tail(first)),
            other => Err(self.error(format!("expected type path, found {other:?}"))),
        }
    }

    fn parse_path_tail(&mut self, first: String) -> String {
        let mut path = first;
        while matches!(self.peek(), Token::ColonColon) {
            self.pos += 1;
            if let Token::Ident(segment) = self.next() {
                path.push_str("::");
                path.push_str(&segment);
            } else {
                break;
            }
        }
        path
    }

    fn skip_inheritance_target(&mut self) -> Result<(), BsnLoadError> {
        match self.next() {
            Token::String(_) => Ok(()),
            Token::Ident(first) => {
                let _ = self.parse_path_tail(first);
                Ok(())
            }
            other => Err(self.error(format!("expected inherited BSN target, found {other:?}"))),
        }
    }

    fn expect_symbol(&mut self, symbol: char) -> Result<(), BsnLoadError> {
        if self.consume_symbol(symbol) {
            Ok(())
        } else {
            Err(self.error(format!("expected '{symbol}'")))
        }
    }

    fn consume_symbol(&mut self, symbol: char) -> bool {
        if self.check_symbol(symbol) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn check_symbol(&self, symbol: char) -> bool {
        matches!(self.peek(), Token::Symbol(found) if *found == symbol)
    }

    fn at_eof(&self) -> bool {
        matches!(self.peek(), Token::Eof)
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn next(&mut self) -> Token {
        let token = self.peek().clone();
        if !matches!(token, Token::Eof) {
            self.pos += 1;
        }
        token
    }

    fn error(&self, message: impl Into<String>) -> BsnLoadError {
        BsnLoadError::Parse(format!("{} near token {}", message.into(), self.pos))
    }
}

fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            c if c.is_whitespace() => {}
            '/' if chars.peek() == Some(&'/') => {
                chars.next();
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut previous = '\0';
                for c in chars.by_ref() {
                    if previous == '*' && c == '/' {
                        break;
                    }
                    previous = c;
                }
            }
            ':' if chars.peek() == Some(&':') => {
                chars.next();
                tokens.push(Token::ColonColon);
            }
            '"' => tokens.push(Token::String(read_string(&mut chars))),
            '-' | '0'..='9' => tokens.push(Token::Number(read_number(ch, &mut chars))),
            c if is_ident_start(c) => tokens.push(Token::Ident(read_ident(c, &mut chars))),
            c @ ('{' | '}' | '(' | ')' | '[' | ']' | ',' | ':' | '#' | '@') => {
                tokens.push(Token::Symbol(c));
            }
            _ => {}
        }
    }

    tokens.push(Token::Eof);
    tokens
}

fn read_string(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut value = String::new();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => break,
            '\\' => {
                let Some(escaped) = chars.next() else {
                    break;
                };
                match escaped {
                    '"' => value.push('"'),
                    '\\' => value.push('\\'),
                    'n' => value.push('\n'),
                    'r' => value.push('\r'),
                    't' => value.push('\t'),
                    other => value.push(other),
                }
            }
            other => value.push(other),
        }
    }
    value
}

fn read_number(first: char, chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut value = String::from(first);
    while let Some(&ch) = chars.peek() {
        if ch.is_ascii_digit() || matches!(ch, '.' | 'e' | 'E' | '-' | '+') {
            value.push(ch);
            chars.next();
        } else {
            break;
        }
    }
    value
}

fn read_ident(first: char, chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut value = String::from(first);
    while let Some(&ch) = chars.peek() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            value.push(ch);
            chars.next();
        } else {
            break;
        }
    }
    value
}

fn parse_json_number(raw: &str) -> Option<serde_json::Value> {
    if raw.contains(['.', 'e', 'E']) {
        let value = raw.parse::<f64>().ok()?;
        serde_json::Number::from_f64(value).map(serde_json::Value::Number)
    } else if let Ok(value) = raw.parse::<i64>() {
        Some(serde_json::Value::Number(value.into()))
    } else {
        raw.parse::<u64>()
            .ok()
            .map(|value| serde_json::Value::Number(value.into()))
    }
}

fn is_ident_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn is_children_path(type_path: &str) -> bool {
    type_path == "Children" || type_path.ends_with("::Children")
}
