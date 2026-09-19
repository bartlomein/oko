use super::*;

fn chunk(path: &str, start_line: usize, text: String) -> Chunk {
    Chunk {
        path: path.into(),
        start_line,
        end_line: start_line + text.split('\n').count() - 1,
        text,
        lexical_score: 1.0,
    }
}

#[test]
fn thirty_long_candidates_survive_with_stable_ids_and_originals_untouched() {
    let chunks: Vec<_> = (0..30)
        .map(|index| {
            let mut lines = vec![format!("pub fn export_{index}() {{")];
            lines.extend((0..118).map(|_| "    let filler = \"x\".repeat(200);".into()));
            lines.push("    encode_video(frame);".into());
            chunk(&format!("src/module_{index}.rs"), 201, lines.join("\n"))
        })
        .collect();
    let original = chunks.clone();
    let items = ranking_previews(
        "Where is video encoded?",
        &chunks,
        RankingIntent::Implementation,
    )
    .unwrap();
    assert_eq!(items.len(), 30);
    for (index, item) in items.iter().enumerate() {
        assert_eq!(item.id, index.to_string());
        assert_eq!(
            item.source.as_deref(),
            Some(format!("src/module_{index}.rs:201-320").as_str())
        );
        assert!(item.text.contains(&format!("201: pub fn export_{index}")));
        assert!(item.text.contains("320:     encode_video(frame);"));
        assert!(item.text.contains(OMITTED));
    }
    let (request, retained) = ranking::prepare_request_with_intent(
        "Where is video encoded?",
        &items,
        RankingIntent::Implementation,
    )
    .unwrap();
    assert_eq!(retained.len(), 30);
    assert!(serde_json::to_vec(&request).unwrap().len() <= ranking::MAX_JEV_REQUEST_BYTES);
    assert_eq!(chunks, original);
}

#[test]
fn unicode_and_json_escapes_stay_within_the_real_request_budget() {
    let chunks: Vec<_> = (0..30)
        .map(|index| {
            chunk(
                &format!("資料/\"module\\{index}.rs"),
                1,
                format!(
                    "fn export_video() {{\n{}\n{}\n}}",
                    "    let video = \"❄🦀資料\\\\\";\t\u{1f}".repeat(100),
                    "    encode_video(frame);".repeat(100),
                ),
            )
        })
        .collect();
    let question = format!("video encoded {}", "❄\\\"\u{1f}".repeat(400));
    let items = ranking_previews(&question, &chunks, RankingIntent::Implementation).unwrap();
    assert_eq!(items.len(), 30);
    let (request, retained) =
        ranking::prepare_request_with_intent(&question, &items, RankingIntent::Implementation)
            .unwrap();
    assert_eq!(retained.len(), 30);
    let serialized = serde_json::to_vec(&request).unwrap();
    assert!(serialized.len() <= ranking::MAX_JEV_REQUEST_BYTES);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&serialized).unwrap(),
        request
    );
    assert!(items.iter().all(|item| item.text.contains(TRUNCATED)));
}

#[test]
fn long_line_preview_keeps_the_relevant_late_fragment() {
    let text = format!(
        "{} encode_video(frame); {}",
        "filler ".repeat(700),
        "tail ".repeat(700)
    );
    let items = ranking_previews(
        "Where is video encoded?",
        &[chunk("unknown.custom", 83, text)],
        RankingIntent::Implementation,
    )
    .unwrap();
    assert!(items[0].text.starts_with("83: [truncated] "));
    assert!(items[0].text.contains("encode_video(frame)"));
    assert!(items[0].text.ends_with(TRUNCATED));
}

#[test]
fn late_long_line_matches_use_original_words_when_stemming_changes_spelling() {
    let text = format!(
        "{} filing paperwork {}",
        "filler ".repeat(700),
        "tail ".repeat(700)
    );
    let items = ranking_previews(
        "file",
        &[chunk("unknown.custom", 83, text)],
        RankingIntent::General,
    )
    .unwrap();
    assert!(items[0].text.contains("filing paperwork"));
}

