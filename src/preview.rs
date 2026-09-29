//! Bounded, query-focused source previews for bounded Jev ranking requests.
//!
//! Previews are ranking input only. Their IDs refer to the original chunks, so
//! callers can return unabridged source after ranking without inventing lines.
use crate::{
    ranking::{self, RankItem, RankingIntent},
    search::{self, Chunk},
    stemmer::stemmer,
};
use anyhow::{Result, bail};
use regex::Regex;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::OnceLock,
};

const DESIRED_TEXT_BYTES: usize = 1_200;
const MIN_TEXT_BYTES: usize = 64;
const HEADER_LINES: usize = 4;
const SIGNATURE_SCAN_LINES: usize = 64;
const CONTEXT_LINES: usize = 12;
const BLOCK_LINES: usize = 12;
// The shortlist's first candidates hold most of the needed code (360 of 404
// expected locations in the 0.6.0 agent suite were in the first 20), so when
// every candidate cannot have a full preview, the first 15 get the room and
// the rest keep their essentials: anchor, signature, first body evidence and
// decision block.
const HEAD_PREVIEWS: usize = 15;
const ESSENTIAL_PREVIEW_BYTES: usize = 640;
const OMITTED: &str = "[omitted]";
const TRUNCATED: &str = "[truncated]";

/// How previews are shaped.
///
/// The shortlist the ranker accepts from is `judged`: the first candidates get
/// the room, imports are not evidence, and a section of several definitions
/// is previewed on the one that matches. Candidates that can only be named in
/// a list (connected files, further keyword matches) keep even shares of the
/// whole chunk: shaped like the shortlist, they fell out of Agent Retrieval
/// Bench's top 20 (R@20 on tests and ripple tasks −0.04 against 0.6.0 run
/// the same day), and agents never read them as excerpts.
#[derive(Clone, Copy)]
struct Shape {
    /// The first candidates get the room when not all fit in full.
    head: bool,
    /// Imports are not evidence, and a section of several definitions is
    /// previewed on the one that matches.
    focused: bool,
}
impl Shape {
    fn judged() -> Self {
        Self {
            head: true,
            focused: true,
        }
    }
    fn listed() -> Self {
        Self {
            head: false,
            focused: false,
        }
    }
}

/// Recover attached declaration context that chunk boundaries put in the
/// previous candidate. Only an adjacent source-backed prefix is added; IDs
/// still refer to the caller's original candidates in exactly the same order.
pub fn ranking_previews_with_context(
    question: &str,
    chunks: &[Chunk],
    corpus: &[Chunk],
    intent: RankingIntent,
) -> Result<Vec<RankItem>> {
    contextual_previews(
        question,
        chunks,
        corpus,
        intent,
        DESIRED_TEXT_BYTES,
        Shape::judged(),
    )
}

/// The same for candidates that can only be named in a list (connected
/// files, further keyword matches).
pub fn listing_previews_with_context(
    question: &str,
    chunks: &[Chunk],
    corpus: &[Chunk],
    intent: RankingIntent,
) -> Result<Vec<RankItem>> {
    contextual_previews(
        question,
        chunks,
        corpus,
        intent,
        DESIRED_TEXT_BYTES,
        Shape::listed(),
    )
}

/// A smaller recovery batch gets fuller evidence within the same wire budget.
pub fn recovery_previews_with_context(
    question: &str,
    chunks: &[Chunk],
    corpus: &[Chunk],
    intent: RankingIntent,
) -> Result<Vec<RankItem>> {
    contextual_previews(question, chunks, corpus, intent, 3_000, Shape::judged())
}

