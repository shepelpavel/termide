//! Flowchart layout + rendering.
//!
//! A layered (Sugiyama-style) layout: nodes are assigned ranks by longest path
//! (cycles broken via DFS back-edge detection), positioned within ranks by an
//! iterative barycenter pass (so edges run as straight as possible), and
//! connected with orthogonal elbow edges that attach to box borders with
//! T-junctions. A binary branch leaves the two opposite sides of its source;
//! when boxes overlap on the cross axis the edge is a straight run (the boxes
//! sit centred under it). Vertical (`TD`/`BT`) and horizontal (`LR`/`RL`)
//! orientations are supported.

use std::collections::HashSet;

use crate::canvas::{label_width, Canvas};
use crate::parser::{Direction, EdgeLine, Flowchart, NodeShape};

/// Placed node box (top-left `(x, y)`, size `w`×`h`).
#[derive(Clone, Copy)]
struct Rect {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
}

impl Rect {
    fn top(&self) -> usize {
        self.y
    }
    fn bottom(&self) -> usize {
        self.y + self.h - 1
    }
    fn left(&self) -> usize {
        self.x
    }
    fn right(&self) -> usize {
        self.x + self.w - 1
    }
    fn cx(&self) -> usize {
        self.x + self.w / 2
    }
    fn cy(&self) -> usize {
        self.y + self.h / 2
    }
    /// Cross-axis coordinate of the `k`-th of `n` fan-out/-in points.
    fn fan_x(&self, k: usize, n: usize) -> usize {
        self.x + (k + 1) * self.w / (n + 1)
    }
    fn fan_y(&self, k: usize, n: usize) -> usize {
        self.y + (k + 1) * self.h / (n + 1)
    }
}

const BOX_H: usize = 3;
/// Rows between stacked ranks (vertical) — the edge channel. Three rows give a
/// label its own row, distinct from the row a crossing edge routes along.
const V_CHANNEL: usize = 3;
/// Columns between stacked ranks (horizontal) — room for arrows + labels.
const H_CHANNEL: usize = 8;
/// Within-rank spacing.
const COL_GAP: usize = 3;
const ROW_GAP: usize = 1;

