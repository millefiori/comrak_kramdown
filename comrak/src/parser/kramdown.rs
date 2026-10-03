//! Kramdown 互換: Kramdown (kramdown 2.x + kramdown-parser-gfm) の表と IAL の規則。
//!
//! `Options::parse.kramdown` が真のときだけ使う。
//!
//! 表は Kramdown と同じく「全行に区切りの `|` がある段落」を表にする。ヘッダ行と区切り行は任意。
//! セルの分割ではコードスパン・`<code>`・wiki link `[[...]]`・ルビ `|親《ルビ》` の中の `|` を区切りにしない。
//! ただし `|` の直後が空白のときはルビの開始とみなさない (Kramdown では `| 本文《…》` のセル区切りが
//! ルビに取り込まれていた)。
//!
//! 公開される本文を通すので、どの走査も入力の長さに比例する時間で終わるようにしている。

use std::collections::{HashMap, HashSet};

use crate::Arena;
use crate::nodes::{Ast, Attributes, LineColumn, Node, NodeTable, NodeValue, TableAlignment};
use crate::parser::table::MAX_AUTOCOMPLETED_CELLS;

/// これより列の多い段落は表にしない。表の描画は 1 行あたり列数の 2 乗の時間がかかる (html.rs の render_table_cell)
const MAX_COLUMNS: usize = 1_000;

/// 行に、セルの区切りになる `|` が 1 つ以上あるか。
pub fn has_separator(line: &str) -> bool {
    let line = line.trim_end_matches(['\n', '\r']);
    if is_separator_line(line) {
        return line.contains('|');
    }
    split_row(line).separators > 0
}

/// 段落を表に置き換える。表にならないときは何もせず false を返す。
pub fn try_table<'a>(arena: &'a Arena<'a>, node: Node<'a>, ast: &mut Ast) -> bool {
    let lines: Vec<&str> = split_lines(&ast.content)
        .map(|line| line.trim_end_matches(['\n', '\r', ' ', '\t']))
        .collect();
    if lines.is_empty() || !lines.iter().all(|line| has_separator(line)) {
        return false;
    }
    let Some((rows, header_rows, alignments)) = parse_rows(&lines) else {
        return false;
    };
    append_rows(arena, node, ast, rows, header_rows, alignments);
    true
}

type Rows = Vec<Vec<String>>;

fn parse_rows(lines: &[&str]) -> Option<(Rows, usize, Vec<TableAlignment>)> {
    let leading_pipe = lines[0].starts_with('|');
    let mut rows: Rows = vec![];
    let mut header_rows = 0;
    let mut alignments: Option<Vec<TableAlignment>> = None;

    for line in lines {
        if is_separator_line(line) {
            if !rows.is_empty() && alignments.is_none() {
                header_rows = rows.len();
                alignments = Some(parse_alignments(line));
            }
            continue;
        }

        let mut cells = split_row(line).cells;
        if leading_pipe && cells.first().is_some_and(|cell| cell.trim().is_empty()) {
            cells.remove(0);
        }
        if cells.last().is_some_and(|cell| cell.trim().is_empty()) {
            cells.pop();
        }
        rows.push(cells.iter().map(|cell| strip_cell(cell).to_string()).collect());
    }

    if rows.len() == header_rows {
        return None;
    }
    let num_columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let num_cells: usize = rows.iter().map(Vec::len).sum();
    if num_columns > MAX_COLUMNS || num_columns * rows.len() - num_cells > MAX_AUTOCOMPLETED_CELLS {
        return None;
    }

    let mut alignments = alignments.unwrap_or_default();
    alignments.resize(num_columns, TableAlignment::None);
    Some((rows, header_rows, alignments))
}

fn append_rows<'a>(
    arena: &'a Arena<'a>,
    node: Node<'a>,
    ast: &mut Ast,
    rows: Rows,
    header_rows: usize,
    alignments: Vec<TableAlignment>,
) {
    let num_columns = alignments.len();
    let num_rows = rows.len();
    let num_nonempty_cells = rows.iter().map(Vec::len).sum();
    let start = ast.sourcepos.start;
    for (i, mut cells) in rows.into_iter().enumerate() {
        cells.resize(num_columns, String::new());
        let line = LineColumn {
            line: start.line + i,
            column: start.column,
        };
        let row = new_node(arena, NodeValue::TableRow(i < header_rows), line);
        node.append(row);
        for cell in cells {
            let cell_node = new_node(arena, NodeValue::TableCell, line);
            {
                let mut cell_ast = cell_node.data_mut();
                cell_ast.content = cell;
                cell_ast.line_offsets.push(0);
            }
            row.append(cell_node);
        }
    }

    ast.content.clear();
    ast.value = NodeValue::Table(Box::new(NodeTable {
        alignments,
        num_columns,
        num_rows,
        num_nonempty_cells,
    }));
}

