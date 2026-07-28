//! Synthetic cross-agent tool-call adapters that reshape a canonical
//! {tool, args} event into an approximate native shape and back.
//! Used by: tests/cross_agent_portability.rs.

pub mod codex_app_server;
pub mod copilot;
pub mod cursor;
pub mod generic;

use serde_json::{Map, Value};

use crate::aerf::AerfResult;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalEvent {
    pub tool: String,
    pub args: Value,
}

impl CanonicalEvent {
    pub fn new(tool: &str, args: Value) -> Self {
        Self {
            tool: tool.to_owned(),
            args,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fidelity {
    Documented,
    BestEffortUnverified,
}

impl Fidelity {
    pub fn label(self) -> &'static str {
        match self {
            Self::Documented => "documented public format",
            Self::BestEffortUnverified => "best-effort, unverified against live docs",
        }
    }
}

pub trait AgentAdapter {
    fn name(&self) -> &'static str;
    fn fidelity(&self) -> Fidelity;
    fn notes(&self) -> &'static str;
    fn to_native(&self, event: &CanonicalEvent) -> AerfResult<Value>;
    #[allow(clippy::wrong_self_convention)]
    fn from_native(&self, native: &Value) -> AerfResult<CanonicalEvent>;

    fn round_trip(&self, event: &CanonicalEvent) -> AerfResult<CanonicalEvent> {
        let native = self.to_native(event)?;
        self.from_native(&native)
    }
}

pub(crate) struct ToolSchema {
    pub canonical: &'static str,
    pub native: &'static str,
    pub fields: &'static [(&'static str, &'static str)],
    pub injected: &'static [(&'static str, &'static str)],
}

pub(crate) fn by_canonical<'a>(
    schemas: &'a [ToolSchema],
    canonical: &str,
) -> Option<&'a ToolSchema> {
    schemas.iter().find(|schema| schema.canonical == canonical)
}

pub(crate) fn by_native<'a>(schemas: &'a [ToolSchema], native: &str) -> Option<&'a ToolSchema> {
    schemas.iter().find(|schema| schema.native == native)
}

pub(crate) fn native_tool(schema: Option<&ToolSchema>, canonical: &str) -> String {
    schema.map(|schema| schema.native.to_owned()).unwrap_or_else(|| canonical.to_owned())
}

pub(crate) fn canonical_tool(schema: Option<&ToolSchema>, native: &str) -> String {
    schema.map(|schema| schema.canonical.to_owned()).unwrap_or_else(|| native.to_owned())
}

pub(crate) fn to_native_args(schema: Option<&ToolSchema>, args: &Value) -> Value {
    let Some(object) = args.as_object() else {
        return args.clone();
    };
    let mut out = Map::new();
    for (key, value) in object {
        out.insert(rename_to_native(schema, key), value.clone());
    }
    if let Some(schema) = schema {
        for (name, injected) in schema.injected {
            out.insert((*name).to_owned(), Value::String((*injected).to_owned()));
        }
    }
    Value::Object(out)
}

pub(crate) fn from_native_args(schema: Option<&ToolSchema>, args: &Value) -> Value {
    let Some(object) = args.as_object() else {
        return args.clone();
    };
    let mut out = Map::new();
    for (key, value) in object {
        if is_injected(schema, key) {
            continue;
        }
        out.insert(rename_to_canonical(schema, key), value.clone());
    }
    Value::Object(out)
}

fn rename_to_native(schema: Option<&ToolSchema>, canonical_field: &str) -> String {
    schema
        .and_then(|schema| {
            schema
                .fields
                .iter()
                .find(|(canonical, _)| *canonical == canonical_field)
                .map(|(_, native)| (*native).to_owned())
        })
        .unwrap_or_else(|| canonical_field.to_owned())
}

fn rename_to_canonical(schema: Option<&ToolSchema>, native_field: &str) -> String {
    schema
        .and_then(|schema| {
            schema
                .fields
                .iter()
                .find(|(_, native)| *native == native_field)
                .map(|(canonical, _)| (*canonical).to_owned())
        })
        .unwrap_or_else(|| native_field.to_owned())
}

fn is_injected(schema: Option<&ToolSchema>, native_field: &str) -> bool {
    schema.is_some_and(|schema| {
        schema.injected.iter().any(|(name, _)| *name == native_field)
    })
}