/// Render a flowchart into canvas lines.
pub fn render_flowchart(fc: &Flowchart) -> Vec<String> {
    let n = fc.nodes.len();
    if n == 0 {
        return vec!["(empty flowchart)".to_string()];
    }

    let rank = assign_ranks(fc);
    let max_rank = *rank.iter().max().unwrap_or(&0);

    let mut groups: Vec<Vec<usize>> = vec![Vec::new(); max_rank + 1];
    for (i, &r) in rank.iter().enumerate() {
        groups[r].push(i);
    }

    // Box width: at least the node label, but also wide enough to seat the
    // widest incident edge label — so a short title with several labelled
    // edges still leaves the labels room (the text stays centred).
    let mut edge_label = vec![0usize; n];
    for e in &fc.edges {
        let w = label_width(&e.label);
        edge_label[e.from] = edge_label[e.from].max(w);
        edge_label[e.to] = edge_label[e.to].max(w);
    }
    let box_w: Vec<usize> = (0..n)
        .map(|i| {
            let body = fc.nodes[i]
                .body
                .iter()
                .map(|l| label_width(l))
                .max()
                .unwrap_or(0);
            label_width(&fc.nodes[i].label)
                .max(1)
                .max(edge_label[i])
                .max(body)
                + 2
        })
        .collect();

    // Box height: 3 for a plain node; taller when it has a body compartment
    // (title row + separator + one row per body line).
    let box_h: Vec<usize> = fc
        .nodes
        .iter()
        .map(|node| {
            if node.body.is_empty() {
                BOX_H
            } else {
                BOX_H + 1 + node.body.len()
            }
        })
        .collect();

    // Tallest box per rank, used to size the dummy nodes that carry long edges.
    let mut rank_h = vec![1usize; max_rank + 1];
    for i in 0..n {
        rank_h[rank[i]] = rank_h[rank[i]].max(box_h[i]);
    }

    // Expand the graph: an edge spanning more than one rank is split into a
    // chain of segments through thin dummy nodes (one per intermediate rank).
    // The layout then routes the chain around the boxes in between instead of
    // crashing straight through them (classic Sugiyama virtual nodes).
    let mut rank_ext = rank.clone();
    let mut bw = box_w.clone();
    let mut bh = box_h.clone();
    let mut segs: Vec<Seg> = Vec::new();
    let mut dummies: Vec<usize> = Vec::new();
    for e in &fc.edges {
        if e.from == e.to {
            continue; // self-loops not drawn yet
        }
        let (ru, rv) = (rank[e.from], rank[e.to]);
        if rv > ru + 1 {
            let mut prev = e.from;
            for (r, &rh) in (ru + 1..rv).zip(&rank_h[ru + 1..rv]) {
                let d = rank_ext.len();
                rank_ext.push(r);
                bw.push(1);
                bh.push(rh);
                dummies.push(d);
                segs.push(Seg {
                    from: prev,
                    to: d,
                    line: e.line,
                    arrow: false,
                    label: if prev == e.from {
                        e.label.clone()
                    } else {
                        String::new()
                    },
                });
                prev = d;
            }
            segs.push(Seg {
                from: prev,
                to: e.to,
                line: e.line,
                arrow: e.arrow,
                label: String::new(),
            });
        } else {
            segs.push(Seg {
                from: e.from,
                to: e.to,
                line: e.line,
                arrow: e.arrow,
                label: e.label.clone(),
            });
        }
    }
    let ext = rank_ext.len();

    let mut groups_ext: Vec<Vec<usize>> = vec![Vec::new(); max_rank + 1];
    for (i, &r) in rank_ext.iter().enumerate() {
        groups_ext[r].push(i);
    }
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); ext];
    let mut succs: Vec<Vec<usize>> = vec![Vec::new(); ext];
    for s in &segs {
        succs[s.from].push(s.to);
        preds[s.to].push(s.from);
    }

    // Widen the within-rank gap so sibling branches sit far enough apart for
    // their edge labels to fit on the connecting run.
    let max_label = edge_label.iter().copied().max().unwrap_or(0);
    let col_gap = COL_GAP.max(max_label + 1);

    let vertical = fc.direction.vertical();
    let rects = layout(
        fc.direction,
        &groups_ext,
        &bw,
        &bh,
        &preds,
        &succs,
        max_rank,
        vertical,
        col_gap,
    );

    // Per-segment fan position among same-source / same-target siblings.
    let mut out_cnt = vec![0usize; ext];
    let mut in_cnt = vec![0usize; ext];
    let mut out_pos = vec![0usize; segs.len()];
    let mut in_pos = vec![0usize; segs.len()];
    for (k, s) in segs.iter().enumerate() {
        out_pos[k] = out_cnt[s.from];
        out_cnt[s.from] += 1;
        in_pos[k] = in_cnt[s.to];
        in_cnt[s.to] += 1;
    }

    let mut c = Canvas::new();
    let mut ticks: Vec<(usize, usize, char)> = Vec::new();
    let mut labels: Vec<(usize, usize, String)> = Vec::new();

    for (k, s) in segs.iter().enumerate() {
        draw_edge(
            &mut c,
            &mut ticks,
            &mut labels,
            &rects[s.from],
            &rects[s.to],
            s.line,
            s.arrow,
            &s.label,
            vertical,
            Fan {
                op: out_pos[k],
                ok: out_cnt[s.from],
                ip: in_pos[k],
                ik: in_cnt[s.to],
            },
        );
    }

    for (i, node) in fc.nodes.iter().enumerate() {
        let r = rects[i];
        c.draw_panel(
            r.x,
            r.y,
            r.w - 2,
            &node.label,
            &node.body,
            corners(node.shape),
        );
    }

    // Attach edges to box borders with T-junctions (over the box outline).
    for (x, y, g) in ticks {
        c.put(x, y, g);
    }

    // Dummy pass-throughs: a continuous line across each dummy's rank band,
    // drawn last so it overwrites any junction stubs left on it.
    for &d in &dummies {
        let r = rects[d];
        if vertical {
            c.vline(r.cx(), r.top(), r.bottom(), '│');
        } else {
            c.hline(r.left(), r.right(), r.cy(), '─');
        }
    }

    // Edge labels last, so a crossing line never overwrites the text.
    for (x, y, text) in labels {
        c.text(x, y, &text);
    }

    c.into_lines()
}

