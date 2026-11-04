use serde_json::Value;

const MCP_PREFIX: &str = "mcp__";
const MCP_LABEL: &str = "MCP";
const SHELL_TOOLS: [&str; 2] = ["Bash", "PowerShell"];
const GIT_COMMANDS: [&str; 2] = ["git", "git.exe"];
const COMMIT_COMMAND: &str = "commit";
const COMMIT_EXCLUSIONS: [&str; 2] = ["--amend", "--dry-run"];
const GIT_OPTIONS_WITH_VALUE: [&str; 4] = ["-C", "-c", "--git-dir", "--work-tree"];
const COMMAND_SEPARATORS: [char; 4] = ['\n', ';', '|', '&'];
const QUOTED_PLACEHOLDER: char = '_';
const CREATED_FILE: &str = "create";

pub fn tool_label(name: &str) -> &str {
    if name.starts_with(MCP_PREFIX) {
        MCP_LABEL
    } else {
        name
    }
}

pub fn commits_requested(tool: &str, input: Option<&Value>) -> u32 {
    if !SHELL_TOOLS.contains(&tool) {
        return 0;
    }
    input
        .and_then(|input| input.get("command"))
        .and_then(Value::as_str)
        .map_or(0, commit_count)
}

fn commit_count(command: &str) -> u32 {
    without_quoted_text(command)
        .split(COMMAND_SEPARATORS)
        .filter(|part| is_commit(part))
        .count() as u32
}

fn is_commit(command: &str) -> bool {
    let mut words = command
        .split_whitespace()
        .map(|word| word.trim_matches(['(', ')']))
        .skip_while(|word| word.contains('='));
    if !words
        .next()
        .is_some_and(|word| GIT_COMMANDS.contains(&word))
    {
        return false;
    }
    while let Some(word) = words.next() {
        if GIT_OPTIONS_WITH_VALUE.contains(&word) {
            words.next();
        } else if !word.starts_with('-') {
            return word == COMMIT_COMMAND && !words.any(|word| COMMIT_EXCLUSIONS.contains(&word));
        }
    }
    false
}

fn without_quoted_text(command: &str) -> String {
    let mut out = String::with_capacity(command.len());
    let mut quote = None;
    let mut chars = command.chars();
    while let Some(c) = chars.next() {
        match quote {
            Some(open) if c == open => quote = None,
            Some('"') if c == '\\' => {
                chars.next();
            }
            Some(_) => {}
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                out.push(QUOTED_PLACEHOLDER);
            }
            None => out.push(c),
        }
    }
    out
}

pub fn changed_lines(result: Option<&Value>) -> (u64, u64) {
    let Some(result) = result else {
        return (0, 0);
    };
    let patch = result.get("structuredPatch").and_then(Value::as_array);
    let lines = patch
        .into_iter()
        .flatten()
        .filter_map(|hunk| hunk.get("lines")?.as_array())
        .flatten()
        .filter_map(Value::as_str);
    let (mut added, mut removed) = (0, 0);
    for line in lines {
        match line.chars().next() {
            Some('+') => added += 1,
            Some('-') => removed += 1,
            _ => {}
        }
    }
    if added == 0
        && removed == 0
        && result.get("type").and_then(Value::as_str) == Some(CREATED_FILE)
    {
        added = result
            .get("content")
            .and_then(Value::as_str)
            .map_or(0, |content| content.lines().count() as u64);
    }
    (added, removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mcp_tools_collapse_so_server_names_stay_private() {
        assert_eq!(tool_label("mcp__acme_crm__search"), "MCP");
        assert_eq!(tool_label("Edit"), "Edit");
    }

    #[test]
    fn counts_real_commits_only() {
        for (command, expected) in [
            ("git commit -m fix", 1),
            ("git add -A && git commit -q -m \"one; git commit two\"", 1),
            ("git -C \"/c/My Repo\" commit -m x", 1),
            ("cd a && git commit -m a\ncd b && git commit -m b", 2),
            ("GIT_AUTHOR_NAME=x git commit -m y", 1),
            ("(git commit -m y)", 1),
            ("git commit -m \"$(cat <<'EOF'\nsay git commit\nEOF\n)\"", 1),
            ("git commit --amend --no-edit", 0),
            ("git commit --dry-run", 0),
            ("git log --grep commit", 0),
            ("echo git commit", 0),
            ("git status", 0),
        ] {
            assert_eq!(commit_count(command), expected, "{command}");
        }
    }

    #[test]
    fn only_shell_tools_can_commit() {
        let input = json!({"command": "git commit -m x"});
        assert_eq!(commits_requested("Bash", Some(&input)), 1);
        assert_eq!(commits_requested("PowerShell", Some(&input)), 1);
        assert_eq!(commits_requested("Read", Some(&input)), 0);
        assert_eq!(commits_requested("Bash", None), 0);
    }

    #[test]
    fn counts_patch_lines_and_created_files() {
        let edit = json!({"structuredPatch": [
            {"lines": [" same", "-old", "+new", "+extra"]},
            {"lines": ["-gone"]},
        ]});
        assert_eq!(changed_lines(Some(&edit)), (2, 2));
        let created = json!({"type": "create", "content": "a\nb\nc", "structuredPatch": []});
        assert_eq!(changed_lines(Some(&created)), (3, 0));
        assert_eq!(changed_lines(Some(&json!("Error: failed"))), (0, 0));
        assert_eq!(
            changed_lines(Some(&json!({"stdout": "+not a patch"}))),
            (0, 0)
        );
        assert_eq!(changed_lines(None), (0, 0));
    }
}
