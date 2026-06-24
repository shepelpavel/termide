//! Layout of parsed diagrams into a character canvas (pseudographics).
//!
//! Phase 1 renders sequence diagrams deterministically: participant boxes in a
//! row, vertical lifelines, and messages as horizontal arrows top-to-bottom.
//! The result is a `Vec<String>` canvas the panel scrolls in two dimensions.

use crate::canvas::{label_width, Canvas};
use crate::parser::{Arrow, ArrowHead, NotePlacement, SeqEvent, Sequence};

/// Lay out a sequence diagram into canvas lines.
pub fn render_sequence(seq: &Sequence) -> Vec<String> {
    if seq.participants.is_empty() {
        return vec!["(empty sequence diagram)".to_string()];
    }
    let n = seq.participants.len();

    // Box inner widths and centers.
    let inner: Vec<usize> = seq
        .participants
        .iter()
        .map(|p| label_width(&p.label).max(1))
        .collect();
    let box_w: Vec<usize> = inner.iter().map(|w| w + 2).collect();

    // Minimum gap between adjacent box edges; widen for long adjacent messages.
    let mut edge_gap = vec![4usize; n.saturating_sub(1)];
    for ev in &seq.events {
        if let SeqEvent::Message { from, to, text, .. } = ev {
            if from != to {
                let a = (*from).min(*to);
                let b = (*from).max(*to);
                if b - a == 1 {
                    edge_gap[a] = edge_gap[a].max(label_width(text) + 2);
                }
            }
        }
    }

    // Center x of each participant.
    let mut center = vec![0usize; n];
    center[0] = box_w[0] / 2;
    for i in 1..n {
        center[i] = center[i - 1] + box_w[i - 1] / 2 + edge_gap[i - 1] + box_w[i] / 2;
    }

    let mut c = Canvas::new();

    // Header boxes (rows 0..=2).
    for i in 0..n {
        let x = center[i] - box_w[i] / 2;
        let label = if seq.participants[i].actor {
            format!("☺{}", seq.participants[i].label)
        } else {
            seq.participants[i].label.clone()
        };
        let inner_w = label_width(&label).max(inner[i]);
        c.draw_box(x, 0, inner_w, &label);
        // Connect the box bottom to its lifeline with a T-junction.
        c.put(center[i], 2, '┬');
    }
    let lifeline_top = 3;
    let mut y = lifeline_top;

    for ev in &seq.events {
        match ev {
            SeqEvent::Message {
                from,
                to,
                text,
                arrow,
            } => {
                if from == to {
                    y = draw_self_message(&mut c, center[*from], y, text, *arrow);
                } else {
                    y = draw_message(&mut c, &center, *from, *to, y, text, *arrow);
                }
            }
            SeqEvent::Note {
                placement,
                targets,
                text,
            } => {
                y = draw_note(&mut c, &center, &box_w, *placement, targets, text, y);
            }
        }
    }

    let bottom = y.max(lifeline_top);
    for &cx in &center {
        c.lifeline(cx, lifeline_top, bottom);
    }

    c.into_lines()
}

fn head_glyph(head: ArrowHead, rightward: bool) -> char {
    match head {
        ArrowHead::Cross => '✗',
        ArrowHead::Open => {
            if rightward {
                '▷'
            } else {
                '◁'
            }
        }
        // Filled and None both get a solid head (None = no explicit head token,
        // but a visible arrowhead still reads better in text).
        _ => {
            if rightward {
                '▶'
            } else {
                '◀'
            }
        }
    }
}

