//! First answers from sections that hold several definitions.
//!
//! Parsed files are chunked along their definitions, and definitions shorter
//! than twenty lines share a section. The ranker judges the whole section, so
//! the answer must show what in it carries the question: in 0.6.0 a shared
//! section showed the wrong definition, a neighbour the question asked about
//! was dropped, and an attribute between a doc comment and its function left
//! the answer as a partial window. These build the corpus the way search does,
//! through the workspace cache, and check each section's shape first.
use oko::{
    context::{ContextPacket, SourceExcerpt, build_packet_with_navigation},
    search::Chunk,
    search_cache::WorkspaceCache,
};

/// The first answer for `question` when the section holding `winner_text` in
/// `path` is the ranker's best match; also the section itself.
fn first_answer(
    files: &[(&str, &str)],
    path: &str,
    winner_text: &str,
    question: &str,
) -> (Chunk, SourceExcerpt) {
    let root = tempfile::tempdir().unwrap();
    let disk = tempfile::tempdir().unwrap();
    for (name, text) in files {
        let file = root.path().join(name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    let mut cache = WorkspaceCache::with_directory(disk.path().to_owned());
    let loaded = cache.load(root.path()).unwrap();
    let chunks = loaded.snapshot.chunks();
    let winner = chunks
        .iter()
        .find(|c| c.path == path && c.text.contains(winner_text))
        .unwrap()
        .clone();
    let packet: ContextPacket = build_packet_with_navigation(
        chunks,
        &[(winner.clone(), 0.9)],
        question,
        loaded.snapshot.navigation(),
    );
    (winner, packet.results[0].excerpt.clone())
}

fn line_of(text: &str, needle: &str) -> usize {
    text.lines().position(|line| line.contains(needle)).unwrap() + 1
}

fn covers(excerpt: &SourceExcerpt, line: usize) -> bool {
    excerpt.start_line <= line && line <= excerpt.end_line
}

const STATUS_CODES: &str = r#"from __future__ import annotations

from enum import IntEnum


class codes(IntEnum):
    """HTTP status codes and reason phrases

    Status codes from the following RFCs are all observed:

        * RFC 7231: Hypertext Transfer Protocol (HTTP/1.1)
        * RFC 6585: Additional HTTP Status Codes
        * RFC 3229: Delta encoding in HTTP
        * RFC 4918: HTTP Extensions for WebDAV
        * RFC 5842: Binding Extensions to WebDAV
        * RFC 7238: Permanent Redirect
        * RFC 2295: Transparent Content Negotiation in HTTP
        * RFC 2774: An HTTP Extension Framework
        * RFC 7540: Hypertext Transfer Protocol Version 2 (HTTP/2)
    """

    def __new__(cls, value: int, phrase: str = "") -> codes:
        obj = int.__new__(cls, value)
        obj._value_ = value

        obj.phrase = phrase  # type: ignore[attr-defined]
        return obj

    def __str__(self) -> str:
        return str(self.value)

    @classmethod
    def get_reason_phrase(cls, value: int) -> str:
        try:
            return codes(value).phrase  # type: ignore
        except ValueError:
            return ""

    @classmethod
    def is_informational(cls, value: int) -> bool:
        """
        Returns `True` for 1xx status codes, `False` otherwise.
        """
        return 100 <= value <= 199

    @classmethod
    def is_success(cls, value: int) -> bool:
        """
        Returns `True` for 2xx status codes, `False` otherwise.
        """
        return 200 <= value <= 299

    @classmethod
    def is_redirect(cls, value: int) -> bool:
        """
        Returns `True` for 3xx status codes, `False` otherwise.
        """
        return 300 <= value <= 399

    CONTINUE = 100, "Continue"
    SWITCHING_PROTOCOLS = 101, "Switching Protocols"
    OK = 200, "OK"
"#;

#[test]
fn reason_phrase_lookup_shows_the_lookup_not_the_constructor() {
    let files = [("pkg/status_codes.py", STATUS_CODES)];
    for question in [
        "HTTP status code reason phrase lookup returning empty string for unrecognized codes",
        "Where is the HTTP status reason-phrase lookup that accepts enum or integer inputs, \
         preserves known phrases, and currently returns an empty string for unrecognized integer codes?",
        "Locate the HTTP status reason-phrase lookup method that accepts integer or enum status codes, \
         preserves known phrases, and returns an empty string for unrecognized integer codes.",
    ] {
        let (section, excerpt) = first_answer(
            &files,
            "pkg/status_codes.py",
            "def get_reason_phrase",
            question,
        );
        for name in [
            "def __new__",
            "def get_reason_phrase",
            "def is_informational",
        ] {
            assert!(section.text.contains(name), "one section holds {name}");
        }
        let decorator = line_of(STATUS_CODES, "def get_reason_phrase") - 1;
        let fallback = line_of(STATUS_CODES, "return \"\"");
        assert_eq!(excerpt.start_line, decorator, "{question}");
        assert!(covers(&excerpt, fallback), "{question}");
        assert!(
            !covers(&excerpt, line_of(STATUS_CODES, "def __new__")),
            "{question}"
        );
        assert!(excerpt.definition_complete);
    }
    // A question about the constructor still gets the constructor.
    let (_, excerpt) = first_answer(
        &files,
        "pkg/status_codes.py",
        "def get_reason_phrase",
        "new member obj value assignment",
    );
    assert_eq!(excerpt.start_line, line_of(STATUS_CODES, "def __new__"));
}

const STANDARD_SINK: &str = r#"use std::io;
use std::time::Instant;

impl<'p, 's, M: Matcher, W: io::Write> Sink for StandardSink<'p, 's, M, W> {
    type Error = io::Error;

    fn binary_data(
        &mut self,
        searcher: &Searcher,
        binary_byte_offset: u64,
    ) -> Result<bool, io::Error> {
        if searcher.binary_detection().quit_byte().is_some() {
            if let Some(ref path) = self.path {
                log::debug!(
                    "ignoring {path}: found binary data at \
                     offset {binary_byte_offset}",
                    path = path.as_path().display(),
                );
            }
        }
        self.binary_byte_offset = Some(binary_byte_offset);
        Ok(true)
    }

    fn begin(&mut self, _searcher: &Searcher) -> Result<bool, io::Error> {
        self.standard.wtr.borrow_mut().reset_count();
        self.start_time = Instant::now();
        self.match_count = 0;
        self.binary_byte_offset = None;
        Ok(true)
    }

    fn finish(
        &mut self,
        searcher: &Searcher,
        finish: &SinkFinish,
    ) -> Result<(), io::Error> {
        if let Some(offset) = self.binary_byte_offset {
            StandardImpl::new(searcher, self).write_binary_message(offset)?;
        }
        if let Some(stats) = self.stats.as_mut() {
            stats.add_elapsed(self.start_time.elapsed());
            stats.add_searches(1);
            if self.match_count > 0 {
                stats.add_searches_with_match(1);
            }
            stats.add_bytes_searched(finish.byte_count());
            stats.add_bytes_printed(self.standard.wtr.borrow().count());
        }
        Ok(())
    }
}
"#;

#[test]
fn printed_bytes_shows_the_reset_with_the_record() {
    let files = [("crates/printer/src/standard.rs", STANDARD_SINK)];
    let (section, excerpt) = first_answer(
        &files,
        "crates/printer/src/standard.rs",
        "fn finish(",
        "standard printer per-search printed bytes counter reset, byte statistics recorded at search finish",
    );
    assert!(section.text.contains("fn begin(") && section.text.contains("fn finish("));
    assert!(covers(&excerpt, line_of(STANDARD_SINK, "reset_count()")));
    assert!(covers(
        &excerpt,
        line_of(STANDARD_SINK, "add_bytes_printed")
    ));
    assert_eq!(excerpt.definitions, 2);
    // The neighbour comes only for a part of the question it does.
    let (_, excerpt) = first_answer(
        &files,
        "crates/printer/src/standard.rs",
        "fn finish(",
        "byte statistics recorded at search finish",
    );
    assert!(!covers(&excerpt, line_of(STANDARD_SINK, "reset_count()")));
    assert!(covers(
        &excerpt,
        line_of(STANDARD_SINK, "add_bytes_printed")
    ));
}

const INTERPOLATE: &str = r#"/// Interpolate capture references in `replacement` and write the result to `dst`.
pub fn interpolate<A, N>(mut replacement: &[u8], mut append: A, mut name_to_index: N, dst: &mut Vec<u8>)
where
    A: FnMut(usize, &mut Vec<u8>),
    N: FnMut(&str) -> Option<usize>,
{
    while !replacement.is_empty() {
        match memchr(b'$', replacement) {
            None => break,
            Some(i) => {
                dst.extend(&replacement[..i]);
                replacement = &replacement[i..];
            }
        }
        if replacement.get(1).map_or(false, |&b| b == b'$') {
            dst.push(b'$');
            replacement = &replacement[2..];
            continue;
        }
        debug_assert!(!replacement.is_empty());
    }
    dst.extend(replacement);
}

impl<'a> From<&'a str> for Ref<'a> {
    #[inline]
    fn from(x: &'a str) -> Ref<'a> {
        Ref::Named(x)
    }
}

impl From<usize> for Ref<'static> {
    #[inline]
    fn from(x: usize) -> Ref<'static> {
        Ref::Number(x)
    }
}

