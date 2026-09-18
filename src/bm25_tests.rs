use super::*;

fn document(path: &str, text: &str) -> Chunk {
    Chunk {
        path: path.into(),
        start_line: 1,
        end_line: 1,
        text: text.into(),
        lexical_score: 0.0,
    }
}

fn score_for(results: &[Chunk], path: &str) -> f64 {
    results
        .iter()
        .find(|chunk| chunk.path == path)
        .unwrap()
        .lexical_score
}

#[test]
fn bm25_matches_hand_calculated_body_and_path_fields() {
    let chunks = [
        document("quartz.rs", "quartz quartz filler"),
        document("other.rs", "filler"),
    ];
    let ranked = rank_lexically(&chunks, "quartz");
    assert_eq!(ranked.len(), 1);
    // N=2, df=1 gives ln(2), body tf=2 and length/average=3/2.
    // Both paths have two tokens; path tf=1 normalizes to one.
    let idf = 2.0_f64.ln();
    let expected = idf * (2.0 * 2.2) / (2.0 + 1.2 * (0.25 + 0.75 * 1.5)) + 0.3 * idf;
    assert!((ranked[0].lexical_score - expected).abs() < 1e-12);
}

#[test]
fn bm25_rare_words_outweigh_common_words() {
    let mut chunks = vec![document("rare.rs", "quartz filler")];
    for index in 0..9 {
        chunks.push(document(&format!("common{index}.rs"), "usual filler"));
    }
    let ranked = rank_lexically(&chunks, "quartz usual");
    assert_eq!(ranked[0].path, "rare.rs");
    assert!(ranked[0].lexical_score > ranked[1].lexical_score);
}

#[test]
fn bm25_term_frequency_saturates_and_irrelevant_length_costs_score() {
    let chunks = [
        document(
            "one.rs",
            "quartz filler filler filler filler filler filler filler",
        ),
        document(
            "two.rs",
            "quartz quartz filler filler filler filler filler filler",
        ),
        document(
            "four.rs",
            "quartz quartz quartz quartz filler filler filler filler",
        ),
        document("short.rs", "quartz"),
    ];
    let ranked = rank_lexically(&chunks, "quartz");
    let one = score_for(&ranked, "one.rs");
    let two = score_for(&ranked, "two.rs");
    let four = score_for(&ranked, "four.rs");
    assert!(one < two && two < four);
    assert!((four - two) / 2.0 < two - one);
    assert!(score_for(&ranked, "short.rs") > one);
}

#[test]
fn bm25_empty_and_path_only_documents_have_finite_scores() {
    assert!(rank_lexically(&[], "quartz").is_empty());
    let chunks = [document("quartz.rs", ""), document("other.rs", "")];
    for question in ["", "where is it", "absent"] {
        assert!(rank_lexically(&chunks, question).is_empty());
    }
    let ranked = rank_lexically(&chunks, "quartz");
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].path, "quartz.rs");
    assert!(ranked[0].lexical_score.is_finite() && ranked[0].lexical_score > 0.0);
    assert!(rank_lexically(&[document("", "")], "quartz").is_empty());
}

#[test]
fn bm25_query_order_duplicates_and_filtering_preserve_scores() {
    let chunks = [
        document("src/first.rs", "quartz amber silver copper"),
        document("src/second.rs", "silver quartz quartz"),
        document("docs/guide.md", "copper copper copper amber"),
    ];
    let expected = rank_lexically(&chunks, "quartz amber silver copper");
    for query in [
        "copper silver amber quartz",
        "silver quartz amber copper quartz silver",
    ] {
        assert_eq!(rank_lexically(&chunks, query), expected);
    }
    let prepared = PreparedCorpus::new(&chunks);
    assert_eq!(
        prepared.rank("quartz amber silver copper", |_| true),
        expected
    );
    let filtered = prepared.rank("quartz amber silver copper", |chunk| {
        chunk.path.starts_with("src/")
    });
    let expected_filtered: Vec<_> = expected
        .into_iter()
        .filter(|chunk| chunk.path.starts_with("src/"))
        .collect();
    assert_eq!(filtered, expected_filtered);
}
