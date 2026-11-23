//! Versioned, metadata-only JSON command catalog built from the same Clap tree as `opera mcp`.

use std::any::TypeId;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::sync::OnceLock;

use clap::{Arg, ArgAction, Command, CommandFactory};
use serde_json::{Value, json};

/// Stack size used for CLI parsing and command dispatch (prevents stack overflows on small
/// platform or test-thread stacks in debug builds).
const WORKER_STACK_SIZE: usize = 16 * 1024 * 1024;

/// Runs `f` on a scoped thread with a 16 MiB stack.
pub(crate) fn with_clap_stack<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(WORKER_STACK_SIZE)
            .spawn_scoped(scope, f)
            .expect("spawn CLI worker thread")
            .join()
            .expect("CLI worker thread panicked")
    })
}

fn root() -> Command {
    with_clap_stack(|| {
        let mut command = crate::Cli::command();
        command.build();
        command
    })
}

fn exposed(command: &Command) -> bool {
    !matches!(command.get_name(), "mcp" | "help")
}

/// All top-level commands served by both `--json-usage` and MCP `tools/list`.
pub(crate) fn commands() -> impl Iterator<Item = Command> {
    with_clap_stack(|| {
        root()
            .get_subcommands()
            .filter(|c| exposed(c))
            .cloned()
            .collect::<Vec<_>>()
    })
    .into_iter()
}

/// Cached list of exposed command names for fast validation in MCP `tools/call`.
pub(crate) fn command_names() -> &'static [String] {
    static NAMES: OnceLock<Vec<String>> = OnceLock::new();
    NAMES.get_or_init(|| commands().map(|c| c.get_name().to_string()).collect())
}

/// Detects `--json-usage` and returns the JSON schema for the root or requested subcommand.
pub(crate) fn requested(args: &[OsString]) -> Option<Result<Value, String>> {
    if !args.iter().any(|a| a == OsStr::new("--json-usage")) {
        return None;
    }
    let args = args.to_vec();
    with_clap_stack(move || requested_inner(&args))
}

fn requested_inner(args: &[OsString]) -> Option<Result<Value, String>> {
    let mut root = crate::Cli::command();
    let matches = match root.clone().try_get_matches_from(args) {
        Ok(matches) => matches,
        Err(error)
            if matches!(
                error.kind(),
                clap::error::ErrorKind::MissingRequiredArgument
                    | clap::error::ErrorKind::MissingSubcommand
            ) =>
        {
            root.clone()
                .ignore_errors(true)
                .try_get_matches_from(args)
                .ok()?
        }
        Err(_) => return None,
    };
    root.build();
    let mut requested = matches.get_flag("json_usage");
    let (mut command, mut matches) = (&root, &matches);
    let mut path = vec!["opera".to_string()];
    let mut unexposed_name: Option<String> = None;
    while let Some((name, child_matches)) = matches.subcommand() {
        command = command.find_subcommand(name)?;
        requested |= child_matches.get_flag("json_usage");
        if !exposed(command) {
            unexposed_name = Some(command.get_name().to_string());
        }
        path.push(command.get_name().to_string());
        matches = child_matches;
    }
    if !requested {
        return None;
    }
    if let Some(name) = unexposed_name {
        return Some(Err(format!(
            "`--json-usage` is not available for `{name}` (not exposed in the tool catalog)"
        )));
    }
    Some(Ok(json!({
        "schema_version": "0.1.0",
        "program": "opera",
        "version": env!("CARGO_PKG_VERSION"),
        "command": describe(command, &path),
    })))
}

fn describe(command: &Command, path: &[String]) -> Value {
    let mut children: Vec<_> = command
        .get_subcommands()
        .filter(|c| exposed(c))
        .cloned()
        .collect();
    children.sort_by(|a, b| a.get_name().cmp(b.get_name()));
    let children: Vec<_> = children
        .into_iter()
        .map(|child| {
            let mut child_path = path.to_vec();
            child_path.push(child.get_name().to_string());
            describe(&child, &child_path)
        })
        .collect();
    let arguments: Vec<_> = command
        .get_arguments()
        .filter(|arg| !arg.is_hide_set() && arg.get_id().as_str() != "help")
        .map(describe_arg)
        .collect();
    json!({
        "name": command.get_name(),
        "path": path,
        "description": command
            .get_long_about()
            .or_else(|| command.get_about())
            .map(ToString::to_string)
            .unwrap_or_default(),
        "arguments": arguments,
        "subcommands": children,
    })
}

fn value_type(arg: &Arg) -> &'static str {
    let id = arg.get_value_parser().type_id();
    if id == TypeId::of::<bool>()
        || matches!(
            arg.get_action(),
            ArgAction::SetTrue
                | ArgAction::SetFalse
                | ArgAction::Help
                | ArgAction::HelpShort
                | ArgAction::HelpLong
                | ArgAction::Version
        )
    {
        return "boolean";
    }
    if id == TypeId::of::<PathBuf>() {
        return "path";
    }
    if id == TypeId::of::<usize>()
        || id == TypeId::of::<u8>()
        || id == TypeId::of::<u16>()
        || id == TypeId::of::<u32>()
        || id == TypeId::of::<i64>()
    {
        return "integer";
    }
    if id == TypeId::of::<f64>() || id == TypeId::of::<f32>() {
        return "number";
    }
    "string"
}

fn describe_arg(arg: &Arg) -> Value {
    let defaults: Vec<String> = arg
        .get_default_values()
        .iter()
        .map(|v| v.to_string_lossy().into_owned())
        .collect();
    let possible: Vec<String> = arg
        .get_possible_values()
        .into_iter()
        .filter(|v| !v.is_hide_set())
        .map(|v| v.get_name().to_string())
        .collect();
    let range = arg.get_num_args();
    let multiple =
        matches!(arg.get_action(), ArgAction::Append) || range.is_some_and(|r| r.max_values() > 1);
    json!({
        "id": arg.get_id().as_str(),
        "positional": arg.is_positional(),
        "short": arg.get_short().map(|c| format!("-{c}")),
        "long": arg.get_long().map(|l| format!("--{l}")),
        "required": arg.is_required_set(),
        "multiple": multiple,
        "min_values": range.map(|r| r.min_values()),
        "max_values": range.map(|r| r.max_values()),
        "type": value_type(arg),
        "default": defaults.first().cloned(),
        "possible_values": possible,
        "help": arg.get_help().map(ToString::to_string).unwrap_or_default(),
    })
}
