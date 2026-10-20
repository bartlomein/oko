//! The parts of a prompt that asks for several things at once.
//!
//! "Return where the counter resets, where statistics are recorded, and where
//! writes update the counter" asks for three places. Judged against the whole
//! prompt, the code for one part scores low beside code that answers several
//! words of it; judged against that part alone, it is a clear match. On the
//! replayed agent tasks the missed part's code rose from 0.34 to 0.80.
//!
//! Splitting is by rule, never by a model, and only when the shape is plain:
//! a list, several questions, repeated `where`/`how` clauses, or several
//! things after a request verb. Each part keeps the prompt's subject, so it
//! still names what it is about. Anything less certain stays one question.

use regex::Regex;
use std::sync::OnceLock;

/// More parts than this are a list of names, not separate needs.
const MAX_PARTS: usize = 4;
/// A part's subject is a short phrase; a long one is the whole prompt again.
const MAX_LEAD_WORDS: usize = 10;
const MIN_CONTENT_WORDS: usize = 2;

const QUESTION_WORDS: &[&str] = &["where", "how", "which", "what", "when", "why"];
const REQUEST_VERBS: &[&str] = &[
    "return", "show", "list", "find", "identify", "include", "locate", "give",
];
const LEAD_VERBS: &[&str] = &[
    "trace",
    "explain",
    "show",
    "find",
    "describe",
    "investigate",
    "look at",
    "locate",
];
/// Words that open a condition on the whole request, not a thing it asks for.
const CONSTRAINTS: &[&str] = &[
    "without",
    "do not",
    "don't",
    "never",
    "avoid",
    "only",
    "ensure",
    "make sure",
    "keep",
    "so that",
    "while",
    "instead of",
    "but not",
    "except",
    "unless",
    "ignoring",
];
/// Asking how things relate is one need, however many things it names.
const RELATIONS: &[&str] = &[
    "between",
    "difference",
    "differ",
    "compare",
    "together",
    "interact",
    "of them",
    "those",
    "the result",
];
const DETERMINERS: &[&str] = &["the", "its", "their", "every", "any", "all"];
const STOP: &[&str] = &[
    "the", "a", "an", "and", "or", "of", "to", "in", "on", "for", "is", "are", "be", "it", "its",
    "their", "that", "this", "these", "with", "by", "from", "at", "as", "where", "how", "which",
    "what", "when", "why", "does", "do",
];

struct Patterns {
    sentence: Regex,
    list_item: Regex,
    log_line: Regex,
    bare: Regex,
    word: Regex,
}

fn patterns() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| Patterns {
        sentence: Regex::new(r"[.?!](?:\s+|$)").unwrap(),
        list_item: Regex::new(r"^\s*(?:\d+[.)]|[-*•])\s+(.+)$").unwrap(),
        log_line: Regex::new(r"(?m)^\s*(?:at |Traceback|thread '.*' panicked|error\[|File \x22)")
            .unwrap(),
        bare: Regex::new(r"^[\w./:#-]+$").unwrap(),
        word: Regex::new(r"[A-Za-z0-9_]+").unwrap(),
    })
}

/// The separate needs of `prompt`, each with the prompt's subject, or nothing
/// when it asks one thing or its shape is not plain enough to split.
pub fn parts(prompt: &str) -> Vec<String> {
    let p = patterns();
    if prompt.contains("```") || p.log_line.find_iter(prompt).count() >= 2 {
        return Vec::new();
    }
    let lines: Vec<&str> = prompt.lines().collect();
    if let Some(found) = listed(&lines) {
        return found;
    }
    let text = lines.join(" ");
    let sentences = sentences(&text);
    if lowered(&text).contains_any(RELATIONS) {
        return Vec::new();
    }
    let questions: Vec<&str> = sentences
        .iter()
        .copied()
        .filter(|sentence| sentence.ends_with('?') && content_words(sentence) >= 3)
        .collect();
    if questions.len() >= 2 && questions.len() == sentences.len() {
        return checked(
            questions
                .iter()
                .map(|q| q.trim_end_matches('?').to_owned())
                .collect(),
            None,
        );
    }
    for (index, sentence) in sentences.iter().enumerate() {
        // "Subject: where A, and where B" keeps the subject before the colon.
        let (before, body) = match sentence.split_once(": ") {
            Some((before, body)) if !before.contains(' ') || word_count(before) <= 8 => {
                (Some(before), body)
            }
            _ => (None, *sentence),
        };
        let body = body.trim_end_matches(['.', '?', '!']);
        let Some(items) = clauses(body).or_else(|| things(body)) else {
            continue;
        };
        let lead = match before {
            Some(before) => lead_in(before),
            // Asked in a later sentence: the first one says what it is about.
            None if index > 0 => lead_in(sentences[0]),
            None => None,
        };
        return checked(items, lead);
    }
    Vec::new()
}

