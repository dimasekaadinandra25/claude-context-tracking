pub use imp::{SUPPORTED, is_enabled, set};

#[cfg(windows)]
mod imp {
    use std::io;
    use std::path::Path;
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    pub const SUPPORTED: bool = true;
    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE_NAME: &str = "ClaudeUsageMonitor";

    fn run_value(exe: &Path) -> String {
        format!("\"{}\"", exe.display())
    }

    pub fn is_enabled() -> bool {
        let Ok(exe) = std::env::current_exe() else {
            return false;
        };
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(RUN_KEY)
            .and_then(|key| key.get_value::<String, _>(VALUE_NAME))
            .is_ok_and(|value| value == run_value(&exe))
    }

    pub fn set(enabled: bool) -> io::Result<()> {
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_KEY)?;
        if enabled {
            return key.set_value(VALUE_NAME, &run_value(&std::env::current_exe()?));
        }
        match key.delete_value(VALUE_NAME) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn run_value_quotes_exe_path() {
            assert_eq!(
                run_value(Path::new(
                    r"C:\Program Files\Claude Usage Monitor\claude-usage-monitor.exe"
                )),
                r#""C:\Program Files\Claude Usage Monitor\claude-usage-monitor.exe""#
            );
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::io;

    pub const SUPPORTED: bool = false;

    pub fn is_enabled() -> bool {
        false
    }

    pub fn set(_enabled: bool) -> io::Result<()> {
        Ok(())
    }
}
