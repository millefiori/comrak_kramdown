use std::panic::{self, AssertUnwindSafe};

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

fn count_nodes(input: &str, options: &Options) -> usize {
    let arena = Arena::new();
    parse_document(&arena, input, options);
    arena.len()
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
    let emphases = 100;
    let input = "*a* ".repeat(emphases);
    let nodes = count_nodes(&input, &Options::default());
    // 文書と段落の 2 つの後に、強調と文字の node が続く
    assert!(nodes > 2 + emphases);
    assert!(panic::catch_unwind(|| markdown_to_html(&input, &options(nodes))).is_ok());
    let payload = panic::catch_unwind(|| markdown_to_html(&input, &options(nodes - 1))).unwrap_err();
    assert!(payload.is::<NodeLimitExceeded>());
}

#[test]
fn every_kind_of_node_stops_at_max_nodes() {
    // node を作る所 (ブロック・inline・リンク・Kramdown と GFM の表・inline 脚注・定義の無い脚注参照・リストと表のセルのタスク) を通る文書
    let mut kramdown = Options::default();
    kramdown.parse.kramdown = true;
    kramdown.extension.footnotes = true;
    kramdown.extension.inline_footnotes = true;
    kramdown.extension.tasklist = true;
    let mut gfm = Options::default();
    gfm.extension.table = true;
    gfm.extension.footnotes = true;
    gfm.extension.tasklist = true;
    gfm.parse.tasklist_in_table = true;
    let documents = [
        (kramdown, "a | b\nc | d\n\n- [ ] e ^[f] [^g] [h](i)\n"),
        (gfm, "x\n| a | b |\n|---|---|\n| [ ] | d |\n\n- [ ] e [^g] [h](i)\n"),
    ];
    for (options, input) in documents {
        let nodes = count_nodes(input, &options);
        for max_nodes in 1..nodes {
            let mut limited = options.clone();
            limited.parse.max_nodes = Some(max_nodes);
            let arena = Arena::new();
            let parsed = panic::catch_unwind(AssertUnwindSafe(|| {
                parse_document(&arena, input, &limited);
            }));
            assert!(parsed.unwrap_err().is::<NodeLimitExceeded>());
            // 上限を見ずに node を作る所があると、止まるまでに上限を超えて作る
            assert_eq!(arena.len(), max_nodes, "{input:?}");
        }
    }
}

#[test]
fn max_nodes_does_not_apply_to_the_next_document() {
    let input = paragraphs();
    assert!(panic::catch_unwind(|| markdown_to_html(&input, &options(NODES - 1))).is_err());
    assert_eq!(markdown_to_html(&input, &Options::default()), "<p>a</p>\n".repeat(PARAGRAPHS));
}
