use crate::export::write_atomic;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::PathBuf;

const DIR_NAME: &str = "claude-usage-monitor";
const CONFIG_FILE: &str = "config.json";
const STATE_FILE: &str = "sync_state.json";
const API_KEY_VAR: &str = "CLAUDE_USAGE_API_KEY";
const SERVER_URL_VAR: &str = "CLAUDE_USAGE_SERVER_URL";
const DEFAULT_SERVER_URL: &str = match option_env!("CLAUDE_USAGE_SERVER_URL") {
    Some(url) => url,
    None => "http://localhost:8080",
};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub server_url: String,
    pub api_key: String,
    pub light_mode: bool,
    pub hide_titles: bool,
    pub key_reminder_shown: bool,
}

impl Config {
    pub fn is_enabled(&self) -> bool {
        !self.server_url.trim().is_empty() && !self.api_key.trim().is_empty()
    }

    pub fn sync_identity(&self) -> SyncIdentity {
        SyncIdentity {
            server_url: self.server_url.trim().trim_end_matches('/').to_owned(),
            key_hash: fingerprint(self.api_key.trim()),
            hide_titles: self.hide_titles,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncIdentity {
    server_url: String,
    key_hash: String,
    hide_titles: bool,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct SyncState {
    watermark: String,
    identity: SyncIdentity,
}

fn fingerprint(text: &str) -> String {
    let hash = text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("{hash:016x}")
}

pub fn dir() -> PathBuf {
    std::env::var_os("CLAUDE_USAGE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::config_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join(DIR_NAME)
        })
}

fn parse<T: for<'de> Deserialize<'de>>(text: &str) -> Option<T> {
    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
}

fn read<T: for<'de> Deserialize<'de>>(file: &str) -> Option<T> {
    parse(&std::fs::read_to_string(dir().join(file)).ok()?)
}

fn editable() -> io::Result<Config> {
    match std::fs::read_to_string(dir().join(CONFIG_FILE)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Config::default()),
        Err(error) => Err(error),
        Ok(text) => parse(&text).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "config.json can't be read; fix or delete it before saving settings",
            )
        }),
    }
}

fn write<T: Serialize>(file: &str, value: &T) -> io::Result<()> {
    write_atomic(&dir().join(file), &serde_json::to_string_pretty(value)?)
}

