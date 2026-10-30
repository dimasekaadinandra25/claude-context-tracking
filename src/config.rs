use crate::export::write_atomic;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::PathBuf;

const DIR_NAME: &str = "claude-usage-monitor";
const CONFIG_FILE: &str = "config.json";
const STATE_FILE: &str = "sync_state.json";
const DEFAULT_SERVER_URL: &str = match option_env!("CLAUDE_USAGE_SERVER_URL") {
    Some(url) => url,
    None => "http://localhost:8080",
};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub server_url: String,
    pub api_key: String,
}

impl Config {
    pub fn is_enabled(&self) -> bool {
        !self.server_url.trim().is_empty() && !self.api_key.trim().is_empty()
    }
}

#[derive(Serialize, Deserialize)]
struct SyncState {
    watermark: String,
}

fn dir() -> PathBuf {
    std::env::var_os("CLAUDE_USAGE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::config_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join(DIR_NAME)
        })
}

fn read<T: for<'de> Deserialize<'de>>(file: &str) -> Option<T> {
    let text = std::fs::read_to_string(dir().join(file)).ok()?;
    serde_json::from_str(&text).ok()
}

fn write<T: Serialize>(file: &str, value: &T) -> io::Result<()> {
    write_atomic(&dir().join(file), &serde_json::to_string_pretty(value)?)
}

fn env_override(field: &mut String, name: &str) {
    if let Some(value) = std::env::var(name).ok().filter(|v| !v.trim().is_empty()) {
        *field = value;
    }
}

pub fn load() -> Config {
    let mut config: Config = read(CONFIG_FILE).unwrap_or_default();
    env_override(&mut config.server_url, "CLAUDE_USAGE_SERVER_URL");
    env_override(&mut config.api_key, "CLAUDE_USAGE_API_KEY");
    with_defaults(config)
}

fn with_defaults(mut config: Config) -> Config {
    if config.server_url.trim().is_empty() {
        DEFAULT_SERVER_URL.clone_into(&mut config.server_url);
    }
    config
}

pub fn save_api_key(api_key: &str) -> io::Result<()> {
    let mut config: Config = read(CONFIG_FILE).unwrap_or_default();
    api_key.trim().clone_into(&mut config.api_key);
    write(CONFIG_FILE, &config)
}

pub fn load_watermark() -> Option<String> {
    read::<SyncState>(STATE_FILE).map(|state| state.watermark)
}

pub fn save_watermark(watermark: &str) -> io::Result<()> {
    write(
        STATE_FILE,
        &SyncState {
            watermark: watermark.to_owned(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_url_defaults_to_built_in() {
        let key_only = Config {
            server_url: " ".into(),
            api_key: "key".into(),
        };
        let config = with_defaults(key_only);
        assert_eq!(config.server_url, DEFAULT_SERVER_URL);
        assert!(config.is_enabled());

        let custom = Config {
            server_url: "http://team:8080".into(),
            api_key: "key".into(),
        };
        assert_eq!(with_defaults(custom).server_url, "http://team:8080");
    }
}
