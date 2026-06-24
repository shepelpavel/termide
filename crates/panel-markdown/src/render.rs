//! Markdown → terminal pseudographics renderer.
//!
//! Parses Markdown with `pulldown-cmark` and lays it out into owned
//! `ratatui` [`Line`]s wrapped to a target width. Block constructs (headings,
//! lists, quotes, code, tables, rules) are rendered as text pseudographics;
//! fenced code blocks are syntax-highlighted via the `highlight` crate.
//!
//! Besides the lines, the renderer returns the screen cell ranges of links and
//! images ([`LinkSpan`]) so the panel can hit-test clicks and open them.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use termide_core::ThemeColors;
use termide_highlight::{global_highlighter, HighlightCache};
use unicode_width::UnicodeWidthStr;

/// One styled word (no internal spaces), with an optional link id.
type Word = (String, Style, Option<usize>);

/// A clickable region: a half-open `[start, end)` column range on a rendered
/// line that opens `url`.
#[derive(Debug, Clone)]
pub struct LinkSpan {
    pub line: usize,
    pub start: u16,
    pub end: u16,
    pub url: String,
}

/// Rendered document: wrapped lines plus link hit-areas.
pub struct Rendered {
    pub lines: Vec<Line<'static>>,
    pub links: Vec<LinkSpan>,
}

/// Render `src` for the given inner `width`.
pub fn render_markdown(src: &str, width: u16, colors: &ThemeColors, is_light: bool) -> Rendered {
    let mut b = Builder::new(width, colors, is_light);
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    for ev in Parser::new_ext(src, opts) {
        b.event(ev);
    }
    b.finish()
}

/// A pending table being collected between `Tag::Table` and its end.
struct Table {
    aligns: usize,
    rows: Vec<Vec<String>>,
    header_rows: usize,
    cur_row: Vec<String>,
}

/// A link span recorded during wrapping, before url ids are resolved.
struct PendingLink {
    line: usize,
    start: u16,
    end: u16,
    url_id: usize,
}

struct Builder<'c> {
    width: usize,
    colors: &'c ThemeColors,
    is_light: bool,

    lines: Vec<Line<'static>>,
    /// Inline runs accumulated for the current block (paragraph/heading/item).
    runs: Vec<Word>,
    /// Emphasis/code/link styling, innermost last.
    style_stack: Vec<Style>,

    /// List nesting; `None` = bullet, `Some(n)` = next ordinal for ordered list.
    list_stack: Vec<Option<u64>>,
    /// Block-quote nesting depth.
    quote_depth: usize,

    /// When inside a table cell, text is captured here instead of `runs`.
    table: Option<Table>,
    in_cell: bool,

    /// Verbatim text of the code block currently being collected.
    code_buf: String,
    code_lang: String,
    in_code: bool,

    /// Pending list-item marker applied to the first wrapped line.
    item_marker: Option<(Vec<Span<'static>>, String)>,

    /// Link/image URLs; words carry an index into this table.
    urls: Vec<String>,
    /// The link id active for the inline text currently being emitted.
    cur_link: Option<usize>,
    /// Recorded link hit-areas (url ids resolved in `finish`).
    pending_links: Vec<PendingLink>,
}

impl<'c> Builder<'c> {
    fn new(width: u16, colors: &'c ThemeColors, is_light: bool) -> Self {
        Self {
            width: (width as usize).max(1),
            colors,
            is_light,
            lines: Vec::new(),
            runs: Vec::new(),
            style_stack: Vec::new(),
            list_stack: Vec::new(),
            quote_depth: 0,
            table: None,
            in_cell: false,
            code_buf: String::new(),
            code_lang: String::new(),
            in_code: false,
            item_marker: None,
            urls: Vec::new(),
            cur_link: None,
            pending_links: Vec::new(),
        }
    }

    fn base(&self) -> Style {
        Style::default().fg(self.colors.fg)
    }

    fn cur_style(&self) -> Style {
        self.style_stack
            .last()
            .copied()
            .unwrap_or_else(|| self.base())
    }

