use oko::search::{Chunk, SHORTLIST_LIMIT, chunk_text, rank_lexically};
use std::collections::HashSet;

fn source(path: &str, start_line: usize, text: &str) -> Chunk {
    Chunk {
        path: path.into(),
        start_line,
        end_line: start_line + text.lines().count().saturating_sub(1),
        text: text.into(),
        lexical_score: 0.0,
    }
}

fn locations(chunks: &[Chunk]) -> Vec<(&str, usize, usize)> {
    chunks
        .iter()
        .map(|chunk| (chunk.path.as_str(), chunk.start_line, chunk.end_line))
        .collect()
}

#[test]
fn distinct_implementations_in_one_file_survive_competing_prose() {
    for (extension, declaration) in [
        ("rs", "pub fn reconcile_invoice_batch_INDEX(input: &str) {"),
        (
            "ts",
            "export function reconcileInvoiceBatchINDEX(input: string) {",
        ),
        ("py", "def reconcile_invoice_batch_INDEX(input):"),
        ("go", "func reconcileInvoiceBatchINDEX(input string) {"),
    ] {
        let path = format!("src/processing.{extension}");
        let implementations: Vec<_> = (0..8)
            .map(|index| {
                let text = format!(
                    "{}\n{}",
                    declaration.replace("INDEX", &index.to_string()),
                    "    calculate intermediate remaining balance\n".repeat(60),
                );
                source(&path, index * 100 + 1, &text)
            })
            .collect();
        let mut corpus: Vec<_> = (0..35)
            .map(|index| {
                source(
                    &format!("docs/reference_{index:02}.md"),
                    1,
                    "reconcile invoice",
                )
            })
            .collect();
        corpus.extend(implementations.iter().cloned());

        let ranked = rank_lexically(&corpus, "reconcile invoice");
        let actual = locations(&ranked);
        for implementation in &implementations {
            assert!(
                actual.contains(&(
                    implementation.path.as_str(),
                    implementation.start_line,
                    implementation.end_line,
                )),
                "{extension}: implementation at line {} was excluded: {actual:?}",
                implementation.start_line,
            );
        }
        assert_eq!(ranked.len(), SHORTLIST_LIMIT);
        assert!(ranked.iter().any(|chunk| chunk.path.starts_with("docs/")));
    }
}

#[test]
fn overlapping_excerpts_do_not_displace_disjoint_evidence_in_one_function() {
    let path = "src/reader.rs";
    let mut lines = vec!["    consume_next_block();"; 260];
    lines[0] = "pub fn decode_archive() {";
    lines[1] = "    open_archive_header();";
    lines[220] = "    validate_archive_checksum();";
    lines[221] = "    reject_corrupt_archive();";
    lines[259] = "}";
    let excerpt = |start, end| source(path, start, &lines[start - 1..end].join("\n"));
    let corpus = vec![
        excerpt(1, 40),
        excerpt(1, 60),
        excerpt(2, 41),
        excerpt(201, 240),
        excerpt(211, 250),
        source("docs/archives.md", 1, "archive storage and retention"),
    ];

    let ranked = rank_lexically(&corpus, "decode archive checksum");
    let file_results: Vec<_> = ranked.iter().filter(|chunk| chunk.path == path).collect();
    assert!(file_results.iter().any(|chunk| chunk.start_line == 1));
    assert!(file_results.iter().any(|chunk| {
        chunk.start_line <= 221 && chunk.end_line >= 222 && chunk.start_line > 60
    }));
    for (index, first) in file_results.iter().enumerate() {
        for second in &file_results[index + 1..] {
            assert!(
                first.end_line < second.start_line || second.end_line < first.start_line,
                "redundant source excerpts survived: {first:?}, {second:?}",
            );
        }
    }
}

#[test]
fn standard_chunk_overlap_preserves_new_evidence_in_long_functions() {
    let mut lines = vec!["    consume_next_block();"; 260];
    lines[0] = "pub fn write_archive() {";
    lines[100] = "    persist_archive_header();";
    lines[200] = "    persist_archive_checksum();";
    lines[250] = "    complete_archive_transaction();";
    lines[259] = "}";
    let corpus = chunk_text("src/archive.rs", &lines.join("\n"));
    assert_eq!(corpus.len(), 3);
    assert!(corpus[0].end_line >= corpus[1].start_line);

    let ranked = rank_lexically(&corpus, "archive");
    assert_eq!(
        locations(&ranked).into_iter().collect::<HashSet<_>>(),
        locations(&corpus).into_iter().collect::<HashSet<_>>(),
        "small context overlap must not suppress mostly new function evidence",
    );
    assert!(
        ranked
            .iter()
            .any(|chunk| chunk.text.contains("persist_archive_checksum"))
    );
    assert!(
        ranked
            .iter()
            .any(|chunk| chunk.text.contains("complete_archive_transaction"))
    );
}

#[test]
fn duplicate_candidates_cannot_fill_the_shortlist_or_hide_other_files() {
    let unique: Vec<_> = (0..=35)
        .map(|index| source(&format!("docs/{index:02}.md"), 1, "archive recovery"))
        .collect();
    let mut corpus = vec![unique[0].clone(); 128];
    corpus.extend(unique.iter().skip(1).cloned());

    let ranked = rank_lexically(&corpus, "archive recovery");
    assert_eq!(ranked, rank_lexically(&unique, "archive recovery"));
    assert_eq!(ranked.len(), SHORTLIST_LIMIT);
    assert_eq!(
        locations(&ranked).into_iter().collect::<HashSet<_>>().len(),
        SHORTLIST_LIMIT,
    );
    assert_eq!(
        ranked
            .iter()
            .filter(|chunk| chunk.path == "docs/00.md")
            .count(),
        1,
    );
}

#[test]
fn mixed_code_and_text_results_are_stable_across_corpus_order() {
    let mut corpus: Vec<_> = [
        (
            "src/writer.rs",
            "pub fn persist_snapshot() {\n    flush();\n}",
        ),
        (
            "src/writer.ts",
            "export function persistSnapshot() {\n    flush();\n}",
        ),
        ("src/writer.py", "def persist_snapshot():\n    flush()"),
        ("src/writer.go", "func persistSnapshot() {\n    flush()\n}"),
        (
            "docs/storage.md",
            "Persist a snapshot after every successful transaction.",
        ),
        (
            "queries/save.sql",
            "-- persist snapshot\nINSERT INTO saved SELECT * FROM current;",
        ),
        ("config/snapshot.json", "{\"strategy\": \"persist\"}"),
        ("script.custom", "persist snapshot using storage writer"),
    ]
    .into_iter()
    .flat_map(|(path, text)| chunk_text(path, text))
    .collect();
    let expected = rank_lexically(&corpus, "persist snapshot");
    assert_eq!(expected.len(), 8);
    for offset in 0..corpus.len() {
        corpus.rotate_left(offset);
        assert_eq!(rank_lexically(&corpus, "persist snapshot"), expected);
        corpus.reverse();
        assert_eq!(rank_lexically(&corpus, "persist snapshot"), expected);
    }
    for (path, original) in [
        ("script.custom", "persist snapshot using storage writer"),
        ("config/snapshot.json", "{\"strategy\": \"persist\"}"),
    ] {
        let result = expected.iter().find(|chunk| chunk.path == path).unwrap();
        assert_eq!(result.text, original);
        assert_eq!((result.start_line, result.end_line), (1, 1));
        assert!(result.lexical_score.is_finite() && result.lexical_score > 0.0);
    }
}
