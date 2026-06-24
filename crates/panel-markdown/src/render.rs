//! Markdown → terminal pseudographics renderer.
//!
//! Parses Markdown with `pulldown-cmark` and drives the shared
//! [`termide_richtext::Builder`] layout engine, which wraps the styled runs to
//! width and emits owned `ratatui` lines plus link hit-areas. This module is
//! only the `pulldown-cmark` → `Builder` adapter; all layout lives in
//! `termide-richtext`.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use termide_core::ThemeColors;
use termide_richtext::Builder;

pub use termide_richtext::{LinkSpan, Rendered};

/// Render `src` for the given inner `width`.
#[must_use]
pub fn render_markdown(src: &str, width: u16, colors: &ThemeColors, is_light: bool) -> Rendered {
    let mut b = Builder::new(width, colors, is_light);
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    for ev in Parser::new_ext(src, opts) {
        event(&mut b, ev);
    }
    b.finish()
}

fn event(b: &mut Builder, ev: Event<'_>) {
    match ev {
        Event::Start(tag) => start(b, tag),
        Event::End(tag) => end(b, tag),
        Event::Text(t) => b.text(&t),
        Event::Code(t) => b.inline_code(&t),
        Event::SoftBreak => b.soft_break(),
        Event::HardBreak => b.hard_break(),
        Event::Rule => b.rule(),
        Event::TaskListMarker(done) => b.task_marker(done),
        Event::Html(t) | Event::InlineHtml(t) => b.text(t.trim_end_matches('\n')),
        _ => {}
    }
}

fn start(b: &mut Builder, tag: Tag<'_>) {
    match tag {
        Tag::Paragraph => {}
        Tag::Heading { level, .. } => b.start_heading(heading_depth(level)),
        Tag::BlockQuote(_) => b.start_quote(),
        Tag::CodeBlock(kind) => {
            let lang = match kind {
                CodeBlockKind::Fenced(info) => {
                    let s = info.into_string();
                    s.split_whitespace().next().unwrap_or("").to_string()
                }
                CodeBlockKind::Indented => String::new(),
            };
            b.start_code_block(&lang);
        }
        Tag::List(start) => b.start_list(start),
        Tag::Item => b.start_item(),
        Tag::Emphasis => b.start_emphasis(),
        Tag::Strong => b.start_strong(),
        Tag::Strikethrough => b.start_strike(),
        Tag::Link { dest_url, .. } => b.start_link(dest_url.into_string()),
        Tag::Image { dest_url, .. } => b.start_image(dest_url.into_string()),
        Tag::Table(aligns) => b.start_table(aligns.len()),
        Tag::TableHead => b.start_table_head(),
        Tag::TableRow => b.start_table_row(),
        Tag::TableCell => b.start_table_cell(),
        _ => {}
    }
}

fn end(b: &mut Builder, tag: TagEnd) {
    match tag {
        TagEnd::Paragraph => b.end_paragraph(),
        TagEnd::Heading(_) => b.end_heading(),
        // A raw HTML block is its own paragraph: flush the accumulated text and
        // separate it, so a following block (e.g. a heading) does not merge
        // onto the same line.
        TagEnd::HtmlBlock => b.flush_block(),
        TagEnd::BlockQuote(_) => b.end_quote(),
        TagEnd::CodeBlock => b.end_code_block(),
        TagEnd::List(_) => b.end_list(),
        TagEnd::Item => b.end_item(),
        TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => b.pop_style(),
        TagEnd::Link | TagEnd::Image => b.end_link(),
        TagEnd::TableCell => b.end_table_cell(),
        TagEnd::TableHead => b.end_table_head(),
        TagEnd::TableRow => b.end_table_row(),
        TagEnd::Table => b.end_table(),
        _ => {}
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

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

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