/// A routed segment of an edge (an edge spanning >1 rank becomes several).
struct Seg {
    from: usize,
    to: usize,
    line: EdgeLine,
    arrow: bool,
    label: String,
}

/// Per-edge fan position among same-source / same-target siblings.
struct Fan {
    op: usize,
    ok: usize,
    ip: usize,
    ik: usize,
}

/// Longest-path rank assignment, ignoring back edges (cycle support).
fn assign_ranks(fc: &Flowchart) -> Vec<usize> {
    let n = fc.nodes.len();
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n];
    for e in &fc.edges {
        if e.from != e.to {
            succ[e.from].push(e.to);
        }
    }

    let mut visited = vec![false; n];
    let mut onstack = vec![false; n];
    let mut order = Vec::new();
    let mut back: HashSet<(usize, usize)> = HashSet::new();
    for u in 0..n {
        if !visited[u] {
            dfs(u, &succ, &mut visited, &mut onstack, &mut order, &mut back);
        }
    }
    order.reverse(); // reverse postorder ≈ topological order

    let mut rank = vec![0usize; n];
    for &u in &order {
        for &v in &succ[u] {
            if !back.contains(&(u, v)) {
                rank[v] = rank[v].max(rank[u] + 1);
            }
        }
    }
    rank
}

fn dfs(
    u: usize,
    succ: &[Vec<usize>],
    visited: &mut [bool],
    onstack: &mut [bool],
    order: &mut Vec<usize>,
    back: &mut HashSet<(usize, usize)>,
) {
    visited[u] = true;
    onstack[u] = true;
    for &v in &succ[u] {
        if onstack[v] {
            back.insert((u, v));
        } else if !visited[v] {
            dfs(v, succ, visited, onstack, order, back);
        }
    }
    onstack[u] = false;
    order.push(u);
}

/// Place every node. The primary axis comes from the rank; the cross axis is
/// solved by [`assign_cross`] to straighten edges.
#[allow(clippy::too_many_arguments)]
fn layout(
    direction: Direction,
    groups: &[Vec<usize>],
    box_w: &[usize],
    box_h: &[usize],
    preds: &[Vec<usize>],
    succs: &[Vec<usize>],
    max_rank: usize,
    vertical: bool,
    col_gap: usize,
) -> Vec<Rect> {
    let n = box_w.len();

    // Cross-axis size per node: box width (vertical) or box height (horizontal).
    let cross_size: Vec<usize> = if vertical {
        box_w.to_vec()
    } else {
        box_h.to_vec()
    };
    let cross_gap = if vertical { col_gap } else { ROW_GAP };
    let center = assign_cross(groups, &cross_size, cross_gap, preds, succs);

    // Primary-axis start of each rank (cumulative; a rank is as tall/wide as
    // its biggest node, so variable-height boxes stack without overlap).
    let prim_channel = if vertical { V_CHANNEL } else { H_CHANNEL };
    let rank_size: Vec<usize> = groups
        .iter()
        .map(|g| {
            g.iter()
                .map(|&i| if vertical { box_h[i] } else { box_w[i] })
                .max()
                .unwrap_or(1)
        })
        .collect();
    let mut rank_start = vec![0usize; groups.len()];
    let mut acc = 0;
    for r in 0..groups.len() {
        rank_start[r] = acc;
        acc += rank_size[r] + prim_channel;
    }

    let mut rects = vec![
        Rect {
            x: 0,
            y: 0,
            w: 1,
            h: BOX_H
        };
        n
    ];
    for (r, g) in groups.iter().enumerate() {
        let prim = if (vertical && direction == Direction::Up)
            || (!vertical && direction == Direction::Left)
        {
            rank_start[max_rank - r]
        } else {
            rank_start[r]
        };
        for &i in g {
            rects[i] = if vertical {
                Rect {
                    x: center[i] - box_w[i] / 2,
                    y: prim,
                    w: box_w[i],
                    h: box_h[i],
                }
            } else {
                Rect {
                    x: prim,
                    y: center[i] - box_h[i] / 2,
                    w: box_w[i],
                    h: box_h[i],
                }
            };
        }
    }
    rects
}