/// Parses a possible reference to a capture group name in the given text,
/// starting at the beginning of `replacement`.
///
/// If no such valid reference could be found, None is returned.
#[inline]
fn find_cap_ref(replacement: &[u8]) -> Option<CaptureRef<'_>> {
    let mut i = 0;
    if replacement.len() <= 1 || replacement[0] != b'$' {
        return None;
    }
    let mut brace = false;
    i += 1;
    if replacement[i] == b'{' {
        brace = true;
        i += 1;
    }
    let mut cap_end = i;
    while replacement.get(cap_end).map_or(false, is_valid_cap_letter) {
        cap_end += 1;
    }
    if cap_end == i {
        return None;
    }
    let cap = std::str::from_utf8(&replacement[i..cap_end])
        .expect("valid UTF-8 capture name");
    if brace {
        if !replacement.get(cap_end).map_or(false, |&b| b == b'}') {
            return None;
        }
        cap_end += 1;
    }
    Some(CaptureRef {
        cap: match cap.parse::<u32>() {
            Ok(i) => Ref::Number(i as usize),
            Err(_) => Ref::Named(cap),
        },
        end: cap_end,
    })
}

/// Returns true if and only if the given byte is allowed in a capture name.
#[inline]
fn is_valid_cap_letter(b: &u8) -> bool {
    match *b {
        b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z' | b'_' => true,
        _ => false,
    }
}
"#;