fn contextual_previews(
    question: &str,
    chunks: &[Chunk],
    corpus: &[Chunk],
    intent: RankingIntent,
    desired_bytes: usize,
    shape: Shape,
) -> Result<Vec<RankItem>> {
    for chunk in chunks.iter().take(ranking::MAX_ITEMS) {
        validate_chunk(chunk)?;
    }
    let paths: HashSet<_> = chunks
        .iter()
        .take(ranking::MAX_ITEMS)
        .map(|chunk| chunk.path.as_str())
        .collect();
    let mut by_path: HashMap<&str, Vec<&Chunk>> = HashMap::new();
    for chunk in corpus {
        if paths.contains(chunk.path.as_str()) && validate_chunk(chunk).is_ok() {
            by_path.entry(&chunk.path).or_default().push(chunk);
        }
    }
    let enriched: Vec<_> = chunks
        .iter()
        .take(ranking::MAX_ITEMS)
        .map(|chunk| {
            let mut result = chunk.clone();
            if chunk.start_line <= 1
                || !chunk
                    .text
                    .lines()
                    .next()
                    .is_some_and(|line| is_declaration(line, &chunk.path))
            {
                return result;
            }
            let start = chunk.start_line.saturating_sub(CONTEXT_LINES).max(1);
            let mut preceding = BTreeMap::new();
            let mut conflicting = HashSet::new();
            for prior in by_path.get(chunk.path.as_str()).into_iter().flatten() {
                if prior.end_line < start || prior.start_line >= chunk.start_line {
                    continue;
                }
                for (offset, line) in prior.text.split('\n').enumerate() {
                    let number = prior.start_line + offset;
                    if number >= start && number < chunk.start_line {
                        if preceding.get(&number).is_some_and(|&old| old != line) {
                            conflicting.insert(number);
                        } else {
                            preceding.insert(number, line);
                        }
                    }
                }
            }
            preceding.retain(|number, _| !conflicting.contains(number));
            let mut adjacent = Vec::new();
            for number in (start..chunk.start_line).rev() {
                let Some(&line) = preceding.get(&number) else {
                    break;
                };
                adjacent.push(line);
            }
            adjacent.reverse();
            let prefix = declaration_context(&adjacent, adjacent.len());
            if let Some(&first) = prefix.first() {
                result.start_line -= adjacent.len() - first;
                result.text = format!("{}\n{}", adjacent[first..].join("\n"), chunk.text);
            }
            result
        })
        .collect();
    previews_with_budget(question, &enriched, intent, desired_bytes, shape)
}

/// Build previews without changing candidate order, IDs, or original chunks.
///
/// The existing request builder is the authority on JSON size, including
/// escaped text, paths, the question, and intent instructions. Shrink all
/// previews before allowing it to omit trailing candidates as a last resort.
pub fn ranking_previews(
    question: &str,
    chunks: &[Chunk],
    intent: RankingIntent,
) -> Result<Vec<RankItem>> {
    previews_with_budget(
        question,
        chunks,
        intent,
        DESIRED_TEXT_BYTES,
        Shape::judged(),
    )
}

fn previews_with_budget(
    question: &str,
    chunks: &[Chunk],
    intent: RankingIntent,
    desired_bytes: usize,
    shape: Shape,
) -> Result<Vec<RankItem>> {
    if chunks.is_empty() {
        return Ok(Vec::new());
    }
    let terms: HashSet<_> = search::tokenize(question)
        .into_iter()
        .filter(|word| {
            ![
                "a", "an", "and", "are", "as", "at", "be", "by", "do", "does", "for", "from",
                "how", "in", "is", "it", "its", "of", "on", "or", "the", "this", "that", "to",
                "what", "when", "where", "which", "who", "why", "with",
            ]
            .contains(&word.as_str())
        })
        .map(|word| stemmer(&word))
        .collect();
    let mut stems = HashMap::new();
    let prepared = chunks
        .iter()
        .take(ranking::MAX_ITEMS)
        .map(|chunk| PreparedPreview::new(chunk, &terms, &mut stems, shape.focused))
        .collect::<Result<Vec<_>>>()?;
    let items_at = |budget| {
        prepared
            .iter()
            .enumerate()
            .map(|(index, preview)| RankItem {
                id: index.to_string(),
                text: preview.render(budget),
                source: Some(format!(
                    "{}:{}-{}",
                    preview.chunk.path, preview.chunk.start_line, preview.chunk.end_line
                )),
            })
            .collect::<Vec<_>>()
    };
    let desired = items_at(desired_bytes);
    if let Ok((_, retained)) = ranking::prepare_request_with_intent(question, &desired, intent)
        && retained.len() == desired.len()
    {
        return Ok(desired);
    }
    if shape.head && prepared.len() > HEAD_PREVIEWS {
        // The first candidates get the largest allowance that fits while the
        // rest keep their essentials; if even that does not fit, share evenly.
        let tiered = |head: usize| {
            prepared
                .iter()
                .enumerate()
                .map(|(index, preview)| RankItem {
                    id: index.to_string(),
                    text: if index < HEAD_PREVIEWS {
                        preview.render(head)
                    } else {
                        preview.render_essential(ESSENTIAL_PREVIEW_BYTES)
                    },
                    source: Some(format!(
                        "{}:{}-{}",
                        preview.chunk.path, preview.chunk.start_line, preview.chunk.end_line
                    )),
                })
                .collect::<Vec<_>>()
        };
        let fits = |items: &[RankItem]| {
            ranking::prepare_request_with_intent(question, items, intent)
                .is_ok_and(|(_, retained)| retained.len() == items.len())
        };
        let floor = tiered(ESSENTIAL_PREVIEW_BYTES);
        if fits(&floor) {
            let (mut low, mut high, mut best) = (ESSENTIAL_PREVIEW_BYTES, desired_bytes, floor);
            while high - low > 16 {
                let middle = low + (high - low) / 2;
                let items = tiered(middle);
                if fits(&items) {
                    low = middle;
                    best = items;
                } else {
                    high = middle;
                }
            }
            return Ok(best);
        }
    }

    let smallest = items_at(MIN_TEXT_BYTES);
    let (_, mut best) = ranking::prepare_request_with_intent(question, &smallest, intent)?;
    if best.len() != smallest.len() {
        return Ok(best);
    }
    // Use the largest common text allowance that fits all candidates. A short
    // source does not waste its allowance, so long snippets can retain context.
    let (mut low, mut high) = (MIN_TEXT_BYTES, desired_bytes);
    while high - low > 16 {
        let middle = low + (high - low) / 2;
        let items = items_at(middle);
        match ranking::prepare_request_with_intent(question, &items, intent) {
            Ok((_, retained)) if retained.len() == items.len() => {
                low = middle;
                best = items;
            }
            _ => high = middle,
        }
    }
    Ok(best)
}

