use eframe::egui;
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
const GITHUB: &str = "https://github.com/";
const GITHUB_API_REPOS: &str = "https://api.github.com/repos/";
const INSTALLER_PREFIX: &str = "claude-usage-monitor-setup-";
const INSTALLER_SUFFIX: &str = ".exe";
const MAX_INSTALLER_BYTES: u64 = 100 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(120);
static STARTED: AtomicBool = AtomicBool::new(false);
const SILENT_ARGS: [&str; 3] = ["/SILENT", "/CLOSEAPPLICATIONS", "/RESTARTAPPLICATIONS"];

pub fn releases_page() -> String {
    format!("{REPOSITORY}/releases/latest")
}

#[derive(Deserialize)]
struct Release {
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

fn installer(release: &str) -> Option<(String, String)> {
    serde_json::from_str::<Release>(release)
        .ok()?
        .assets
        .into_iter()
        .find(|asset| {
            asset.name.starts_with(INSTALLER_PREFIX) && asset.name.ends_with(INSTALLER_SUFFIX)
        })
        .map(|asset| (asset.name, asset.browser_download_url))
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .user_agent(crate::platform::APP_NAME)
        .build()
        .into()
}

fn install_latest() -> Option<()> {
    if !cfg!(windows) {
        return None;
    }
    let api = format!(
        "{}/releases/latest",
        REPOSITORY.replacen(GITHUB, GITHUB_API_REPOS, 1)
    );
    let agent = agent();
    let release = agent
        .get(&api)
        .header("Accept", "application/vnd.github+json")
        .call()
        .ok()?
        .body_mut()
        .read_to_string()
        .ok()?;
    let (name, url) = installer(&release)?;
    let bytes = agent
        .get(&url)
        .call()
        .ok()?
        .body_mut()
        .with_config()
        .limit(MAX_INSTALLER_BYTES)
        .read_to_vec()
        .ok()?;
    let path = std::env::temp_dir().join(name);
    std::fs::write(&path, bytes).ok()?;
    std::process::Command::new(path)
        .args(SILENT_ARGS)
        .spawn()
        .ok()
        .map(drop)
}

pub fn start(ctx: egui::Context) {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        if install_latest().is_none() {
            ctx.open_url(egui::OpenUrl::new_tab(releases_page()));
        }
        STARTED.store(false, Ordering::SeqCst);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_windows_installer_from_the_latest_release() {
        let release = r#"{"tag_name":"v0.3.0","assets":[
            {"name":"claude-usage-monitor-linux","browser_download_url":"https://x/linux"},
            {"name":"claude-usage-monitor-setup-0.3.0.exe","browser_download_url":"https://x/setup.exe"}
        ]}"#;
        assert_eq!(
            installer(release),
            Some((
                "claude-usage-monitor-setup-0.3.0.exe".to_owned(),
                "https://x/setup.exe".to_owned()
            ))
        );
        assert_eq!(installer(r#"{"assets":[]}"#), None);
        assert_eq!(installer("not json"), None);
        assert!(releases_page().ends_with("/releases/latest"));
    }
}