#[test]
fn previews_cover_different_query_terms_before_repeated_keyword_lines() {
    let mut lines = vec!["fn import_csv() {".to_string()];
    lines.extend((0..80).map(|_| "    log(\"import CSV import CSV\");".to_string()));
    lines.push("    points.push(parse_row(record));".into());
    let items = ranking_previews(
        "Where does CSV import parse records into points?",
        &[chunk("arbitrary.ext", 1, lines.join("\n"))],
        RankingIntent::Implementation,
    )
    .unwrap();
    assert!(items[0].text.contains("1: fn import_csv()"));
    assert!(
        items[0]
            .text
            .contains("82:     points.push(parse_row(record));")
    );
}

#[test]
fn real_declarations_survive_query_rich_assignments_and_leading_comments() {
    let candidates: Vec<_> = (0..30)
        .map(|index| {
            let mut lines = vec![
                "/// Runs one import job.".to_string(),
                format!("pub(crate) fn process_import_{index}("),
                "    job: &mut ImportJob,".to_string(),
                "    progress: &Sender<Progress>,".to_string(),
                ") -> Result<()> {".to_string(),
            ];
            lines.extend((0..114).map(|_| {
                "    let csv_record_encoding = parse_csv_record_encoding(input);".to_string()
            }));
            lines.push("}".to_string());
            chunk(&format!("src/imports/{index}.rs"), 51, lines.join("\n"))
        })
        .collect();
    let question = "Where is CSV import implemented with record parsing and encoding?";
    let items = ranking_previews(question, &candidates, RankingIntent::Implementation).unwrap();
    let (request, retained) =
        ranking::prepare_request_with_intent(question, &items, RankingIntent::Implementation)
            .unwrap();
    assert_eq!(retained.len(), 30);
    assert!(serde_json::to_vec(&request).unwrap().len() <= ranking::MAX_JEV_REQUEST_BYTES);
    for (index, item) in retained.iter().enumerate() {
        assert!(
            item.text
                .contains(&format!("52: pub(crate) fn process_import_{index}(")),
            "{}",
            item.text
        );
        assert!(item.text.contains("parse_csv_record_encoding(input)"));
    }
}

#[test]
fn multiline_declaration_names_survive_before_keyword_heavy_body_lines() {
    for (path, declaration, name_line) in [
        (
            "src/documents.js",
            "export async function\n    synchronize_documents(\n        source,\n        target\n    ) {",
            "9:     synchronize_documents(",
        ),
        (
            "src/documents.py",
            "async def synchronize_documents(\n    source,\n    target,\n):",
            "8: async def synchronize_documents(",
        ),
    ] {
        let source = format!(
            "// File synchronization entry point.\n{declaration}\n{}",
            "    let read_write_document = read_write_document_changes();\n".repeat(80)
        );
        let items = ranking_previews(
            "read write document changes",
            &[chunk(path, 7, source)],
            RankingIntent::Implementation,
        )
        .unwrap();
        assert!(items[0].text.contains(name_line), "{}", items[0].text);
        assert!(items[0].text.contains("read_write_document_changes()"));
    }
}

#[test]
fn long_declaration_keeps_its_name_when_matching_parameters_appear_late() {
    let signature = format!(
        "fn reconcile_documents({}target_document_changes: Changes) {{",
        "unrelated_argument: Data, ".repeat(100)
    );
    let items = ranking_previews(
        "target document changes",
        &[chunk("src/state.rs", 41, signature)],
        RankingIntent::Implementation,
    )
    .unwrap();
    assert!(items[0].text.starts_with("41: fn reconcile_documents("));
    assert!(items[0].text.contains(TRUNCATED));
}

#[test]
fn python_comments_do_not_displace_the_function_declaration() {
    let mut lines = vec![
        "# Export records and data.".to_string(),
        "def persist_rows(rows, transaction_factory, retry_policy, audit_writer):".to_string(),
    ];
    lines.extend((0..100).map(|_| "    # Export records and data.".to_string()));
    lines.push("    return write_rows(rows)".to_string());
    let candidates: Vec<_> = (0..30)
        .map(|index| chunk(&format!("src/{index}.py"), 21, lines.join("\n")))
        .collect();
    let question = "Where are records and data exported?";
    let items = ranking_previews(question, &candidates, RankingIntent::Implementation).unwrap();
    let (request, retained) =
        ranking::prepare_request_with_intent(question, &items, RankingIntent::Implementation)
            .unwrap();
    assert_eq!(retained.len(), 30);
    assert!(serde_json::to_vec(&request).unwrap().len() <= ranking::MAX_JEV_REQUEST_BYTES);
    for item in retained {
        assert!(
            item.text.contains(
                "22: def persist_rows(rows, transaction_factory, retry_policy, audit_writer):"
            ),
            "{}",
            item.text
        );
    }
}

