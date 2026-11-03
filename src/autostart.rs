pub use imp::{SUPPORTED, is_enabled, set};

pub const HIDDEN_ARG: &str = "--hidden";

pub fn launched_hidden(mut args: impl Iterator<Item = String>) -> bool {
    args.any(|arg| arg == HIDDEN_ARG)
}

pub fn upgrade() -> std::io::Result<()> {
    if is_enabled() { set(true) } else { Ok(()) }
}

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
        format!("\"{}\" {}", exe.display(), super::HIDDEN_ARG)
    }

    fn runs(value: &str, exe: &Path) -> bool {
        let quoted = format!("\"{}\"", exe.display());
        value == quoted || value == run_value(exe)
    }

    pub fn is_enabled() -> bool {
        let Ok(exe) = std::env::current_exe() else {
            return false;
        };
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(RUN_KEY)
            .and_then(|key| key.get_value::<String, _>(VALUE_NAME))
            .is_ok_and(|value| runs(&value, &exe))
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

        const EXE: &str = r"C:\Program Files\Claude Usage Monitor\claude-usage-monitor.exe";

        #[test]
        fn run_value_quotes_exe_path_and_starts_hidden() {
            assert_eq!(
                run_value(Path::new(EXE)),
                r#""C:\Program Files\Claude Usage Monitor\claude-usage-monitor.exe" --hidden"#
            );
        }

        #[test]
        fn old_and_hidden_entries_both_count_as_enabled() {
            let exe = Path::new(EXE);
            assert!(runs(&format!("\"{EXE}\""), exe));
            assert!(runs(&run_value(exe), exe));
            assert!(!runs(&format!("\"{EXE}\" --other"), exe));
            assert!(!runs(
                r#""C:\Other\claude-usage-monitor.exe" --hidden"#,
                exe
            ));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_only_with_flag() {
        assert!(launched_hidden(
            ["app.exe", "--hidden"].map(String::from).into_iter()
        ));
        assert!(!launched_hidden(["app.exe"].map(String::from).into_iter()));
        assert!(!launched_hidden(
            ["app.exe", "--dump"].map(String::from).into_iter()
        ));
    }
}