/// Draw a horizontal message arrow between two lifelines; returns the next
/// free row. The line attaches to the source lifeline with a `├`/`┤` junction
/// and crosses any intervening lifelines with `┼`.
fn draw_message(
    c: &mut Canvas,
    centers: &[usize],
    from: usize,
    to: usize,
    y: usize,
    text: &str,
    arrow: Arrow,
) -> usize {
    let (cx0, cx1) = (centers[from], centers[to]);
    let rightward = cx1 > cx0;
    let (lo, hi) = (cx0.min(cx1), cx0.max(cx1));
    let line_ch = if arrow.dashed { '┄' } else { '─' };

    // Label centered above the line.
    if !text.is_empty() {
        let span = hi - lo;
        let start = lo + span.saturating_sub(label_width(text)) / 2;
        c.text(start, y, text);
    }
    let line_y = y + 1;
    c.hline(lo + 1, hi - 1, line_y, line_ch);

    // Cross intervening lifelines.
    for &cx in centers {
        if cx > lo && cx < hi {
            c.put(cx, line_y, '┼');
        }
    }
    // Source junction.
    c.put(cx0, line_y, if rightward { '├' } else { '┤' });
    // Arrowhead just before the target lifeline.
    let head = head_glyph(arrow.head, rightward);
    if rightward {
        c.put(hi - 1, line_y, head);
    } else {
        c.put(lo + 1, line_y, head);
    }
    line_y + 1
}

/// Draw a self-message loop on a single lifeline; returns the next free row.
fn draw_self_message(c: &mut Canvas, cx: usize, y: usize, text: &str, arrow: Arrow) -> usize {
    let line_ch = if arrow.dashed { '┄' } else { '─' };
    let w = 3;
    if !text.is_empty() {
        c.text(cx + 2, y, text);
    }
    c.put(cx, y + 1, '├');
    c.hline(cx + 1, cx + w, y + 1, line_ch);
    c.put(cx + w, y + 1, '┐');
    c.put(cx + w, y + 2, '│');
    c.put(cx + w, y + 3, '┘');
    c.hline(cx + 1, cx + w - 1, y + 3, line_ch);
    c.put(cx, y + 3, head_glyph(arrow.head, false));
    y + 4
}

/// Draw a note box near its target(s); returns the next free row.
fn draw_note(
    c: &mut Canvas,
    center: &[usize],
    box_w: &[usize],
    placement: NotePlacement,
    targets: &[usize],
    text: &str,
    y: usize,
) -> usize {
    let inner = label_width(text).max(1);
    let x = match placement {
        NotePlacement::LeftOf => {
            let cx = center[targets[0]];
            cx.saturating_sub(box_w[targets[0]] / 2 + inner + 3)
        }
        NotePlacement::RightOf => center[targets[0]] + box_w[targets[0]] / 2 + 2,
        NotePlacement::Over => {
            let first = *targets.iter().min().unwrap();
            let last = *targets.iter().max().unwrap();
            let mid = (center[first] + center[last]) / 2;
            mid.saturating_sub(inner / 2 + 1)
        }
    };
    c.draw_box(x, y, inner, text);
    y + 3
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_sequence;

    fn render(src: &str) -> String {
        render_sequence(&parse_sequence(src)).join("\n")
    }

    #[test]
    fn renders_boxes_and_arrow() {
        let out = render("sequenceDiagram\nA->>B: Hello");
        assert!(out.contains('┌') && out.contains('┘'), "no boxes:\n{out}");
        assert!(out.contains('A') && out.contains('B'), "no labels:\n{out}");
        assert!(out.contains("Hello"), "no message label:\n{out}");
        assert!(out.contains('▶'), "no arrowhead:\n{out}");
        assert!(out.contains('│'), "no lifelines:\n{out}");
    }

    #[test]
    fn leftward_arrow_points_left() {
        let out = render("sequenceDiagram\nA->>B: x\nB->>A: y");
        assert!(out.contains('◀'), "no left arrowhead:\n{out}");
    }

    #[test]
    fn dashed_uses_dashed_line() {
        let out = render("sequenceDiagram\nA-->>B: x");
        assert!(out.contains('┄'), "no dashed line:\n{out}");
    }

    #[test]
    fn junctions_connect_and_cross_lifelines() {
        // A->>C spans B's lifeline → cross; source attaches with a T-junction.
        let out = render("sequenceDiagram\nA->>B: x\nA->>C: span");
        assert!(out.contains('├'), "no source junction:\n{out}");
        assert!(out.contains('┼'), "no crossing junction:\n{out}");
    }

    #[test]
    fn empty_is_handled() {
        let out = render_sequence(&parse_sequence("sequenceDiagram"));
        assert_eq!(out, vec!["(empty sequence diagram)".to_string()]);
    }
}