#[test]
fn typed_method_declarations_survive_keyword_heavy_bodies() {
    for (extension, header) in [
        (
            "java",
            "public static String reconcileDocuments(Input input) {",
        ),
        (
            "cs",
            "public static string ReconcileDocuments(Input input) {",
        ),
        ("cpp", "void reconcile_documents(Input input) {"),
    ] {
        let source = format!(
            "// Synchronization implementation.\n{header}\n{}\n}}",
            "    document.apply_document_changes();\n".repeat(80)
        );
        let candidates: Vec<_> = (0..30)
            .map(|index| chunk(&format!("src/{index}.{extension}"), 21, source.clone()))
            .collect();
        let question = "Where are document changes applied?";
        let items = ranking_previews(question, &candidates, RankingIntent::Implementation).unwrap();
        let (request, retained) =
            ranking::prepare_request_with_intent(question, &items, RankingIntent::Implementation)
                .unwrap();
        assert_eq!(retained.len(), 30);
        assert!(serde_json::to_vec(&request).unwrap().len() <= ranking::MAX_JEV_REQUEST_BYTES);
        for item in retained {
            assert!(
                item.text.contains(&format!("22: {header}")),
                "{extension}: {}",
                item.text
            );
        }
    }
}

#[test]
fn typed_control_flow_does_not_displace_method_declarations() {
    let header = "public static String reconcileDocuments(Input input) {";
    let source = format!(
        "{header}\n    if (input.isEmpty()) {{\n        return \"\";\n    }}\n    else if (document_changes_applied()) {{\n{}\n    }}\n}}",
        "        document.apply_document_changes();\n".repeat(80)
    );
    let items = ranking_previews(
        "Where are document changes applied?",
        &[chunk("src/Document.java", 21, source)],
        RankingIntent::Implementation,
    )
    .unwrap();
    assert!(items[0].text.contains(&format!("21: {header}")));
}

#[test]
fn documentation_and_unknown_languages_use_correct_noncontiguous_line_numbers() {
    let mut lines = vec!["# Background".to_string()];
    lines.extend((0..80).map(|_| "Some unrelated background. ".repeat(20)));
    lines.push("The atomic replacement writes to a temporary file first.".into());
    let items = ranking_previews(
        "How does atomic replacement work?",
        &[chunk("guide.custom", 11, lines.join("\n"))],
        RankingIntent::Explanation,
    )
    .unwrap();
    assert!(items[0].text.contains("11: # Background"));
    assert!(items[0].text.contains("92: The atomic replacement"));
    assert!(items[0].text.contains(OMITTED));
    assert_eq!(items[0].source.as_deref(), Some("guide.custom:11-92"));
}

#[test]
fn small_sources_are_complete_and_limits_are_bounded() {
    let source = chunk("auth.rs", 12, "fn authenticate() {\n    check();\n}".into());
    let items = ranking_previews(
        "authentication",
        std::slice::from_ref(&source),
        RankingIntent::Implementation,
    )
    .unwrap();
    assert_eq!(
        items[0].text,
        "12: fn authenticate() {\n13:     check();\n14: }"
    );
    assert!(
        ranking_previews("anything", &[], RankingIntent::General)
            .unwrap()
            .is_empty()
    );
    let many = vec![source; 31];
    assert_eq!(
        ranking_previews("", &many, RankingIntent::General)
            .unwrap()
            .len(),
        30
    );
}

#[test]
fn extreme_question_budgets_drop_only_a_suffix_then_error_when_impossible() {
    let chunks: Vec<_> = (0..30)
        .map(|index| chunk(&format!("src/{index}.rs"), 1, "fn encode() {}".into()))
        .collect();
    let items =
        ranking_previews(&"x".repeat(30_000), &chunks, RankingIntent::Implementation).unwrap();
    assert!(!items.is_empty());
    assert!(items.len() < chunks.len());
    assert_eq!(
        items.iter().map(|item| item.id.clone()).collect::<Vec<_>>(),
        (0..items.len())
            .map(|index| index.to_string())
            .collect::<Vec<_>>()
    );
    assert!(ranking_previews(&"x".repeat(32_000), &chunks, RankingIntent::General).is_err());
    let mut invalid = chunks[0].clone();
    invalid.end_line += 1;
    assert!(ranking_previews("encode", &[invalid], RankingIntent::General).is_err());
}