fn validate_chunk(chunk: &Chunk) -> Result<()> {
    if chunk.start_line == 0
        || chunk.end_line < chunk.start_line
        || chunk.end_line - chunk.start_line != chunk.text.split('\n').count() - 1
    {
        bail!("Cannot preview a chunk with inconsistent source line numbers.");
    }
    Ok(())
}

struct PreparedPreview<'a> {
    chunk: &'a Chunk,
    lines: Vec<&'a str>,
    focus: Vec<usize>,
    priority: Vec<usize>,
    /// How many of `priority` are the essentials: the anchor, its
    /// annotations and signature, the first body evidence, a decision block.
    essential: usize,
}

impl<'a> PreparedPreview<'a> {
    fn new(
        chunk: &'a Chunk,
        terms: &HashSet<String>,
        stems: &mut HashMap<String, String>,
        focused: bool,
    ) -> Result<Self> {
        let lines: Vec<_> = chunk.text.split('\n').collect();
        validate_chunk(chunk)?;
        // An import names what the question asks about without doing it.
        let imports = if focused {
            search::import_lines(&chunk.path, &lines)
        } else {
            vec![false; lines.len()]
        };
        let mut scored = Vec::with_capacity(lines.len());
        let mut focus = Vec::with_capacity(lines.len());
        let mut matches = Vec::with_capacity(lines.len());
        for (index, line) in lines.iter().enumerate() {
            let lower = line.to_ascii_lowercase();
            let mut matching = HashSet::new();
            let mut first_match = None;
            for word in search::tokenize(line)
                .into_iter()
                .filter(|_| !imports[index])
            {
                // Find the original token, not its stem: e.g. "filing" stems
                // to "file", which need not occur verbatim in the source.
                let stem = stems
                    .entry(word.clone())
                    .or_insert_with_key(|word| stemmer(word));
                if terms.contains(stem) {
                    matching.insert(stem.clone());
                    if let Some(position) = lower.find(&word) {
                        first_match =
                            Some(first_match.map_or(position, |old: usize| old.min(position)));
                    }
                }
            }
            scored.push((index, matching.len()));
            focus.push(first_match.unwrap_or(0));
            matches.push(matching);
        }
        scored.sort_by_key(|&(index, matches)| (std::cmp::Reverse(matches), index));
        // A merged section holds several declarations; the one whose block
        // holds the most of the question heads the preview, not the one whose
        // own line does.
        let declarations: Vec<usize> = (0..lines.len())
            .filter(|&index| !imports[index] && is_declaration(lines[index], &chunk.path))
            .collect();
        let block_header = (focused && declarations.len() > 1)
            .then(|| {
                declarations
                    .iter()
                    .map(|&start| {
                        let end = declarations
                            .iter()
                            .copied()
                            .find(|&other| {
                                other > start
                                    && indentation(lines[other]) <= indentation(lines[start])
                            })
                            .unwrap_or(lines.len());
                        let covered: HashSet<&String> =
                            matches[start..end].iter().flatten().collect();
                        (start, covered.len())
                    })
                    .max_by_key(|&(start, covered)| (covered, std::cmp::Reverse(start)))
                    .filter(|&(_, covered)| covered > 0)
                    .map(|(start, _)| start)
            })
            .flatten();
        let header = block_header
            .or_else(|| {
                scored
                    .iter()
                    .map(|&(index, _)| index)
                    .find(|&index| is_declaration(lines[index], &chunk.path))
            })
            // A Markdown-shaped line may instead be a Python/shell comment.
            // Prefer an actual declaration whenever this chunk contains one.
            .or_else(|| {
                scored
                    .iter()
                    .map(|&(index, _)| index)
                    .find(|&index| is_heading(lines[index]))
            });
        let anchor = header.unwrap_or_else(|| scored[0].0);
        let mut priority = vec![anchor];
        let mut documentation = Vec::new();
        if let Some(header) = header {
            focus[header] = 0;
            if is_declaration(lines[header], &chunk.path) {
                // Keep source-backed annotations and documentation attached to
                // the declaration. These let the ranker distinguish tests,
                // routes, wrappers, and implementations without guessed labels.
                let context = declaration_context(&lines, header);
                // Attributes affect what this declaration represents, whereas
                // a long prose block must not crowd out evidence of its work.
                let annotation = context
                    .iter()
                    .copied()
                    .find(|&index| is_annotation_start(lines[index].trim()));
                for index in context {
                    if annotation.is_some_and(|start| index >= start) {
                        priority.push(index);
                    } else {
                        documentation.push(index);
                    }
                }
                if let Some(end) = signature_end(&lines, header, &chunk.path) {
                    // Long parameter lists must not crowd out the actual work.
                    // Prefer a matching body statement, then a nearby statement
                    // if none match. Comments remain available through coverage.
                    let body_end = ((end + 1)..lines.len())
                        .find(|&index| {
                            is_declaration(lines[index], &chunk.path)
                                && indentation(lines[index]) <= indentation(lines[header])
                        })
                        .unwrap_or(lines.len());
                    if let Some(&(index, _)) = scored.iter().find(|&&(index, _)| {
                        index > end && index < body_end && is_body_evidence(lines[index])
                    }) {
                        priority.push(index);
                    }
                }
            }
            // Keep the beginning of the signature, including names that start
            // on a subsequent line, without spending the entire body allowance.
            for index in header..(header + HEADER_LINES).min(lines.len()) {
                if index != header {
                    priority.push(index);
                }
                focus[index] = 0;
                let line = lines[index].trim_end();
                if line.contains('{')
                    || line.contains("=>")
                    || line.ends_with(':')
                    || line.ends_with(';')
                    || (index == header && line.trim_start().starts_with('#'))
                {
                    break;
                }
            }
        }
        // A predicate alone cannot show whether matching records are retained,
        // rejected or transformed. Reserve a small, contiguous decision block
        // before isolated keyword matches spend the preview allowance.
        if let Some(block) = scored.iter().find_map(|&(index, count)| {
            (count > 0)
                .then(|| decision_block(&lines, index, &chunk.path))
                .flatten()
        }) {
            priority.extend(block);
        }
        // Counted before duplicates are dropped; they keep their places first.
        let essentials: HashSet<usize> = priority.iter().copied().collect();
        // Prefer covering different parts of the question before repeating
        // the same keyword-heavy comments or diagnostics. This is source- and
        // language-independent, and bounded even for a very long question.
        let mut covered = priority
            .iter()
            .flat_map(|&index| matches[index].iter().cloned())
            .collect::<HashSet<_>>();
        // Evidence inside the heading definition comes first: a type alias
        // above a function can name more of the question than the checks
        // inside it do.
        // The heading declaration's span: from the comments and attributes
        // above it to the end of its body (the first line back at its depth
        // after the signature; a closing brace belongs to the body).
        let python = matches!(chunk.path.rsplit('.').next(), Some("py" | "pyi"));
        let span = |declaration: usize| {
            let start = declaration_context(&lines, declaration)
                .first()
                .copied()
                .unwrap_or(declaration);
            let from = signature_end(&lines, declaration, &chunk.path).unwrap_or(declaration) + 1;
            let depth = indentation(lines[declaration]);
            let end = (from..lines.len())
                .find(|&index| {
                    !lines[index].trim().is_empty() && indentation(lines[index]) <= depth
                })
                .map(|index| {
                    let closer = lines[index].trim_start().starts_with(['}', ')', ']']);
                    if closer && !python { index + 1 } else { index }
                })
                .unwrap_or(lines.len());
            (start, end.max(declaration + 1))
        };
        let block = header
            .filter(|&h| focused && is_declaration(lines[h], &chunk.path))
            .map(span);
        let inside = |index: usize| block.is_none_or(|(start, end)| start <= index && index < end);
        for _ in 0..8 {
            let Some((index, fresh)) = matches
                .iter()
                .enumerate()
                .filter(|&(index, _)| inside(index))
                .map(|(index, matching)| (index, matching.difference(&covered).count()))
                .max_by_key(|&(index, fresh)| {
                    (fresh, matches[index].len(), std::cmp::Reverse(index))
                })
            else {
                break;
            };
            if fresh == 0 {
                break;
            }
            covered.extend(matches[index].iter().cloned());
            priority.push(index);
        }
        priority.extend(
            scored
                .iter()
                .filter(|&&(index, matches)| matches > 0 && inside(index))
                .map(|&(index, _)| index),
        );
        priority.extend(documentation);
        priority.extend(
            scored
                .iter()
                .filter(|&&(index, matches)| matches > 0 && !inside(index))
                .map(|&(index, _)| index),
        );
        // One line either side gives small statements and multiline headers
        // context, after retaining the strongest matches themselves.
        for index in priority.clone() {
            if index > 0 {
                priority.push(index - 1);
            }
            if index + 1 < lines.len() {
                priority.push(index + 1);
            }
        }
        priority.extend(0..lines.len());
        let mut seen = HashSet::new();
        priority.retain(|index| seen.insert(*index));
        // The ranker judges the definition that heads the preview. Outside
        // it, only declaration lines and lines holding part of the question
        // are shown, so a neighbour's long comment cannot make the section
        // look like something else.
        if let Some(block) = block {
            priority.retain(|&index| {
                (block.0 <= index && index < block.1)
                    || (!imports[index]
                        && (is_declaration(lines[index], &chunk.path)
                            || !matches[index].is_empty()))
            });
        }
        let essential = priority
            .iter()
            .take_while(|index| essentials.contains(index))
            .count();
        Ok(Self {
            chunk,
            lines,
            focus,
            priority,
            essential,
        })
    }

