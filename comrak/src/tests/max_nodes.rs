use std::panic;

use crate::{Arena, NodeLimitExceeded, Options, markdown_to_html, parse_document};

// "a\n\n" の段落は、段落と文字の 2 つの node になる。文書の node が 1 つ足される
const PARAGRAPHS: usize = 100;
const NODES: usize = 1 + PARAGRAPHS * 2;

fn options(max_nodes: usize) -> Options<'static> {
    let mut options = Options::default();
    options.parse.max_nodes = Some(max_nodes);
    options
}

fn paragraphs() -> String {
    "a\n\n".repeat(PARAGRAPHS)
}

#[test]
fn documents_at_max_nodes_parse() {
    let arena = Arena::new();
    parse_document(&arena, &paragraphs(), &options(NODES));
    assert_eq!(arena.len(), NODES);
}

#[test]
fn documents_beyond_max_nodes_unwind_with_node_limit_exceeded() {
    let input = paragraphs();
    let payload = panic::catch_unwind(|| markdown_to_html(&input, &options(NODES - 1))).unwrap_err();
    assert!(payload.is::<NodeLimitExceeded>());
}

#[test]
fn inline_nodes_count_toward_max_nodes() {
    let input = "*a* ".repeat(PARAGRAPHS);
    let arena = Arena::new();
    parse_document(&arena, &input, &Options::default());
    let nodes = arena.len();
    // 文書と段落の 2 つの後に、強調と文字の node が続く
    assert!(nodes > PARAGRAPHS * 2);
    assert!(panic::catch_unwind(|| markdown_to_html(&input, &options(nodes))).is_ok());
    let payload = panic::catch_unwind(|| markdown_to_html(&input, &options(nodes - 1))).unwrap_err();
    assert!(payload.is::<NodeLimitExceeded>());
}

#[test]
fn max_nodes_does_not_apply_to_the_next_document() {
    let input = paragraphs();
    assert!(panic::catch_unwind(|| markdown_to_html(&input, &options(NODES - 1))).is_err());
    assert_eq!(markdown_to_html(&input, &Options::default()), "<p>a</p>\n".repeat(PARAGRAPHS));
}