fn new_node<'a>(arena: &'a Arena<'a>, value: NodeValue, start: LineColumn) -> Node<'a> {
    let mut ast = Ast::new(value, start);
    ast.open = false;
    arena.alloc(ast.into())
}

/// Ruby の String#strip と同じく、前後の ASCII の空白と NUL を落とす (全角空白は残す)。
fn strip_cell(cell: &str) -> &str {
    cell.trim_matches(|c: char| c.is_ascii_whitespace() || c == '\0' || c == '\u{0b}')
}

/// Kramdown の TABLE_SEP_LINE: `+|: \t-` だけでできていて `-` を含む行。
fn is_separator_line(line: &str) -> bool {
    line.contains('-') && line.bytes().all(|b| matches!(b, b'+' | b'|' | b':' | b' ' | b'\t' | b'-'))
}

/// Kramdown の TABLE_HSEP_ALIGN を区切り行に順に当てる。
fn parse_alignments(line: &str) -> Vec<TableAlignment> {
    let bytes = line.as_bytes();
    let mut alignments = vec![];
    let mut i = 0;
    let mut consumed_until = 0;
    while i < bytes.len() {
        if bytes[i] != b'-' {
            i += 1;
            continue;
        }
        let left = i > consumed_until && bytes[i - 1] == b':';
        while i < bytes.len() && bytes[i] == b'-' {
            i += 1;
        }
        let right = bytes.get(i) == Some(&b':');
        if right {
            i += 1;
        }
        consumed_until = i;
        alignments.push(match (left, right) {
            (false, false) => TableAlignment::None,
            (true, false) => TableAlignment::Left,
            (false, true) => TableAlignment::Right,
            (true, true) => TableAlignment::Center,
        });
    }
    alignments
}

struct Row {
    cells: Vec<String>,
    separators: usize,
}

/// 閉じの探索に失敗したものを覚えておく。後ろの位置から探し直しても見つからないので、
/// 同じ探索を繰り返して行の長さの 2 乗の時間がかかるのを防ぐ。
#[derive(Default)]
struct Unclosed {
    code_element: bool,
    ruby: bool,
}

/// バッククォートの列ごとに、コードスパンとして閉じるならその終わりの位置 (列の始まりの位置で引く)。
/// 列の長さが毎回違う入力でも、行を 1 回走査するだけで決まる。
fn code_span_ends(line: &str) -> HashMap<usize, Option<usize>> {
    let bytes = line.as_bytes();
    let mut runs = vec![];
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let len = bytes[i..].iter().take_while(|&&b| b == b'`').count();
            runs.push((i, len));
            i += len;
        } else {
            i += 1;
        }
    }

    let mut ends = HashMap::with_capacity(runs.len());
    let mut next_run_end_by_len: HashMap<usize, usize> = HashMap::new();
    for &(start, len) in runs.iter().rev() {
        ends.insert(start, next_run_end_by_len.get(&len).copied());
        next_run_end_by_len.insert(len, start + len);
    }
    ends
}

/// 行をセルに分ける。
fn split_row(line: &str) -> Row {
    let bytes = line.as_bytes();
    let mut cells = vec![String::new()];
    let mut separators = 0;
    let mut unclosed = Unclosed::default();
    let code_spans = if line.contains('`') { code_span_ends(line) } else { HashMap::new() };
    let mut i = 0;

    while i < bytes.len() {
        let rest = &line[i..];
        let protected = match bytes[i] {
            b'`' => Some(code_span_len(rest, i, &code_spans)),
            b'[' => wiki_link_len(rest),
            b'<' => code_element_len(rest, &mut unclosed),
            b'|' => ruby_len(rest, &mut unclosed),
            _ if rest.starts_with('｜') => ruby_len(rest, &mut unclosed),
            _ => None,
        };
        if let Some(len) = protected {
            cells.last_mut().unwrap().push_str(&rest[..len]);
            i += len;
            continue;
        }

        if rest.starts_with("\\|") {
            cells.last_mut().unwrap().push('|');
            i += 2;
        } else if bytes[i] == b'|' {
            cells.push(String::new());
            separators += 1;
            i += 1;
        } else {
            let c = rest.chars().next().unwrap();
            cells.last_mut().unwrap().push(c);
            i += c.len_utf8();
        }
    }

    Row { cells, separators }
}

