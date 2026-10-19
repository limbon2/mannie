//! The tool registry (docs/architecture/extension-points.md, tool registry).

use std::sync::Arc;

use crate::activity::ACTIVITY_ARGUMENT;
use crate::tool::Tool;
use crate::tools;

/// Why a tool cannot join the registry.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegisterError {
    /// Another tool already has the name.
    #[error("a tool named {name} is already registered")]
    Duplicate {
        /// The name.
        name: String,
    },
    /// The tool has an argument named like the one that every call carries
    /// for the owner ([`ACTIVITY_ARGUMENT`]), which the runtime takes out of
    /// the arguments before a tool reads them.
    #[error(
        "the tool {name} has an argument named activity, which every call carries for the owner"
    )]
    ReservedArgument {
        /// The tool's name.
        name: String,
    },
}

/// The registered tools, in the order of registration, which is the order in
/// which the model sees them. A feature that adds a tool registers it here,
/// in the composition root.
#[derive(Debug, Clone, Default)]
pub struct ToolRegistry {
    tools: Vec<Arc<dyn Tool>>,
}

impl ToolRegistry {
    /// A registry without tools.
    pub fn new() -> Self {
        Self::default()
    }

    /// A registry with the system tools of this platform. `run_command` is
    /// registered only where the process supervisor contains commands:
    /// Windows and Linux (docs/features/system-tools/commands.md).
    pub fn with_system_tools() -> Self {
        Self {
            tools: tools::system_tools(),
        }
    }

    /// Registers a tool. Names are unique, and no tool may take the argument
    /// that every call carries for the owner.
    pub fn register(&mut self, tool: Arc<dyn Tool>) -> Result<(), RegisterError> {
        let name = tool.name();
        if self.get(name).is_some() {
            return Err(RegisterError::Duplicate {
                name: name.to_owned(),
            });
        }
        if takes_activity(tool.parameters()) {
            return Err(RegisterError::ReservedArgument {
                name: name.to_owned(),
            });
        }
        self.tools.push(tool);
        Ok(())
    }

    /// The tool with a name.
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools
            .iter()
            .find(|tool| tool.name() == name)
            .map(Arc::clone)
    }

    /// Every tool, in the order of registration.
    pub fn tools(&self) -> &[Arc<dyn Tool>] {
        &self.tools
    }
}

