use eframe::egui;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
const GITHUB: &str = "https://github.com/";
const GITHUB_API_REPOS: &str = "https://api.github.com/repos/";
const INSTALLER_PREFIX: &str = "claude-usage-monitor-setup-";
const INSTALLER_SUFFIX: &str = ".exe";
const CHECKSUM_SUFFIX: &str = ".sha256";
const MAX_CHECKSUM_BYTES: u64 = 1024;
const MAX_INSTALLER_BYTES: u64 = 100 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(120);
static STARTED: AtomicBool = AtomicBool::new(false);
const SILENT_ARGS: [&str; 3] = ["/SILENT", "/CLOSEAPPLICATIONS", "/RESTARTAPPLICATIONS"];

pub fn releases_page() -> String {
    format!("{REPOSITORY}/releases/latest")
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, PartialEq)]
struct Installer {
    name: String,
    url: String,
    checksum_url: String,
}

fn installer(release: &str, current: &str) -> Option<Installer> {
    let release = serde_json::from_str::<Release>(release).ok()?;
    if !crate::sync::is_newer(release.tag_name.trim_start_matches('v'), current) {
        return None;
    }
    let ours = format!("{REPOSITORY}/releases/download/");
    let url_of = |name: &str| {
        release
            .assets
            .iter()
            .find(|asset| asset.name == name && asset.browser_download_url.starts_with(&ours))
            .map(|asset| asset.browser_download_url.clone())
    };
    let name = release
        .assets
        .iter()
        .map(|asset| asset.name.as_str())
        .find(|name| name.starts_with(INSTALLER_PREFIX) && name.ends_with(INSTALLER_SUFFIX))?
        .to_owned();
    Some(Installer {
        url: url_of(&name)?,
        checksum_url: url_of(&format!("{name}{CHECKSUM_SUFFIX}"))?,
        name,
    })
}

fn matches_checksum(bytes: &[u8], checksum: &str) -> bool {
    let actual: String = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    checksum
        .split_whitespace()
        .next()
        .is_some_and(|expected| expected.eq_ignore_ascii_case(&actual))
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
    let found = installer(&release, crate::platform::APP_VERSION)?;
    let checksum = agent
        .get(&found.checksum_url)
        .call()
        .ok()?
        .body_mut()
        .with_config()
        .limit(MAX_CHECKSUM_BYTES)
        .read_to_string()
        .ok()?;
    let bytes = agent
        .get(&found.url)
        .call()
        .ok()?
        .body_mut()
        .with_config()
        .limit(MAX_INSTALLER_BYTES)
        .read_to_vec()
        .ok()?;
    if !matches_checksum(&bytes, &checksum) {
        return None;
    }
    let path = std::env::temp_dir().join(found.name);
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

    fn release(tag: &str, host: &str) -> String {
        let download = format!("{host}/releases/download/{tag}");
        format!(
            r#"{{"tag_name":"{tag}","assets":[
            {{"name":"claude-usage-monitor-linux","browser_download_url":"{download}/linux"}},
            {{"name":"claude-usage-monitor-setup-0.9.0.exe","browser_download_url":"{download}/setup.exe"}},
            {{"name":"claude-usage-monitor-setup-0.9.0.exe.sha256","browser_download_url":"{download}/setup.exe.sha256"}}
        ]}}"#
        )
    }

    #[test]
    fn picks_a_newer_installer_with_its_checksum_from_our_repository_only() {
        let found = installer(&release("v0.9.0", REPOSITORY), "0.3.3").expect("installer");
        assert_eq!(found.name, "claude-usage-monitor-setup-0.9.0.exe");
        assert!(found.url.starts_with(REPOSITORY) && found.checksum_url.ends_with(".sha256"));
        assert_eq!(installer(&release("v0.3.3", REPOSITORY), "0.3.3"), None);
        assert_eq!(
            installer(&release("v0.9.0", "https://evil.example/x/y"), "0.3.3"),
            None
        );
        let without_checksum = release("v0.9.0", REPOSITORY).replace(".exe.sha256", ".exe.txt");
        assert_eq!(installer(&without_checksum, "0.3.3"), None);
        assert_eq!(installer("not json", "0.3.3"), None);
        assert!(releases_page().ends_with("/releases/latest"));
    }

    #[test]
    fn checksum_must_match_the_downloaded_bytes() {
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(matches_checksum(
            b"abc",
            &format!("{abc}  claude-usage-monitor-setup.exe\n")
        ));
        assert!(matches_checksum(b"abc", &abc.to_uppercase()));
        assert!(!matches_checksum(b"abd", abc));
        assert!(!matches_checksum(b"abc", ""));
    }
}
