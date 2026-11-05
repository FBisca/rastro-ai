//! The catalog: one entry per operation.
use crate::{requests::*, results::*};
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
    /// CLI opens a Reader for this operation, without projection or recovery.
    pub read_path: bool,
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
        read_path: T::CLI_READ_PATH,
    }
}

// Normalize the command tree into variants and operation entries, then generate all dispatch.
// Grouping is clap structure; the numeric slot preserves the published schema's order.
macro_rules! operations {
    (@collect [$($variants:tt)*] [$($groups:tt)*] [$($entries:tt)*]) => {
        #[derive(clap::Subcommand, Debug)]
        pub enum Request { $($variants)* }
        $($groups)*
        operations!(@generate $($entries)*);
    };
    (@collect [$($variants:tt)*] [$($groups:tt)*] [$($entries:tt)*]
        $(#[$meta:meta])* $group:ident / $enum:ident {
            $( $variant:ident($input:ident => $output:ident) as $response:ident {
                $($details:tt)*
            } ),* $(,)?
        }
        $($rest:tt)*
    ) => {
        operations!(@collect
            [$($variants)* $(#[$meta])* $group { #[command(subcommand)] command: $enum },]
            [$($groups)* #[derive(clap::Subcommand, Debug)] pub enum $enum {
                $( $variant($input), )*
            }]
            [$($entries)* $(
                ($input => $output, $response, value,
                 [Request::$group { command: $enum::$variant(value) }],
                 [Request::$group { command: $enum::$variant(value) }],
                 { $($details)* })
            )*]
            $($rest)*
        );
    };
    (@collect [$($variants:tt)*] [$($groups:tt)*] [$($entries:tt)*]
        $variant:ident($input:ident => $output:ident) {
            $($details:tt)*
        }
        $($rest:tt)*
    ) => {
        operations!(@collect
            [$($variants)* $variant($input),]
            [$($groups)*]
            [$($entries)* ($input => $output, $variant, value,
                [Request::$variant(value)], [Request::$variant(value)],
                { $($details)* })]
            $($rest)*
        );
    };
    (@generate $(
        ($input:ident => $output:ident, $response:ident, $value:ident,
         [$pattern:pat], [$constructor:expr], {
            slot: $slot:literal, name: $name:literal,
            surfaces: [$($surface:ident),*], read_only: $read_only:literal
            $(, reader: $reader:ident)?
            $(, cli_read_path: $cli_read_path:literal)?
         })
    )*) => {
        $(impl Operation for $input {
            const NAME: &'static str = $name;
            const SURFACES: &'static [Surface] = &[$(Surface::$surface),*];
            const READ_ONLY: bool = $read_only;
            $(const CLI_READ_PATH: bool = $cli_read_path;)?
            type Output = $output;
            fn into_request(self) -> Request {
                let $value = self;
                $constructor
            }
            fn run<S: rastro_core::Storage>(self, rastro: &mut rastro_core::Rastro<S>) -> anyhow::Result<Self::Output> {
                $input::run(self, rastro)
            }
            $(fn run_read<S: rastro_core::ReadStorage>(self, reader: &mut rastro_core::Reader<S>) -> anyhow::Result<Self::Output> {
                $input::$reader(self, reader)
            })?
        })*

        impl Request {
            pub fn name(&self) -> &'static str {
                match self { $( $pattern => { let _ = $value; $input::NAME }, )* }
            }
            /// Runs the operation against an open store.
            pub fn run<S: rastro_core::Storage>(self, rastro: &mut rastro_core::Rastro<S>) -> anyhow::Result<Response> {
                match self { $( $pattern => Operation::run($value, rastro).map(Response::$response), )* }
            }
            /// Runs on a read-only view; unsupported operations are rejected without writing.
            pub fn run_read<S: rastro_core::ReadStorage>(self, reader: &mut rastro_core::Reader<S>) -> anyhow::Result<Response> {
                match self { $( $pattern => Operation::run_read($value, reader).map(Response::$response), )* }
            }
        }

        /// What an operation returned.
        #[derive(Debug)]
        pub enum Response { $( $response($output), )* }
        impl Response {
            pub fn operation(&self) -> &'static str {
                match self { $( Self::$response(_) => $input::NAME, )* }
            }
            /// The `result` member of the envelope.
            pub fn result(&self) -> Value {
                match self { $( Self::$response(value) => serde_json::to_value(value), )* }
                    .expect("results serialise")
            }
        }

        /// Every operation, in the order the schema lists them.
        pub static CATALOG: [OperationSpec; operations!(@count $($input)*)] = {
            let mut entries = [$(spec::<$input>()),*];
            let mut occupied = [false; operations!(@count $($input)*)];
            $(
                assert!(!occupied[$slot], "duplicate operation schema slot");
                occupied[$slot] = true;
                entries[$slot] = spec::<$input>();
            )*
            entries
        };
    };
    (@count $($input:ident)*) => { <[()]>::len(&[$(operations!(@unit $input)),*]) };
    (@unit $input:ident) => { () };
    ($($tree:tt)*) => { operations!(@collect [] [] [] $($tree)*); };
}

// Add one entry here, its request/result types, and typed execution methods. `reader` is
// explicit: SourceList is read-only too, but is not supported by a read-only Reader.
operations! {
    /// Register and list sources.
    Source / SourceRequest {
        Add(SourceAddRequest => SourceAddResult) as SourceAdd {
            slot: 0, name: "source.add", surfaces: [Cli], read_only: false
        },
        List(SourceListRequest => SourceListResult) as SourceList {
            slot: 1, name: "source.list", surfaces: [Cli], read_only: true
        },
    }
    Coverage(CoverageRequest => CoverageResult) {
        slot: 2, name: "coverage", surfaces: [Cli, Mcp], read_only: true, reader: run_read
    }
    Sync(SyncRequest => SyncResult) {
        slot: 3, name: "sync", surfaces: [Cli], read_only: false
    }
    /// Maintain the search index.
    Index / IndexRequest {
        Rebuild(IndexRebuildRequest => IndexRebuildResult) as IndexRebuild {
            slot: 5, name: "index.rebuild", surfaces: [Cli], read_only: false
        },
    }
    Search(SearchRequest => SearchResponse) {
        slot: 4, name: "search", surfaces: [Cli, Mcp], read_only: true, reader: run_read
    }
    /// Maintain the claims projection.
    Claims / ClaimsRequest {
        Rebuild(ClaimsRebuildRequest => ClaimsRebuildResult) as ClaimsRebuild {
            slot: 7, name: "claims.rebuild", surfaces: [Cli], read_only: false
        },
    }
    About(AboutRequest => AboutResult) {
        slot: 8, name: "about", surfaces: [Cli, Mcp], read_only: true, reader: run_read, cli_read_path: true
    }
    Why(WhyRequest => WhyResult) {
        slot: 6, name: "why", surfaces: [Cli, Mcp], read_only: true, reader: run_read, cli_read_path: true
    }
}

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

    /// surfaces/R2: the CLI tree and the catalog name the same operations
    #[test]
    #[allow(non_snake_case)]
    fn the_clap_tree_names_exactly_the_cataloged_operations() {
        let mut from_clap = command_names(&Request::augment_subcommands(Command::new("rastro")));
        from_clap.sort();
        let mut from_catalog: Vec<_> = CATALOG.iter().map(|s| s.name.to_string()).collect();
        from_catalog.sort();
        assert_eq!(from_clap, from_catalog);
    }
}