/// Whether a tool's schema of its arguments has an argument named
/// [`ACTIVITY_ARGUMENT`]. The registry refuses such a tool, and so does the
/// runtime for a plugin's tool, which never joins the registry.
pub fn takes_activity(parameters: &serde_json::Value) -> bool {
    parameters
        .get("properties")
        .and_then(serde_json::Value::as_object)
        .is_some_and(|properties| properties.contains_key(ACTIVITY_ARGUMENT))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::Arc;

    use futures_util::FutureExt;
    use futures_util::future::BoxFuture;
    use mannie_types::tools::ToolFailure;
    use serde_json::{Value, json};

    use super::{RegisterError, ToolRegistry, takes_activity};
    use crate::tool::{
        Done, Execution, PermissionClass, PrepareError, Prepared, Registered, Tool, ToolContext,
    };
    use crate::tools::system_info::SystemInfo;

    /// A tool that owns its name and description, as a plugin's tool does,
    /// and refuses every call.
    #[derive(Debug)]
    struct LabelThings {
        name: String,
        description: String,
        parameters: Value,
    }

    impl LabelThings {
        /// The tool under a name, with a schema of its arguments.
        fn new(name: &str, parameters: Value) -> Self {
            Self {
                name: name.to_owned(),
                description: format!("Labels things as {name} does."),
                parameters,
            }
        }
    }

    impl Tool for LabelThings {
        fn name(&self) -> &str {
            &self.name
        }

        fn description(&self) -> &str {
            &self.description
        }

        fn parameters(&self) -> &Value {
            &self.parameters
        }

        fn class(&self) -> PermissionClass {
            PermissionClass::ReadOnly
        }

        fn prepare(
            &self,
            _context: ToolContext,
            _arguments: String,
        ) -> BoxFuture<'static, Result<Prepared, PrepareError>> {
            async { Err(PrepareError::invalid("the registry refuses this tool")) }.boxed()
        }

        fn execute(&self, _execution: Execution) -> BoxFuture<'static, Done> {
            async {
                Done::failed(ToolFailure::IoError {
                    kind: "the registry refuses this tool".to_owned(),
                })
            }
            .boxed()
        }
    }

    #[test]
    fn the_system_tools_have_distinct_names() {
        let registry = ToolRegistry::with_system_tools();

        let names: HashSet<_> = registry.tools().iter().map(|tool| tool.name()).collect();
        assert_eq!(names.len(), registry.tools().len());
        assert!(registry.get("read_file").is_some());
        assert!(registry.get("format_disk").is_none());
    }

    #[test]
    fn a_second_tool_with_the_same_name_is_refused() {
        let mut registry = ToolRegistry::with_system_tools();

        let duplicate = registry.register(Arc::new(Registered::<SystemInfo>::new()));

        assert_eq!(
            duplicate,
            Err(RegisterError::Duplicate {
                name: "system_info".to_owned()
            })
        );
    }

    #[test]
    fn a_tool_that_owns_its_name_is_found_by_it_and_a_second_with_the_name_is_refused() {
        let mut registry = ToolRegistry::new();
        let parameters = json!({ "type": "object", "properties": {} });

        registry
            .register(Arc::new(LabelThings::new(
                "label_things",
                parameters.clone(),
            )))
            .unwrap();
        let duplicate = registry.register(Arc::new(LabelThings::new("label_things", parameters)));

        let found = registry.get("label_things").unwrap();
        assert_eq!(found.name(), "label_things");
        assert_eq!(found.description(), "Labels things as label_things does.");
        assert_eq!(
            duplicate,
            Err(RegisterError::Duplicate {
                name: "label_things".to_owned()
            })
        );
        assert_eq!(registry.tools().len(), 1);
    }

    #[test]
    fn a_tool_with_an_argument_named_activity_is_refused() {
        let mut registry = ToolRegistry::new();
        let tool = LabelThings::new(
            "label_things",
            json!({
                "type": "object",
                "properties": { "activity": { "type": "string" } },
                "required": ["activity"],
                "additionalProperties": false,
            }),
        );

        let refused = registry.register(Arc::new(tool));

        assert_eq!(
            refused,
            Err(RegisterError::ReservedArgument {
                name: "label_things".to_owned()
            })
        );
        assert!(registry.tools().is_empty());
    }

    #[test]
    fn only_a_schema_whose_properties_name_the_activity_takes_it() {
        assert!(takes_activity(
            &json!({ "type": "object", "properties": { "activity": {} } })
        ));
        assert!(!takes_activity(
            &json!({ "type": "object", "properties": { "text": {} } })
        ));
        // The name elsewhere in the schema is no argument.
        assert!(!takes_activity(
            &json!({ "type": "object", "required": ["activity"] })
        ));
        assert!(!takes_activity(&json!({ "type": "object" })));
        assert!(!takes_activity(&json!({ "properties": ["activity"] })));
        assert!(!takes_activity(&json!("activity")));
    }

    #[test]
    fn no_system_tool_takes_the_activity() {
        let registry = ToolRegistry::with_system_tools();

        let taking: Vec<_> = registry
            .tools()
            .iter()
            .filter(|tool| takes_activity(tool.parameters()))
            .map(|tool| tool.name())
            .collect();

        assert_eq!(taking, Vec::<&str>::new());
    }

    #[test]
    fn run_command_exists_where_commands_are_contained() {
        let registry = ToolRegistry::with_system_tools();

        assert_eq!(
            registry.get("run_command").is_some(),
            cfg!(any(windows, target_os = "linux", target_os = "freebsd"))
        );
    }
}
