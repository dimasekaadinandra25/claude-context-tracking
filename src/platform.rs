use std::path::{Path, PathBuf};
use std::process::Command;

pub const APP_NAME: &str = "Claude Usage Monitor";
const DATA_DIR_NAME: &str = "ClaudeUsageMonitor";

pub fn claude_dir() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".claude"))
}

pub fn projects_dir() -> PathBuf {
    claude_dir().join("projects")
}

pub fn find_files(root: &Path, matches: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for path in entries.flatten().map(|entry| entry.path()) {
            if path.is_dir() {
                dirs.push(path);
            } else if matches(&path) {
                files.push(path);
            }
        }
    }
    files.sort();
    files
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

#[cfg(test)]
pub struct TempDir(PathBuf);

#[cfg(test)]
impl TempDir {
    pub fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "claude-usage-monitor-{name}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

#[cfg(test)]
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
