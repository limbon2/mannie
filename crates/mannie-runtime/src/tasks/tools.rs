//! The task tools (docs/features/child-agents/tasks.md, the task tools):
//! their definitions as a model receives them, and the checking of their
//! calls. The tools of plugins' executors are the executors' own (ADR-0031).
//!
//! The schemas are written here, strict like the tools' own: every property
//! is required and nothing else is allowed. A test checks them against the
//! types that parse the arguments, and `tests/tasks.rs` compares them with a
//! reviewed snapshot in `tests/fixtures/task_definitions.json`.

use mannie_provider::ToolDefinition;
use mannie_tools::{clean_label, with_activity};
use mannie_types::tasks::{MAX_INSTRUCTION_BYTES, MAX_TITLE_CHARS};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::texts;

/// The tool that starts a task.
pub(crate) const START_TASK: &str = "start_task";

/// The tool that gives a child a further task.
pub(crate) const FOLLOW_UP_TASK: &str = "follow_up_task";

/// The tool that waits for tasks and returns their reports.
pub(crate) const WAIT_FOR_TASKS: &str = "wait_for_tasks";

/// The most tasks that one wait names.
const MAX_AWAITED: usize = 64;

/// The arguments of `start_task`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StartArguments {
    title: String,
    agent: String,
    instruction: String,
    background: bool,
}

/// The arguments of `follow_up_task`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FollowUpArguments {
    task: u32,
    instruction: String,
    background: bool,
}

/// The arguments of `wait_for_tasks`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitArguments {
    tasks: Vec<u32>,
}

/// A checked call of a task tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TaskCall {
    /// A new child, with its first task.
    Start {
        /// The title, cleaned for display.
        title: String,
        /// The child's agent definition.
        agent: String,
        /// The instruction.
        instruction: String,
        /// Whether the parent continues while the child works.
        background: bool,
    },
    /// A further task for an existing child.
    FollowUp {
        /// The child's number.
        number: u32,
        /// The instruction.
        instruction: String,
        /// Whether the parent continues while the child works.
        background: bool,
    },
    /// A wait for the reports of tasks, by their children's numbers; empty
    /// for every task of the reply whose report is unread.
    Wait {
        /// The children's numbers, without repeats, in the order named.
        numbers: Vec<u32>,
    },
}

/// The definitions of the task tools, as a model receives them. `agents` are
/// the child agents that `start_task` can start.
pub(crate) fn definitions(agents: &[&str]) -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: START_TASK.to_owned(),
            description: texts::START_TASK.to_owned(),
            parameters: with_activity(&start_parameters(agents)),
        },
        ToolDefinition {
            name: FOLLOW_UP_TASK.to_owned(),
            description: texts::FOLLOW_UP_TASK.to_owned(),
            parameters: with_activity(&follow_up_parameters()),
        },
        ToolDefinition {
            name: WAIT_FOR_TASKS.to_owned(),
            description: texts::WAIT_FOR_TASKS.to_owned(),
            parameters: with_activity(&wait_parameters()),
        },
    ]
}

fn start_parameters(agents: &[&str]) -> Value {
    json!({
        "additionalProperties": false,
        "properties": {
            "agent": {
                "description": texts::TASK_AGENT_ARGUMENT,
                "enum": agents,
                "type": "string",
            },
            "background": {
                "description": texts::TASK_BACKGROUND_ARGUMENT,
                "type": "boolean",
            },
            "instruction": {
                "description": texts::TASK_INSTRUCTION_ARGUMENT,
                "type": "string",
            },
            "title": {
                "description": texts::TASK_TITLE_ARGUMENT,
                "type": "string",
            },
        },
        "required": ["agent", "background", "instruction", "title"],
        "type": "object",
    })
}

