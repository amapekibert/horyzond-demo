//! Configuration validation and same-user IPC control utility.

use wm_config::{ConfigManager, ConfigPath, ReloadOutcome};
use wm_ipc::{Request, VERSION, request};
use wm_script::ScriptLimits;

fn main() {
    let mut arguments = std::env::args_os().skip(1);
    let Some(command) = arguments.next() else {
        usage();
    };
    let subcommand = (command == "config" || command == "layout")
        .then(|| arguments.next())
        .flatten();
    if !((command == "config"
        && matches!(subcommand.as_deref(), Some(value) if value == "check" || value == "status"))
        || ((command == "status" || command == "windows" || command == "workspaces")
            && subcommand.is_none())
        || (command == "layout" && subcommand.as_deref() == Some(std::ffi::OsStr::new("select"))))
    {
        usage();
    }
    let params = if command == "layout" {
        let layout = arguments.next().unwrap_or_else(|| usage());
        serde_json::json!({ "layout": layout.to_string_lossy() })
    } else {
        serde_json::Value::Null
    };
    let mut config = None;
    while let Some(argument) = arguments.next() {
        if argument == "--config-dir" {
            config = Some(ConfigPath::from_override(
                arguments.next().unwrap_or_else(|| usage()),
            ));
        } else {
            usage();
        }
    }
    let path = config
        .map_or_else(ConfigPath::default_for_current_user, Ok)
        .unwrap_or_else(|error| fail(&error.to_string()));
    if command == "status" || command == "windows" || command == "workspaces" {
        let method = match command.to_str() {
            Some("status") => "status",
            Some("windows") => "windows.list",
            Some("workspaces") => "workspaces.list",
            _ => unreachable!("validated online command"),
        };
        online(&path, method, serde_json::Value::Null);
        return;
    }
    if command == "layout" {
        online(&path, "layout.select", params);
        return;
    }
    let mut manager = ConfigManager::new(&path, ScriptLimits::default())
        .unwrap_or_else(|error| fail(&error.to_string()));
    match manager.load_initial() {
        ReloadOutcome::Applied { generation } => {
            if subcommand.as_deref() == Some(std::ffi::OsStr::new("status")) {
                println!("configuration generation {generation} is active");
            } else {
                println!("configuration is valid (generation {generation})");
            }
        }
        ReloadOutcome::Rejected { diagnostic } => fail(&diagnostic),
        ReloadOutcome::Unchanged => unreachable!("initial load evaluates configuration"),
    }
}
fn usage() -> ! {
    eprintln!(
        "Usage: horyctl <status|windows|workspaces|layout select ID|config <check|status>> [--config-dir PATH]"
    );
    std::process::exit(2)
}
fn fail(message: &str) -> ! {
    eprintln!("horyctl: {message}");
    std::process::exit(1)
}

fn online(path: &ConfigPath, method: &str, params: serde_json::Value) {
    let socket = path
        .ipc_socket_path()
        .unwrap_or_else(|error| fail(&error.to_string()));
    let response = request(
        &socket,
        &Request {
            version: VERSION,
            id: 1,
            method: method.to_owned(),
            params,
        },
    )
    .unwrap_or_else(|error| fail(&error.to_string()));
    match (response.result, response.error) {
        (Some(result), None) => println!("{result}"),
        (_, Some(error)) => fail(&error),
        _ => fail("invalid IPC response"),
    }
}