/// Two to four list lines, with the line before them as the subject.
fn listed(lines: &[&str]) -> Option<Vec<String>> {
    let p = patterns();
    let first = lines.iter().position(|line| p.list_item.is_match(line))?;
    let items: Vec<String> = lines[first..]
        .iter()
        .map_while(|line| p.list_item.captures(line))
        .map(|caps| caps[1].trim().trim_end_matches(['.', ';', ',']).to_owned())
        .collect();
    if items.len() < 2 {
        return None;
    }
    let lead = first
        .checked_sub(1)
        .map(|before| lines[before].trim().trim_end_matches(':'))
        .and_then(lead_in);
    Some(checked(items, lead))
}

/// "where A, where B, and where C": clauses that each open with a question word.
fn clauses(body: &str) -> Option<Vec<String>> {
    let body = strip_word(body, REQUEST_VERBS)
        .or_else(|| strip_word(body, LEAD_VERBS))
        .unwrap_or(body);
    let lowered = body.to_ascii_lowercase();
    let opens = |at: usize| {
        QUESTION_WORDS
            .iter()
            .any(|word| lowered[at..].starts_with(&format!("{word} ")))
    };
    if !opens(0) {
        return None;
    }
    let mut cuts = vec![0];
    for separator in [", and ", "; ", ", ", " and "] {
        let mut from = 0;
        while let Some(at) = lowered[from..].find(separator).map(|at| at + from) {
            let next = at + separator.len();
            if opens(next)
                && !cuts
                    .iter()
                    .any(|&cut| cut > at.saturating_sub(2) && cut <= next)
            {
                cuts.push(next);
            }
            from = next;
        }
    }
    cuts.sort_unstable();
    (cuts.len() >= 2).then(|| split_at(body, &cuts))
}

/// "Return the lookup guards and the shared definition": things named after a
/// request verb, each opening with a determiner.
fn things(body: &str) -> Option<Vec<String>> {
    let rest = strip_word(body, REQUEST_VERBS)?;
    let lowered = rest.to_ascii_lowercase();
    let opens = |at: usize| {
        DETERMINERS
            .iter()
            .any(|word| lowered[at..].starts_with(&format!("{word} ")))
    };
    if !opens(0) {
        return None;
    }
    let mut cuts = vec![0];
    for separator in [", and ", ", as well as ", " as well as ", ", ", " and "] {
        let mut from = 0;
        while let Some(at) = lowered[from..].find(separator).map(|at| at + from) {
            let next = at + separator.len();
            if opens(next)
                && !cuts
                    .iter()
                    .any(|&cut| cut > at.saturating_sub(2) && cut <= next)
            {
                cuts.push(next);
            }
            from = next;
        }
    }
    cuts.sort_unstable();
    (cuts.len() >= 2).then(|| split_at(rest, &cuts))
}

/// `text` cut at `cuts`, each piece without the separator that ended it.
fn split_at(text: &str, cuts: &[usize]) -> Vec<String> {
    cuts.iter()
        .enumerate()
        .map(|(index, &start)| {
            let end = cuts.get(index + 1).copied().unwrap_or(text.len());
            text[start..end]
                .trim()
                .trim_end_matches(',')
                .trim_end_matches(';')
                .trim_end_matches(" and")
                .trim_end_matches(" as well as")
                .trim_end_matches(',')
                .trim()
                .to_owned()
        })
        .collect()
}