    fn add_url(&mut self, url: String) -> Option<usize> {
        if url.is_empty() {
            return None;
        }
        self.urls.push(url);
        Some(self.urls.len() - 1)
    }

    fn push_text(&mut self, text: &str) {
        if self.in_cell {
            if let Some(t) = self.table.as_mut() {
                if let Some(cell) = t.cur_row.last_mut() {
                    cell.push_str(text);
                }
            }
            return;
        }
        let style = self.cur_style();
        let link = self.cur_link;
        self.runs.push((text.to_string(), style, link));
    }

    /// Append a literal styled span without word-splitting (markers, prefixes).
    fn push_span(&mut self, text: impl Into<String>, style: Style) {
        let link = self.cur_link;
        self.runs.push((text.into(), style, link));
    }

    fn push_blank(&mut self) {
        if self.lines.last().is_some_and(|l| l.spans.is_empty()) {
            return;
        }
        self.lines.push(Line::default());
    }

    /// Indentation prefix for the current list/quote context.
    fn context_prefix(&self) -> Vec<Span<'static>> {
        let mut spans = Vec::new();
        for _ in 0..self.quote_depth {
            spans.push(Span::styled(
                "│ ",
                Style::default().fg(self.colors.disabled),
            ));
        }
        let indent = self.list_stack.len().saturating_sub(1) * 2;
        if indent > 0 {
            spans.push(Span::raw(" ".repeat(indent)));
        }
        spans
    }

    fn link_style(&self) -> Style {
        Style::default()
            .fg(self.colors.info)
            .add_modifier(Modifier::UNDERLINED)
    }

    fn event(&mut self, ev: Event<'_>) {
        match ev {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => {
                if self.in_code {
                    self.code_buf.push_str(&t);
                } else {
                    self.push_text(&t);
                }
            }
            Event::Code(t) => {
                let style = Style::default().fg(self.colors.success);
                self.push_span(t.into_string(), style);
            }
            Event::SoftBreak => self.push_text(" "),
            Event::HardBreak => {
                let prefix = self.context_prefix();
                self.flush_inline(prefix.clone(), prefix);
            }
            Event::Rule => {
                self.push_blank();
                self.lines.push(Line::styled(
                    "─".repeat(self.width),
                    Style::default().fg(self.colors.disabled),
                ));
                self.push_blank();
            }
            Event::TaskListMarker(done) => {
                let mark = if done { "[x] " } else { "[ ] " };
                self.push_span(mark, Style::default().fg(self.colors.info));
            }
            Event::Html(t) | Event::InlineHtml(t) => self.push_text(t.trim_end_matches('\n')),
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {}
            Tag::Heading { level, .. } => {
                self.push_blank();
                let weight = Style::default()
                    .fg(self.colors.info)
                    .add_modifier(Modifier::BOLD);
                let hashes = "#".repeat(heading_depth(level));
                self.push_span(
                    format!("{hashes} "),
                    Style::default().fg(self.colors.disabled),
                );
                self.style_stack.push(weight);
            }
            Tag::BlockQuote(_) => {
                self.push_blank();
                self.quote_depth += 1;
            }
            Tag::CodeBlock(kind) => {
                self.push_blank();
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => {
                        let s = info.into_string();
                        s.split_whitespace().next().unwrap_or("").to_string()
                    }
                    CodeBlockKind::Indented => String::new(),
                };
                self.in_code = true;
                self.code_lang = lang;
                self.code_buf.clear();
            }
            Tag::List(start) => {
                if !self.runs.is_empty() || self.item_marker.is_some() {
                    let prefix = self.context_prefix();
                    self.flush_inline(prefix.clone(), prefix);
                }
                self.list_stack.push(start);
            }
            Tag::Item => {
                let prefix = self.context_prefix();
                let marker = match self.list_stack.last_mut() {
                    Some(Some(n)) => {
                        let m = format!("{n}. ");
                        *n += 1;
                        m
                    }
                    _ => "• ".to_string(),
                };
                self.item_marker = Some((prefix, marker));
            }
            Tag::Emphasis => {
                let s = self.cur_style().add_modifier(Modifier::ITALIC);
                self.style_stack.push(s);
            }
            Tag::Strong => {
                let s = self.cur_style().add_modifier(Modifier::BOLD);
                self.style_stack.push(s);
            }
            Tag::Strikethrough => {
                let s = self.cur_style().add_modifier(Modifier::CROSSED_OUT);
                self.style_stack.push(s);
            }
            Tag::Link { dest_url, .. } => {
                self.cur_link = self.add_url(dest_url.into_string());
                let s = self.link_style();
                self.style_stack.push(s);
            }
            Tag::Image { dest_url, .. } => {
                // Clickable pictogram + alt text; the alt children render as the
                // link label. Opening the link launches the browser.
                self.cur_link = self.add_url(dest_url.into_string());
                let s = self.link_style();
                self.push_span("🖼 ", s);
                self.style_stack.push(s);
            }
            Tag::Table(aligns) => {
                self.table = Some(Table {
                    aligns: aligns.len(),
                    rows: Vec::new(),
                    header_rows: 0,
                    cur_row: Vec::new(),
                });
                self.push_blank();
            }
            Tag::TableHead => {
                if let Some(t) = self.table.as_mut() {
                    t.cur_row = Vec::new();
                }
            }
            Tag::TableRow => {
                if let Some(t) = self.table.as_mut() {
                    t.cur_row = Vec::new();
                }
            }
            Tag::TableCell => {
                if let Some(t) = self.table.as_mut() {
                    t.cur_row.push(String::new());
                }
                self.in_cell = true;
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                let prefix = self.context_prefix();
                self.flush_inline(prefix.clone(), prefix);
                if self.quote_depth == 0 && self.list_stack.is_empty() {
                    self.push_blank();
                }
            }
            TagEnd::Heading(_) => {
                self.style_stack.pop();
                let prefix = self.context_prefix();
                self.flush_inline(prefix.clone(), prefix);
                self.push_blank();
            }
            // A raw HTML block is its own paragraph: flush the accumulated
            // text and separate it, so a following block (e.g. a heading)
            // does not merge onto the same line.
            TagEnd::HtmlBlock => {
                let prefix = self.context_prefix();
                self.flush_inline(prefix.clone(), prefix);
                self.push_blank();
            }
            TagEnd::BlockQuote(_) => {
                self.quote_depth = self.quote_depth.saturating_sub(1);
                if self.quote_depth == 0 {
                    self.push_blank();
                }
            }
            TagEnd::CodeBlock => {
                self.in_code = false;
                self.flush_code_block();
                self.push_blank();
            }
            TagEnd::List(_) => {
                self.list_stack.pop();
                if self.list_stack.is_empty() {
                    self.push_blank();
                }
            }
            TagEnd::Item => {
                if !self.runs.is_empty() || self.item_marker.is_some() {
                    let prefix = self.context_prefix();
                    self.flush_inline(prefix.clone(), prefix);
                }
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.style_stack.pop();
            }
            TagEnd::Link | TagEnd::Image => {
                self.style_stack.pop();
                self.cur_link = None;
            }
            TagEnd::TableCell => {
                self.in_cell = false;
            }
            TagEnd::TableHead => {
                if let Some(t) = self.table.as_mut() {
                    let row = std::mem::take(&mut t.cur_row);
                    t.rows.push(row);
                    t.header_rows = 1;
                }
            }
            TagEnd::TableRow => {
                if let Some(t) = self.table.as_mut() {
                    let row = std::mem::take(&mut t.cur_row);
                    t.rows.push(row);
                }
            }
            TagEnd::Table => {
                self.flush_table();
                self.push_blank();
            }
            _ => {}
        }
    }

    /// Render the captured code block with per-line syntax highlighting.
    fn flush_code_block(&mut self) {
        let code = std::mem::take(&mut self.code_buf);
        let lang = std::mem::take(&mut self.code_lang);

        // A ```mermaid block renders as the diagram itself (reusing the shared
        // mermaid crate); unsupported diagram kinds fall through to code.
        if lang.eq_ignore_ascii_case("mermaid") {
            if let Some(diagram) = termide_mermaid::render_to_lines(&code) {
                let bar = Style::default().fg(self.colors.disabled);
                for line in diagram {
                    let spans = vec![Span::styled("┊ ", bar), Span::styled(line, self.base())];
                    self.lines.push(Line::from(spans));
                }
                return;
            }
        }

        let mut cache = HighlightCache::new(global_highlighter(), self.is_light, self.colors.fg);
        if !lang.is_empty() {
            cache.set_syntax(&lang);
            if !cache.has_syntax() {
                cache.set_syntax_from_path(std::path::Path::new(&format!("code.{lang}")));
            }
        }
        if cache.has_syntax() {
            cache.set_document(&code);
        }
        let bar = Style::default().fg(self.colors.disabled);
        for (i, line) in code.lines().enumerate() {
            let mut spans: Vec<Span<'static>> = vec![Span::styled("┊ ", bar)];
            if cache.has_syntax() {
                for (text, style) in cache.get_line_segments(i, line) {
                    spans.push(Span::styled(text.to_string(), *style));
                }
            } else {
                spans.push(Span::styled(line.to_string(), self.base()));
            }
            self.lines.push(Line::from(spans));
        }
    }

    /// Draw the collected table with box-drawing borders.
    fn flush_table(&mut self) {
        let Some(t) = self.table.take() else { return };
        if t.rows.is_empty() {
            return;
        }
        let ncols = t
            .aligns
            .max(t.rows.iter().map(|r| r.len()).max().unwrap_or(0));
        if ncols == 0 {
            return;
        }
        let mut widths = vec![0usize; ncols];
        for row in &t.rows {
            for (c, cell) in row.iter().enumerate() {
                widths[c] = widths[c].max(cell.trim().width());
            }
        }
        for w in &mut widths {
            *w = (*w).max(1);
        }
        let overhead = 3 * ncols + 1;
        let budget = self.width.saturating_sub(overhead).max(ncols);
        let total: usize = widths.iter().sum();
        if total > budget {
            for w in &mut widths {
                let scaled = (*w * budget) / total.max(1);
                *w = scaled.max(1);
            }
        }

        let dis = Style::default().fg(self.colors.disabled);
        let border = |left: &str, mid: &str, right: &str, fill: &str| -> Line<'static> {
            let mut s = String::from(left);
            for (i, w) in widths.iter().enumerate() {
                s.push_str(&fill.repeat(w + 2));
                s.push_str(if i + 1 == widths.len() { right } else { mid });
            }
            Line::styled(s, dis)
        };

        self.lines.push(border("┌", "┬", "┐", "─"));
        for (ri, row) in t.rows.iter().enumerate() {
            let header = ri < t.header_rows;
            let mut spans: Vec<Span<'static>> = vec![Span::styled("│", dis)];
            for (c, w) in widths.iter().enumerate() {
                let raw = row.get(c).map(|s| s.trim()).unwrap_or("");
                let cell = clip(raw, *w);
                let pad = w.saturating_sub(cell.width());
                let style = if header {
                    self.base().add_modifier(Modifier::BOLD)
                } else {
                    self.base()
                };
                spans.push(Span::raw(" "));
                spans.push(Span::styled(cell, style));
                spans.push(Span::raw(" ".repeat(pad + 1)));
                spans.push(Span::styled("│", dis));
            }
            self.lines.push(Line::from(spans));
            if header && t.header_rows > 0 && ri + 1 == t.header_rows {
                self.lines.push(border("├", "┼", "┤", "─"));
            }
        }
        self.lines.push(border("└", "┴", "┘", "─"));
    }

    /// Wrap accumulated inline runs to width and append as lines, applying the
    /// prefixes (first line vs continuation) and recording link hit-areas.
    fn flush_inline(&mut self, first_prefix: Vec<Span<'static>>, cont_prefix: Vec<Span<'static>>) {
        let (first_prefix, cont_prefix) = if let Some((pfx, marker)) = self.item_marker.take() {
            let mut first = pfx.clone();
            first.push(Span::styled(
                marker.clone(),
                Style::default().fg(self.colors.info),
            ));
            let mut cont = pfx;
            cont.push(Span::raw(" ".repeat(marker.width())));
            (first, cont)
        } else {
            (first_prefix, cont_prefix)
        };

        let runs = std::mem::take(&mut self.runs);
        if runs.is_empty() {
            return;
        }
        let words = split_words(&runs);
        if words.is_empty() {
            return;
        }

        let first_w = prefix_width(&first_prefix);
        let cont_w = prefix_width(&cont_prefix);

        let base = self.lines.len();
        let mut out: Vec<Line<'static>> = Vec::new();
        let mut cur: Vec<Span<'static>> = first_prefix;
        let mut cur_text_w = 0usize;
        let mut cur_prefix_w = first_w;
        let mut avail = self.width.saturating_sub(first_w).max(1);

        for (text, style, link) in words {
            let ww = text.width();
            if cur_text_w > 0 && cur_text_w + 1 + ww > avail {
                out.push(Line::from(std::mem::take(&mut cur)));
                cur = cont_prefix.clone();
                cur_text_w = 0;
                cur_prefix_w = cont_w;
                avail = self.width.saturating_sub(cont_w).max(1);
            }
            if cur_text_w > 0 {
                cur.push(Span::raw(" "));
                cur_text_w += 1;
            }
            let start = cur_prefix_w + cur_text_w;
            if let Some(url_id) = link {
                self.pending_links.push(PendingLink {
                    line: base + out.len(),
                    start: start as u16,
                    end: (start + ww) as u16,
                    url_id,
                });
            }
            cur.push(Span::styled(text, style));
            cur_text_w += ww;
        }
        out.push(Line::from(cur));
        self.lines.extend(out);
    }

    fn finish(mut self) -> Rendered {
        while self.lines.last().is_some_and(|l| l.spans.is_empty()) {
            self.lines.pop();
        }
        let line_count = self.lines.len();
        let links = self
            .pending_links
            .into_iter()
            .filter(|p| p.line < line_count)
            .map(|p| LinkSpan {
                line: p.line,
                start: p.start,
                end: p.end,
                url: self.urls[p.url_id].clone(),
            })
            .collect();
        Rendered {
            lines: self.lines,
            links,
        }
    }
}