#[test]
fn escaped_length_matches_serde_json_and_fragments_preserve_character_boundaries() {
    let text = "a\"\\\n\r\t\u{8}\u{c}\u{1f}\0🦀❄資料";
    assert_eq!(
        escaped_len(text),
        serde_json::to_string(text).unwrap().len() - 2
    );
    for budget in 24..100 {
        let line = format!("{} target {}", "🦀❄資料".repeat(30), "🦀❄資料".repeat(30));
        let result = fragment(&line, line.find("target").unwrap(), budget);
        assert!(result.contains(TRUNCATED));
        assert!(!result.contains('\u{fffd}'));
        assert!(escaped_len(&result) <= budget.max(27));
    }
}

#[test]
fn annotations_and_body_evidence_survive_crowded_ranking_requests() {
    for (extension, annotation, declaration, body) in [
        (
            "rs",
            "#[test]",
            "fn persistence_contract() {",
            "    assert_eq!(store.commit(batch), expected);",
        ),
        (
            "py",
            "@pytest.mark.parametrize(\n    \"mode\",\n    [\"durable\", \"buffered\"],\n)",
            "def persistence_contract(mode):",
            "    assert store.commit(batch) == expected",
        ),
        (
            "java",
            "@Test",
            "public void persistenceContract() {",
            "    assertEquals(expected, store.commit(batch));",
        ),
        (
            "cs",
            "[Fact]",
            "public void PersistenceContract() {",
            "    Assert.Equal(expected, store.Commit(batch));",
        ),
    ] {
        let text = format!(
            "{annotation}\n{declaration}\n{}\n{body}\n}}",
            "    // persistence storage durability contract\n".repeat(100)
        );
        let chunks: Vec<_> = (0..30)
            .map(|index| chunk(&format!("src/{index}.{extension}"), 1, text.clone()))
            .collect();
        let items = ranking_previews(
            "persistence storage durability contract",
            &chunks,
            RankingIntent::Implementation,
        )
        .unwrap();
        assert_eq!(items.len(), 30);
        for item in &items {
            assert!(
                item.text.contains(annotation.lines().next().unwrap()),
                "{}",
                item.text
            );
            assert!(item.text.contains(declaration), "{}", item.text);
            assert!(item.text.contains(body), "{}", item.text);
        }
        let (request, retained) = ranking::prepare_request_with_intent(
            "persistence storage durability contract",
            &items,
            RankingIntent::Implementation,
        )
        .unwrap();
        assert_eq!(retained.len(), 30);
        assert!(serde_json::to_vec(&request).unwrap().len() <= ranking::MAX_JEV_REQUEST_BYTES);
    }
}

#[test]
fn long_signatures_leave_room_for_actual_implementation_evidence() {
    for (extension, header, ending, body) in [
        (
            "rs",
            "pub fn persist_records(",
            ") -> Result<()> {",
            "    transaction.commit(batch)?;",
        ),
        (
            "py",
            "def persist_records(",
            "):",
            "    transaction.commit(batch)",
        ),
        (
            "ts",
            "export function persist_records(",
            ") {",
            "    transaction.commit(batch);",
        ),
    ] {
        let source = format!(
            "{header}\n{}\n{ending}\n{body}\n}}",
            "    records: Records,\n".repeat(30)
        );
        let candidates: Vec<_> = (0..30)
            .map(|index| chunk(&format!("src/{index}.{extension}"), 1, source.clone()))
            .collect();
        let items = ranking_previews(
            "where records are persisted",
            &candidates,
            RankingIntent::Implementation,
        )
        .unwrap();
        assert_eq!(items.len(), 30);
        for item in items {
            assert!(item.text.contains(header), "{}", item.text);
            assert!(item.text.contains(body), "{}", item.text);
        }
    }
}