/// The parts, each with the subject, when every one is a need of its own.
fn checked(items: Vec<String>, lead: Option<String>) -> Vec<String> {
    let p = patterns();
    let items: Vec<String> = items
        .into_iter()
        .filter(|item| !lowered(item).starts_with_any(CONSTRAINTS))
        .collect();
    let plain = (2..=MAX_PARTS).contains(&items.len())
        && items.iter().all(|item| {
            content_words(item) >= MIN_CONTENT_WORDS
                && !p.bare.is_match(item)
                && !lowered(item).contains_any(RELATIONS)
        });
    if !plain {
        return Vec::new();
    }
    items
        .into_iter()
        .map(|item| match &lead {
            Some(lead) => format!("{lead}: {item}"),
            None => item,
        })
        .collect()
}

/// The subject of a request: its first clause, without the request verb or
/// the conditions on it, in at most `MAX_LEAD_WORDS` words.
fn lead_in(sentence: &str) -> Option<String> {
    let mut text = sentence
        .trim()
        .trim_end_matches(['.', ':', '?', '!'])
        .to_owned();
    if let Some(rest) = strip_word(&text, LEAD_VERBS) {
        text = rest.to_owned();
    }
    if let Some(rest) = strip_word(&text, &["how"]) {
        text = rest.to_owned();
    }
    let lowered = text.to_ascii_lowercase();
    let cut = CONSTRAINTS
        .iter()
        .filter_map(|word| lowered.find(&format!(" {word} ")))
        .min();
    if let Some(cut) = cut {
        text.truncate(cut);
    }
    // "from the trait to the engine": the start of the path names it.
    let lowered = text.to_ascii_lowercase();
    if lowered.contains(" from ") {
        for ending in [" to ", " into "] {
            if let Some(at) = lowered.rfind(ending)
                && lowered[..at].contains(" from ")
            {
                text.truncate(at);
                break;
            }
        }
    }
    // A clause after a comma is a second thing, not more of the subject.
    if let Some(at) = text.find(", ") {
        text.truncate(at);
    }
    let mut words: Vec<&str> = text.split_whitespace().take(MAX_LEAD_WORDS).collect();
    while words
        .last()
        .is_some_and(|word| STOP.contains(&word.to_ascii_lowercase().as_str()))
    {
        words.pop();
    }
    (content_words(&words.join(" ")) >= MIN_CONTENT_WORDS).then(|| words.join(" "))
}

/// `text` after one of `words` and a space, ignoring case.
fn strip_word<'a>(text: &'a str, words: &[&str]) -> Option<&'a str> {
    let lowered = text.to_ascii_lowercase();
    words
        .iter()
        .find(|word| lowered.starts_with(&format!("{word} ")))
        .map(|word| text[word.len() + 1..].trim_start())
}

