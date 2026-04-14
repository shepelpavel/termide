//! Tree view types and algorithms for Git Status Panel.
//!
//! Groups files by directory into a collapsible tree, similar to VS Code Source Control.

use std::collections::HashSet;
use std::path::PathBuf;

/// A single node in the file tree (directory or file).
#[derive(Debug, Clone)]
pub struct TreeNode {
    /// Display label ("src" for directories, "main.rs" for files)
    pub label: String,
    /// Full relative path
    pub full_path: PathBuf,
    /// Nesting depth (0 = top-level)
    pub depth: usize,
    /// Whether this is a directory or file, with associated data
    pub kind: TreeNodeKind,
}

/// Kind of tree node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeNodeKind {
    Directory {
        expanded: bool,
    },
    File {
        file_index: usize,
        status: char,
        untracked: bool,
    },
}

/// Input file info for tree building.
pub struct FileEntry {
    pub path: PathBuf,
    pub index: usize,
    pub status: char,
    pub untracked: bool,
}

/// Build a flat tree from a sorted list of files.
///
/// For each file, directory nodes are emitted for path segments that haven't been seen yet.
/// `collapsed_dirs` determines which directories start collapsed.
pub fn build_tree(files: &[FileEntry], collapsed_dirs: &HashSet<PathBuf>) -> Vec<TreeNode> {
    if files.is_empty() {
        return Vec::new();
    }

    let mut nodes = Vec::new();
    // Stack of (directory_path, depth) currently open
    let mut dir_stack: Vec<(PathBuf, usize)> = Vec::new();

    for file in files {
        let components: Vec<&str> = file
            .path
            .components()
            .filter_map(|c| {
                if let std::path::Component::Normal(s) = c {
                    s.to_str()
                } else {
                    None
                }
            })
            .collect();

        if components.is_empty() {
            continue;
        }

        // The last component is the filename
        let dir_components = &components[..components.len() - 1];
        let file_name = components[components.len() - 1];

        // Find common prefix with current dir_stack
        let mut common = 0;
        for (i, (dir_path, _)) in dir_stack.iter().enumerate() {
            if i < dir_components.len() {
                let expected: PathBuf = dir_components[..=i].iter().collect();
                if *dir_path == expected {
                    common = i + 1;
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        // Pop directories that are no longer in the path
        dir_stack.truncate(common);

        // Push new directories
        for i in common..dir_components.len() {
            let dir_path: PathBuf = dir_components[..=i].iter().collect();
            let depth = i;
            let expanded = !collapsed_dirs.contains(&dir_path);
            nodes.push(TreeNode {
                label: dir_components[i].to_string(),
                full_path: dir_path.clone(),
                depth,
                kind: TreeNodeKind::Directory { expanded },
            });
            dir_stack.push((dir_path, depth));
        }

        // Add the file node
        let file_depth = dir_components.len();
        nodes.push(TreeNode {
            label: file_name.to_string(),
            full_path: file.path.clone(),
            depth: file_depth,
            kind: TreeNodeKind::File {
                file_index: file.index,
                status: file.status,
                untracked: file.untracked,
            },
        });
    }

    nodes
}

/// Compute indices of visible nodes (skipping children of collapsed directories).
pub fn compute_visible_nodes(tree: &[TreeNode]) -> Vec<usize> {
    let mut visible = Vec::new();
    let mut skip_below_depth: Option<usize> = None;

    for (i, node) in tree.iter().enumerate() {
        if let Some(max_depth) = skip_below_depth {
            if node.depth > max_depth {
                continue;
            }
            // We've exited the collapsed subtree
            skip_below_depth = None;
        }

        visible.push(i);

        if let TreeNodeKind::Directory { expanded: false } = node.kind {
            skip_below_depth = Some(node.depth);
        }
    }

    visible
}

/// Collect all file paths under a directory node (recursively).
pub fn collect_files_under(tree: &[TreeNode], dir_index: usize) -> Vec<PathBuf> {
    let dir_depth = tree[dir_index].depth;
    let mut files = Vec::new();

    for node in &tree[dir_index + 1..] {
        if node.depth <= dir_depth {
            break;
        }
        if let TreeNodeKind::File { .. } = node.kind {
            files.push(node.full_path.clone());
        }
    }

    files
}

/// Compute tree-drawing prefixes for visible nodes in O(n) time.
///
/// Scans visible nodes in reverse to pre-compute which depth levels
/// have a subsequent sibling, avoiding the O(n²) forward scan.
pub fn compute_tree_prefixes(tree: &[TreeNode], visible: &[usize]) -> Vec<String> {
    let max_depth = visible
        .iter()
        .map(|&idx| tree[idx].depth)
        .max()
        .unwrap_or(0);

    // has_next_at_level[lvl] is true when a later visible node exists at that depth
    // (before being "cut off" by a shallower node).
    let mut has_next_at_level = vec![false; max_depth + 1];

    // Build prefixes in reverse, then reverse the result
    let mut prefixes: Vec<String> = Vec::with_capacity(visible.len());

    for &tree_idx in visible.iter().rev() {
        let depth = tree[tree_idx].depth;

        if depth == 0 {
            // Clear all levels — root node resets everything
            has_next_at_level.fill(false);
            // Mark this level as having a node for nodes processed earlier
            has_next_at_level[0] = true;
            prefixes.push(String::new());
            continue;
        }

        let mut prefix = String::with_capacity(depth * 3);
        for (lvl, has_next) in has_next_at_level[1..=depth].iter().enumerate() {
            let lvl = lvl + 1; // offset since we sliced from index 1
            if lvl == depth {
                if *has_next {
                    prefix.push_str("├─ ");
                } else {
                    prefix.push_str("└─ ");
                }
            } else if *has_next {
                prefix.push_str("│  ");
            } else {
                prefix.push_str("   ");
            }
        }
        prefixes.push(prefix);

        // This node "occupies" its depth: deeper levels no longer have siblings
        for val in &mut has_next_at_level[(depth + 1)..=max_depth] {
            *val = false;
        }
        // Nodes processed earlier (which appear before this one) will see this as a sibling
        has_next_at_level[depth] = true;
    }

    prefixes.reverse();
    prefixes
}

/// Get the aggregate status for files under a directory.
/// Counts files per status and picks the majority. Ties broken by: M > D > A/R > ?
pub fn aggregate_dir_status(tree: &[TreeNode], dir_index: usize) -> (char, bool) {
    let dir_depth = tree[dir_index].depth;
    let mut deleted = 0u32;
    let mut modified = 0u32;
    let mut added = 0u32;
    let mut untracked = 0u32;

    for node in &tree[dir_index + 1..] {
        if node.depth <= dir_depth {
            break;
        }
        if let TreeNodeKind::File {
            status,
            untracked: ut,
            ..
        } = node.kind
        {
            if ut {
                untracked += 1;
            } else {
                match status {
                    'D' => deleted += 1,
                    'M' => modified += 1,
                    'A' | 'R' => added += 1,
                    _ => {}
                }
            }
        }
    }

    // All deleted — show deleted; else majority wins, ties: M > D > A > ?
    if deleted > 0 && modified == 0 && added == 0 && untracked == 0 {
        ('D', false)
    } else {
        let (best_count, best_status, best_ut) = [
            (modified, 'M', false),
            (deleted, 'D', false),
            (added, 'A', false),
            (untracked, '?', true),
        ]
        .into_iter()
        .max_by_key(|(c, _, _)| *c)
        .unwrap();

        if best_count > 0 {
            (best_status, best_ut)
        } else {
            (' ', false)
        }
    }
}

/// Count the number of file nodes under a directory.
pub fn count_files_under(tree: &[TreeNode], dir_index: usize) -> usize {
    let dir_depth = tree[dir_index].depth;
    let mut count = 0;
    for node in &tree[dir_index + 1..] {
        if node.depth <= dir_depth {
            break;
        }
        if matches!(node.kind, TreeNodeKind::File { .. }) {
            count += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_file(path: &str, index: usize, status: char) -> FileEntry {
        FileEntry {
            path: PathBuf::from(path),
            index,
            status,
            untracked: false,
        }
    }

    #[test]
    fn test_build_flat_files() {
        let files = vec![make_file("a.rs", 0, 'M'), make_file("b.rs", 1, 'A')];
        let tree = build_tree(&files, &HashSet::new());
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].depth, 0);
        assert_eq!(tree[0].label, "a.rs");
        assert_eq!(tree[1].depth, 0);
        assert_eq!(tree[1].label, "b.rs");
    }

    #[test]
    fn test_build_nested_dirs() {
        let files = vec![
            make_file("src/foo/a.rs", 0, 'M'),
            make_file("src/foo/b.rs", 1, 'M'),
            make_file("src/bar.rs", 2, 'A'),
        ];
        let tree = build_tree(&files, &HashSet::new());
        // src/ (depth 0), foo/ (depth 1), a.rs (depth 2), b.rs (depth 2), bar.rs (depth 1)
        assert_eq!(tree.len(), 5);
        assert_eq!(tree[0].label, "src");
        assert_eq!(tree[0].depth, 0);
        assert!(matches!(
            tree[0].kind,
            TreeNodeKind::Directory { expanded: true }
        ));
        assert_eq!(tree[1].label, "foo");
        assert_eq!(tree[1].depth, 1);
        assert_eq!(tree[2].label, "a.rs");
        assert_eq!(tree[2].depth, 2);
        assert_eq!(tree[3].label, "b.rs");
        assert_eq!(tree[3].depth, 2);
        assert_eq!(tree[4].label, "bar.rs");
        assert_eq!(tree[4].depth, 1);
    }

    #[test]
    fn test_visibility_with_collapse() {
        let mut collapsed = HashSet::new();
        collapsed.insert(PathBuf::from("src/foo"));

        let files = vec![
            make_file("src/foo/a.rs", 0, 'M'),
            make_file("src/foo/b.rs", 1, 'M'),
            make_file("src/bar.rs", 2, 'A'),
        ];
        let tree = build_tree(&files, &collapsed);
        let visible = compute_visible_nodes(&tree);
        // src/, foo/ (collapsed), bar.rs — a.rs and b.rs are hidden
        assert_eq!(visible.len(), 3);
        assert_eq!(tree[visible[0]].label, "src");
        assert_eq!(tree[visible[1]].label, "foo");
        assert_eq!(tree[visible[2]].label, "bar.rs");
    }

    #[test]
    fn test_collect_files_under() {
        let files = vec![
            make_file("src/foo/a.rs", 0, 'M'),
            make_file("src/foo/b.rs", 1, 'M'),
            make_file("src/bar.rs", 2, 'A'),
        ];
        let tree = build_tree(&files, &HashSet::new());
        // dir index 0 = src/, should contain all 3 files
        let collected = collect_files_under(&tree, 0);
        assert_eq!(collected.len(), 3);
        // dir index 1 = foo/, should contain 2 files
        let collected = collect_files_under(&tree, 1);
        assert_eq!(collected.len(), 2);
    }

    #[test]
    fn test_tree_prefixes() {
        let files = vec![make_file("src/a.rs", 0, 'M'), make_file("src/b.rs", 1, 'M')];
        let tree = build_tree(&files, &HashSet::new());
        let visible = compute_visible_nodes(&tree);
        let prefixes = compute_tree_prefixes(&tree, &visible);
        // src/ (depth 0) -> no prefix
        assert_eq!(prefixes[0], "");
        // a.rs (depth 1, has sibling b.rs) -> "├─ "
        assert_eq!(prefixes[1], "├─ ");
        // b.rs (depth 1, last) -> "└─ "
        assert_eq!(prefixes[2], "└─ ");
    }
}
