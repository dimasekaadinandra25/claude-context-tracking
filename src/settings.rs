use crate::autostart;
use crate::config;
use crate::platform::APP_VERSION;
use crate::theme::RED;
use eframe::egui::{self, RichText};
use std::io;

const FIELD_WIDTH: f32 = 320.0;
const PRIVACY_HINT: &str = "Sync sends only models and token counts, not what your sessions are about. Changing this re-sends all sessions so the dashboard updates.";

pub struct Settings {
    api_key: String,
    key_from_env: bool,
    start_with_windows: bool,
    started_with_windows: bool,
    hide_titles: bool,
    error: Option<String>,
}

impl Settings {
    pub fn load() -> Self {
        let saved = config::saved();
        let start_with_windows = autostart::is_enabled();
        Self {
            api_key: saved.api_key,
            key_from_env: config::api_key_from_env(),
            start_with_windows,
            started_with_windows: start_with_windows,
            hide_titles: saved.hide_titles,
            error: None,
        }
    }

    fn save(&self) -> io::Result<()> {
        config::save_settings(&self.api_key, self.hide_titles)?;
        if self.start_with_windows != self.started_with_windows {
            autostart::set(self.start_with_windows)?;
        }
        Ok(())
    }

    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        let mut open = true;
        let mut save = false;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("API key");
                    ui.add_enabled(
                        !self.key_from_env,
                        egui::TextEdit::singleline(&mut self.api_key)
                            .password(true)
                            .desired_width(FIELD_WIDTH),
                    );
                });
                if self.key_from_env {
                    ui.label(RichText::new("Set by environment variable").small().weak());
                }
                if autostart::SUPPORTED {
                    ui.checkbox(&mut self.start_with_windows, "Start with Windows");
                }
                ui.checkbox(
                    &mut self.hide_titles,
                    "Hide session titles and project names",
                )
                .on_hover_text(PRIVACY_HINT);
                save = ui.button("Save").clicked();
                if let Some(error) = &self.error {
                    ui.colored_label(RED, format!("Couldn't save: {error}"));
                }
                ui.label(
                    RichText::new(format!("Version {APP_VERSION}"))
                        .small()
                        .weak(),
                );
            });
        if save {
            match self.save() {
                Ok(()) => return true,
                Err(error) => self.error = Some(error.to_string()),
            }
        }
        !open
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{TempDir, lock_env};

    #[test]
    fn api_key_from_environment_is_never_saved() {
        let _env = lock_env();
        let dir = TempDir::new("settings-env");
        unsafe {
            std::env::set_var("CLAUDE_USAGE_CONFIG_DIR", dir.path());
            std::env::set_var("CLAUDE_USAGE_API_KEY", "from-env");
        }
        config::save_settings("from-file", false).unwrap();
        let mut settings = Settings::load();
        assert!(settings.key_from_env);
        assert_eq!(settings.api_key, "from-file");
        settings.hide_titles = true;
        settings.save().unwrap();
        assert_eq!(config::load().api_key, "from-env");
        unsafe { std::env::remove_var("CLAUDE_USAGE_API_KEY") };
        let saved = config::saved();
        assert_eq!(saved.api_key, "from-file");
        assert!(saved.hide_titles);
    }
}
