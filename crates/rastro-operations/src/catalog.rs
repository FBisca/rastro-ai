//! The catalog: one entry per operation.
use crate::requests::*;
use clap::{Args, Command};
use schemars::{JsonSchema, Schema, generate::SchemaSettings, transform::RecursiveTransform};
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Where an operation is offered.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Surface {
    Cli,
    Mcp,
}
impl Surface {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Mcp => "mcp",
        }
    }
}

/// One entry of the catalog. Descriptions and defaults come from the request type's doc comments
/// and serde defaults, so the schema and the CLI help cannot say different things.
pub struct OperationSpec {
    pub name: &'static str,
    pub surfaces: &'static [Surface],
    pub(crate) input: fn() -> Value,
    pub(crate) output: fn() -> Value,
    pub(crate) command: fn() -> Command,
    decode: fn(Value) -> serde_json::Result<Request>,
    /// Whether the operation only reads.
    pub read_only: bool,
}

/// One input of an operation.
#[derive(Clone, Debug)]
pub struct Input {
    pub name: String,
    pub description: String,
    pub default: Option<Value>,
    pub positional: bool,
}

/// Objects in a result are closed: a key the type doesn't emit is not valid. Keys that may be
/// left out are not nullable, because the types never print `null` for them.
fn close_objects(schema: &mut Schema) {
    let Some(object) = schema.as_object_mut() else {
        return;
    };
    if !object.contains_key("properties") {
        return;
    }
    object
        .entry("additionalProperties")
        .or_insert(Value::Bool(false));
    let required: Vec<String> = object
        .get("required")
        .and_then(Value::as_array)
        .map(|r| {
            r.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    if let Some(Value::Object(properties)) = object.get_mut("properties") {
        for (name, property) in properties {
            if required.contains(name) {
                continue;
            }
            if let Some(Value::Array(types)) = property.get_mut("type") {
                types.retain(|t| t != "null");
                if types.len() == 1 {
                    property["type"] = types[0].clone();
                }
            }
        }
    }
}
/// A request, as the surfaces accept it.
fn input_root<T: JsonSchema>() -> Value {
    schemars::schema_for!(T).to_value()
}
/// A result, as the code prints it.
fn output_root<T: JsonSchema>() -> Value {
    SchemaSettings::draft2020_12()
        .for_serialize()
        .with_transform(RecursiveTransform(close_objects))
        .into_generator()
        .into_root_schema_for::<T>()
        .to_value()
}
fn command<T: Args>(name: &'static str) -> Command {
    T::augment_args(Command::new(name))
}
const fn spec<T: Operation + Args + JsonSchema + DeserializeOwned>() -> OperationSpec {
    OperationSpec {
        name: T::NAME,
        surfaces: T::SURFACES,
        input: input_root::<T>,
        output: output_root::<T::Output>,
        command: || command::<T>(T::NAME),
        decode: |arguments| serde_json::from_value::<T>(arguments).map(T::into_request),
        read_only: T::READ_ONLY,
    }
}

/// Every operation, in the order the schema lists them.
pub static CATALOG: [OperationSpec; 6] = [
    spec::<SourceAddRequest>(),
    spec::<SourceListRequest>(),
    spec::<CoverageRequest>(),
    spec::<SyncRequest>(),
    spec::<SearchRequest>(),
    spec::<IndexRebuildRequest>(),
];

pub fn find(name: &str) -> Option<&'static OperationSpec> {
    CATALOG.iter().find(|spec| spec.name == name)
}
/// The operation a command-line path such as `["source", "add"]` names; extra words are ignored.
pub fn find_path(words: &[&str]) -> Option<&'static OperationSpec> {
    CATALOG.iter().find(|spec| {
        let parts: Vec<_> = spec.name.split('.').collect();
        words.len() >= parts.len() && words[..parts.len()] == parts[..]
    })
}

impl OperationSpec {
    pub fn description(&self) -> String {
        let schema = (self.input)();
        schema["description"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_string()
    }
    /// The JSON Schema of the request, with its own `$defs`.
    pub fn input_schema(&self) -> Value {
        (self.input)()
    }
    pub fn output_schema(&self) -> Value {
        (self.output)()
    }
    /// The request these JSON arguments make; unknown or missing arguments are an error.
    pub fn decode(&self, arguments: Value) -> serde_json::Result<Request> {
        (self.decode)(arguments)
    }
    pub fn exposed_on(&self, surface: Surface) -> bool {
        self.surfaces.contains(&surface)
    }
    pub fn inputs(&self) -> Vec<Input> {
        let schema = (self.input)();
        let command = (self.command)();
        let Some(properties) = schema["properties"].as_object() else {
            return Vec::new();
        };
        properties
            .iter()
            .map(|(name, property)| Input {
                name: name.clone(),
                description: property["description"]
                    .as_str()
                    .unwrap_or_default()
                    .trim()
                    .into(),
                default: property.get("default").cloned(),
                positional: command
                    .get_arguments()
                    .any(|a| a.get_id() == name.as_str() && a.is_positional()),
            })
            .collect()
    }
}

/// Which operation each clap command is: the command tree must match the catalog's names.
pub fn command_names(command: &Command) -> Vec<String> {
    let mut names = Vec::new();
    for sub in command.get_subcommands() {
        let nested = command_names(sub);
        if nested.is_empty() {
            names.push(sub.get_name().to_string());
        } else {
            names.extend(
                nested
                    .into_iter()
                    .map(|n| format!("{}.{n}", sub.get_name())),
            );
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Subcommand;

    /// S2: the CLI tree and the catalog name the same operations
    #[test]
    #[allow(non_snake_case)]
    fn S2_the_clap_tree_names_exactly_the_cataloged_operations() {
        let mut from_clap = command_names(&Request::augment_subcommands(Command::new("rastro")));
        from_clap.sort();
        let mut from_catalog: Vec<_> = CATALOG.iter().map(|s| s.name.to_string()).collect();
        from_catalog.sort();
        assert_eq!(from_clap, from_catalog);
    }
}
