//! Claude Code hooks installed by `oko setup`. They tell the main agent and every
//! subagent, in plain facts, that Oko is loaded, and remind an agent that greps
//! for code without using Oko. A hook never blocks a tool call: anything
//! unexpected prints `{}` and the call proceeds as if no hook existed.
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const USAGE: &str = "Usage: oko hook session-start|subagent-start|pre-tool-use\n\nAnswers one Claude Code hook event read from stdin with JSON on stdout.\nInstalled by `oko setup --client claude`; it never blocks a tool call.";

// Claude Code treats imperative text from hooks as a possible prompt
// injection, so these state facts and leave the choice to the agent.
const SESSION_CONTEXT: &str = "This repository is set up with Oko code search. The MCP tool mcp__oko__search is loaded: it takes a plain-language question about the code (where something is handled, what calls a function, where a name is defined) and returns the relevant code as exact file excerpts with line numbers, ranked by relevance. grep suits an exact string such as an error message or a config key. Subagents do not receive project instructions, so a prompt that hands code search to a subagent can name mcp__oko__search.";
const SUBAGENT_CONTEXT: &str = "This repository is set up with Oko code search. The MCP tool mcp__oko__search is available here: it takes a plain-language question about the code (where something is handled, what calls a function, where a name is defined) and returns the relevant code as exact file excerpts with line numbers, ranked by relevance; an excerpt is the same text a file read would show. grep suits an exact string such as an error message or a config key.";

const DELEGATION: &str = "This repository has Oko code search: locate code with the mcp__oko__search tool first (a plain-language question returns the relevant code with line numbers), and use grep for exact strings.";

/// A reminder goes with the first code search without Oko, and again after
/// this many more; any Oko search resets the count. Loading the index to add
/// matches here cost a second per grep on large repositories, so the reminder
/// stays text only.
const REMIND_EVERY: u64 = 4;

/// Counts only need to outlast one session; older ones are removed when a
/// new count starts, so the state directory does not grow without end.
const STATE_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

pub fn run(args: &[String]) -> Result<()> {
    let [event] = args else {
        println!("{USAGE}");
        return Ok(());
    };
    let mut input = String::new();
    let _ = std::io::stdin().take(1 << 20).read_to_string(&mut input);
    let answer = respond(event, &input, &state_directory()).unwrap_or(None);
    println!("{}", answer.unwrap_or_else(|| json!({})));
    Ok(())
}

fn respond(event: &str, input: &str, state: &Path) -> Result<Option<Value>> {
    let context = match event {
        "session-start" => Some(SESSION_CONTEXT.to_owned()),
        "subagent-start" => Some(SUBAGENT_CONTEXT.to_owned()),
        "pre-tool-use" => {
            let input: Value = serde_json::from_str(input).context("hook input is not JSON")?;
            if let Some(updated) = delegated_prompt(&input) {
                return Ok(Some(json!({"hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "updatedInput": updated,
                }})));
            }
            reminder(&input, state)
        }
        _ => None,
    };
    let name = match event {
        "session-start" => "SessionStart",
        "subagent-start" => "SubagentStart",
        _ => "PreToolUse",
    };
    Ok(context.map(|context| {
        json!({"hookSpecificOutput": {"hookEventName": name, "additionalContext": context}})
    }))
}

/// Counts code searches made without Oko, per session and per subagent, and
/// returns the reminder when one is due.
fn reminder(input: &Value, state: &Path) -> Option<String> {
    let session = input["session_id"].as_str().unwrap_or("session");
    let agent = input["agent_id"].as_str().unwrap_or("main");
    let key: String = format!("{session}-{agent}")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let counter = state.join(key);
    if input["tool_name"]
        .as_str()
        .is_some_and(|name| name.starts_with("mcp__oko__"))
    {
        let _ = fs::remove_file(&counter);
        return None;
    }
    let query = search_query(input)?;
    let searches = fs::read_to_string(&counter)
        .ok()
        .and_then(|text| text.trim().parse::<u64>().ok())
        .unwrap_or(0)
        + 1;
    let _ = fs::create_dir_all(state);
    if searches == 1 {
        remove_stale_counts(state);
    }
    let _ = fs::write(&counter, searches.to_string());
    (searches % REMIND_EVERY == 1).then(|| {
        format!(
            "mcp__oko__search is loaded in this session. For \"{query}\" it returns the ranked code with line numbers, including where a name is defined and what uses it, from one plain-language question."
        )
    })
}