#[test]
fn capture_predicate_comes_with_the_parser_it_serves() {
    let files = [("crates/matcher/src/interpolate.rs", INTERPOLATE)];
    let predicate = line_of(INTERPOLATE, "fn is_valid_cap_letter");
    let parser_doc = line_of(INTERPOLATE, "Parses a possible reference");
    // The doc comment matches the question; `#[inline]` sits between it and
    // the function it documents.
    let (section, excerpt) = first_answer(
        &files,
        "crates/matcher/src/interpolate.rs",
        "fn find_cap_ref",
        "replacement capture reference name parsing for $name references",
    );
    assert!(section.text.contains("impl From<usize>") && section.text.contains("fn find_cap_ref"));
    assert_eq!(excerpt.start_line, parser_doc);
    assert!(
        covers(&excerpt, predicate + 4),
        "the predicate the parser calls comes with it"
    );
    assert!(excerpt.definition_complete && !excerpt.truncated);
    // The predicate's own documentation leads to the predicate, whole.
    let (_, excerpt) = first_answer(
        &files,
        "crates/matcher/src/interpolate.rs",
        "fn is_valid_cap_letter",
        "byte allowed in a capture name",
    );
    assert_eq!(excerpt.start_line, predicate - 2);
    assert!(covers(&excerpt, predicate + 5));
    assert!(excerpt.definition_complete && !excerpt.truncated);
}

#[test]
fn a_file_head_section_shows_the_function_not_its_imports() {
    let mut lines = vec![
        "import type { AstroConfig, ImageMetadata } from '../../types/public/index.js';".to_owned(),
        "import { isRemoteAllowed } from '@astrojs/internal-helpers/remote';".into(),
        "import { AstroError, AstroErrorData } from '../../core/errors/index.js';".into(),
        "import { imageMetadata } from './metadata.js';".into(),
        "".into(),
        "type RemoteImageConfig = Pick<AstroConfig['image'], 'domains' | 'remotePatterns'>;".into(),
        "".into(),
        "/**".into(),
        " * Infers the dimensions of a remote image by streaming its bytes.".into(),
        " */".into(),
        "export async function inferRemoteSize(url: string, imageConfig?: RemoteImageConfig): Promise<ImageMetadata> {".into(),
        "\tif (!isRemoteAllowed(url, imageConfig)) {".into(),
        "\t\tthrow new AstroError(AstroErrorData.RemoteImageNotAllowed);".into(),
        "\t}".into(),
    ];
    lines.extend((0..60).map(|n| format!("\tconst chunk{n} = await read(reader, {n});")));
    lines.push("\t// Validate that the final URL (after redirects) is allowed".into());
    lines.push("\tif (!isRemoteAllowed(response.url, imageConfig)) {".into());
    lines.push("\t\tthrow new AstroError(AstroErrorData.RemoteImageNotAllowed);".into());
    lines.push("\t}".into());
    lines.push("\treturn imageMetadata(buffer);".into());
    lines.push("}".into());
    let source = lines.join("\n") + "\n";
    let files = [("src/assets/utils/remoteProbe.ts", source.as_str())];
    let (section, excerpt) = first_answer(
        &files,
        "src/assets/utils/remoteProbe.ts",
        "import { isRemoteAllowed }",
        "remote image probing: initial and final URL isRemoteAllowed authorization checks",
    );
    assert_eq!(section.start_line, 1, "the section begins with the imports");
    assert_eq!(excerpt.start_line, line_of(&source, "/**"));
    assert!(covers(&excerpt, line_of(&source, "isRemoteAllowed(url")));
    assert!(covers(
        &excerpt,
        line_of(&source, "isRemoteAllowed(response.url")
    ));
    assert!(excerpt.definition_complete && !excerpt.truncated);
    // An import naming more of the question than any code line does not
    // turn the answer into a partial window from line 1.
    let (_, excerpt) = first_answer(
        &files,
        "src/assets/utils/remoteProbe.ts",
        "import { isRemoteAllowed }",
        "AstroError AstroErrorData from core errors",
    );
    assert_eq!(excerpt.start_line, line_of(&source, "/**"));
    assert!(excerpt.definition_complete && !excerpt.truncated);
}