fn heading_depth(level: HeadingLevel) -> usize {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn prefix_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|s| s.content.width()).sum()
}

/// Split styled runs into whitespace-delimited words, preserving style + link.
fn split_words(runs: &[Word]) -> Vec<Word> {
    let mut words = Vec::new();
    for (text, style, link) in runs {
        for piece in text.split(' ') {
            if piece.is_empty() {
                continue;
            }
            words.push((piece.to_string(), *style, *link));
        }
    }
    words
}

/// Clip a string to a display width (best-effort, char boundary).
fn clip(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for ch in s.chars() {
        let cw = ch.to_string().width();
        if w + cw > width.saturating_sub(1) {
            out.push('…');
            break;
        }
        out.push(ch);
        w += cw;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colors() -> ThemeColors {
        ThemeColors::default()
    }

    fn render(src: &str) -> Vec<String> {
        render_markdown(src, 80, &colors(), false)
            .lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn heading_has_marker_and_text() {
        let out = render("# Title");
        assert!(out.iter().any(|l| l.contains("# Title")), "{out:?}");
    }

    #[test]
    fn html_block_does_not_merge_into_next_heading() {
        let out = render("<p>Some HTML paragraph.</p>\n\n## A heading\n\nbody\n");
        // The HTML text and the heading must land on separate lines.
        assert!(
            out.iter()
                .any(|l| l.contains("Some HTML paragraph.") && !l.contains("A heading")),
            "{out:?}"
        );
        assert!(
            out.iter()
                .any(|l| l.contains("# A heading") && !l.contains("HTML")),
            "{out:?}"
        );
    }

    #[test]
    fn nested_lists_indent() {
        let out = render("- a\n  - b\n- c");
        let joined = out.join("\n");
        assert!(joined.contains("• a"), "{out:?}");
        assert!(joined.contains("• b"), "{out:?}");
        let b = out.iter().find(|l| l.contains("• b")).unwrap();
        assert!(b.starts_with("  "), "nested not indented: {b:?}");
    }

    #[test]
    fn ordered_list_numbers() {
        let out = render("1. one\n2. two");
        let joined = out.join("\n");
        assert!(joined.contains("1. one"), "{out:?}");
        assert!(joined.contains("2. two"), "{out:?}");
    }

    #[test]
    fn blockquote_prefixed() {
        let out = render("> quoted");
        assert!(
            out.iter().any(|l| l.contains("│") && l.contains("quoted")),
            "{out:?}"
        );
    }

    #[test]
    fn table_drawn_with_box() {
        let md = "| A | B |\n|---|---|\n| 1 | 2 |";
        let out = render(md);
        let joined = out.join("\n");
        assert!(joined.contains("┌"), "no top border: {out:?}");
        assert!(joined.contains("├"), "no header sep: {out:?}");
        assert!(joined.contains("└"), "no bottom border: {out:?}");
        assert!(joined.contains("A") && joined.contains("B"), "{out:?}");
    }

    #[test]
    fn embedded_mermaid_renders_diagram() {
        let md = "```mermaid\nsequenceDiagram\nA->>B: hi\n```";
        let out = render(md);
        let joined = out.join("\n");
        // The diagram (boxes + arrow), not the raw source line.
        assert!(joined.contains('┌'), "no diagram boxes: {out:?}");
        assert!(joined.contains('▶'), "no arrowhead: {out:?}");
    }

    #[test]
    fn fenced_code_rendered() {
        let md = "```rust\nlet x = 1;\n```";
        let out = render(md);
        assert!(out.iter().any(|l| l.contains("let x = 1;")), "{out:?}");
    }

    #[test]
    fn horizontal_rule() {
        let out = render("a\n\n---\n\nb");
        assert!(out.iter().any(|l| l.contains("───")), "{out:?}");
    }

    #[test]
    fn wraps_long_paragraph() {
        let long = "word ".repeat(40);
        let out = render_markdown(&long, 20, &colors(), false);
        assert!(out.lines.len() > 1, "expected wrapping into multiple lines");
        for l in &out.lines {
            let w: usize = l.spans.iter().map(|s| s.content.width()).sum();
            assert!(w <= 20, "line exceeds width: {w}");
        }
    }

    #[test]
    fn link_hit_area_recorded() {
        let out = render_markdown("see [docs](https://example.com) now", 80, &colors(), false);
        assert_eq!(out.links.len(), 1, "{:?}", out.links);
        let link = &out.links[0];
        assert_eq!(link.url, "https://example.com");
        // The hit area should cover the visible "docs" label.
        let line: String = out.lines[link.line]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        let slice: String = line
            .chars()
            .skip(link.start as usize)
            .take((link.end - link.start) as usize)
            .collect();
        assert_eq!(slice, "docs", "line={line:?} span={link:?}");
    }

    #[test]
    fn image_is_clickable_with_icon() {
        let out = render_markdown("![alt text](https://img.test/a.png)", 80, &colors(), false);
        assert!(!out.links.is_empty(), "image should be a link");
        assert_eq!(out.links[0].url, "https://img.test/a.png");
        let joined: String = out.lines[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(joined.contains("🖼"), "no icon: {joined:?}");
        assert!(joined.contains("alt text"), "no alt: {joined:?}");
    }
}
