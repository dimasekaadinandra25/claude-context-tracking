use crate::session::Session;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const SYNTHETIC_MODEL: &str = "<synthetic>";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    #[serde(rename = "type")]
    kind: Option<String>,
    session_id: Option<String>,
    timestamp: Option<String>,
    cwd: Option<String>,
    ai_title: Option<String>,
    #[serde(default)]
    is_sidechain: bool,
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    id: Option<String>,
    model: Option<String>,
    usage: Option<Usage>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Usage {
    input_tokens: u64,
    cache_creation_input_tokens: u64,
    cache_read_input_tokens: u64,
}

impl Usage {
    fn context(&self) -> u64 {
        self.input_tokens + self.cache_creation_input_tokens + self.cache_read_input_tokens
    }
}

#[derive(Default)]
pub struct Index {
    sessions: HashMap<String, Session>,
    seen_messages: HashSet<String>,
    offsets: HashMap<PathBuf, u64>,
}

impl Index {
    pub fn scan(&mut self, root: &Path, mut on_progress: impl FnMut(usize, usize)) -> bool {
        let files = jsonl_files(root);
        let mut changed = false;
        for (i, file) in files.iter().enumerate() {
            changed |= self.read_appended(file);
            on_progress(i + 1, files.len());
        }
        changed
    }

    pub fn reset_baselines(&mut self) {
        self.sessions.values_mut().for_each(Session::reset_baseline);
    }

    pub fn snapshot(&self) -> Vec<Session> {
        self.sessions
            .values()
            .filter(|s| s.has_requests())
            .cloned()
            .collect()
    }

    fn read_appended(&mut self, path: &Path) -> bool {
        let offset = self.offsets.get(path).copied().unwrap_or(0);
        if !std::fs::metadata(path).is_ok_and(|m| m.len() > offset) {
            return false;
        }
        let mut buf = Vec::new();
        let read = File::open(path).and_then(|mut file| {
            file.seek(SeekFrom::Start(offset))?;
            file.read_to_end(&mut buf)
        });
        if read.is_err() {
            return false;
        }
        let Some(end) = buf.iter().rposition(|&b| b == b'\n').map(|i| i + 1) else {
            return false;
        };
        let fallback_id = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        for line in String::from_utf8_lossy(&buf[..end]).lines() {
            self.ingest(line, &fallback_id);
        }
        self.offsets.insert(path.to_path_buf(), offset + end as u64);
        true
    }

    fn ingest(&mut self, line: &str, fallback_id: &str) {
        let Ok(record) = serde_json::from_str::<Record>(line) else {
            return;
        };
        let id = record.session_id.unwrap_or_else(|| fallback_id.to_owned());
        let session = self
            .sessions
            .entry(id.clone())
            .or_insert_with(|| Session::new(id));

        if record.kind.as_deref() == Some("ai-title")
            && let Some(title) = record.ai_title
        {
            session.title = title;
        }
        if session.project.is_empty()
            && let Some(cwd) = record.cwd
        {
            session.project = cwd;
        }
        if record.is_sidechain {
            return;
        }
        let Some(Message {
            id: Some(message_id),
            model: Some(model),
            usage: Some(usage),
        }) = record.message
        else {
            return;
        };
        let at = record.timestamp.unwrap_or_default();
        if model == SYNTHETIC_MODEL
            || !self.seen_messages.insert(message_id)
            || at < session.latest_at
        {
            return;
        }
        session.record_request(model, usage.context(), at);
    }
}

fn jsonl_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for path in entries.flatten().map(|e| e.path()) {
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|ext| ext == "jsonl") {
                files.push(path);
            }
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(session: &str, id: &str, at: &str, context: u64) -> String {
        format!(
            r#"{{"type":"assistant","sessionId":"{session}","cwd":"C:\\Code\\RAG","timestamp":"{at}","message":{{"id":"{id}","model":"claude-opus-5-5","usage":{{"input_tokens":0,"cache_read_input_tokens":{context}}}}}}}"#
        )
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "claude-usage-monitor-{name}-{}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn ingest_reads_title_and_skips_junk_duplicates_and_subagents() {
        let mut index = Index::default();
        let line = request("s1", "m1", "2026-01-01T00:00:00Z", 1_102);
        index.ingest(&line, "file");
        index.ingest(&line.replace("00:00:00", "00:00:01"), "file");
        index.ingest("not json", "file");
        index.ingest(
            &request("s1", "m2", "2026-01-01T00:00:02Z", 9_999)
                .replace(r#""type""#, r#""isSidechain":true,"type""#),
            "file",
        );
        index.ingest(
            r#"{"type":"ai-title","aiTitle":"Hello","sessionId":"s1"}"#,
            "file",
        );

        let session = &index.sessions["s1"];
        assert_eq!(session.name(), "Hello");
        assert_eq!(session.latest_context, 1_102);
        assert_eq!(session.new_context, 0);
        assert_eq!(session.context_window, 1_000_000);
    }

    #[test]
    fn scan_reads_only_complete_appended_lines() {
        let dir = TempDir::new("scan");
        let (s1, s2) = (dir.0.join("s1.jsonl"), dir.0.join("s2.jsonl"));
        let first = request("s1", "m1", "2026-01-01T09:00:00Z", 10_000);
        let second = request("s1", "m2", "2026-01-01T09:10:00Z", 100_000);
        std::fs::write(&s1, format!("{first}\n{}", &second[..20])).unwrap();

        let mut index = Index::default();
        assert!(index.scan(&dir.0, |_, _| {}));
        index.reset_baselines();
        assert_eq!(index.sessions["s1"].latest_context, 10_000);

        std::fs::write(&s1, format!("{first}\n{second}\n")).unwrap();
        std::fs::write(
            &s2,
            format!(
                "{}\n{}\n",
                request("s2", "n1", "2026-01-01T09:03:00Z", 10_000),
                request("s2", "n2", "2026-01-01T09:04:00Z", 100_000)
            ),
        )
        .unwrap();
        assert!(index.scan(&dir.0, |_, _| {}));
        assert_eq!(index.sessions["s1"].new_context_label(), "+90.0K");
        assert_eq!(index.sessions["s2"].new_context_label(), "+90.0K");
        assert!(!index.scan(&dir.0, |_, _| {}));
    }
}
