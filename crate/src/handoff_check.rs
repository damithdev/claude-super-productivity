//! Stop hook (`sp-cli hook handoff-check`): every handoff written this session has an SP task, and
//! the handoff it supersedes has none open.
//!
//! Blocks the stop once (exit 2) with what is missing; stop_hook_active lets the retry through.
//! SP not reachable -> warn only, never block.

use std::path::Path;
use std::sync::LazyLock;

use fancy_regex::Regex;
use serde_json::Value;

use crate::util::splitlines;

static STEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"HANDOFF[\w.\-]*?(?=\.md|\]\]|`|\s|\)|$)").unwrap());
static NONE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^supersedes\W*(none|nothing)").unwrap());

fn basename(p: &str) -> &str {
    p.rsplit(['/', '\\']).next().unwrap_or(p)
}

fn written_handoffs<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    for line in lines {
        if !line.contains("HANDOFF") && !line.contains("handoffs") {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(line) else { continue };
        let Some(content) = entry.pointer("/message/content").and_then(Value::as_array) else { continue };
        for c in content {
            let name = c.get("name").and_then(Value::as_str);
            if c.get("type").and_then(Value::as_str) != Some("tool_use") || !matches!(name, Some("Write" | "Edit")) {
                continue;
            }
            let p = c.pointer("/input/file_path").and_then(Value::as_str).unwrap_or_default();
            let norm = p.replace('\\', "/");
            if p.ends_with(".md")
                && (basename(p).starts_with("HANDOFF") || norm.contains("/handoffs/"))
                && !paths.iter().any(|q| q == p)
            {
                paths.push(p.to_owned());
            }
        }
    }
    paths
}

fn superseded(text: &str) -> Vec<String> {
    for line in splitlines(text).into_iter().take(20) {
        let lower = line.to_lowercase();
        let low = lower.trim_start_matches(['#', '*', '-', ' ']);
        if low.starts_with("supersedes") {
            // "Supersedes: none (X covers a different topic)" names a file it does not supersede
            if NONE.is_match(low).unwrap_or(false) {
                return Vec::new();
            }
            return STEM
                .find_iter(line)
                .filter_map(Result::ok)
                .map(|m| m.as_str().to_owned())
                .filter(|s| s != "HANDOFF")
                .collect();
        }
    }
    Vec::new()
}

/// super-productivity:handoff-sp-task: notes = path, blank line, the Continue with block verbatim (no label).
fn has_prompt(notes: &str, stem: &str) -> bool {
    let lines = splitlines(notes);
    let Some(at) = lines.iter().position(|l| l.contains(stem)) else { return false };
    lines[at + 1..].iter().any(|l| !l.trim().is_empty() && !l.contains(stem))
}

fn notes(t: &Value) -> &str {
    t.get("notes").and_then(Value::as_str).unwrap_or_default()
}

fn id(t: &Value) -> String {
    match t.get("id") {
        Some(Value::String(s)) => s.clone(),
        Some(v) => v.to_string(),
        None => "None".to_owned(),
    }
}

fn problems(paths: &[String], tasks: &[Value], read: impl Fn(&str) -> String) -> Vec<String> {
    let mut out = Vec::new();
    for p in paths {
        let base = basename(p);
        let stem = &base[..base.len() - 3];
        let mine: Vec<&Value> = tasks.iter().filter(|t| notes(t).contains(stem)).collect();
        if mine.is_empty() {
            out.push(format!(
                "{p}: no SP task carries this path. Add one (tag handoff; notes = path, blank line, Continue with block; run the super-productivity:handoff-sp-task skill)."
            ));
        } else if !mine.iter().any(|t| has_prompt(notes(t), stem)) {
            out.push(format!("{p}: SP task {} has the path but no Continue with prompt below it.", id(mine[0])));
        }
        for old in superseded(&read(p)) {
            for t in tasks {
                let n = notes(t);
                let done = t.get("isDone").and_then(Value::as_bool) == Some(true);
                if !done && n.contains(&old) && !n.contains(stem) {
                    out.push(format!(
                        "{p} supersedes {old}, but its SP task {} is still open. Close it or repoint it at the new handoff.",
                        id(t)
                    ));
                }
            }
        }
    }
    out
}