#[test]
fn attached_annotations_are_recovered_across_real_chunk_boundaries() {
    let source =
        "def previous():\n    finish()\n\n@Test\ndef storage_contract():\n    assert commit()\n";
    let corpus = search::chunk_text("src/storage.py", source);
    let target = corpus
        .iter()
        .find(|chunk| chunk.text.starts_with("def storage_contract"))
        .unwrap()
        .clone();
    assert!(!target.text.contains("@Test"));
    let original = target.clone();
    let items =
        ranking_previews_with_context("storage", &[target], &corpus, RankingIntent::Implementation)
            .unwrap();
    assert_eq!(items[0].id, "0");
    assert!(items[0].text.contains("4: @Test"), "{}", items[0].text);
    assert!(items[0].text.contains("5: def storage_contract():"));
    assert!(!items[0].text.contains("finish()"));
    assert!(
        items[0]
            .source
            .as_deref()
            .unwrap()
            .starts_with("src/storage.py:4-")
    );
    assert!(!original.text.contains("@Test"));
}

#[test]
fn context_recovery_never_crosses_missing_lines_or_files() {
    let candidates = [chunk(
        "src/store.rs",
        10,
        "fn commit() {\n    persist();\n}".into(),
    )];
    let corpus = [
        chunk("src/store.rs", 8, "#[test]".into()),
        chunk("src/other.rs", 9, "#[test]".into()),
    ];
    let items = ranking_previews_with_context(
        "commit",
        &candidates,
        &corpus,
        RankingIntent::Implementation,
    )
    .unwrap();
    assert!(!items[0].text.contains("#[test]"));
    assert_eq!(items[0].source.as_deref(), Some("src/store.rs:10-12"));
}

#[test]
fn conflicting_or_malformed_corpus_cannot_supply_declaration_context() {
    let candidates = [chunk(
        "src/store.py",
        10,
        "def commit():\n    persist()".into(),
    )];
    let annotation = chunk("src/store.py", 9, "@Test".into());
    let conflict = chunk("src/store.py", 9, "@Production".into());
    for corpus in [
        vec![annotation.clone(), conflict.clone()],
        vec![conflict, annotation.clone()],
    ] {
        let items = ranking_previews_with_context(
            "commit",
            &candidates,
            &corpus,
            RankingIntent::Implementation,
        )
        .unwrap();
        assert!(!items[0].text.contains('@'));
        assert_eq!(items[0].source.as_deref(), Some("src/store.py:10-11"));
    }
    let mut malformed = annotation.clone();
    malformed.end_line += 1;
    let items = ranking_previews_with_context(
        "commit",
        &candidates,
        &[malformed],
        RankingIntent::Implementation,
    )
    .unwrap();
    assert!(!items[0].text.contains('@'));
    let mut malformed_candidate = candidates[0].clone();
    malformed_candidate.end_line += 1;
    assert!(
        ranking_previews_with_context(
            "commit",
            &[malformed_candidate],
            &[annotation],
            RankingIntent::Implementation
        )
        .is_err()
    );
}

#[test]
fn long_attached_documentation_cannot_displace_distinct_body_evidence() {
    let source = format!(
        "{}\n#[instrument]\nfn synchronize_records() {{\n{}\n    persist_unique_records(transaction);\n}}",
        "/// Synchronizes records between several independently managed sources.".to_owned()
            + &"\n/// This documentation describes retry settings, arguments and usage examples."
                .repeat(11),
        "    log_sync_status();\n".repeat(90),
    );
    let candidates: Vec<_> = (0..30)
        .map(|index| chunk(&format!("src/{index}.rs"), 1, source.clone()))
        .collect();
    let items = ranking_previews(
        "where synchronize records persists unique records",
        &candidates,
        RankingIntent::Implementation,
    )
    .unwrap();
    assert_eq!(items.len(), 30);
    for item in items {
        assert!(item.text.contains("#[instrument]"), "{}", item.text);
        assert!(
            item.text.contains("fn synchronize_records()"),
            "{}",
            item.text
        );
        assert!(
            item.text.contains("persist_unique_records(transaction)"),
            "{}",
            item.text
        );
    }
}

