use crate::session::{Session, format_tokens};
use std::fmt::Write as _;
use std::io;
use std::path::Path;

const HEADER: &str =
    "machine\tsession\tmodel\tstart_context\tlatest_context\tnew_context\tcompactions\n";

pub fn to_tsv(machine: &str, sessions: &[Session]) -> String {
    let mut sorted: Vec<&Session> = sessions.iter().collect();
    sorted.sort_by(|a, b| b.latest_at.cmp(&a.latest_at));
    let mut out = String::from(HEADER);
    for s in sorted {
        let _ = writeln!(
            out,
            "{machine}\t{}\t{}\t{}\t{}\t{}\t{}",
            s.name(),
            s.model,
            format_tokens(s.start_context),
            s.latest_label(),
            s.new_context_label(),
            s.compactions
        );
    }
    out
}

pub fn write_atomic(path: &Path, contents: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::cached;

    #[test]
    fn tsv_lists_most_recent_session_first() {
        let mut older = Session::new("a".into());
        older.title = "Older".into();
        older.record_request(
            "claude-opus-5-5".into(),
            cached(10_000),
            "2026-01-01T09:00:00Z".into(),
        );
        let mut newer = Session::new("b".into());
        newer.title = "Newer".into();
        newer.record_request(
            "claude-opus-5-5".into(),
            cached(20_000),
            "2026-01-01T10:00:00Z".into(),
        );

        let tsv = to_tsv("PC", &[older, newer]);
        let lines: Vec<&str> = tsv.lines().collect();
        assert_eq!(lines[0], HEADER.trim_end());
        assert_eq!(
            lines[1],
            "PC\tNewer\tclaude-opus-5-5\t20.0K\t20.0K / 1M (2%)\t+0\t0"
        );
        assert!(lines[2].starts_with("PC\tOlder\t"));
    }
}
