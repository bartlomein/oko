use crate::stemmer::stemmer;
use anyhow::{Context, Result, bail};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    fs,
    path::Path,
    process::Command,
    sync::OnceLock,
};

pub const MAX_FILE_BYTES: usize = 256 * 1024;
pub const CHUNK_LINES: usize = 40;
pub const CHUNK_OVERLAP: usize = 5;
pub const FUNCTION_CHUNK_LINES: usize = 120;
pub const SHORTLIST_LIMIT: usize = 30;
pub const RESULT_LIMIT: usize = 5;
const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "are", "by", "do", "does", "for", "how", "in", "is", "it", "its", "of", "on",
    "or", "the", "this", "that", "to", "what", "when", "where", "which", "who", "why",
];
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Chunk {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub text: String,
    pub lexical_score: f64,
}
struct Patterns {
    acronym: Regex,
    camel: Regex,
    words: Regex,
    lines: Regex,
    extension: Regex,
    declaration: Regex,
    comment: Regex,
}
fn patterns() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| {
        // ECMAScript whitespace, excluding Rust regex's additional U+0085.
        let ws = "[\\t\\n\\v\\f\\r \\u{00a0}\\u{1680}\\u{2000}-\\u{200a}\\u{2028}\\u{2029}\\u{202f}\\u{205f}\\u{3000}\\u{feff}]";
        Patterns {
            acronym: Regex::new("([A-Z]+)([A-Z][a-z])").unwrap(),
            camel: Regex::new("([a-z0-9])([A-Z])").unwrap(),
            words: Regex::new("[a-z0-9]+").unwrap(),
            lines: Regex::new("\\r\\n|\\n|\\r").unwrap(),
            extension: Regex::new(r"\.(?:rs|[cm]?js|jsx|ts|tsx|py)$").unwrap(),
            declaration: Regex::new(&format!(r"^{ws}*(?:(?:pub(?:\([^)]*\))?|async|unsafe|const|export|default){ws}+)*(?:fn|function\*?|def){ws}+[A-Za-z0-9_]+")).unwrap(),
            comment: Regex::new(&format!(r"^{ws}*(?://|#|/\*\*|\*)")).unwrap(),
        }
    })
}
pub fn tokenize(value: &str) -> Vec<String> {
    let p = patterns();
    let a = p.acronym.replace_all(value, "$1 $2");
    let b = p.camel.replace_all(&a, "$1 $2").to_lowercase();
    p.words
        .find_iter(&b)
        .map(|m| m.as_str().to_owned())
        .collect()
}
pub fn compare_text(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}
pub fn compare_chunks(a: &Chunk, b: &Chunk) -> Ordering {
    b.lexical_score
        .partial_cmp(&a.lexical_score)
        .unwrap_or(Ordering::Equal)
        .then_with(|| compare_text(&a.path, &b.path))
        .then(a.start_line.cmp(&b.start_line))
        .then(a.end_line.cmp(&b.end_line))
}
pub fn chunk_text(path: &str, text: &str) -> Vec<Chunk> {
    let p = patterns();
    let mut lines: Vec<&str> = p.lines.split(text).collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    let mut declarations = HashSet::new();
    let mut boundaries = vec![0];
    if p.extension.is_match(path) {
        for (i, line) in lines.iter().enumerate() {
            if !p.declaration.is_match(line) {
                continue;
            }
            let mut start = i;
            while start > 0 && p.comment.is_match(lines[start - 1]) {
                start -= 1;
            }
            declarations.insert(start);
            if !boundaries.contains(&start) {
                boundaries.push(start);
            }
        }
    }
    if !boundaries.contains(&lines.len()) {
        boundaries.push(lines.len());
    }
    let mut chunks = vec![];
    for section in boundaries.windows(2) {
        let (mut start, section_end) = (section[0], section[1]);
        let limit = if declarations.contains(&start) {
            FUNCTION_CHUNK_LINES
        } else {
            CHUNK_LINES
        };
        while start < section_end {
            let end = section_end.min(start + limit);
            chunks.push(Chunk {
                path: path.into(),
                start_line: start + 1,
                end_line: end,
                text: lines[start..end].join("\n"),
                lexical_score: 0.0,
            });
            if end == section_end {
                break;
            }
            start += limit - CHUNK_OVERLAP;
        }
    }
    chunks
}
fn normalize(term: String, stems: &mut HashMap<String, String>) -> String {
    stems.entry(term).or_insert_with_key(|s| stemmer(s)).clone()
}
pub fn rank_lexically(chunks: &[Chunk], question: &str) -> Vec<Chunk> {
    let mut stems = HashMap::new();
    let terms: HashSet<String> = tokenize(question)
        .into_iter()
        .filter(|s| !STOP_WORDS.contains(&s.as_str()))
        .map(|s| normalize(s, &mut stems))
        .collect();
    if terms.is_empty() {
        return vec![];
    }
    let mut ranked = Vec::new();
    for chunk in chunks {
        let content: HashSet<String> = tokenize(&chunk.text)
            .into_iter()
            .map(|s| normalize(s, &mut stems))
            .collect();
        let path: HashSet<String> = tokenize(&chunk.path)
            .into_iter()
            .map(|s| normalize(s, &mut stems))
            .collect();
        let score = terms
            .iter()
            .map(|s| if content.contains(s) { 10 } else { 0 })
            .sum::<usize>()
            + terms
                .iter()
                .map(|s| if path.contains(s) { 3 } else { 0 })
                .sum::<usize>();
        if score > 0 {
            let mut c = chunk.clone();
            c.lexical_score = score as f64;
            ranked.push(c);
        }
    }
    ranked.sort_by(compare_chunks);
    ranked.truncate(SHORTLIST_LIMIT);
    ranked
}
fn js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{000b}' | '\u{000c}' | '\r' | ' ' | '\u{00a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}
pub fn search_workspace(cwd: &Path, question: &str) -> Result<Vec<Chunk>> {
    let output = Command::new("rg")
        .arg("--files")
        .current_dir(cwd)
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!("Could not run `rg --files`; install ripgrep and try again.")
            } else {
                anyhow::Error::new(e)
            }
        })?;
    if !matches!(output.status.code(), Some(0 | 1)) {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "File discovery failed: {}",
            if stderr.trim().is_empty() {
                format!(
                    "rg exited with code {}",
                    output
                        .status
                        .code()
                        .map_or_else(|| "null".into(), |c| c.to_string())
                )
            } else {
                stderr.trim().to_owned()
            }
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut files: Vec<String> = stdout
        .split('\n')
        .map(|s| s.strip_suffix('\r').unwrap_or(s))
        .filter(|s| !s.is_empty())
        .map(|s| s.replace('\\', "/"))
        .collect();
    files.sort_by(|a, b| compare_text(a, b));
    files.dedup();
    let mut chunks = vec![];
    for file in files {
        let read = || -> Result<Option<String>> {
            let path = cwd.join(&file);
            if fs::metadata(&path)?.len() > MAX_FILE_BYTES as u64 {
                return Ok(None);
            }
            let bytes = fs::read(path).context("reading search file")?;
            if bytes.len() > MAX_FILE_BYTES || bytes.contains(&0) {
                return Ok(None);
            }
            // TextDecoder strips one leading UTF-8 BOM by default.
            let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
            Ok(std::str::from_utf8(bytes).ok().map(str::to_owned))
        };
        if let Ok(Some(text)) = read()
            && !text.trim_matches(js_whitespace).is_empty()
        {
            chunks.extend(chunk_text(&file, &text));
        }
    }
    Ok(rank_lexically(&chunks, question))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn code_names_and_word_variants() {
        assert_eq!(
            tokenize("HTTPServer parseRaceboxCSV save_project_v2"),
            [
                "http", "server", "parse", "racebox", "csv", "save", "project", "v2"
            ]
        );
        let chunks = chunk_text("project.rs", "fn atomic_rename() {}\n");
        assert_eq!(
            rank_lexically(&chunks, "where is renaming done atomically")[0].lexical_score,
            20.0
        );
    }
    #[test]
    fn function_boundaries_and_overlap() {
        let text = format!(
            "header\r\n// doc\r\npub(crate) async fn parse() {{\r\n{}",
            vec!["body"; 125].join("\r")
        );
        let c = chunk_text("x.rs", &text);
        assert_eq!(
            c.iter()
                .map(|c| (c.start_line, c.end_line))
                .collect::<Vec<_>>(),
            [(1, 1), (2, 121), (117, 128)]
        );
        assert!(chunk_text("x.rs", "").is_empty());
    }
    #[test]
    fn deterministic_utf16_ties_and_limit() {
        assert_eq!(compare_text("\u{10000}", "\u{e000}"), Ordering::Less);
        let chunks: Vec<_> = (0..40)
            .rev()
            .flat_map(|i| chunk_text(&format!("{i:02}.txt"), "needle"))
            .collect();
        let ranked = rank_lexically(&chunks, "needle");
        assert_eq!(ranked.len(), 30);
        assert_eq!(ranked[0].path, "00.txt");
        assert_eq!(ranked[29].path, "29.txt");
        assert!(rank_lexically(&chunks, "where is it").is_empty());
    }
    #[test]
    fn workspace_ignores_and_decoding_limits() {
        let temp = std::env::temp_dir().join(format!(
            "oko-rust-search-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(temp.join(".git")).unwrap();
        fs::write(temp.join(".gitignore"), "ignored.ts\n").unwrap();
        fs::write(temp.join("good.ts"), "const needle = true;\n").unwrap();
        fs::write(temp.join("bom.txt"), b"\xef\xbb\xbfneedle").unwrap();
        fs::write(temp.join("ignored.ts"), "needle").unwrap();
        fs::write(temp.join(".hidden"), "needle").unwrap();
        fs::write(temp.join("binary.dat"), b"needle\0").unwrap();
        fs::write(temp.join("invalid.dat"), b"needle\xc3\x28").unwrap();
        fs::write(temp.join("large.txt"), vec![b'n'; MAX_FILE_BYTES + 1]).unwrap();
        let result = search_workspace(&temp, "needle");
        fs::remove_dir_all(&temp).unwrap();
        let result = result.unwrap();
        assert_eq!(
            result.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(),
            ["bom.txt", "good.ts"]
        );
        assert_eq!(result[0].text, "needle");
    }
}