    fn render(&self, budget: usize) -> String {
        self.render_from(budget, &self.priority)
    }

    /// Only the essential lines, within `budget`.
    fn render_essential(&self, budget: usize) -> String {
        self.render_from(budget, &self.priority[..self.essential])
    }

    fn render_from(&self, budget: usize, priority: &[usize]) -> String {
        let mut selected = BTreeMap::new();
        // A wholly omitted chunk is one marker. Track JSON text size as lines
        // are inserted instead of repeatedly rendering every selected line.
        let mut encoded_bytes = OMITTED.len();
        for &index in priority {
            let previous = selected
                .range(..index)
                .next_back()
                .map_or(0, |(&index, _)| index + 1);
            let next = selected
                .range(index..)
                .next()
                .map_or(self.lines.len(), |(&index, _)| index);
            // Inserting into an omitted interval replaces that interval with
            // zero, one, or two gaps. A JSON newline costs two bytes.
            let gap_delta = i32::from(index > previous) + i32::from(index + 1 < next) - 1;
            let number_bytes = (self.chunk.start_line + index).to_string().len();
            let mut allowance = (budget / 3).clamp(24, 400);
            loop {
                let fragment = fragment(self.lines[index], self.focus[index], allowance);
                let mut proposed_bytes = encoded_bytes + number_bytes + 4 + escaped_len(&fragment);
                if gap_delta < 0 {
                    proposed_bytes -= OMITTED.len() + 2;
                } else {
                    proposed_bytes += gap_delta as usize * (OMITTED.len() + 2);
                }
                let excess = proposed_bytes.saturating_sub(budget);
                if excess == 0 {
                    selected.insert(index, fragment);
                    encoded_bytes = proposed_bytes;
                    break;
                }
                if allowance < excess + 24 {
                    break;
                }
                allowance -= excess;
            }
        }
        // Very large source line numbers still need an honest, nonempty
        // preview. The final serialized request check handles their overhead.
        if selected.is_empty() {
            let index = priority[0];
            selected.insert(index, fragment(self.lines[index], self.focus[index], 24));
        }
        self.render_selected(&selected)
    }