/// Deletes counts not written for a day: sessions long over.
fn remove_stale_counts(state: &Path) {
    let Ok(entries) = fs::read_dir(state) else {
        return;
    };
    for entry in entries.flatten() {
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|written| written.elapsed().is_ok_and(|age| age > STATE_MAX_AGE));
        if stale {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// The task prompt of an exploring subagent, with a line naming Oko. Subagents
/// see neither project instructions nor, in practice, a hook's context as
/// reason enough to leave grep; they follow their task. This is the line the
/// guidance asks the main agent to write, added when it did not.
fn delegated_prompt(input: &Value) -> Option<Value> {
    if input["tool_name"] != "Agent" {
        return None;
    }
    let tool_input = input["tool_input"].as_object()?;
    let explorer = tool_input
        .get("subagent_type")
        .and_then(Value::as_str)
        .is_none_or(|kind| matches!(kind, "Explore" | "Plan" | "general-purpose"));
    let prompt = tool_input.get("prompt")?.as_str()?;
    if !explorer || prompt.to_lowercase().contains("oko") {
        return None;
    }
    let mut updated = tool_input.clone();
    updated.insert("prompt".into(), json!(format!("{prompt}\n\n{DELEGATION}")));
    Some(Value::Object(updated))
}

/// Where reminder counts live; they only need to outlast one session.
fn state_directory() -> PathBuf {
    std::env::var_os("OKO_HOOK_STATE")
        .map(PathBuf::from)
        .or_else(|| dirs::cache_dir().map(|path| path.join("oko").join("hooks")))
        .unwrap_or_else(|| std::env::temp_dir().join("oko-hooks"))
}

/// The words a code search is looking for, from Grep, Glob, or a grep-like
/// shell command; `None` for anything that is not a search of code.
fn search_query(input: &Value) -> Option<String> {
    let tool_input = &input["tool_input"];
    let pattern = match input["tool_name"].as_str()? {
        "Grep" => tool_input["pattern"].as_str()?.to_owned(),
        "Glob" => tool_input["pattern"].as_str()?.to_owned(),
        "Bash" => bash_pattern(tool_input["command"].as_str()?)?,
        _ => return None,
    };
    words(&pattern)
}

/// Letters, digits, and underscores of a regex or glob, joined by spaces: what
/// the pattern names, not how it matches. Short fragments and escapes are noise.
fn words(pattern: &str) -> Option<String> {
    let mut cleaned = String::new();
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            // `\b`, `\s`, `\w` and friends are syntax, not text to find.
            chars.next();
            cleaned.push(' ');
        } else if c.is_alphanumeric() || c == '_' {
            cleaned.push(c);
        } else {
            cleaned.push(' ');
        }
    }
    let words: Vec<&str> = cleaned
        .split_whitespace()
        .filter(|word| word.chars().count() >= 3)
        .collect();
    (!words.is_empty()).then(|| words.join(" "))
}

/// The pattern of the first grep-like command in a shell line.
fn bash_pattern(command: &str) -> Option<String> {
    let tokens = shell_tokens(command);
    let mut i = 0;
    while i < tokens.len() {
        let start = i;
        while i < tokens.len() && !is_separator(&tokens[i]) {
            i += 1;
        }
        let segment: Vec<&str> = tokens[start..i]
            .iter()
            .map(String::as_str)
            // `FOO=1 grep …` sets the environment for the command.
            .skip_while(|token| token.contains('=') && !token.starts_with('-'))
            .collect();
        if let Some(pattern) = segment_pattern(&segment) {
            return Some(pattern);
        }
        i += 1;
    }
    None
}

fn is_separator(token: &str) -> bool {
    matches!(token, "|" | "||" | "&&" | ";" | "&")
}

fn segment_pattern(segment: &[&str]) -> Option<String> {
    let (program, args) = segment.split_first()?;
    let program = program.rsplit('/').next().unwrap_or(program);
    match program {
        "grep" | "egrep" | "fgrep" | "rg" | "ag" | "ack" => pattern_argument(args),
        "git" if args.first() == Some(&"grep") => pattern_argument(&args[1..]),
        "find" => args
            .windows(2)
            .find(|pair| matches!(pair[0], "-name" | "-iname" | "-path" | "-ipath"))
            .map(|pair| pair[1].to_owned()),
        _ => None,
    }
}

