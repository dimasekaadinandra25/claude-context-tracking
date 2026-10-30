use crate::platform;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const LISTING_CHARS_PER_TOKEN: f64 = 2.4;
const MEMORY_CHARS_PER_TOKEN: f64 = 2.6;
const PROJECT_MEMORY_FILES: [&str; 3] = ["CLAUDE.md", ".claude/CLAUDE.md", "CLAUDE.local.md"];
const BLOCK_SCALAR_INDICATORS: [&str; 6] = [">", "|", ">-", "|-", ">+", "|+"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub name: String,
    pub tokens: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Overhead {
    pub memory_files: Vec<Item>,
    pub agents: Vec<Item>,
    pub skills: u64,
}

impl Overhead {
    pub const EMPTY: Self = Self {
        memory_files: Vec::new(),
        agents: Vec::new(),
        skills: 0,
    };

    pub fn memory_tokens(&self) -> u64 {
        self.memory_files.iter().map(|item| item.tokens).sum()
    }

    pub fn agent_tokens(&self) -> u64 {
        self.agents.iter().map(|item| item.tokens).sum()
    }

    pub fn total(&self) -> u64 {
        self.memory_tokens() + self.agent_tokens() + self.skills
    }
}

struct Source {
    prefix: Option<String>,
    root: PathBuf,
}

pub fn estimate(claude_dir: &Path, project: &str) -> Overhead {
    let project_dir = (!project.is_empty()).then(|| Path::new(project));
    let sources = sources(claude_dir, project_dir);
    Overhead {
        memory_files: memory_files(claude_dir, project, project_dir),
        agents: sources
            .iter()
            .flat_map(|source| listing(source, &md_files(&source.root.join("agents"))))
            .collect(),
        skills: sources
            .iter()
            .flat_map(|source| {
                let mut files = platform::find_files(&source.root.join("skills"), |path| {
                    path.file_name().is_some_and(|name| name == "SKILL.md")
                });
                files.extend(md_files(&source.root.join("commands")));
                listing(source, &files)
            })
            .map(|item| item.tokens)
            .sum(),
    }
}

fn sources(claude_dir: &Path, project_dir: Option<&Path>) -> Vec<Source> {
    let mut sources = vec![Source {
        prefix: None,
        root: claude_dir.to_path_buf(),
    }];
    if let Some(dir) = project_dir {
        sources.push(Source {
            prefix: None,
            root: dir.join(".claude"),
        });
    }
    sources.extend(enabled_plugins(claude_dir));
    sources
}

fn enabled_plugins(claude_dir: &Path) -> Vec<Source> {
    let settings = read_json(&claude_dir.join("settings.json"));
    let installed = read_json(&claude_dir.join("plugins").join("installed_plugins.json"));
    let Some(enabled) = settings.get("enabledPlugins").and_then(Value::as_object) else {
        return Vec::new();
    };
    enabled
        .iter()
        .filter(|(_, on)| on.as_bool() == Some(true))
        .filter_map(|(id, _)| {
            let install_path = installed
                .get("plugins")?
                .get(id)?
                .get(0)?
                .get("installPath")?
                .as_str()?;
            Some(Source {
                prefix: Some(id.split('@').next()?.to_owned()),
                root: PathBuf::from(install_path),
            })
        })
        .collect()
}

fn listing(source: &Source, files: &[PathBuf]) -> Vec<Item> {
    files
        .iter()
        .filter_map(|path| {
            let fields = frontmatter(&std::fs::read_to_string(path).ok()?);
            let name = fields
                .get("name")
                .cloned()
                .unwrap_or_else(|| default_name(path));
            let name = match &source.prefix {
                Some(prefix) => format!("{prefix}:{name}"),
                None => name,
            };
            let description_len = fields.get("description").map_or(0, String::len);
            Some(Item {
                tokens: estimate_tokens(name.len() + description_len, LISTING_CHARS_PER_TOKEN),
                name,
            })
        })
        .collect()
}

fn default_name(path: &Path) -> String {
    let file_stem = path.file_stem().unwrap_or_default();
    let name = if file_stem == "SKILL" {
        path.parent().and_then(Path::file_name).unwrap_or(file_stem)
    } else {
        file_stem
    };
    name.to_string_lossy().into_owned()
}

fn memory_files(claude_dir: &Path, project: &str, project_dir: Option<&Path>) -> Vec<Item> {
    let mut paths = vec![claude_dir.join("CLAUDE.md")];
    if let Some(dir) = project_dir {
        let ancestors: Vec<&Path> = dir.ancestors().collect();
        for ancestor in ancestors.into_iter().rev() {
            paths.extend(PROJECT_MEMORY_FILES.map(|name| ancestor.join(name)));
        }
        paths.push(
            claude_dir
                .join("projects")
                .join(encode_project(project))
                .join("memory")
                .join("MEMORY.md"),
        );
    }
    paths
        .into_iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            Some(Item {
                name: display_path(&path),
                tokens: estimate_tokens(text.len(), MEMORY_CHARS_PER_TOKEN),
            })
        })
        .collect()
}

