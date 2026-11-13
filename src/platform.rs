use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const APP_NAME: &str = "Claude Usage Monitor";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
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

#[cfg(windows)]
pub fn git_bash() -> Option<PathBuf> {
    let configured = std::env::var_os("CLAUDE_CODE_GIT_BASH_PATH").map(PathBuf::from);
    let beside_git = std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .find(|dir| dir.join("git.exe").is_file())?
            .parent()
            .map(|root| root.join("bin").join("bash.exe"))
    });
    let default = std::env::var_os("ProgramFiles")
        .map(|dir| PathBuf::from(dir).join("Git").join("bin").join("bash.exe"));
    [configured, beside_git, default]
        .into_iter()
        .flatten()
        .find(|path| path.is_file())
}

#[cfg(windows)]
pub fn use_exe_icon_on_taskbar(window: &impl raw_window_handle::HasWindowHandle) {
    const WM_SETICON: u32 = 0x0080;
    const ICON_BIG: usize = 1;
    const IMAGE_ICON: u32 = 1;
    const LR_DEFAULTSIZE: u32 = 0x0040;
    const APP_ICON_ID: usize = 1;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> isize;
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn LoadImageW(
            instance: isize,
            name: *const u16,
            kind: u32,
            cx: i32,
            cy: i32,
            flags: u32,
        ) -> isize;
        fn SendMessageW(window: isize, message: u32, wparam: usize, lparam: isize) -> isize;
    }
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let raw_window_handle::RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return;
    };
    unsafe {
        let icon = LoadImageW(
            GetModuleHandleW(std::ptr::null()),
            APP_ICON_ID as *const u16,
            IMAGE_ICON,
            0,
            0,
            LR_DEFAULTSIZE,
        );
        if icon != 0 {
            SendMessageW(win32.hwnd.get(), WM_SETICON, ICON_BIG, icon);
        }
    }
}

#[cfg(not(windows))]
pub fn use_exe_icon_on_taskbar(_window: &impl raw_window_handle::HasWindowHandle) {}

pub fn data_file() -> PathBuf {
    data_dir().join("usage.txt")
}

pub fn crash_log() -> PathBuf {
    data_dir().join("crash.log")
}

pub fn instance_lock_file() -> PathBuf {
    data_dir().join("instance.lock")
}

pub fn lock_instance(path: &Path) -> Result<File, TryLockError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(TryLockError::Error)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(TryLockError::Error)?;
    file.try_lock()?;
    Ok(file)
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
pub fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());
    ENV.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_lock_is_exclusive_until_dropped() {
        let dir = TempDir::new("lock");
        let path = dir.path().join("nested").join("instance.lock");
        let first = lock_instance(&path).unwrap();
        assert!(matches!(
            lock_instance(&path),
            Err(TryLockError::WouldBlock)
        ));
        drop(first);
        assert!(lock_instance(&path).is_ok());
    }
}