/// The first operand that is not an option, or the value of `-e`/`--regexp`.
fn pattern_argument(args: &[&str]) -> Option<String> {
    const TAKES_VALUE: &[&str] = &[
        "-A",
        "-B",
        "-C",
        "-m",
        "-g",
        "-t",
        "-T",
        "-f",
        "--glob",
        "--type",
        "--type-not",
        "--include",
        "--exclude",
        "--exclude-dir",
        "--max-count",
        "--context",
        "--after-context",
        "--before-context",
        "--file",
        "--max-depth",
        "-d",
    ];
    let mut i = 0;
    while i < args.len() {
        let arg = args[i];
        if arg == "-e" || arg == "--regexp" {
            return args.get(i + 1).map(|value| (*value).to_owned());
        }
        if let Some(value) = arg.strip_prefix("--regexp=") {
            return Some(value.to_owned());
        }
        if arg == "--" {
            return args.get(i + 1).map(|value| (*value).to_owned());
        }
        if arg.starts_with('-') && arg.len() > 1 {
            if TAKES_VALUE.contains(&arg) {
                i += 1;
            }
            i += 1;
            continue;
        }
        return Some(arg.to_owned());
    }
    None
}

/// Words and operators of a shell line, with quotes removed. Good enough to
/// find a grep's pattern; anything it cannot follow simply yields no match.
fn shell_tokens(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                started = true;
                for next in chars.by_ref() {
                    if next == '\'' {
                        break;
                    }
                    current.push(next);
                }
            }
            '"' => {
                started = true;
                while let Some(next) = chars.next() {
                    match next {
                        '"' => break,
                        // Inside double quotes a backslash escapes only these;
                        // elsewhere it stays, as in a regex's `\b`.
                        '\\' if matches!(chars.peek(), Some('$' | '`' | '"' | '\\')) => {
                            current.push(chars.next().unwrap());
                        }
                        _ => current.push(next),
                    }
                }
            }
            '\\' => {
                started = true;
                if let Some(escaped) = chars.next() {
                    current.push(escaped);
                }
            }
            '|' | '&' | ';' => {
                if started {
                    tokens.push(std::mem::take(&mut current));
                    started = false;
                }
                let mut op = c.to_string();
                if chars.peek() == Some(&c) {
                    op.push(c);
                    chars.next();
                }
                tokens.push(op);
            }
            c if c.is_whitespace() => {
                if started {
                    tokens.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            _ => {
                started = true;
                current.push(c);
            }
        }
    }
    if started {
        tokens.push(current);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(tool: &str, key: &str, value: &str) -> Option<String> {
        search_query(&json!({"tool_name": tool, "tool_input": {key: value}}))
    }

    #[test]
    fn finds_the_pattern_of_grep_like_commands() {
        for (command, expected) in [
            (
                "grep -rn \"dispatch_request\" src/",
                Some("dispatch_request"),
            ),
            (
                "rg -n 'fn handle_upload' --type rust",
                Some("handle_upload"),
            ),
            (
                "cd app && grep -R -A 3 -e 'Guardian' lib | head -20",
                Some("Guardian"),
            ),
            (
                "git grep -n full_dispatch_request",
                Some("full_dispatch_request"),
            ),
            ("find . -name '*router*.ts'", Some("router")),
            ("LC_ALL=C grep -rn \"\\bwsgi_app\\b\" .", Some("wsgi_app")),
            (
                "rg --glob '*.go' \"func (c \\*Context) Next\"",
                Some("func Context Next"),
            ),
            ("grep -c x file.txt", None),
            ("ls -la src", None),
            ("cat src/app.py | grep", None),
            ("echo 'grep dispatch'", None),
        ] {
            assert_eq!(
                query("Bash", "command", command).as_deref(),
                expected,
                "{command}"
            );
        }
        assert_eq!(
            query("Grep", "pattern", "class\\s+Flask").as_deref(),
            Some("class Flask")
        );
        assert_eq!(query("Glob", "pattern", "**/*.py"), None);
        assert_eq!(query("Read", "file_path", "src/app.py"), None);
    }

    #[test]
    fn session_and_subagent_context_use_the_documented_shape() {
        for (event, name) in [
            ("session-start", "SessionStart"),
            ("subagent-start", "SubagentStart"),
        ] {
            let answer = respond(event, "{}", Path::new("/nonexistent"))
                .unwrap()
                .unwrap();
            assert_eq!(answer["hookSpecificOutput"]["hookEventName"], name);
            let context = answer["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .unwrap();
            assert!(context.contains("mcp__oko__search"));
            // Facts, not orders: imperative hook text reads like an injection.
            for word in ["MUST", "IMPORTANT", "Always", "Never", "You must"] {
                assert!(!context.contains(word), "{event}: {word}");
            }
        }
        assert_eq!(
            respond("unknown", "{}", Path::new("/nonexistent")).unwrap(),
            None
        );
    }

    #[test]
    fn exploring_subagents_are_told_about_oko_once_and_nothing_else_changes() {
        let state = tempfile::tempdir().unwrap();
        let call = |kind: Option<&str>, prompt: &str| {
            let mut tool_input =
                json!({"description": "Find dispatch", "prompt": prompt, "model": "haiku"});
            if let Some(kind) = kind {
                tool_input["subagent_type"] = json!(kind);
            }
            let input = json!({"session_id": "s", "tool_name": "Agent", "tool_input": tool_input});
            respond("pre-tool-use", &input.to_string(), state.path()).unwrap()
        };
        let answer = call(Some("Explore"), "Where is dispatch handled?").unwrap();
        let output = &answer["hookSpecificOutput"];
        assert_eq!(output["hookEventName"], "PreToolUse");
        assert!(output.get("permissionDecision").is_none());
        let updated = &output["updatedInput"];
        assert_eq!(updated["description"], "Find dispatch");
        assert_eq!(updated["model"], "haiku");
        assert_eq!(updated["subagent_type"], "Explore");
        let prompt = updated["prompt"].as_str().unwrap();
        assert!(prompt.starts_with("Where is dispatch handled?\n\n"));
        assert!(prompt.contains("mcp__oko__search"));
        assert!(call(None, "Map the request flow").is_some());
        // Already told, or a custom agent with its own brief: left alone.
        assert_eq!(call(Some("Explore"), "Use Oko to find dispatch"), None);
        assert_eq!(call(Some("code-reviewer"), "Review this diff"), None);
    }

    #[test]
    fn a_new_count_removes_counts_from_sessions_long_over() {
        let state = tempfile::tempdir().unwrap();
        let old = state.path().join("old-session-main");
        fs::write(&old, "3").unwrap();
        let day_and_more =
            std::time::SystemTime::now() - STATE_MAX_AGE - std::time::Duration::from_secs(60);
        fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(day_and_more)
            .unwrap();
        let recent = state.path().join("recent-session-main");
        fs::write(&recent, "2").unwrap();
        let grep = json!({"session_id": "new", "tool_name": "Grep", "tool_input": {"pattern": "wsgi_app"}});
        assert!(
            respond("pre-tool-use", &grep.to_string(), state.path())
                .unwrap()
                .is_some()
        );
        assert!(!old.exists());
        assert!(recent.exists());
        assert!(state.path().join("new-main").exists());
    }

    #[test]
    fn reminders_come_first_then_every_fourth_search_until_oko_is_used() {
        let state = tempfile::tempdir().unwrap();
        let grep = |session: &str, agent: Option<&str>| {
            let mut input = json!({"session_id": session, "tool_name": "Bash",
                "tool_input": {"command": "grep -rn dispatch_request src"}});
            if let Some(agent) = agent {
                input["agent_id"] = json!(agent);
            }
            respond("pre-tool-use", &input.to_string(), state.path())
                .unwrap()
                .is_some()
        };
        let pattern: Vec<bool> = (0..6).map(|_| grep("s1", None)).collect();
        assert_eq!(pattern, [true, false, false, false, true, false]);
        // A subagent keeps its own count.
        assert!(grep("s1", Some("agent-1")));
        // An Oko search resets the count, so the next grep is reminded again.
        let oko = json!({"session_id": "s1", "tool_name": "mcp__oko__search",
            "tool_input": {"question": "where is dispatch handled"}});
        assert_eq!(
            respond("pre-tool-use", &oko.to_string(), state.path()).unwrap(),
            None
        );
        assert!(grep("s1", None));
        // Not a code search: no count, no reminder.
        let listing =
            json!({"session_id": "s2", "tool_name": "Bash", "tool_input": {"command": "ls -la"}});
        assert_eq!(
            respond("pre-tool-use", &listing.to_string(), state.path()).unwrap(),
            None
        );
        // The documented PreToolUse shape, with no permission decision.
        let first =
            json!({"session_id": "s3", "tool_name": "Grep", "tool_input": {"pattern": "wsgi_app"}});
        let answer = respond("pre-tool-use", &first.to_string(), state.path())
            .unwrap()
            .unwrap();
        assert_eq!(answer["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        assert!(
            answer["hookSpecificOutput"]
                .get("permissionDecision")
                .is_none()
        );
        let context = answer["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(context.contains("\"wsgi_app\""));
        assert!(respond("pre-tool-use", "not json", state.path()).is_err());
    }
}
