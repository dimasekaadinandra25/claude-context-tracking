use std::path::{Path, PathBuf};
use std::process::Command;

pub const APP_NAME: &str = "Claude Usage Monitor";
const DATA_DIR_NAME: &str = "ClaudeUsageMonitor";

pub fn projects_dir() -> PathBuf {
    let config_dir = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".claude"));
    config_dir.join("projects")
}

pub fn data_file() -> PathBuf {
    data_dir().join("usage.txt")
}

pub fn crash_log() -> PathBuf {
    data_dir().join("crash.log")
}

fn data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(DATA_DIR_NAME)
}

pub fn machine_name() -> String {
    std::env::var("COMPUTERNAME")
        .ok()
        .or_else(|| {
            let output = Command::new("hostname").output().ok()?;
            Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "unknown".to_owned())
}

pub fn open_in_default_app(path: &Path) {
    let opener = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = Command::new(opener).arg(path).spawn();
}
