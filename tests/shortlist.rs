use oko::RankingIntent;
use oko::search::{Chunk, SHORTLIST_LIMIT, chunk_text, rank_lexically, rank_lexically_with_intent};
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

#[test]
fn tests_do_not_take_the_slots_reserved_for_implementations() {
    // Tests repeat the vocabulary of what they exercise and outnumber it.
    let mut corpus: Vec<_> = (0..40)
        .map(|index| {
            source(
                &format!("tests/client/test_auth_{index}.py"),
                1,
                "def test_async_auth_flow_closes_response_body():\n    assert async auth flow response body closed",
            )
        })
        .collect();
    corpus.push(source(
        "httpx/_client.py",
        1645,
        "async def _send_handling_auth(self, request, auth):\n    auth_flow = auth.async_auth_flow(request)\n    await response.aclose()",
    ));
    let question = "where is the async auth flow response body closed";
    let implementation =
        rank_lexically_with_intent(&corpus, question, RankingIntent::Implementation);
    assert_eq!(implementation.len(), SHORTLIST_LIMIT);
    assert_eq!(
        implementation[0].path, "httpx/_client.py",
        "the only implementation leads the reserved source slots"
    );
    assert!(
        implementation[1..]
            .iter()
            .all(|chunk| chunk.path.starts_with("tests/")),
        "tests still compete for the remaining slots"
    );

    // A question about tests keeps them in the reserved slots.
    let about_tests = rank_lexically_with_intent(
        &corpus,
        "which test covers the async auth flow response body",
        RankingIntent::Implementation,
    );
    assert!(about_tests[0].path.starts_with("tests/"));

    // Names that merely contain "test" are not test files.
    for path in [
        "src/contest.rs",
        "src/latest_release.py",
        "src/attestation/verify.ts",
    ] {
        let corpus = [source(path, 1, "fn async_auth_flow() {}")];
        let ranked =
            rank_lexically_with_intent(&corpus, "async auth flow", RankingIntent::Implementation);
        assert_eq!(ranked.len(), 1, "{path}");
    }
}

#[test]
fn a_short_helper_beside_a_strong_match_reaches_the_shortlist() {
    // Forty files outrank the helper on words alone; one match beats them all.
    let mut corpus: Vec<_> = (0..40)
        .map(|index| {
            source(
                &format!("src/other_{index}.rs"),
                1,
                "fn capture_name() {\n    // capture name\n}",
            )
        })
        .collect();
    corpus.push(source(
        "src/interpolate.rs",
        92,
        "fn parse_capture_name_reference(replacement: &[u8]) {\n    // capture name reference parsing in the replacement\n}",
    ));
    let helper = source(
        "src/interpolate.rs",
        95,
        "/// Whether the byte is allowed in a capture name.\nfn is_valid_cap_letter(b: &u8) -> bool {\n    b.is_ascii_alphanumeric()\n}",
    );
    corpus.push(helper.clone());
    let question = "capture name reference parsing in the replacement";
    let ranked = rank_lexically(&corpus, question);
    assert_eq!(ranked.len(), SHORTLIST_LIMIT);
    assert_eq!(locations(&ranked)[0], ("src/interpolate.rs", 92, 94));
    assert!(
        locations(&ranked).contains(&("src/interpolate.rs", 95, 98)),
        "the adjoining helper takes one of the last slots: {:?}",
        locations(&ranked)
    );

    // Adjacency alone is not evidence: a neighbour that matches nothing stays out.
    let last = corpus.len() - 1;
    corpus[last] = source(
        "src/interpolate.rs",
        134,
        "fn unrelated() -> bool {\n    true\n}",
    );
    let ranked = rank_lexically(&corpus, question);
    assert!(!locations(&ranked).contains(&("src/interpolate.rs", 95, 97)));

    // Nor is it lifted beside a weak match.
    let mut weak = corpus.clone();
    weak[last] = helper;
    weak[last - 1] = source(
        "src/interpolate.rs",
        92,
        &"fn find_cap_ref() {}\n".repeat(42),
    );
    let ranked = rank_lexically(&weak, question);
    assert!(!locations(&ranked).contains(&("src/interpolate.rs", 95, 98)));
}