fn md_files(dir: &Path) -> Vec<PathBuf> {
    platform::find_files(dir, |path| path.extension().is_some_and(|ext| ext == "md"))
}

fn read_json(path: &Path) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null)
}

fn frontmatter(text: &str) -> HashMap<String, String> {
    let mut fields = HashMap::new();
    let Some(rest) = text.trim_start_matches('\u{feff}').strip_prefix("---") else {
        return fields;
    };
    let Some(end) = rest.find("\n---") else {
        return fields;
    };
    let mut key: Option<String> = None;
    for line in rest[..end].lines().map(|line| line.trim_end_matches('\r')) {
        let is_continuation = line.starts_with([' ', '\t']);
        match (is_continuation, line.split_once(':')) {
            (false, Some((name, value))) => {
                let value = value.trim();
                let value = if BLOCK_SCALAR_INDICATORS.contains(&value) {
                    ""
                } else {
                    value.trim_matches(['"', '\''])
                };
                fields.insert(name.trim().to_owned(), value.to_owned());
                key = Some(name.trim().to_owned());
            }
            (true, _) => {
                if let Some(value) = key.as_ref().and_then(|k| fields.get_mut(k)) {
                    if !value.is_empty() {
                        value.push(' ');
                    }
                    value.push_str(line.trim());
                }
            }
            (false, None) => key = None,
        }
    }
    fields
}

fn encode_project(project: &str) -> String {
    project
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn display_path(path: &Path) -> String {
    match dirs::home_dir().and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf)) {
        Some(relative) => Path::new("~").join(relative).display().to_string(),
        None => path.display().to_string(),
    }
}

fn estimate_tokens(chars: usize, chars_per_token: f64) -> u64 {
    (chars as f64 / chars_per_token).round() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::TempDir;

    fn write(path: &Path, contents: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn frontmatter_reads_plain_quoted_and_folded_values() {
        let fields = frontmatter(
            "---\r\nname: builder\r\ndescription: >\r\n  Surgical edit.\r\n  Small scope.\r\ntitle: \"Hi\"\r\n---\r\nbody",
        );
        assert_eq!(fields["name"], "builder");
        assert_eq!(fields["description"], "Surgical edit. Small scope.");
        assert_eq!(fields["title"], "Hi");
        assert!(frontmatter("no frontmatter").is_empty());
    }

    #[test]
    fn encode_project_matches_claude_code_folder_names() {
        assert_eq!(
            encode_project(r"c:\Code\Claude Usage Monitor"),
            "c--Code-Claude-Usage-Monitor"
        );
    }

    #[test]
    fn estimate_collects_memory_agents_and_skills_from_enabled_plugins() {
        let temp = TempDir::new("overhead");
        let claude = temp.path().join("claude");
        let project = temp.path().join("work").join("app");
        let plugin = temp.path().join("plugin");
        let disabled = temp.path().join("disabled");

        write(
            &claude.join("settings.json"),
            r#"{"enabledPlugins":{"demo@market":true,"off@market":false}}"#,
        );
        write(
            &claude.join("plugins").join("installed_plugins.json"),
            &serde_json::json!({"plugins": {
                "demo@market": [{"installPath": plugin}],
                "off@market": [{"installPath": disabled}],
            }})
            .to_string(),
        );
        write(
            &plugin.join("agents").join("helper.md"),
            "---\nname: helper\ndescription: Helps a lot.\n---\n",
        );
        write(
            &plugin.join("skills").join("lint").join("SKILL.md"),
            "---\ndescription: Lints code.\n---\n",
        );
        write(
            &disabled.join("agents").join("ghost.md"),
            "---\nname: ghost\n---\n",
        );
        write(&project.join("CLAUDE.md"), &"x".repeat(26));
        let project_name = project.to_string_lossy().into_owned();
        write(
            &claude
                .join("projects")
                .join(encode_project(&project_name))
                .join("memory")
                .join("MEMORY.md"),
            &"y".repeat(52),
        );

        let overhead = estimate(&claude, &project_name);
        assert_eq!(
            overhead.agents,
            vec![Item {
                name: "demo:helper".into(),
                tokens: 10
            }]
        );
        assert_eq!(
            overhead.skills,
            estimate_tokens(
                "demo:lint".len() + "Lints code.".len(),
                LISTING_CHARS_PER_TOKEN
            )
        );
        let memory: Vec<u64> = overhead
            .memory_files
            .iter()
            .map(|item| item.tokens)
            .collect();
        assert_eq!(memory, vec![10, 20]);
        assert_eq!(overhead.total(), 10 + overhead.skills + 30);
    }
}
