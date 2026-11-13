use crate::platform::{self, APP_VERSION};
use std::fs;
use std::io::Write;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_LOG_BYTES: u64 = 64 * 1024;
const HEARTBEAT: Duration = Duration::from_secs(5 * 60);

fn stamp(at: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(at)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

pub fn log(line: &str) {
    let path = platform::app_log();
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if fs::metadata(&path).is_ok_and(|meta| meta.len() > MAX_LOG_BYTES) {
        let _ = fs::rename(&path, path.with_extension("old.log"));
    }
    let _ = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| writeln!(file, "{}  {line}", stamp(SystemTime::now())));
}

fn unix(at: SystemTime) -> u64 {
    at.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn modified(path: &std::path::Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|meta| meta.modified()).ok()
}

fn unclean_exit(
    started: SystemTime,
    last_alive: SystemTime,
    crashed_at: Option<SystemTime>,
    booted_at: Option<SystemTime>,
) -> String {
    let cause = if crashed_at.is_some_and(|at| at >= started) {
        "it crashed, details in crash.log".to_owned()
    } else if let Some(boot) = booted_at.filter(|boot| *boot > last_alive) {
        format!(
            "Windows shut down or restarted (started again {})",
            stamp(boot)
        )
    } else {
        "it was closed without Quit (Task Manager, an installer or signing out)".to_owned()
    };
    format!(
        "previous run (started {}) was last alive {} and did not quit: {cause}",
        stamp(started),
        stamp(last_alive)
    )
}

pub fn started(hidden: bool) {
    let marker = platform::running_marker();
    if let (Ok(text), Some(last_alive)) = (fs::read_to_string(&marker), modified(&marker)) {
        let started = text
            .trim()
            .parse()
            .map_or(last_alive, |secs| UNIX_EPOCH + Duration::from_secs(secs));
        log(&unclean_exit(
            started,
            last_alive,
            modified(&platform::crash_log()),
            platform::booted_at(),
        ));
    }
    let _ = fs::write(&marker, unix(SystemTime::now()).to_string());
    log(&format!(
        "started v{APP_VERSION}{}",
        if hidden { " in the tray" } else { "" }
    ));
}

pub fn quit(reason: &str) {
    log(&format!("quit {reason}"));
    let _ = fs::remove_file(platform::running_marker());
}

#[derive(Default)]
pub struct Heartbeat {
    last: Option<Instant>,
}

impl Heartbeat {
    pub fn tick(&mut self) {
        if self.last.is_some_and(|at| at.elapsed() < HEARTBEAT) {
            return;
        }
        self.last = Some(Instant::now());
        let _ = fs::File::options()
            .write(true)
            .open(platform::running_marker())
            .and_then(|file| file.set_modified(SystemTime::now()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unclean_exit_names_the_likely_cause() {
        let at = |secs| UNIX_EPOCH + Duration::from_secs(secs);
        let crash = unclean_exit(at(100), at(500), Some(at(400)), None);
        assert!(
            crash.ends_with("it crashed, details in crash.log"),
            "{crash}"
        );
        let old_crash = unclean_exit(at(100), at(500), Some(at(50)), Some(at(900)));
        assert!(
            old_crash.contains("Windows shut down or restarted"),
            "{old_crash}"
        );
        let killed = unclean_exit(at(100), at(500), None, Some(at(10)));
        assert!(killed.ends_with("signing out)"), "{killed}");
    }
}