fn follow_up_parameters() -> Value {
    json!({
        "additionalProperties": false,
        "properties": {
            "background": {
                "description": texts::TASK_BACKGROUND_ARGUMENT,
                "type": "boolean",
            },
            "instruction": {
                "description": texts::FOLLOW_UP_INSTRUCTION_ARGUMENT,
                "type": "string",
            },
            "task": {
                "description": texts::TASK_NUMBER_ARGUMENT,
                "type": "integer",
            },
        },
        "required": ["background", "instruction", "task"],
        "type": "object",
    })
}

fn wait_parameters() -> Value {
    json!({
        "additionalProperties": false,
        "properties": {
            "tasks": {
                "description": texts::TASK_NUMBERS_ARGUMENT,
                "items": { "type": "integer" },
                "type": "array",
            },
        },
        "required": ["tasks"],
        "type": "object",
    })
}

/// Parses a call's arguments. A model that calls a tool without arguments
/// may send nothing at all, or `null`, which count as an empty object.
fn parse<I: DeserializeOwned>(arguments: &str) -> Result<I, String> {
    let trimmed = arguments.trim();
    let text = if trimmed.is_empty() || trimmed == "null" {
        "{}"
    } else {
        arguments
    };
    serde_json::from_str(text).map_err(|error| error.to_string())
}

/// Checks an instruction: not blank, and not longer than the limit.
pub(crate) fn check_instruction(instruction: &str) -> Result<(), String> {
    if instruction.trim().is_empty() {
        return Err("the instruction is empty".to_owned());
    }
    if instruction.len() > MAX_INSTRUCTION_BYTES {
        return Err(format!(
            "the instruction is longer than {MAX_INSTRUCTION_BYTES} bytes"
        ));
    }
    Ok(())
}