    fn render_selected(&self, selected: &BTreeMap<usize, String>) -> String {
        let mut rendered = String::new();
        let mut next = 0;
        for (&index, text) in selected {
            if index > next {
                rendered.push_str(OMITTED);
                rendered.push('\n');
            }
            rendered.push_str(&(self.chunk.start_line + index).to_string());
            rendered.push_str(": ");
            rendered.push_str(text);
            rendered.push('\n');
            next = index + 1;
        }
        if next < self.lines.len() {
            rendered.push_str(OMITTED);
        } else {
            rendered.pop();
        }
        rendered
    }
}

/// Only retain context actually present in the candidate; never invent test or
/// implementation labels from a name. Multiline annotations are bounded too.
fn declaration_context(lines: &[&str], header: usize) -> Vec<usize> {
    let earliest = header.saturating_sub(CONTEXT_LINES);
    let mut start = header;
    let mut pending = 0isize;
    for index in (earliest..header).rev() {
        let line = lines[index].trim();
        let annotation = is_annotation_start(line);
        let comment = line.starts_with("//")
            || line.starts_with('#')
            || line.starts_with("/*")
            || line.starts_with('*');
        let delta = line.chars().fold(0isize, |depth, ch| match ch {
            ')' | ']' => depth + 1,
            '(' | '[' => depth - 1,
            _ => depth,
        });
        if !annotation && !comment && pending <= 0 && delta <= 0 {
            break;
        }
        pending += delta;
        if annotation || comment {
            start = index;
            pending = 0;
        }
    }
    (start..header).collect()
}