fn sentences(text: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut start = 0;
    for end in patterns().sentence.find_iter(text) {
        let sentence = text[start..end.start() + 1].trim();
        if !sentence.is_empty() {
            found.push(sentence);
        }
        start = end.end();
    }
    let rest = text[start..].trim();
    if !rest.is_empty() {
        found.push(rest);
    }
    found
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

fn content_words(text: &str) -> usize {
    patterns()
        .word
        .find_iter(text)
        .filter(|word| !STOP.contains(&word.as_str().to_ascii_lowercase().as_str()))
        .count()
}

struct Lowered(String);

fn lowered(text: &str) -> Lowered {
    Lowered(format!(" {} ", text.to_ascii_lowercase()))
}

impl Lowered {
    fn contains_any(&self, words: &[&str]) -> bool {
        words
            .iter()
            .any(|word| self.0.contains(&format!(" {word} ")))
    }
    fn starts_with_any(&self, words: &[&str]) -> bool {
        words
            .iter()
            .any(|word| self.0.starts_with(&format!(" {word} ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_where_clauses_after_return_take_the_first_sentence_as_subject() {
        assert_eq!(
            parts(
                "Trace how the standard printer records bytes printed for a single search \
                 without losing the lifetime count. Return where the per-search counter resets, \
                 where byte statistics are recorded at search finish, and where successful \
                 writes update the counter."
            ),
            [
                "the standard printer records bytes printed for a single search: where the per-search counter resets",
                "the standard printer records bytes printed for a single search: where byte statistics are recorded at search finish",
                "the standard printer records bytes printed for a single search: where successful writes update the counter",
            ]
        );
        assert_eq!(
            parts(
                "Trace replacement expansion from the Captures trait to the byte interpolation \
                 engine. Return where capture indices slice the haystack, where doubled dollar \
                 signs become one literal dollar, and where named references are resolved."
            )[0],
            "replacement expansion from the Captures trait: where capture indices slice the haystack"
        );
    }

    #[test]
    fn things_after_a_request_verb_and_a_subject_before_a_colon() {
        assert_eq!(
            parts(
                "Trace how a dot-separated action name is decoded and resolved without \
                 traversing inherited or prototype-related properties. Return the lookup guards \
                 and the shared forbidden-key definition."
            ),
            [
                "a dot-separated action name is decoded and resolved: the lookup guards",
                "a dot-separated action name is decoded and resolved: the shared forbidden-key definition",
            ]
        );
        assert_eq!(
            parts(
                "Trace remote image dimension probing: locate the initial and final URL \
                 authorization checks, and the shared helper that enforces protocol, domain, and \
                 remote-pattern authorization. Return these three decisions."
            ),
            [
                "remote image dimension probing: the initial and final URL authorization checks",
                "remote image dimension probing: the shared helper that enforces protocol, domain, and remote-pattern authorization",
            ]
        );
    }

    #[test]
    fn clauses_after_trace_and_subjects_that_stop_at_a_clause() {
        assert_eq!(
            parts(
                "Trace how response Content-Encoding values select decoders, how multiple \
                 decoders are combined, and why decoding runs in reverse application order. \
                 Return the response-side selection and the multi-decoder implementation."
            ),
            [
                "how response Content-Encoding values select decoders",
                "how multiple decoders are combined",
                "why decoding runs in reverse application order",
            ]
        );
        assert_eq!(
            parts(
                "Trace how the CLI hidden-file setting reaches the directory traversal builder \
                 and then the ignore matcher that rejects hidden directory entries. Return the \
                 CLI wiring and the matcher decision."
            ),
            [
                "the CLI hidden-file setting reaches the directory traversal builder: the CLI wiring",
                "the CLI hidden-file setting reaches the directory traversal builder: the matcher decision",
            ]
        );
    }

    #[test]
    fn lists_and_several_questions() {
        assert_eq!(
            parts(
                "Look at the prefetch hook:\n1. where the prompt is read\n2. how the timeout is \
                 enforced\n3. where results are formatted"
            ),
            [
                "the prefetch hook: where the prompt is read",
                "the prefetch hook: how the timeout is enforced",
                "the prefetch hook: where results are formatted",
            ]
        );
        assert_eq!(
            parts(
                "Where is the watcher debounce set? How is a rescan triggered after a branch \
                 switch?"
            ),
            [
                "Where is the watcher debounce set",
                "How is a rescan triggered after a branch switch",
            ]
        );
    }

    #[test]
    fn one_need_stays_one_question() {
        for prompt in [
            "Find where max_tokens and timeout_ms are set.",
            "Fix the bug where the watcher misses renames and the index goes stale.",
            "Update src/jev.rs, src/answer.rs and src/prefetch.rs to use the new client.",
            "What is the difference between ChunkIndex and FileIndex?",
            "Which structs implement Searcher, and which of them are used by the MCP server?",
            "Return file paths and line numbers.",
            "Trace how config is loaded without reading env vars and without touching the cache.",
            "Rename SearchHit to Hit and update all callers.",
            "How do the parser and the lexer share the span table?",
            "Where are Alpha, Beta, Gamma, Delta and Epsilon registered?",
            "Where does Astro identify internal paths that should be excluded from normal URL handling, including backslash-prefixed host-like paths?",
            "thread 'main' panicked at src/main.rs:3\nat foo\nat bar\nwhy does this happen and how do I fix it?",
        ] {
            assert!(parts(prompt).is_empty(), "{prompt}: {:?}", parts(prompt));
        }
    }
}