/// コードスパン: CommonMark と同じく、同じ長さのバッククォートの列で閉じる。閉じなければバッククォートだけ。
/// セルの分割だけに使うので、エスケープした `` \` `` や 81 個以上の列 (comrak はコードスパンにしない) も保護する。
fn code_span_len(rest: &str, start: usize, code_spans: &HashMap<usize, Option<usize>>) -> usize {
    let open = rest.bytes().take_while(|&b| b == b'`').count();
    match code_spans.get(&start) {
        Some(Some(end)) => end - start,
        _ => open,
    }
}

/// `[[` + `[` と `]` 以外の 1 文字以上 + `]]`
pub fn wiki_link_len(s: &str) -> Option<usize> {
    let rest = s.strip_prefix("[[")?;
    let end = rest.find([']', '['])?;
    (end > 0 && rest[end..].starts_with("]]")).then_some(2 + end + 2)
}

fn code_element_len(s: &str, unclosed: &mut Unclosed) -> Option<usize> {
    if unclosed.code_element || !s.starts_with("<code") {
        return None;
    }
    let len = s.find('>').and_then(|open_end| {
        let close = s[open_end..].find("</code>")?;
        Some(open_end + close + "</code>".len())
    });
    unclosed.code_element = len.is_none();
    len
}

/// `[｜|]([^《｜|]+)《([^》]+)》`。`|` の直後が空白ならセル区切りとみなす。
fn ruby_len(s: &str, unclosed: &mut Unclosed) -> Option<usize> {
    if unclosed.ruby {
        return None;
    }
    let rest = if let Some(rest) = s.strip_prefix('|') {
        if rest.starts_with([' ', '\t']) {
            return None;
        }
        rest
    } else {
        s.strip_prefix('｜')?
    };
    let base_len = rest.find(['《', '｜', '|'])?;
    if base_len == 0 || !rest[base_len..].starts_with('《') {
        return None;
    }
    let text = &rest[base_len + '《'.len_utf8()..];
    let Some(text_len) = text.find('》') else {
        unclosed.ruby = true;
        return None;
    };
    (text_len > 0).then(|| s.len() - text.len() + text_len + '》'.len_utf8())
}

/// 閉じるフェンスが後ろに無いフェンスの開始行 (1 始まり)。
/// Kramdown は閉じないフェンスをコードにせず段落の文字として扱う (CommonMark は文書の終わりまでコードにする)。
/// 引用やリストの中も見るため、行頭の空白と `>` を除いてから判定する。
pub fn unclosed_fence_lines(input: &str) -> HashSet<usize> {
    let lines: Vec<&str> = split_lines(input)
        .map(|line| line.trim_start_matches([' ', '\t', '>']).trim_end_matches(['\n', '\r', ' ', '\t']))
        .collect();
    let closes: Vec<Option<(u8, usize)>> = lines.iter().map(|line| fence_close(line)).collect();

    // 各行以降にある閉じフェンスの最長 (` と ~ ごと)。開始行が閉じるかを行数に比例する時間で決める
    let mut longest_after = vec![[0usize; 2]; lines.len() + 1];
    for i in (0..lines.len()).rev() {
        longest_after[i] = longest_after[i + 1];
        if let Some((fence, len)) = closes[i] {
            let slot = &mut longest_after[i][fence_slot(fence)];
            *slot = (*slot).max(len);
        }
    }

    let mut unclosed = HashSet::new();
    let mut i = 0;
    while i < lines.len() {
        let Some((fence, len)) = fence_open(lines[i]) else {
            i += 1;
            continue;
        };
        if longest_after[i + 1][fence_slot(fence)] < len {
            unclosed.insert(i + 1);
            i += 1;
            continue;
        }
        i += 1;
        // longest_after で閉じる行が後ろにあると分かっているので、境界の検査は要らない
        while !closes[i].is_some_and(|(f, l)| f == fence && l >= len) {
            i += 1;
        }
        i += 1;
    }
    unclosed
}

fn fence_slot(fence: u8) -> usize {
    usize::from(fence == b'~')
}

fn fence_close(line: &str) -> Option<(u8, usize)> {
    let fence = *line.as_bytes().first()?;
    ((fence == b'`' || fence == b'~') && line.bytes().all(|b| b == fence)).then_some((fence, line.len()))
}

