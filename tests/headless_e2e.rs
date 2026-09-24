//! Headless P5 scenarios driven through the installed command-line surface.

use std::path::Path;
use std::process::{Child, Command, Output};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn root() -> std::path::PathBuf {
    #[cfg(unix)]
    let temporary_directory = std::path::PathBuf::from("/tmp");
    #[cfg(not(unix))]
    let temporary_directory = std::env::temp_dir();
    temporary_directory.join(format!(
        "horyzond-headless-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ))
}

fn horyctl(runtime: &Path, config: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_horyctl"))
        .args(arguments)
        .arg("--config-dir")
        .arg(config)
        .env("XDG_RUNTIME_DIR", runtime)
        .output()
        .expect("horyctl starts")
}

fn json(runtime: &Path, config: &Path, arguments: &[&str]) -> serde_json::Value {
    let output = horyctl(runtime, config, arguments);
    assert!(
        output.status.success(),
        "horyctl failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("JSON response")
}

fn mode(runtime: &Path, config: &Path, expected: &str) {
    assert_eq!(
        json(runtime, config, &["status"])["mode"],
        serde_json::Value::String(expected.to_owned())
    );
}

#[test]
fn headless_cli_drives_profiles_and_modal_recovery() {
    let root = root();
    let runtime = root.join("runtime");
    let config = root.join("config");
    std::fs::create_dir_all(&runtime).expect("runtime directory");
    let server = Server(
        Command::new(env!("CARGO_BIN_EXE_horyzond"))
            .arg("--config-dir")
            .arg(&config)
            .env("XDG_RUNTIME_DIR", &runtime)
            .spawn()
            .expect("server starts"),
    );
    for _ in 0..50 {
        if horyctl(&runtime, &config, &["status"]).status.success() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(horyctl(&runtime, &config, &["status"]).status.success());

    for (key, layout) in [
        ("1", "spatial"),
        ("2", "scrolling"),
        ("3", "tiling"),
        ("4", "stacking"),
    ] {
        json(&runtime, &config, &["input", "press", "Super+Space"]);
        mode(&runtime, &config, "layout_select");
        json(&runtime, &config, &["input", "press", key]);
        mode(&runtime, &config, "normal");
        assert_eq!(
            json(&runtime, &config, &["status"])["active_layout"],
            layout
        );
    }

    for (key, selected) in [
        ("Super+I", "insert"),
        ("Super+P", "passthrough"),
        ("Super+M", "move"),
        ("Super+R", "resize"),
    ] {
        let response = json(&runtime, &config, &["input", "press", key]);
        assert_eq!(response["consumed"], true);
        mode(&runtime, &config, selected);
        assert_eq!(
            json(&runtime, &config, &["input", "release", key])["consumed"],
            true
        );
        json(&runtime, &config, &["input", "press", "Escape"]);
        mode(&runtime, &config, "normal");
    }

    assert_eq!(
        json(&runtime, &config, &["input", "press", "Super+Enter"])["dispatched"],
        "PendingSpawn"
    );
    mode(&runtime, &config, "pending_spawn");
    assert_eq!(
        json(&runtime, &config, &["status"])["pending_spawn"]["executable"],
        "ghostty"
    );
    assert!(
        json(&runtime, &config, &["input", "press", "Escape"])["dispatched"]
            .as_str()
            .expect("dispatch description")
            .contains("cancelled: true")
    );
    mode(&runtime, &config, "normal");
    assert!(json(&runtime, &config, &["status"])["pending_spawn"].is_null());

    drop(server);
    std::fs::remove_dir_all(root).expect("cleanup");
}
