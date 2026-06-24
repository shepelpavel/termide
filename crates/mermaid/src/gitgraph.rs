//! Git graph rendering: commits as nodes on per-branch lanes (columns), with
//! branch/merge connectors. A compact top-to-bottom commit history.

use crate::parser::{GitGraph, GitOp};

/// Column of a lane (3 chars apart so connectors have room).
fn col(lane: usize) -> usize {
    lane * 3
}

/// A row with `│` on every active lane, returned as a char vector.
fn lane_bars(lanes: usize) -> Vec<char> {
    let mut row = vec![' '; col(lanes.saturating_sub(1)) + 1];
    for l in 0..lanes {
        row[col(l)] = '│';
    }
    row
}

fn finish(mut row: Vec<char>, label: &str) -> String {
    let mut s: String = {
        // Trim trailing spaces from the lane area.
        while matches!(row.last(), Some(' ')) {
            row.pop();
        }
        row.into_iter().collect()
    };
    if !label.is_empty() {
        s.push_str(&format!("  {label}"));
    }
    s
}

/// Render a git graph into lines.
#[must_use]
pub fn render_gitgraph(g: &GitGraph) -> Vec<String> {
    if g.ops.is_empty() {
        return vec!["(empty git graph)".to_string()];
    }
    let mut lanes: Vec<String> = vec!["main".to_string()];
    let mut cur = 0usize;
    let mut out: Vec<String> = Vec::new();

    for op in &g.ops {
        match op {
            GitOp::Commit { label } => {
                let mut row = lane_bars(lanes.len());
                row[col(cur)] = '●';
                let lbl = if label.is_empty() {
                    format!("[{}]", lanes[cur])
                } else {
                    format!("[{}] {label}", lanes[cur])
                };
                out.push(finish(row, &lbl));
            }
            GitOp::Branch(name) => {
                lanes.push(name.clone());
                let nl = lanes.len() - 1;
                let mut row = lane_bars(lanes.len());
                // Fork connector from the current lane to the new one.
                row[col(cur)] = '├';
                row[col(cur) + 1..col(nl)].fill('─');
                row[col(nl)] = '╮';
                out.push(finish(row, &format!("branch {name}")));
                cur = nl;
            }
            GitOp::Checkout(name) => {
                cur = lanes.iter().position(|b| b == name).unwrap_or_else(|| {
                    lanes.push(name.clone());
                    lanes.len() - 1
                });
            }
            GitOp::Merge(name) => {
                let target = lanes.iter().position(|b| b == name).unwrap_or(cur);
                let mut row = lane_bars(lanes.len());
                let (lo, hi) = (cur.min(target), cur.max(target));
                row[col(cur)] = '◆';
                for cell in &mut row[col(lo) + 1..col(hi)] {
                    if *cell == ' ' {
                        *cell = '─';
                    }
                }
                out.push(finish(row, &format!("merge {name}")));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_gitgraph;

    fn render(src: &str) -> String {
        render_gitgraph(&parse_gitgraph(src)).join("\n")
    }

    #[test]
    fn commits_and_branch() {
        let out = render("gitGraph\ncommit\nbranch dev\ncommit\ncheckout main\ncommit\nmerge dev");
        assert!(out.contains('●'), "no commit node: {out}");
        assert!(out.contains("branch dev"), "no branch: {out}");
        assert!(out.contains("merge dev"), "no merge: {out}");
        assert!(out.contains('◆'), "no merge node: {out}");
    }

    #[test]
    fn commit_labels() {
        let out = render("gitGraph\ncommit id: \"init\"\ncommit tag: \"v1\"");
        assert!(out.contains("init"), "no id label: {out}");
        assert!(out.contains("v1"), "no tag label: {out}");
    }

    #[test]
    fn empty_handled() {
        assert_eq!(
            render_gitgraph(&parse_gitgraph("gitGraph")),
            vec!["(empty git graph)"]
        );
    }
}
