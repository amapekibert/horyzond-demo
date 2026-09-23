//! Offline configuration utility; IPC transport integration follows in P5.

use wm_config::{ConfigManager, ConfigPath, ReloadOutcome};
use wm_script::ScriptLimits;

fn main() {
    let mut arguments = std::env::args_os().skip(1);
    let Some(command) = arguments.next() else {
        usage();
    };
    let subcommand = arguments.next();
    if command != "config"
        || !matches!(
            subcommand.as_deref(),
            Some(value) if value == "check" || value == "status"
        )
    {
        usage();
    }
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
    eprintln!("Usage: horyctl config <check|status> [--config-dir PATH]");
    std::process::exit(2)
}
fn fail(message: &str) -> ! {
    eprintln!("horyctl: {message}");
    std::process::exit(1)
}