fn head(path: &str, chars: usize) -> String {
    let bytes = std::fs::read(path).unwrap_or_default();
    String::from_utf8_lossy(&bytes).chars().take(chars).collect()
}

pub fn run(raw: &str) -> u8 {
    let Ok(payload) = serde_json::from_str::<Value>(raw) else { return 0 };
    if payload.get("stop_hook_active").and_then(Value::as_bool) == Some(true) {
        return 0;
    }
    let Some(transcript) = payload.get("transcript_path").and_then(Value::as_str) else { return 0 };
    let Ok(bytes) = std::fs::read(transcript) else { return 0 };
    let text = String::from_utf8_lossy(&bytes);
    let paths: Vec<String> = written_handoffs(text.lines()).into_iter().filter(|p| Path::new(p).exists()).collect();
    if paths.is_empty() {
        return 0;
    }
    let tasks = match crate::sp::call("GET", "/tasks?includeDone=true", None) {
        Ok(Value::Array(tasks)) => tasks,
        Ok(other) => {
            eprintln!("handoff-check: SP not reachable (unexpected tasks: {other}); handoff tasks not checked.");
            return 0;
        }
        Err(e) => {
            eprintln!("handoff-check: SP not reachable ({e}); handoff tasks not checked.");
            return 0;
        }
    };
    let found = problems(&paths, &tasks, |p| head(p, 4000));
    if found.is_empty() {
        return 0;
    }
    eprintln!("Handoff written without a matching SP task:\n- {}", found.join("\n- "));
    2
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NEW: &str = "C:/v/HANDOFF-2026-10-28-b.md";

    #[test]
    fn finds_written_handoffs_and_skips_junk() {
        let line = json!({"message": {"content": [{"type": "tool_use", "name": "Write", "input": {"file_path": NEW}}]}})
            .to_string();
        let other = json!({"message": {"content": [{"type": "tool_use", "name": "Read", "input": {"file_path": NEW}}]}})
            .to_string();
        let folder = json!({"message": {"content": [{"type": "tool_use", "name": "Edit",
            "input": {"file_path": r"C:\v\handoffs\next.md"}}]}})
        .to_string();
        let lines = [line.as_str(), "junk HANDOFF", other.as_str(), line.as_str(), folder.as_str()];
        assert_eq!(written_handoffs(lines.into_iter()), vec![NEW.to_owned(), r"C:\v\handoffs\next.md".to_owned()]);
    }

    #[test]
    fn reads_what_a_handoff_supersedes() {
        assert_eq!(superseded("Supersedes: `HANDOFF-2026-10-27-a.md` (same folder)."), vec!["HANDOFF-2026-10-27-a"]);
        assert_eq!(superseded("Supersedes: none."), Vec::<String>::new());
        assert_eq!(
            superseded("Supersedes: none (`HANDOFF-2026-10-15-x.md` here covers a different topic)."),
            Vec::<String>::new()
        );
        assert_eq!(superseded("# T\n- **Supersedes:** [[HANDOFF-2026-10-27-a]] and HANDOFF-x y"), vec![
            "HANDOFF-2026-10-27-a",
            "HANDOFF-x"
        ]);
        assert_eq!(superseded("Supersedes HANDOFF.md"), Vec::<String>::new());
    }

    #[test]
    fn checks_tasks_like_the_selftest() {
        let text = |_: &str| "Supersedes: [[HANDOFF-2026-10-27-a]]".to_owned();
        let old = json!({"id": "o", "isDone": false, "notes": "HANDOFF-2026-10-27-a.md"});
        let good = json!({"id": "n", "isDone": false,
            "notes": "C:/v/HANDOFF-2026-10-28-b.md\n\n/workday\nRead first: C:/v/HANDOFF-2026-10-28-b.md"});
        let bare = json!({"id": "b", "isDone": false, "notes": "C:/v/HANDOFF-2026-10-28-b.md\n"});
        let paths = [NEW.to_owned()];
        assert!(problems(&paths, std::slice::from_ref(&bare), text)[0].contains("no Continue with"));
        assert!(problems(&paths, std::slice::from_ref(&good), text).is_empty());
        assert_eq!(problems(&paths, &[good, old.clone()], text).len(), 1);
        assert!(problems(&paths, &[old], text)[0].contains("no SP task"));
    }
}
