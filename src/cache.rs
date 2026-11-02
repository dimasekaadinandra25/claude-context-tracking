use crate::config;
use crate::export::write_atomic;
use crate::index::Index;
use crate::platform::APP_VERSION;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const FORMAT_VERSION: u32 = 2;
const FILE_NAME: &str = "index_cache.json";
const SAVE_INTERVAL: Duration = Duration::from_secs(5 * 60);

#[derive(Serialize)]
struct CacheRef<'a> {
    format: u32,
    app_version: &'a str,
    index: &'a Index,
}

#[derive(Deserialize)]
struct Cache {
    format: u32,
    app_version: String,
    index: Index,
}

pub fn path() -> PathBuf {
    config::dir().join(FILE_NAME)
}

pub fn load(path: &Path) -> Option<Index> {
    let text = std::fs::read_to_string(path).ok()?;
    let cache: Cache = serde_json::from_str(&text).ok()?;
    (cache.format == FORMAT_VERSION
        && cache.app_version == APP_VERSION
        && cache.index.files_intact())
    .then_some(cache.index)
}

pub fn save(path: &Path, index: &Index) -> io::Result<()> {
    let cache = CacheRef {
        format: FORMAT_VERSION,
        app_version: APP_VERSION,
        index,
    };
    write_atomic(path, &serde_json::to_string(&cache)?)
}

pub struct Saver {
    path: PathBuf,
    saved_at: Option<Instant>,
    dirty: bool,
}

impl Saver {
    pub fn new(path: PathBuf, dirty: bool) -> Self {
        Self {
            path,
            saved_at: None,
            dirty,
        }
    }

    pub fn mark(&mut self, changed: bool) {
        self.dirty |= changed;
    }

    pub fn tick(&mut self, index: &Index) {
        if !self.saved_at.is_some_and(|at| at.elapsed() < SAVE_INTERVAL) {
            self.flush(index);
        }
    }

    pub fn flush(&mut self, index: &Index) {
        if self.dirty {
            self.saved_at = Some(Instant::now());
            self.dirty = save(&self.path, index).is_err();
        }
    }
}

pub mod pairs {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::BTreeMap;

    pub fn serialize<K, V, S>(map: &BTreeMap<K, V>, serializer: S) -> Result<S::Ok, S::Error>
    where
        K: Serialize,
        V: Serialize,
        S: Serializer,
    {
        serializer.collect_seq(map)
    }

    pub fn deserialize<'de, K, V, D>(deserializer: D) -> Result<BTreeMap<K, V>, D::Error>
    where
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de>,
        D: Deserializer<'de>,
    {
        Vec::<(K, V)>::deserialize(deserializer).map(|pairs| pairs.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::TempDir;

    fn request(id: &str, minute: u32, context: u64) -> String {
        format!(
            r#"{{"type":"assistant","sessionId":"s1","cwd":"C:\\Code\\RAG","timestamp":"2026-01-01T09:{minute:02}:00Z","message":{{"id":"{id}","model":"claude-opus-5-5","usage":{{"input_tokens":3,"cache_read_input_tokens":{context},"output_tokens":7}}}}}}"#
        ) + "\n"
    }

    fn scanned(root: &Path) -> Index {
        let mut index = Index::default();
        index.scan(root, |_, _| {});
        index.reset_baselines();
        index
    }

    fn setup(name: &str) -> (TempDir, PathBuf, PathBuf) {
        let dir = TempDir::new(name);
        let root = dir.path().join("projects");
        std::fs::create_dir_all(&root).unwrap();
        let transcript = root.join("s1.jsonl");
        std::fs::write(
            &transcript,
            request("m1", 0, 10_000) + &request("m2", 5, 40_000),
        )
        .unwrap();
        (dir, root, transcript)
    }

    #[test]
    fn round_trip_restores_index() {
        let (dir, root, _) = setup("cache-round-trip");
        let index = scanned(&root);
        let file = dir.path().join(FILE_NAME);
        save(&file, &index).unwrap();
        assert_eq!(load(&file), Some(index));
    }

    #[test]
    fn version_mismatch_or_garbage_is_ignored() {
        let (dir, root, _) = setup("cache-version");
        let file = dir.path().join(FILE_NAME);
        save(&file, &scanned(&root)).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        let stale = text.replacen(
            &format!(r#""app_version":"{APP_VERSION}""#),
            r#""app_version":"0.0.0""#,
            1,
        );
        assert_ne!(stale, text);
        std::fs::write(&file, stale).unwrap();
        assert_eq!(load(&file), None);
        std::fs::write(&file, text.replacen(r#""format":2"#, r#""format":0"#, 1)).unwrap();
        assert_eq!(load(&file), None);
        std::fs::write(&file, "{").unwrap();
        assert_eq!(load(&file), None);
    }

    #[test]
    fn truncated_transcript_invalidates_cache() {
        let (dir, root, transcript) = setup("cache-truncated");
        let file = dir.path().join(FILE_NAME);
        save(&file, &scanned(&root)).unwrap();
        std::fs::write(&transcript, request("m1", 0, 10_000)).unwrap();
        assert_eq!(load(&file), None);
    }

    #[test]
    fn cached_index_plus_appended_lines_matches_full_scan() {
        let (dir, root, transcript) = setup("cache-append");
        let file = dir.path().join(FILE_NAME);
        save(&file, &scanned(&root)).unwrap();
        std::fs::remove_file(&transcript).unwrap();
        std::fs::write(
            &transcript,
            request("m1", 0, 10_000)
                + &request("m2", 5, 40_000)
                + &request("m3", 10, 90_000)
                + &request("m2", 11, 40_000),
        )
        .unwrap();
        std::fs::write(root.join("s2.jsonl"), request("m4", 12, 95_000)).unwrap();

        let mut cached = load(&file).unwrap();
        assert!(cached.scan(&root, |_, _| {}));
        cached.reset_baselines();
        assert_eq!(cached, scanned(&root));
    }
}