fn env_value(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn env_override(field: &mut String, name: &str) {
    if let Some(value) = env_value(name) {
        *field = value;
    }
}

pub fn api_key_from_env() -> bool {
    env_value(API_KEY_VAR).is_some()
}

pub fn saved() -> Config {
    read(CONFIG_FILE).unwrap_or_default()
}

pub fn load() -> Config {
    let mut config = saved();
    env_override(&mut config.server_url, SERVER_URL_VAR);
    env_override(&mut config.api_key, API_KEY_VAR);
    with_defaults(config)
}

fn with_defaults(mut config: Config) -> Config {
    if config.server_url.trim().is_empty() {
        DEFAULT_SERVER_URL.clone_into(&mut config.server_url);
    }
    config
}

pub fn save_settings(api_key: &str, hide_titles: bool) -> io::Result<()> {
    let mut config = editable()?;
    api_key.trim().clone_into(&mut config.api_key);
    config.hide_titles = hide_titles;
    write(CONFIG_FILE, &config)
}

pub fn save_light_mode(light_mode: bool) -> io::Result<()> {
    let mut config = editable()?;
    config.light_mode = light_mode;
    write(CONFIG_FILE, &config)
}

pub fn claim_key_reminder() -> bool {
    let Ok(mut config) = editable() else {
        return false;
    };
    if config.key_reminder_shown {
        return false;
    }
    config.key_reminder_shown = true;
    write(CONFIG_FILE, &config).is_ok()
}

pub fn load_watermark(config: &Config) -> Option<String> {
    read::<SyncState>(STATE_FILE)
        .filter(|state| state.identity == config.sync_identity())
        .map(|state| state.watermark)
}

pub fn save_watermark(config: &Config, watermark: &str) -> io::Result<()> {
    write(
        STATE_FILE,
        &SyncState {
            watermark: watermark.to_owned(),
            identity: config.sync_identity(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{TempDir, lock_env};

    fn synced() -> Config {
        Config {
            server_url: "http://team:8080/".into(),
            api_key: "key".into(),
            ..Config::default()
        }
    }

    #[test]
    fn sync_identity_ignores_light_mode_and_trailing_slash() {
        let base = synced();
        let same = Config {
            light_mode: true,
            server_url: " http://team:8080 ".into(),
            api_key: "key ".into(),
            ..synced()
        };
        assert_eq!(base.sync_identity(), same.sync_identity());
        for changed in [
            Config {
                api_key: "other".into(),
                ..synced()
            },
            Config {
                server_url: "http://other:8080".into(),
                ..synced()
            },
            Config {
                hide_titles: true,
                ..synced()
            },
        ] {
            assert_ne!(base.sync_identity(), changed.sync_identity());
        }
    }

    #[test]
    fn watermark_only_applies_to_the_identity_that_saved_it() {
        let _env = lock_env();
        let dir = TempDir::new("config-identity");
        unsafe { std::env::set_var("CLAUDE_USAGE_CONFIG_DIR", dir.path()) };
        let config = synced();
        save_watermark(&config, "2026-10-30T01:00:00Z").unwrap();
        assert_eq!(
            load_watermark(&config).as_deref(),
            Some("2026-10-30T01:00:00Z")
        );
        let new_key = Config {
            api_key: "new".into(),
            ..synced()
        };
        assert_eq!(load_watermark(&new_key), None);
        let private = Config {
            hide_titles: true,
            ..synced()
        };
        assert_eq!(load_watermark(&private), None);
        std::fs::write(
            dir.path().join(STATE_FILE),
            r#"{"watermark":"2026-10-30T01:00:00Z"}"#,
        )
        .unwrap();
        assert_eq!(load_watermark(&config), None);
    }

    #[test]
    fn key_reminder_is_claimed_once_and_kept_by_settings() {
        let _env = lock_env();
        let dir = TempDir::new("config-reminder");
        unsafe { std::env::set_var("CLAUDE_USAGE_CONFIG_DIR", dir.path()) };
        assert!(claim_key_reminder());
        assert!(!claim_key_reminder());
        save_settings("key", true).unwrap();
        save_light_mode(true).unwrap();
        assert!(saved().key_reminder_shown);
        assert!(!claim_key_reminder());
    }

    #[test]
    fn config_with_byte_order_mark_is_read() {
        let _env = lock_env();
        let dir = TempDir::new("config-bom");
        unsafe { std::env::set_var("CLAUDE_USAGE_CONFIG_DIR", dir.path()) };
        std::fs::write(
            dir.path().join(CONFIG_FILE),
            "\u{feff}{\"api_key\": \"kept\", \"light_mode\": true}",
        )
        .unwrap();
        let config = saved();
        assert_eq!(config.api_key, "kept");
        assert!(config.light_mode);
    }

    #[test]
    fn unreadable_config_is_never_overwritten() {
        let _env = lock_env();
        let dir = TempDir::new("config-broken");
        unsafe { std::env::set_var("CLAUDE_USAGE_CONFIG_DIR", dir.path()) };
        let path = dir.path().join(CONFIG_FILE);
        std::fs::write(&path, "{\"api_key\": \"kept\",").unwrap();
        assert!(!claim_key_reminder());
        assert!(save_light_mode(true).is_err());
        assert!(save_settings("other", false).is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\"api_key\": \"kept\","
        );
    }

    #[test]
    fn server_url_defaults_to_built_in() {
        let key_only = Config {
            server_url: " ".into(),
            api_key: "key".into(),
            ..Config::default()
        };
        let config = with_defaults(key_only);
        assert_eq!(config.server_url, DEFAULT_SERVER_URL);
        assert!(config.is_enabled());

        let custom = Config {
            server_url: "http://team:8080".into(),
            api_key: "key".into(),
            ..Config::default()
        };
        assert_eq!(with_defaults(custom).server_url, "http://team:8080");
    }
}
