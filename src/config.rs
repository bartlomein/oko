//! Preserve dotenv's literal values and permissive parsing during the Rust port.
//! Parser pattern adapted from dotenv (BSD-2-Clause); see THIRD_PARTY_NOTICES.md.
use regex::Regex;
use std::{collections::HashMap, sync::OnceLock};

const WS: &str = r"[\t\n\v\f\r \u{00a0}\u{1680}\u{2000}-\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}\u{feff}]";

pub fn trim(value: &str) -> &str {
    value.trim_matches(|c| {
        matches!(c, '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{00a0}' | '\u{1680}' |
        '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' |
        '\u{3000}' | '\u{feff}')
    })
}

pub fn parse_env(text: &str) -> HashMap<String, String> {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        let pattern = r#"(?m)^\s*(?:export\s+)?([A-Za-z0-9_.-]+)(?:\s*=\s*?|:\s+?)(\s*'(?:\\'|[^'])*'|\s*"(?:\\"|[^"])*"|\s*`(?:\\`|[^`])*`|[^#\r\n]+)?\s*(?:#[^\n]*)?$"#;
        Regex::new(&pattern.replace(r"\s", WS)).unwrap()
    });
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut values = HashMap::new();
    for capture in pattern.captures_iter(&normalized) {
        let raw = trim(capture.get(2).map_or("", |m| m.as_str()));
        let quote = raw.as_bytes().first().copied();
        let quoted = matches!(quote, Some(b'\'' | b'"' | b'`'))
            && raw.len() >= 2
            && raw.as_bytes().last().copied() == quote;
        let mut value = if quoted {
            raw[1..raw.len() - 1].to_owned()
        } else {
            raw.to_owned()
        };
        if quote == Some(b'"') {
            value = value.replace("\\n", "\n").replace("\\r", "\r");
        }
        values.insert(capture[1].to_owned(), value);
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_values_comments_duplicates_and_malformed_lines() {
        let values = parse_env(
            "\u{feff}export KEY=old\r\nbroken line\rKEY=\"$NOT_EXPANDED\\nline # literal\" # comment\nOTHER: yes\nEMPTY=\nSINGLE='a\\nb'\n",
        );
        assert_eq!(values["KEY"], "$NOT_EXPANDED\nline # literal");
        assert_eq!(values["OTHER"], "yes");
        assert_eq!(values["EMPTY"], "");
        assert_eq!(values["SINGLE"], "a\\nb");
        assert_eq!(trim("\u{feff} x \u{feff}"), "x");
        assert_eq!(trim("\u{85}"), "\u{85}");
    }
}
