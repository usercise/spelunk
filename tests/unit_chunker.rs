//! Unit tests for the chunker module (no I/O, no SQLite).

use spelunk::indexer::{Chunk, ChunkKind};

// ── sliding_window ───────────────────────────────────────────────────────────

#[test]
fn sliding_window_single_chunk_when_file_fits() {
    let src = "line1\nline2\nline3";
    let chunks = spelunk::indexer::sliding_window(src, "test.txt", "text", 10, 2);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].start_line, 1);
    assert_eq!(chunks[0].end_line, 3);
    assert_eq!(chunks[0].content, "line1\nline2\nline3");
}

#[test]
fn sliding_window_produces_overlap() {
    // 6 lines, window=4, overlap=2 → step=2
    // chunk1: lines 1-4, chunk2: lines 3-6
    let src = (1..=6)
        .map(|n| format!("line{n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let chunks = spelunk::indexer::sliding_window(&src, "test.txt", "text", 4, 2);
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].start_line, 1);
    assert_eq!(chunks[0].end_line, 4);
    assert_eq!(chunks[1].start_line, 3);
    assert_eq!(chunks[1].end_line, 6);
}

#[test]
fn sliding_window_empty_source_returns_no_chunks() {
    let chunks = spelunk::indexer::sliding_window("", "test.txt", "text", 10, 2);
    assert!(chunks.is_empty());
}

#[test]
fn sliding_window_all_chunks_are_verbatim() {
    let src = "a\nb\nc\nd\ne\nf\ng\nh";
    let chunks = spelunk::indexer::sliding_window(src, "f.txt", "text", 3, 1);
    for c in &chunks {
        assert!(matches!(c.kind, ChunkKind::Verbatim));
    }
}

// ── Chunk::embedding_text ────────────────────────────────────────────────────

fn make_chunk(name: Option<&str>, docstring: Option<&str>, content: &str) -> Chunk {
    Chunk {
        file_path: "src/lib.rs".into(),
        language: "rust".into(),
        kind: ChunkKind::Function,
        name: name.map(str::to_string),
        start_line: 1,
        end_line: 5,
        content: content.to_string(),
        docstring: docstring.map(str::to_string),
        parent_scope: None,
        summary: None,
    }
}

#[test]
fn embedding_text_with_name() {
    let c = make_chunk(Some("my_fn"), None, "fn my_fn() {}");
    assert_eq!(c.embedding_text(), "title: my_fn | text: fn my_fn() {}");
}

#[test]
fn embedding_text_without_name_uses_none() {
    let c = make_chunk(None, None, "let x = 1;");
    assert_eq!(c.embedding_text(), "title: none | text: let x = 1;");
}

#[test]
fn embedding_text_prepends_docstring() {
    let c = make_chunk(Some("foo"), Some("/// Does foo."), "fn foo() {}");
    assert_eq!(
        c.embedding_text(),
        "title: foo | text: /// Does foo.\nfn foo() {}"
    );
}