/// Solve cross-axis centers: start packed, then a few barycenter sweeps pull
/// each node toward the average of its neighbors while preserving order and
/// minimum spacing. Returns a center coordinate per node (shifted to start 0).
fn assign_cross(
    groups: &[Vec<usize>],
    size: &[usize],
    gap: usize,
    preds: &[Vec<usize>],
    succs: &[Vec<usize>],
) -> Vec<usize> {
    let n = size.len();
    let mut center = vec![0i64; n];

    // Initial packing per rank.
    for g in groups {
        let mut edge = 0i64;
        for &i in g {
            center[i] = edge + (size[i] / 2) as i64;
            edge += (size[i] + gap) as i64;
        }
    }

    let sweeps = 6;
    for s in 0..sweeps {
        let down = s % 2 == 0;
        let order: Vec<usize> = if down {
            (0..groups.len()).collect()
        } else {
            (0..groups.len()).rev().collect()
        };
        for &r in &order {
            let g = &groups[r];
            // Desired center = mean of neighbor centers in the adjacent rank.
            let mut desired = vec![0i64; g.len()];
            for (idx, &i) in g.iter().enumerate() {
                let neigh = if down { &preds[i] } else { &succs[i] };
                desired[idx] = if neigh.is_empty() {
                    center[i]
                } else {
                    neigh.iter().map(|&p| center[p]).sum::<i64>() / neigh.len() as i64
                };
            }
            // Place left-to-right honoring desired + minimum spacing.
            let mut prev_right = i64::MIN / 4;
            for (idx, &i) in g.iter().enumerate() {
                let half = (size[i] / 2) as i64;
                let min_center = prev_right + gap as i64 + half;
                let c = desired[idx].max(min_center);
                center[i] = c;
                prev_right = c + half;
            }
        }
    }

    // Shift so the minimum left edge is 0.
    let min_left = (0..n)
        .map(|i| center[i] - (size[i] / 2) as i64)
        .min()
        .unwrap_or(0);
    (0..n).map(|i| (center[i] - min_left) as usize).collect()
}

fn corners(shape: NodeShape) -> [char; 4] {
    match shape {
        // Rounded outline for round/stadium/circle; everything else is a clean
        // rectangle. Diagonal `╱╲` glyphs read poorly in a character grid, so
        // diamonds/hexagons use square corners too.
        NodeShape::Round | NodeShape::Stadium | NodeShape::Circle => ['╭', '╮', '╰', '╯'],
        _ => ['┌', '┐', '└', '┘'],
    }
}

type Ticks = Vec<(usize, usize, char)>;

fn line_glyphs(line: EdgeLine) -> (char, char) {
    match line {
        EdgeLine::Solid => ('│', '─'),
        EdgeLine::Dotted => ('┊', '┄'),
        EdgeLine::Thick => ('┃', '━'),
    }
}