/// Checks a call of a task tool, without the activity, which the runtime took
/// out. `agents` are the child agents that `start_task` can start. The error
/// says what is wrong, for the model.
pub(crate) fn parse_call(tool: &str, arguments: &str, agents: &[&str]) -> Result<TaskCall, String> {
    match tool {
        START_TASK => {
            let input: StartArguments = parse(arguments)?;
            let title = clean_label(&input.title, MAX_TITLE_CHARS)
                .ok_or_else(|| "the title is empty".to_owned())?;
            if !agents.contains(&input.agent.as_str()) {
                return Err(format!(
                    "there is no agent {:?}; use one of {}",
                    input.agent,
                    agents.join(", ")
                ));
            }
            check_instruction(&input.instruction)?;
            Ok(TaskCall::Start {
                title,
                agent: input.agent,
                instruction: input.instruction,
                background: input.background,
            })
        }
        FOLLOW_UP_TASK => {
            let input: FollowUpArguments = parse(arguments)?;
            if input.task == 0 {
                return Err("tasks are numbered from 1".to_owned());
            }
            check_instruction(&input.instruction)?;
            Ok(TaskCall::FollowUp {
                number: input.task,
                instruction: input.instruction,
                background: input.background,
            })
        }
        WAIT_FOR_TASKS => {
            let input: WaitArguments = parse(arguments)?;
            if input.tasks.len() > MAX_AWAITED {
                return Err(format!("a wait names at most {MAX_AWAITED} tasks"));
            }
            if input.tasks.contains(&0) {
                return Err("tasks are numbered from 1".to_owned());
            }
            let mut numbers = Vec::with_capacity(input.tasks.len());
            for number in input.tasks {
                if !numbers.contains(&number) {
                    numbers.push(number);
                }
            }
            Ok(TaskCall::Wait { numbers })
        }
        other => Err(format!("{other} is not a task tool")),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{FOLLOW_UP_TASK, START_TASK, TaskCall, WAIT_FOR_TASKS, definitions, parse_call};

    const AGENTS: [&str; 2] = ["explore", "general"];

    /// A value of a property, as its schema allows.
    fn sample(schema: &Value) -> Value {
        if let Some(choices) = schema.get("enum").and_then(Value::as_array) {
            return choices[0].clone();
        }
        match schema.get("type").and_then(Value::as_str) {
            Some("string") => json!("Find where sessions expire"),
            Some("boolean") => json!(false),
            Some("integer") => json!(1),
            Some("array") => json!([1]),
            other => panic!("no sample for {other:?}"),
        }
    }

    #[test]
    fn every_schema_is_strict_and_parses_as_its_tool_reads_it() {
        for definition in definitions(&AGENTS) {
            let parameters = &definition.parameters;
            let properties = parameters["properties"].as_object().unwrap();
            let mut required: Vec<&str> = parameters["required"]
                .as_array()
                .unwrap()
                .iter()
                .map(|name| name.as_str().unwrap())
                .collect();
            let mut names: Vec<&str> = properties.keys().map(String::as_str).collect();
            required.sort_unstable();
            names.sort_unstable();
            assert_eq!(required, names, "{}", definition.name);
            assert_eq!(parameters["additionalProperties"], json!(false));
            let mut arguments = serde_json::Map::new();
            for (name, schema) in properties {
                if name != "activity" {
                    arguments.insert(name.clone(), sample(schema));
                }
            }
            let call = parse_call(
                &definition.name,
                &Value::Object(arguments).to_string(),
                &AGENTS,
            );
            assert!(call.is_ok(), "{}: {call:?}", definition.name);
        }
    }

    #[test]
    fn a_start_is_checked_and_its_title_cleaned() {
        let call = parse_call(
            START_TASK,
            r#"{"title":"  Find\nthe \u202esessions  ","agent":"explore","instruction":"Look.","background":true}"#,
            &AGENTS,
        );

        assert_eq!(
            call,
            Ok(TaskCall::Start {
                title: "Find the sessions".to_owned(),
                agent: "explore".to_owned(),
                instruction: "Look.".to_owned(),
                background: true,
            })
        );
        for refused in [
            r#"{"title":" ","agent":"explore","instruction":"Look.","background":false}"#,
            r#"{"title":"T","agent":"conversation","instruction":"Look.","background":false}"#,
            r#"{"title":"T","agent":"explore","instruction":"  ","background":false}"#,
            r#"{"title":"T","agent":"explore","instruction":"Look.","background":false,"extra":1}"#,
            r#"{"title":"T","agent":"explore","instruction":"Look."}"#,
        ] {
            assert!(
                parse_call(START_TASK, refused, &AGENTS).is_err(),
                "{refused}"
            );
        }
        let long = format!(
            r#"{{"title":"T","agent":"general","instruction":"{}","background":false}}"#,
            "x".repeat(64 * 1024 + 1)
        );
        parse_call(START_TASK, &long, &AGENTS).unwrap_err();
    }

    #[test]
    fn follow_ups_and_waits_name_tasks_from_one() {
        assert_eq!(
            parse_call(
                FOLLOW_UP_TASK,
                r#"{"task":2,"instruction":"Now the tests.","background":false}"#,
                &AGENTS
            ),
            Ok(TaskCall::FollowUp {
                number: 2,
                instruction: "Now the tests.".to_owned(),
                background: false,
            })
        );
        parse_call(
            FOLLOW_UP_TASK,
            r#"{"task":0,"instruction":"Now.","background":false}"#,
            &AGENTS,
        )
        .unwrap_err();
        assert_eq!(
            parse_call(WAIT_FOR_TASKS, r#"{"tasks":[3,1,3]}"#, &AGENTS),
            Ok(TaskCall::Wait {
                numbers: vec![3, 1]
            })
        );
        assert_eq!(
            parse_call(WAIT_FOR_TASKS, r#"{"tasks":[]}"#, &AGENTS),
            Ok(TaskCall::Wait {
                numbers: Vec::new()
            })
        );
        parse_call(WAIT_FOR_TASKS, r#"{"tasks":[0]}"#, &AGENTS).unwrap_err();
        parse_call("read_file", "{}", &AGENTS).unwrap_err();
    }
}
