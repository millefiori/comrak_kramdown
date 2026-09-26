use std::panic;

use comrak::arena_tree::NodeEdge;
use comrak::nodes::{Node, NodeValue};
use comrak::{Arena, Options, markdown_to_html, parse_document};
use magnus::{Error, RString, Ruby, function, prelude::*};

fn options() -> Options<'static> {
    let mut options = Options::default();
    options.parse.kramdown = true;
    options.extension.strikethrough = true;
    options.extension.footnotes = true;
    options.extension.description_lists = true;
    options.extension.cjk_friendly_emphasis = true;
    options.render.r#unsafe = true;
    options.render.hardbreaks = true;
    options
}

/// 変換と同じ解析で、wiki link として扱われる `[[...]]` の中身を出現順に返す。
/// コード・HTML コメント・エスケープした括弧・リンクの中 (markdown のリンクと生の `<a>` / `<tt>`) は含めない。
fn collect_wiki_links(markdown: &str) -> Vec<String> {
    let arena = Arena::new();
    let root = parse_document(&arena, markdown, &options());
    let mut links = vec![];
    // 深い入れ子でもスタックを使い切らないよう、再帰せず辿る
    let mut skipped_depth = 0usize;
    let mut raw_link_depth = 0usize;
    for edge in root.traverse() {
        match edge {
            NodeEdge::Start(node) if is_skipped(node) => skipped_depth += 1,
            NodeEdge::End(node) if is_skipped(node) => skipped_depth -= 1,
            // 閉じない <a> の影響は段落などの外に出さない (表示でも Nokogiri がそこで閉じる)
            NodeEdge::Start(node) if node.data().value.contains_inlines() => raw_link_depth = 0,
            NodeEdge::Start(node) if skipped_depth == 0 => {
                if let NodeValue::HtmlInline(raw) = &node.data().value {
                    match raw_link_tag(raw) {
                        Some(true) => raw_link_depth += 1,
                        Some(false) => raw_link_depth = raw_link_depth.saturating_sub(1),
                        None if raw_link_depth == 0 => {
                            if let Some(inner) = raw.strip_prefix("[[").and_then(|r| r.strip_suffix("]]")) {
                                links.push(inner.to_string());
                            }
                        }
                        None => {}
                    }
                }
            }
            _ => {}
        }
    }
    links
}

fn is_skipped(node: Node<'_>) -> bool {
    matches!(node.data().value, NodeValue::Link(..) | NodeValue::Image(..) | NodeValue::Code(..))
}

/// 生の HTML が `<a>` / `<tt>` の開きタグなら Some(true)、閉じタグなら Some(false)。自己閉じは数えない。
/// 表示側の libxml2 と同じく、`/>` の直前が空白・引用符・タグ名のときだけ自己閉じとみなす。
/// `<a href=/foo/>` の `/` は引用符の無い属性値の一部で、開きタグになる。
fn is_self_closing(raw: &str) -> bool {
    let Some(body) = raw.trim_end().strip_suffix("/>").and_then(|b| b.strip_prefix('<')) else {
        return false;
    };
    body.ends_with([' ', '\t', '\r', '\n', '"', '\'']) || body.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn raw_link_tag(raw: &str) -> Option<bool> {
    if is_self_closing(raw) {
        return None;
    }
    let tag = raw.strip_prefix('<')?;
    let (opening, tag) = match tag.strip_prefix('/') {
        Some(tag) => (false, tag),
        None => (true, tag),
    };
    let name_len = tag.find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(tag.len());
    let name = &tag[..name_len];
    (name.eq_ignore_ascii_case("a") || name.eq_ignore_ascii_case("tt")).then_some(opening)
}

/// magnus は Rust の panic を rescue できない fatal にするので、RuntimeError に変える
fn catch_panic<T>(ruby: &Ruby, work: impl FnOnce() -> T + panic::UnwindSafe) -> Result<T, Error> {
    panic::catch_unwind(work).map_err(|payload| {
        let detail = payload
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_default();
        Error::new(ruby.exception_runtime_error(), format!("markdown の変換中に panic した: {detail}"))
    })
}

fn to_html(ruby: &Ruby, markdown: RString) -> Result<String, Error> {
    let markdown = markdown.to_string()?;
    catch_panic(ruby, || markdown_to_html(&markdown, &options()))
}

fn wiki_links(ruby: &Ruby, markdown: RString) -> Result<Vec<String>, Error> {
    let markdown = markdown.to_string()?;
    catch_panic(ruby, || collect_wiki_links(&markdown))
}

#[magnus::init]
fn init(ruby: &Ruby) -> Result<(), Error> {
    let module = ruby.define_module("ComrakKramdown")?;
    module.define_singleton_method("to_html", function!(to_html, 1))?;
    module.define_singleton_method("wiki_links", function!(wiki_links, 1))?;
    Ok(())
}