#[test]
fn headerless_guard_keeps_nested_decisions_before_repeated_keyword_mentions() {
    for (extension, block, evidence) in [
        (
            "ts",
            "    if (pendingRecords.length > 0) {\n        const accepted = pendingRecords.filter(record => {\n            if (record.externalKey) {\n                return !index.containsExternal(record.externalKey);\n            }\n            return !index.containsLocal(record.key);\n        });\n        persist(accepted);\n    }",
            "return !index.containsLocal(record.key);",
        ),
        (
            "rs",
            "    if !pending_records.is_empty() {\n        let accepted = pending_records.into_iter().filter(|record| {\n            if let Some(key) = record.external_key {\n                return !index.contains_external(key);\n            }\n            !index.contains_local(record.key)\n        }).collect();\n        persist(accepted);\n    }",
            "!index.contains_local(record.key)",
        ),
        (
            "py",
            "    if pending_records:\n        accepted = []\n        for record in pending_records:\n            if record.external_key:\n                present = index.contains_external(record.external_key)\n            else:\n                present = index.contains_local(record.key)\n            if not present:\n                accepted.append(record)\n        persist(accepted)",
            "present = index.contains_local(record.key)",
        ),
    ] {
        let text = format!(
            "{}\n{block}\n    finish();",
            "    trace(\"pending records checked before persisting records\");\n".repeat(35)
        );
        let candidates: Vec<_> = (0..30)
            .map(|index| {
                chunk(
                    &format!("src/worker_{index}.{extension}"),
                    201,
                    text.clone(),
                )
            })
            .collect();
        let question = "where pending records are checked before persisting records";
        let items = ranking_previews(question, &candidates, RankingIntent::Implementation).unwrap();
        assert_eq!(items.len(), 30);
        for item in &items {
            assert!(item.text.contains(evidence), "{extension}: {}", item.text);
            assert!(
                item.text.contains("persist(accepted)"),
                "{extension}: {}",
                item.text
            );
            for line in item.text.lines().filter(|line| !line.starts_with('[')) {
                let (number, content) = line.split_once(": ").unwrap();
                let number: usize = number.parse().unwrap();
                if !content.contains(TRUNCATED) {
                    assert_eq!(content, text.lines().nth(number - 201).unwrap());
                }
            }
        }
        let (request, retained) =
            ranking::prepare_request_with_intent(question, &items, RankingIntent::Implementation)
                .unwrap();
        assert_eq!(retained.len(), 30);
        assert!(serde_json::to_vec(&request).unwrap().len() <= ranking::MAX_JEV_REQUEST_BYTES);
    }
}

#[test]
fn decision_context_stays_bounded_and_does_not_promote_prose() {
    let source = [
        "    if pending_records:",
        "        accepted = select(pending_records)",
        "        persist(accepted)",
        "    unrelated_operation()",
    ];
    assert_eq!(decision_block(&source, 0, "worker.py"), Some(vec![0, 1, 2]));
    assert_eq!(decision_block(&source, 0, "guide.md"), None);
    assert_eq!(decision_block(&source, 0, "unknown.custom"), None);
    assert_eq!(
        decision_block(
            &["    // if pending_records {", "        prose"],
            0,
            "worker.rs"
        ),
        None,
    );
    let mut long_block = vec!["    if !pending_records.is_empty() {"];
    long_block.extend(std::iter::repeat_n("        inspect();", 100));
    assert_eq!(
        decision_block(&long_block, 0, "worker.rs").unwrap().len(),
        BLOCK_LINES,
    );
}

#[test]
fn recovery_previews_add_evidence_without_exceeding_wire_budget() {
    let chunks: Vec<_> = (0..16)
        .map(|index| {
            chunk(
                &format!("module{index}.rs"),
                1,
                (0..80)
                    .map(|line| {
                        format!("let parcel_{line} = dispatch(\"{}\");", "🦀\\\"".repeat(8))
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
        })
        .collect();
    let normal = ranking_previews_with_context(
        "parcel dispatch",
        &chunks,
        &chunks,
        RankingIntent::Explanation,
    )
    .unwrap();
    let recovery = recovery_previews_with_context(
        "parcel dispatch",
        &chunks,
        &chunks,
        RankingIntent::Explanation,
    )
    .unwrap();
    assert_eq!(normal.len(), recovery.len());
    assert!(
        normal
            .iter()
            .zip(&recovery)
            .any(|(a, b)| b.text.len() > a.text.len())
    );
    let (request, kept) = ranking::prepare_request_with_intent(
        "parcel dispatch",
        &recovery,
        RankingIntent::Explanation,
    )
    .unwrap();
    assert_eq!(kept.len(), 16);
    assert!(serde_json::to_vec(&request).unwrap().len() <= ranking::MAX_JEV_REQUEST_BYTES);
    for (a, b) in normal.iter().zip(&recovery) {
        assert_eq!(a.source, b.source);
        assert_eq!(a.id, b.id);
    }
}