fn is_annotation_start(line: &str) -> bool {
    line.starts_with('@')
        || line.starts_with("#[")
        || line.starts_with("[<")
        || (line.starts_with('[') && line.ends_with(']'))
}

fn indentation(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// A bounded preview hint, not a source-boundary parser. Delimiter balance
/// avoids treating Python parameter annotations as the end of a signature.
fn signature_end(lines: &[&str], header: usize, path: &str) -> Option<usize> {
    let mut parens = 0isize;
    let python = path.ends_with(".py") || path.ends_with(".pyi");
    for (index, line) in lines
        .iter()
        .enumerate()
        .take((header + SIGNATURE_SCAN_LINES).min(lines.len()))
        .skip(header)
    {
        let line = line.trim();
        if index > header && is_declaration(line, path) {
            return None;
        }
        for ch in line.chars() {
            match ch {
                '(' => parens += 1,
                ')' => parens -= 1,
                _ => {}
            }
        }
        if parens <= 0 && (line.ends_with('{') || (python && line.ends_with(':'))) {
            return Some(index);
        }
        if parens <= 0 && line.ends_with(';') {
            return None;
        }
    }
    None
}

fn is_body_evidence(line: &str) -> bool {
    let line = line.trim();
    !line.is_empty()
        && !line.starts_with("//")
        && !line.starts_with('#')
        && !line.starts_with("/*")
        && !line.starts_with('*')
        && line.chars().any(|ch| ch.is_alphanumeric())
}

/// A bounded context hint, not a parser or a claim that the block is complete.
/// Indentation identifies nearby children without counting delimiters inside
/// strings/comments. Unindented/minified code keeps the ordinary preview path.
fn decision_block(lines: &[&str], anchor: usize, path: &str) -> Option<Vec<usize>> {
    let extension = path.rsplit_once('.')?.1;
    if !matches!(
        extension,
        "rs" | "js"
            | "mjs"
            | "cjs"
            | "jsx"
            | "ts"
            | "tsx"
            | "py"
            | "pyi"
            | "go"
            | "java"
            | "cs"
            | "c"
            | "h"
            | "cc"
            | "cpp"
            | "hpp"
            | "swift"
            | "kt"
    ) {
        return None;
    }
    let line = lines[anchor].trim();
    let keyword = line.split(|ch: char| !ch.is_ascii_alphabetic()).next()?;
    if !matches!(keyword, "if" | "for" | "while" | "match" | "switch")
        || !(line.ends_with('{') || (matches!(extension, "py" | "pyi") && line.ends_with(':')))
    {
        return None;
    }
    let indent = indentation(lines[anchor]);
    let mut block = vec![anchor];
    for (index, line) in lines
        .iter()
        .enumerate()
        .take(anchor + BLOCK_LINES)
        .skip(anchor + 1)
    {
        if line.trim().is_empty() {
            block.push(index);
            continue;
        }
        if indentation(line) <= indent {
            if line.trim_start().starts_with('}') {
                block.push(index);
            }
            break;
        }
        block.push(index);
    }
    (block.len() > 1).then_some(block)
}

fn is_declaration(line: &str, path: &str) -> bool {
    static DECLARATION: OnceLock<Regex> = OnceLock::new();
    DECLARATION
        .get_or_init(|| {
            Regex::new(
                r"^\s*(?:(?:pub(?:\([^)]*\))?|async|unsafe|const|export|default|public|private|protected|static|final|override|abstract|internal|open|suspend)\s+)*(?:(?:fn|function\*?|def|fun|func|class|struct|interface|enum|trait|impl)\b|(?:const|let|var)\s+[A-Za-z_]\w*\s*=\s*(?:async\s+)?(?:function\b|(?:\([^\n)]*\)|[A-Za-z_]\w*)\s*=>))",
            )
            .expect("valid declaration hint regex")
        })
        .is_match(line)
        || is_typed_declaration(line, path)
}

fn is_typed_declaration(line: &str, path: &str) -> bool {
    if !path.rsplit_once('.').is_some_and(|(_, extension)| {
        matches!(extension, "java" | "cs" | "c" | "h" | "cc" | "cpp" | "hpp")
    }) {
        return false;
    }
    let first = line.split_ascii_whitespace().next().unwrap_or("");
    if matches!(
        first,
        "return" | "throw" | "else" | "if" | "for" | "while" | "switch" | "catch" | "new" | "do"
    ) {
        return false;
    }
    // Match the existing typed-symbol hints used by local retrieval. Keep the
    // same supported extensions and single-line signature scope, and exclude
    // control-flow keywords so `else if (...) {` is never a method header.
    static TYPED: OnceLock<Regex> = OnceLock::new();
    TYPED
        .get_or_init(|| {
            Regex::new(
                r"^[ \t]*(?:[A-Za-z_][A-Za-z0-9_.<>,?\[\]:*&]*[ \t]+)+([A-Za-z_][A-Za-z0-9_]*)[ \t]*\([^;\n]*\)[ \t]*(?:\{|throws\b)",
            )
            .expect("valid typed declaration hint regex")
        })
        .captures(line)
        .is_some_and(|captures| {
            !matches!(
                &captures[1],
                "if"
                    | "for"
                    | "while"
                    | "switch"
                    | "catch"
                    | "using"
                    | "lock"
                    | "foreach"
                    | "checked"
                    | "unchecked"
                    | "synchronized"
                    | "try"
            )
        })
}

fn is_heading(line: &str) -> bool {
    let line = line.trim_start();
    let hashes = line.bytes().take_while(|&byte| byte == b'#').count();
    (1..=6).contains(&hashes)
        && line[hashes..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace)
}

/// JSON string content size (the enclosing quotes are supplied by the request).
fn escaped_len(text: &str) -> usize {
    text.chars()
        .map(|ch| match ch {
            '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
            '\0'..='\u{1f}' => 6,
            _ => ch.len_utf8(),
        })
        .sum()
}

fn fragment(line: &str, focus: usize, budget: usize) -> String {
    if escaped_len(line) <= budget {
        return line.to_string();
    }
    let room = budget.saturating_sub(2 * (TRUNCATED.len() + 1)).max(1);
    let mut start = focus.saturating_sub(room / 3).min(line.len());
    while !line.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = start;
    let mut used = 0;
    for ch in line[start..].chars() {
        let cost = escaped_len(ch.encode_utf8(&mut [0; 4]));
        if used + cost > room && end > start {
            break;
        }
        used += cost;
        end += ch.len_utf8();
    }
    let mut result = String::new();
    if start > 0 {
        result.push_str(TRUNCATED);
        result.push(' ');
    }
    result.push_str(&line[start..end]);
    if end < line.len() {
        result.push(' ');
        result.push_str(TRUNCATED);
    }
    result
}

#[cfg(test)]
#[path = "preview_tests.rs"]
mod tests;