/// Corner glyph connecting a vertical arm (`up`) and a horizontal arm (`right`).
fn corner(up: bool, right: bool) -> char {
    match (up, right) {
        (true, true) => '└',
        (true, false) => '┘',
        (false, true) => '┌',
        (false, false) => '┐',
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_edge(
    c: &mut Canvas,
    ticks: &mut Ticks,
    labels: &mut Vec<(usize, usize, String)>,
    from: &Rect,
    to: &Rect,
    line: EdgeLine,
    arrow: bool,
    label: &str,
    vertical: bool,
    fan: Fan,
) {
    let (vch, hch) = line_glyphs(line);

    if vertical {
        if from.y == to.y {
            return side_edge(c, ticks, labels, from, to, line, arrow, label, true);
        }
        let down = to.y > from.y;
        let head = if down { '▼' } else { '▲' };
        let src_b = if down { from.bottom() } else { from.top() };
        let sy = if down {
            from.bottom() + 1
        } else {
            from.top() - 1
        };
        let tgt_b = if down { to.top() } else { to.bottom() };
        let ty = if down { to.top() - 1 } else { to.bottom() + 1 };
        let s_tick = if down { '┬' } else { '┴' };
        let t_tick = if down { '┴' } else { '┬' };
        let entry_x = to.fan_x(fan.ip, fan.ik);

        // Binary fork: the two edges leave the left/right sides of the source.
        if fan.ok == 2 {
            let left = to.cx() < from.cx();
            let (outer, border, stick) = if left {
                (from.left() - 1, from.left(), '┤')
            } else {
                (from.right() + 1, from.right(), '├')
            };
            let ay = from.cy();
            c.hline(outer.min(entry_x), outer.max(entry_x), ay, hch);
            c.vline(entry_x, ay.min(ty), ay.max(ty), vch);
            c.put(entry_x, ay, corner(ty < ay, outer > entry_x));
            if arrow {
                c.put(entry_x, ty, head);
            }
            ticks.push((border, ay, stick));
            ticks.push((entry_x, tgt_b, t_tick));
            // Label on the run, anchored just outside the source box (so the
            // box never clobbers it) and extending toward the child.
            let len = label.chars().count();
            let lx = if left {
                outer.saturating_sub(len)
            } else {
                outer
            };
            label_at(labels, label, lx, ay);
            return;
        }

        // Boxes overlapping on x → one straight drop; the boxes sit centred
        // under it even when their widths differ.
        let lo = from.left().max(to.left()) + 1;
        let hi = from.right().min(to.right()).saturating_sub(1);
        if lo <= hi {
            let col = from.cx().clamp(lo, hi);
            c.vline(col, sy.min(ty), sy.max(ty), vch);
            if arrow {
                c.put(col, ty, head);
            }
            ticks.push((col, src_b, s_tick));
            ticks.push((col, tgt_b, t_tick));
            // Label on the row just past the source, above where crossing edges
            // route (their horizontal jog sits at the channel midpoint).
            label_at(labels, label, col + 1, sy);
            return;
        }

        // General elbow, fanning out from the source's bottom/top edge.
        let sx = from.fan_x(fan.op, fan.ok);
        let mid = (sy + ty) / 2;
        c.vline(sx, sy.min(mid), sy.max(mid), vch);
        c.hline(sx.min(entry_x), sx.max(entry_x), mid, hch);
        c.vline(entry_x, mid.min(ty), mid.max(ty), vch);
        // Corners last (after every segment) so a later vline/hline can't
        // overwrite the bend glyph with `│`/`─`. Direction-based corners are
        // robust when the channel is 1 row tall (`mid` coincides with a port).
        c.put(sx, mid, corner(down, entry_x > sx));
        c.put(entry_x, mid, corner(!down, sx > entry_x));
        if arrow {
            c.put(entry_x, ty, head);
        }
        ticks.push((sx, src_b, s_tick));
        ticks.push((entry_x, tgt_b, t_tick));
        // Label beside the source's vertical drop (top of the channel), not on
        // the shared jog row. Align it away from the drop in the edge's own
        // direction so sibling labels (e.g. a left and a right branch leaving
        // the same box) spread apart instead of colliding.
        let lx = if entry_x < sx {
            sx.saturating_sub(label.chars().count())
        } else {
            sx + 1
        };
        label_at(labels, label, lx, sy);
    } else {
        if from.x == to.x {
            return side_edge(c, ticks, labels, from, to, line, arrow, label, false);
        }
        let right = to.x > from.x;
        let head = if right { '▶' } else { '◀' };
        let src_b = if right { from.right() } else { from.left() };
        let sx = if right {
            from.right() + 1
        } else {
            from.left() - 1
        };
        let tgt_b = if right { to.left() } else { to.right() };
        let tx = if right { to.left() - 1 } else { to.right() + 1 };
        let s_tick = if right { '├' } else { '┤' };
        let t_tick = if right { '┤' } else { '├' };
        let entry_y = to.fan_y(fan.ip, fan.ik);

        // Binary fork: edges leave the top/bottom sides of the source.
        if fan.ok == 2 {
            let up = to.cy() < from.cy();
            let (outer, border, stick) = if up {
                (from.top() - 1, from.top(), '┴')
            } else {
                (from.bottom() + 1, from.bottom(), '┬')
            };
            let ax = from.cx();
            c.vline(ax, outer.min(entry_y), outer.max(entry_y), vch);
            c.hline(ax.min(tx), ax.max(tx), entry_y, hch);
            c.put(ax, entry_y, corner(outer < entry_y, tx > ax));
            if arrow {
                c.put(tx, entry_y, head);
            }
            ticks.push((ax, border, stick));
            ticks.push((tgt_b, entry_y, t_tick));
            let lx = (ax + tx) / 2;
            label_at(
                labels,
                label,
                lx.saturating_sub(label.chars().count() / 2),
                entry_y,
            );
            return;
        }

        // Boxes overlapping on y → one straight horizontal run.
        let lo = from.top().max(to.top()) + 1;
        let hi = from.bottom().min(to.bottom()).saturating_sub(1);
        if lo <= hi {
            let row = from.cy().clamp(lo, hi);
            c.hline(sx.min(tx), sx.max(tx), row, hch);
            if arrow {
                c.put(tx, row, head);
            }
            ticks.push((src_b, row, s_tick));
            ticks.push((tgt_b, row, t_tick));
            label_at(labels, label, sx.min(tx) + 1, row.saturating_sub(1));
            return;
        }

        // General elbow, fanning out from the source's right/left edge.
        let sy = from.fan_y(fan.op, fan.ok);
        let mid = (sx + tx) / 2;
        c.hline(sx.min(mid), sx.max(mid), sy, hch);
        c.vline(mid, sy.min(entry_y), sy.max(entry_y), vch);
        c.hline(mid.min(tx), mid.max(tx), entry_y, hch);
        // Corners last so the second hline can't overwrite the bend glyph.
        c.put(mid, sy, corner(entry_y < sy, sx > mid));
        c.put(mid, entry_y, corner(sy < entry_y, tx > mid));
        if arrow {
            c.put(tx, entry_y, head);
        }
        ticks.push((src_b, sy, s_tick));
        ticks.push((tgt_b, entry_y, t_tick));
        label_at(labels, label, mid + 1, sy.min(entry_y).saturating_sub(1));
    }
}

/// Queue a non-empty edge label at `(x, y)`. Labels are stamped after all
/// lines/boxes so a crossing line can never clobber the text.
fn label_at(labels: &mut Vec<(usize, usize, String)>, label: &str, x: usize, y: usize) {
    if !label.is_empty() {
        labels.push((x, y, label.to_string()));
    }
}

/// A same-rank edge: a short straight arrow between facing box sides, attached
/// with T-junctions.
#[allow(clippy::too_many_arguments)]
fn side_edge(
    c: &mut Canvas,
    ticks: &mut Ticks,
    labels: &mut Vec<(usize, usize, String)>,
    from: &Rect,
    to: &Rect,
    line: EdgeLine,
    arrow: bool,
    label: &str,
    vertical: bool,
) {
    let (vch, hch) = line_glyphs(line);
    if vertical {
        let y = from.cy();
        let right = to.x > from.x;
        let (x0, x1, head, sb, tb, st, tt) = if right {
            (
                from.right() + 1,
                to.left() - 1,
                '▶',
                from.right(),
                to.left(),
                '├',
                '┤',
            )
        } else {
            (
                to.right() + 1,
                from.left() - 1,
                '◀',
                from.left(),
                to.right(),
                '┤',
                '├',
            )
        };
        if x0 <= x1 {
            c.hline(x0, x1, y, hch);
            label_at(labels, label, x0, y.saturating_sub(1));
            if arrow {
                c.put(if right { x1 } else { x0 }, y, head);
            }
            ticks.push((sb, y, st));
            ticks.push((tb, y, tt));
        }
    } else {
        let x = from.cx();
        let down = to.y > from.y;
        let (y0, y1, head, sb, tb, st, tt) = if down {
            (
                from.bottom() + 1,
                to.top() - 1,
                '▼',
                from.bottom(),
                to.top(),
                '┬',
                '┴',
            )
        } else {
            (
                to.bottom() + 1,
                from.top() - 1,
                '▲',
                from.top(),
                to.bottom(),
                '┴',
                '┬',
            )
        };
        if y0 <= y1 {
            c.vline(x, y0, y1, vch);
            if arrow {
                c.put(x, if down { y1 } else { y0 }, head);
            }
            ticks.push((x, sb, st));
            ticks.push((x, tb, tt));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_flowchart;

    fn render(src: &str) -> String {
        render_flowchart(&parse_flowchart(src)).join("\n")
    }

    #[test]
    fn renders_nodes_and_edge() {
        let out = render("flowchart TD\nA[Start] --> B[End]");
        assert!(out.contains("Start") && out.contains("End"), "{out}");
        assert!(out.contains('┌'), "no box: {out}");
        assert!(out.contains('▼'), "no down arrowhead: {out}");
    }

    #[test]
    fn decision_branches_with_labels() {
        let out = render("flowchart TD\nA{ok?} -->|yes| B[Go]\nA -->|no| C[Stop]");
        assert!(out.contains("yes"), "no edge label: {out}");
        assert!(out.contains("no"), "no edge label: {out}");
        assert!(out.contains("ok?"), "no decision label: {out}");
        // Edges attach to boxes with T-junctions rather than detached lines.
        assert!(out.contains('┬') || out.contains('┴') || out.contains('├') || out.contains('┤'));
    }

    #[test]
    fn horizontal_uses_side_arrows() {
        let out = render("flowchart LR\nA --> B --> C");
        assert!(out.contains('▶'), "no right arrowhead: {out}");
    }

    #[test]
    fn dotted_and_thick_lines() {
        let out = render("flowchart TD\nA -.-> B\nB ==> C");
        assert!(out.contains('┊') || out.contains('┄'), "no dotted: {out}");
        assert!(out.contains('┃') || out.contains('━'), "no thick: {out}");
    }

    #[test]
    fn empty_handled() {
        assert_eq!(
            render_flowchart(&parse_flowchart("flowchart TD")),
            vec!["(empty flowchart)"]
        );
    }

    #[test]
    fn cycle_does_not_hang() {
        // A -> B -> A: the back edge must be ignored during ranking.
        let out = render("flowchart TD\nA --> B\nB --> A");
        assert!(out.contains('A') && out.contains('B'), "{out}");
    }
}