fn fence_open(line: &str) -> Option<(u8, usize)> {
    let fence = *line.as_bytes().first()?;
    if fence != b'`' && fence != b'~' {
        return None;
    }
    let len = line.bytes().take_while(|&b| b == fence).count();
    if len < 3 || (fence == b'`' && line[len..].contains('`')) {
        return None;
    }
    Some((fence, len))
}

/// Parser::parse と同じく \r・\n・\r\n で行に分ける。
fn split_lines(input: &str) -> impl Iterator<Item = &str> {
    let bytes = input.as_bytes();
    let mut ix = 0;
    std::iter::from_fn(move || {
        if ix >= bytes.len() {
            return None;
        }
        let start = ix;
        while ix < bytes.len() && bytes[ix] != b'\n' && bytes[ix] != b'\r' {
            ix += 1;
        }
        if ix < bytes.len() && bytes[ix] == b'\r' {
            ix += 1;
        }
        if ix < bytes.len() && bytes[ix] == b'\n' {
            ix += 1;
        }
        Some(&input[start..ix])
    })
}

/// ブロック IAL の行 `{: .class #id}`。
/// 参照名は目次の `toc` だけを扱い、Kramdown と同じ id `markdown-toc` にする。`key="value"` は扱わない。
pub fn parse_ial(line: &str) -> Option<Attributes> {
    let line = line.trim_end_matches(['\n', '\r', ' ', '\t']);
    let inner = line.strip_prefix("{:")?.strip_suffix('}')?;
    if inner.is_empty() || inner.starts_with([':', '/']) || inner.contains('}') {
        return None;
    }

    let mut attrs = Attributes::default();
    for token in inner.split_ascii_whitespace() {
        if let Some(class) = token.strip_prefix('.') {
            attrs.classes.push(class.to_string());
        } else if let Some(id) = token.strip_prefix('#') {
            attrs.id = Some(id.to_string());
        } else if token == "toc" && attrs.id.is_none() {
            attrs.id = Some("markdown-toc".to_string());
        }
    }
    Some(attrs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(line: &str) -> Vec<String> {
        split_row(line).cells
    }

    fn kramdown_options() -> crate::Options<'static> {
        let mut options = crate::Options::default();
        options.parse.kramdown = true;
        options
    }

    fn render(input: &str) -> String {
        crate::markdown_to_html(input, &kramdown_options())
    }

    #[test]
    fn splits_outside_protected_spans() {
        assert_eq!(cells("`a|b` | c"), ["`a|b` ", " c"]);
        assert_eq!(cells("[[a|b]] | c"), ["[[a|b]] ", " c"]);
        assert_eq!(cells("|漢字《かんじ》 | c"), ["|漢字《かんじ》 ", " c"]);
        assert_eq!(cells("a | b 《c》 d | e"), ["a ", " b 《c》 d ", " e"]);
        assert_eq!(cells("a \\| b | c"), ["a | b ", " c"]);
        assert_eq!(cells("<code>a|b</code> | c"), ["<code>a|b</code> ", " c"]);
        assert_eq!(cells("``a|`b`` | `c|d | e"), ["``a|`b`` ", " `c", "d ", " e"]);
    }

    #[test]
    fn separator_lines() {
        assert!(is_separator_line("- | -"));
        assert!(is_separator_line("|:--|--:|"));
        assert!(!is_separator_line("| a |"));
        assert!(!is_separator_line("|"));
    }

    #[test]
    fn alignments() {
        use TableAlignment::*;
        assert_eq!(parse_alignments("|:--|:-:|--:|---|"), [Left, Center, Right, None]);
    }

    #[test]
    fn too_many_autocompleted_cells_stay_paragraph() {
        let wide = format!("{}\n", "a|".repeat(MAX_COLUMNS));
        assert!(render(&format!("{wide}{}", "a|\n".repeat(400))).starts_with("<table>"));
        assert!(render(&format!("{wide}{}", "a|\n".repeat(600))).starts_with("<p>"));
    }

    #[test]
    fn too_many_columns_stay_paragraph() {
        assert!(render(&format!("{}\n", "a|".repeat(MAX_COLUMNS))).starts_with("<table>"));
        assert!(render(&format!("{}\n", "a|".repeat(MAX_COLUMNS + 1))).starts_with("<p>"));
    }

    #[test]
    fn only_list_like_thematic_breaks_interrupt_paragraph() {
        assert_eq!(render("文章\n* * *\n次\n"), "<p>文章</p>\n<hr />\n<p>次</p>\n");
        assert!(render("文章\n- - -\n").contains("<hr />"));
        assert!(render("文章\n続き\n----\n").starts_with("<p>文章\n続き\n----"));
    }

    #[test]
    fn every_line_needs_separator() {
        assert!(render("a | b\nc\n").starts_with("<p>"));
        assert!(render("a | b\nc | d\n").starts_with("<table>"));
    }

    #[test]
    fn rows_split_on_bare_carriage_return() {
        assert_eq!(render("a | b\rc | d\n"), render("a | b\nc | d\n"));
        assert!(render("a | `b\rc` | d\n").starts_with("<table>"));
        assert!(render("a | [[b\rc]] | d\n").starts_with("<table>"));
    }

    // 閉じの無い開始が行に並んでも、行の長さに比例する時間で終わる (2 乗だと数十秒かかる長さ)
    #[test]
    fn unclosed_openers_scan_in_linear_time() {
        for line in [
            "<code>".repeat(40_000) + "|",
            "<code".repeat(80_000) + ">|",
            "|a《".repeat(40_000),
            "`".repeat(300) + &"x`".repeat(20_000) + "|",
            (1..=1_000).map(|n| "`".repeat(n) + "x").collect::<String>() + &"y".repeat(1_000_000) + "|",
        ] {
            let started = std::time::Instant::now();
            has_separator(&line);
            assert!(started.elapsed().as_secs() < 2, "{}", &line[..20]);
        }
    }

    #[test]
    fn inline_footnotes_scan_in_linear_time() {
        let mut options = kramdown_options();
        options.extension.footnotes = true;
        options.extension.inline_footnotes = true;
        let render = |input: &str| crate::markdown_to_html(input, &options);

        assert!(render("^[[^[x]").contains("^[[<sup id=\"fnref:__inline_1\">"));
        assert!(render("^[a \\] b]").contains("<p>a ] b&nbsp;"));
        for input in ["^[".repeat(160_000), "[]^[x".repeat(40_000), "^[[]".repeat(80_000), "^[*".repeat(100_000)] {
            let started = std::time::Instant::now();
            render(&input);
            assert!(started.elapsed().as_secs() < 2, "{}", &input[..10]);
        }
    }

    #[test]
    fn nested_footnote_references_scan_in_linear_time() {
        let mut options = kramdown_options();
        options.extension.footnotes = true;
        options.extension.inline_footnotes = true;
        let render = |input: &str| crate::markdown_to_html(input, &options);

        // 内側の `[^]` は脚注名が空で失敗するだけなので、外側は `[^*a*]` と同じく脚注名として読む (強調にしない)
        assert_eq!(render("[^*a*[^]]"), "<p>[^*a*[^]]</p>\n");
        // 走査が止まった後に開いた `[^b]` は走査する
        assert!(render("[^a `c`] [^b]\n\n[^b]: x").starts_with("<p>[^a <code>c</code>] <sup id=\"fnref:b\">"));
        for input in [
            "[^".repeat(50_000) + &"]".repeat(50_000),
            "[^x".repeat(50_000) + &"]".repeat(50_000),
            "^[".repeat(50_000) + &"]".repeat(50_000),
            // 1 回目の停止 (コード) の後に開いた `[` も、種類の違う 2 回目の停止 (複数行の HTML) の後は省く
            "[^a `c`] ".to_string() + &"[^x".repeat(20_000) + "<a\nb>" + &"]".repeat(20_000),
        ] {
            let started = std::time::Instant::now();
            render(&input);
            assert!(started.elapsed().as_secs() < 2, "{}", &input[..12]);
        }
    }

    #[test]
    fn unclosed_fences() {
        assert_eq!(unclosed_fence_lines("a\n```\nb\n"), HashSet::from([2]));
        assert!(unclosed_fence_lines("```\nb\n```\n").is_empty());
        assert_eq!(unclosed_fence_lines("```ruby\nx\n~~~\n```\ny\n```\n"), HashSet::from([6]));
        assert_eq!(unclosed_fence_lines("```\nx\n\n~~~\ny\n~~~\n"), HashSet::from([1]));
        assert!(unclosed_fence_lines("> ```\n> x\n> ```\n").is_empty());
    }

    #[test]
    fn ial() {
        let attrs = parse_ial("{: .inline-list #x data-a=\"b\"}").unwrap();
        assert_eq!(attrs.classes, ["inline-list"]);
        assert_eq!(attrs.id.as_deref(), Some("x"));
        assert!(attrs.pairs.is_empty());
        assert_eq!(parse_ial("{:toc}").unwrap().id.as_deref(), Some("markdown-toc"));
        assert!(parse_ial("{::comment}").is_none());
        assert!(parse_ial("{: .a} x").is_none());
    }
}
