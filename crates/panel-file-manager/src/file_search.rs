//! File and content search state for file manager.
//!
//! Replaces TreeSearchModal's result display — search results are shown
//! in the file manager panel instead of a modal.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use regex::RegexBuilder;
use termide_core::util::is_binary_file;
use termide_git::{get_git_status, GitStatus, GitStatusCache};

/// Maximum total results to collect
const MAX_RESULTS: usize = 500;

/// Content match info for a single line match
#[derive(Debug, Clone)]
pub(crate) struct ContentMatch {
    pub line_number: usize,
    pub matched_line: String,
    pub match_start: usize,
    pub match_end: usize,
}

/// A node in the search result tree
#[derive(Debug, Clone)]
pub(crate) struct ResultTreeNode {
    pub name: String,
    pub full_path: PathBuf,
    pub depth: usize,
    pub is_dir: bool,
    pub git_status: GitStatus,
    pub content_match: Option<ContentMatch>,
    /// Content mode only: this node is a per-file group header (path + count),
    /// not a match row. Its `match_count` matches rows follow it.
    pub is_file_header: bool,
    /// Number of matches in the file (only meaningful for `is_file_header`).
    pub match_count: usize,
    /// Content mode only: a collapsed header hides its match rows.
    pub collapsed: bool,
}

/// Search mode for file search
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileSearchMode {
    FileGlob,
    Content,
}

/// Search results from background thread
enum SearchResults {
    FileResults(Vec<FileResult>),
    ContentResults(Vec<ContentResult>),
}

#[derive(Debug, Clone)]
struct FileResult {
    full_path: PathBuf,
    relative_path: String,
    git_status: GitStatus,
    is_dir: bool,
}

#[derive(Debug, Clone)]
struct ContentResult {
    full_path: PathBuf,
    relative_path: String,
    line_number: usize,
    matched_line: String,
    match_start: usize,
    match_end: usize,
    git_status: GitStatus,
}

/// Persistent search state for file manager
pub(crate) struct FileSearchState {
    pub mode: FileSearchMode,
    pub tree_nodes: Vec<ResultTreeNode>,
    pub tree_prefixes: Vec<String>,
    pub result_count: usize,
    pub cursor: usize,
    pub scroll_offset: usize,
    pub is_searching: bool,
    search_receiver: Option<mpsc::Receiver<SearchResults>>,
    search_cancel: Option<Arc<AtomicBool>>,
    base_path: PathBuf,
    max_file_size: u64,
    /// Content replace: text typed in the Replace field (for preview/apply).
    replace_text: Option<String>,
    /// Effective regex pattern used by the last content search (escaped when
    /// literal), kept so replace can re-match files.
    search_pattern: String,
    /// Whether the last content search treated the query as a regex.
    search_use_regex: bool,
    /// Case sensitivity of the last content search.
    search_case_sensitive: bool,
}

impl std::fmt::Debug for FileSearchState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileSearchState")
            .field("mode", &self.mode)
            .field("result_count", &self.result_count)
            .field("cursor", &self.cursor)
            .field("is_searching", &self.is_searching)
            .finish()
    }
}

impl FileSearchState {
    /// Create new file glob search state
    pub fn new_file_glob(base_path: PathBuf) -> Self {
        Self {
            mode: FileSearchMode::FileGlob,
            tree_nodes: Vec::new(),
            tree_prefixes: Vec::new(),
            result_count: 0,
            cursor: 0,
            scroll_offset: 0,
            is_searching: false,
            search_receiver: None,
            search_cancel: None,
            base_path,
            max_file_size: 0,
            replace_text: None,
            search_pattern: String::new(),
            search_use_regex: false,
            search_case_sensitive: false,
        }
    }

    /// Create new content search state
    pub fn new_content(base_path: PathBuf, max_file_size: u64) -> Self {
        Self {
            mode: FileSearchMode::Content,
            tree_nodes: Vec::new(),
            tree_prefixes: Vec::new(),
            result_count: 0,
            cursor: 0,
            scroll_offset: 0,
            is_searching: false,
            search_receiver: None,
            search_cancel: None,
            base_path,
            max_file_size,
            replace_text: None,
            search_pattern: String::new(),
            search_use_regex: false,
            search_case_sensitive: false,
        }
    }

    /// Start file search in background thread
    pub fn start_file_search(&mut self, mask: &str) {
        if mask.is_empty() {
            self.tree_nodes.clear();
            self.tree_prefixes.clear();
            self.result_count = 0;
            self.cursor = 0;
            self.scroll_offset = 0;
            self.is_searching = false;
            return;
        }

        // Cancel previous search
        if let Some(cancel) = self.search_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }

        let cancel = Arc::new(AtomicBool::new(false));
        self.search_cancel = Some(cancel.clone());

        let (tx, rx) = mpsc::channel();
        let base_path = self.base_path.clone();
        let mask = mask.to_string();

        self.search_receiver = Some(rx);
        self.is_searching = true;

        std::thread::spawn(move || {
            // Build the git status cache on the worker thread so the
            // search panel opens without blocking the UI on a slow
            // `git status` (large or network-mounted repos).
            let git_cache = get_git_status(&base_path);
            let results = search_files(&base_path, &mask, &cancel, git_cache.as_ref());
            if !cancel.load(Ordering::Relaxed) {
                let _ = tx.send(SearchResults::FileResults(results));
            }
        });
    }

    /// Start content search in background thread
    pub fn start_content_search(
        &mut self,
        mask: &str,
        content_pattern: &str,
        use_regex: bool,
        case_sensitive: bool,
    ) {
        if mask.is_empty() || content_pattern.is_empty() {
            self.tree_nodes.clear();
            self.tree_prefixes.clear();
            self.result_count = 0;
            self.cursor = 0;
            self.scroll_offset = 0;
            self.is_searching = false;
            return;
        }

        // Literal search escapes the query; regex uses it verbatim.
        let pattern = if use_regex {
            content_pattern.to_string()
        } else {
            regex::escape(content_pattern)
        };

        // Validate the (effective) regex with the requested case sensitivity.
        if RegexBuilder::new(&pattern)
            .case_insensitive(!case_sensitive)
            .build()
            .is_err()
        {
            return;
        }

        // Remember how this search matched, so replace can re-match files.
        self.search_pattern = pattern.clone();
        self.search_use_regex = use_regex;
        self.search_case_sensitive = case_sensitive;
        self.replace_text = None;

        // Cancel previous search
        if let Some(cancel) = self.search_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }

        let cancel = Arc::new(AtomicBool::new(false));
        self.search_cancel = Some(cancel.clone());

        let (tx, rx) = mpsc::channel();
        let base_path = self.base_path.clone();
        let mask = mask.to_string();
        let max_file_size = self.max_file_size;

        self.search_receiver = Some(rx);
        self.is_searching = true;

        std::thread::spawn(move || {
            let git_cache = get_git_status(&base_path);
            let results = search_content(
                &base_path,
                &mask,
                &pattern,
                case_sensitive,
                &cancel,
                git_cache.as_ref(),
                max_file_size,
            );
            if !cancel.load(Ordering::Relaxed) {
                let _ = tx.send(SearchResults::ContentResults(results));
            }
        });
    }

    /// Poll for search results (call from tick())
    pub fn poll_results(&mut self) -> bool {
        if let Some(rx) = &self.search_receiver {
            match rx.try_recv() {
                Ok(results) => {
                    match results {
                        SearchResults::FileResults(items) => {
                            self.build_file_tree(items);
                        }
                        SearchResults::ContentResults(items) => {
                            self.build_content_tree(items);
                        }
                    }
                    self.cursor = 0;
                    self.scroll_offset = 0;
                    self.is_searching = false;
                    self.search_receiver = None;
                    // Move cursor to the first selectable result row.
                    if let Some(i) = self.tree_nodes.iter().position(|n| self.is_result_row(n)) {
                        self.cursor = i;
                    }
                    return true;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.is_searching = false;
                    self.search_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        false
    }

    /// Navigate to next result (skip dirs)
    pub fn next_result(&mut self) {
        if self.tree_nodes.is_empty() {
            return;
        }
        let start = self.cursor;
        let len = self.tree_nodes.len();
        let mut pos = (start + 1) % len;
        while pos != start {
            if self.is_result_row(&self.tree_nodes[pos]) && !self.is_hidden_by_collapse(pos) {
                self.cursor = pos;
                self.ensure_visible();
                return;
            }
            pos = (pos + 1) % len;
        }
    }

    /// Navigate to previous result (skip dirs)
    pub fn prev_result(&mut self) {
        if self.tree_nodes.is_empty() {
            return;
        }
        let start = self.cursor;
        let len = self.tree_nodes.len();
        let mut pos = if start == 0 { len - 1 } else { start - 1 };
        while pos != start {
            if self.is_result_row(&self.tree_nodes[pos]) && !self.is_hidden_by_collapse(pos) {
                self.cursor = pos;
                self.ensure_visible();
                return;
            }
            pos = if pos == 0 { len - 1 } else { pos - 1 };
        }
    }

    /// A selectable result row: a matched file in FileGlob mode, a match line
    /// in Content mode (file-group headers are not result rows).
    fn is_result_row(&self, node: &ResultTreeNode) -> bool {
        match self.mode {
            FileSearchMode::FileGlob => !node.is_dir,
            FileSearchMode::Content => node.content_match.is_some(),
        }
    }

    /// Get match info: (current_index, total_count)
    pub fn get_match_info(&self) -> Option<(usize, usize)> {
        if self.result_count == 0 {
            return None;
        }
        // Count how many result rows precede the cursor.
        let mut current = 0;
        for (idx, node) in self.tree_nodes.iter().enumerate() {
            if idx == self.cursor {
                return Some((current.min(self.result_count - 1), self.result_count));
            }
            if self.is_result_row(node) {
                current += 1;
            }
        }
        Some((0, self.result_count))
    }

    /// Get the selected result for opening
    pub fn get_selected_result(&self) -> Option<SelectedSearchResult> {
        let node = self.tree_nodes.get(self.cursor)?;
        if node.is_dir {
            return None;
        }
        match self.mode {
            FileSearchMode::FileGlob => {
                Some(SelectedSearchResult::NavigateToFile(node.full_path.clone()))
            }
            FileSearchMode::Content => {
                let line = node
                    .content_match
                    .as_ref()
                    .map(|m| m.line_number)
                    .unwrap_or(1);
                Some(SelectedSearchResult::OpenAtLine {
                    path: node.full_path.clone(),
                    line,
                })
            }
        }
    }

    /// Max visible nodes for this mode
    pub fn max_visible_nodes(&self) -> usize {
        match self.mode {
            FileSearchMode::FileGlob => 15,
            FileSearchMode::Content => 40,
        }
    }

    /// How many display lines a node takes. Every visible node is a single
    /// line now (file header, match row, or file/dir); rows hidden under a
    /// collapsed header take none.
    pub fn node_display_lines(&self, idx: usize) -> usize {
        if idx >= self.tree_nodes.len() || self.is_hidden_by_collapse(idx) {
            return 0;
        }
        // The cursor match expands to a two-line -old/+new preview while a
        // replacement is being composed.
        if idx == self.cursor
            && self.has_replace_preview()
            && self.tree_nodes[idx].content_match.is_some()
        {
            return 2;
        }
        1
    }

    fn ensure_visible(&mut self) {
        let max_vis = self.max_visible_nodes();
        let lines_to_cursor = self.count_lines(self.scroll_offset, self.cursor);
        if lines_to_cursor >= max_vis {
            self.scroll_offset = self.find_scroll_for_cursor(max_vis);
        } else if self.cursor < self.scroll_offset {
            self.scroll_offset = self.cursor;
        }
    }

    fn count_lines(&self, from: usize, to: usize) -> usize {
        if to < from || from >= self.tree_nodes.len() {
            return 0;
        }
        let end = to.min(self.tree_nodes.len());
        let mut lines = 0;
        for i in from..end {
            lines += self.node_display_lines(i);
        }
        lines
    }

    fn find_scroll_for_cursor(&self, max_vis: usize) -> usize {
        let mut lines = self.node_display_lines(self.cursor);
        let mut start = self.cursor;
        while start > 0 && lines < max_vis {
            start -= 1;
            lines += self.node_display_lines(start);
        }
        if lines > max_vis && start < self.cursor {
            start + 1
        } else {
            start
        }
    }

    fn build_file_tree(&mut self, items: Vec<FileResult>) {
        self.result_count = items.len();
        let (nodes, prefixes) = build_tree_nodes(
            items
                .iter()
                .map(|i| TreeBuildItem {
                    relative_path: &i.relative_path,
                    full_path: &i.full_path,
                    git_status: i.git_status,
                    is_dir: i.is_dir,
                    content_match: None,
                })
                .collect(),
        );
        self.tree_nodes = nodes;
        self.tree_prefixes = prefixes;
    }

    /// Build the content-search display list: one collapsible header row per
    /// file (relative path + match count) followed by one row per match
    /// (line number + matched line). `items` arrive sorted by relative path,
    /// so files are already grouped.
    fn build_content_tree(&mut self, items: Vec<ContentResult>) {
        self.result_count = items.len();

        // Count matches per file so the header can show the total.
        let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for it in &items {
            *counts.entry(it.relative_path.as_str()).or_default() += 1;
        }

        let mut nodes: Vec<ResultTreeNode> = Vec::new();
        let mut current_file: Option<String> = None;
        for it in &items {
            if current_file.as_deref() != Some(it.relative_path.as_str()) {
                current_file = Some(it.relative_path.clone());
                nodes.push(ResultTreeNode {
                    name: it.relative_path.clone(),
                    full_path: it.full_path.clone(),
                    depth: 0,
                    is_dir: false,
                    git_status: it.git_status,
                    content_match: None,
                    is_file_header: true,
                    match_count: counts.get(it.relative_path.as_str()).copied().unwrap_or(0),
                    collapsed: false,
                });
            }
            nodes.push(ResultTreeNode {
                name: String::new(),
                full_path: it.full_path.clone(),
                depth: 1,
                is_dir: false,
                git_status: it.git_status,
                content_match: Some(ContentMatch {
                    line_number: it.line_number,
                    matched_line: it.matched_line.clone(),
                    match_start: it.match_start,
                    match_end: it.match_end,
                }),
                is_file_header: false,
                match_count: 0,
                collapsed: false,
            });
        }

        self.tree_prefixes = vec![String::new(); nodes.len()];
        self.tree_nodes = nodes;
    }

    /// Whether the match row at `idx` is hidden because its file header is
    /// collapsed.
    fn is_hidden_by_collapse(&self, idx: usize) -> bool {
        if self
            .tree_nodes
            .get(idx)
            .map(|n| n.is_file_header)
            .unwrap_or(true)
        {
            return false;
        }
        // Walk back to the owning header.
        for j in (0..idx).rev() {
            if self.tree_nodes[j].is_file_header {
                return self.tree_nodes[j].collapsed;
            }
        }
        false
    }

    /// Number of file-group headers (i.e. distinct files with matches).
    pub fn file_header_count(&self) -> usize {
        self.tree_nodes.iter().filter(|n| n.is_file_header).count()
    }

    /// Store the in-progress replacement text (Content mode), used for the
    /// preview and as the default for apply.
    pub fn set_replace_text(&mut self, text: Option<String>) {
        self.replace_text = text;
    }

    /// True when a non-empty replacement is being composed — the cursor match
    /// then shows a `-old/+new` preview.
    pub fn has_replace_preview(&self) -> bool {
        self.replace_text.as_deref().is_some_and(|t| !t.is_empty())
    }

    /// Compute the post-replace version of `matched_line` for the preview.
    /// Returns `None` when no replacement is active.
    pub fn preview_replacement(&self, matched_line: &str) -> Option<String> {
        let rep = self.replace_text.as_deref()?;
        if rep.is_empty() {
            return None;
        }
        let re = RegexBuilder::new(&self.search_pattern)
            .case_insensitive(!self.search_case_sensitive)
            .build()
            .ok()?;
        let new = if self.search_use_regex {
            re.replace_all(matched_line, rep).into_owned()
        } else {
            re.replace_all(matched_line, regex::NoExpand(rep))
                .into_owned()
        };
        Some(new)
    }

    /// Apply `replace_with` to every matched file on disk, re-matching at
    /// apply time. Returns (files_changed, occurrences_replaced).
    pub fn replace_all(&self, replace_with: &str) -> (usize, usize) {
        if self.mode != FileSearchMode::Content || self.search_pattern.is_empty() {
            return (0, 0);
        }
        let re = match RegexBuilder::new(&self.search_pattern)
            .case_insensitive(!self.search_case_sensitive)
            .build()
        {
            Ok(r) => r,
            Err(_) => return (0, 0),
        };

        let mut files_changed = 0;
        let mut occurrences = 0;
        for node in self.tree_nodes.iter().filter(|n| n.is_file_header) {
            let content = match std::fs::read_to_string(&node.full_path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let n = re.find_iter(&content).count();
            if n == 0 {
                continue;
            }
            let new_content = if self.search_use_regex {
                re.replace_all(&content, replace_with).into_owned()
            } else {
                re.replace_all(&content, regex::NoExpand(replace_with))
                    .into_owned()
            };
            if new_content != content && std::fs::write(&node.full_path, new_content).is_ok() {
                files_changed += 1;
                occurrences += n;
            }
        }
        (files_changed, occurrences)
    }
}

/// Result when user selects a search result
#[derive(Debug, Clone)]
pub(crate) enum SelectedSearchResult {
    NavigateToFile(PathBuf),
    OpenAtLine { path: PathBuf, line: usize },
}

// ─── Tree building ───────────────────────────────────────────────────────

struct TreeBuildItem<'a> {
    relative_path: &'a str,
    full_path: &'a Path,
    git_status: GitStatus,
    is_dir: bool,
    content_match: Option<ContentMatch>,
}

fn build_tree_nodes(items: Vec<TreeBuildItem<'_>>) -> (Vec<ResultTreeNode>, Vec<String>) {
    if items.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let mut nodes: Vec<ResultTreeNode> = Vec::new();
    let mut added_dirs: HashSet<PathBuf> = HashSet::new();

    for item in &items {
        let rel_path = Path::new(item.relative_path);
        let components: Vec<&std::ffi::OsStr> = rel_path.iter().collect();

        // Add ancestor directories
        for depth in 0..components.len().saturating_sub(1) {
            let dir_path: PathBuf = components[..=depth].iter().collect();
            if !added_dirs.contains(&dir_path) {
                added_dirs.insert(dir_path.clone());
                let dir_name = components[depth].to_string_lossy().into_owned();
                nodes.push(ResultTreeNode {
                    name: dir_name,
                    full_path: Path::new(item.full_path)
                        .ancestors()
                        .nth(components.len() - 1 - depth)
                        .unwrap_or(item.full_path)
                        .to_path_buf(),
                    depth,
                    is_dir: true,
                    git_status: GitStatus::Unmodified,
                    content_match: None,
                    is_file_header: false,
                    match_count: 0,
                    collapsed: false,
                });
            }
        }

        // Add the item itself
        let depth = components.len().saturating_sub(1);
        let name = components
            .last()
            .map(|c| c.to_string_lossy().into_owned())
            .unwrap_or_default();

        nodes.push(ResultTreeNode {
            name,
            full_path: item.full_path.to_path_buf(),
            depth,
            is_dir: item.is_dir,
            git_status: item.git_status,
            content_match: item.content_match.clone(),
            is_file_header: false,
            match_count: 0,
            collapsed: false,
        });
    }

    // Sort by path to maintain tree structure
    nodes.sort_by(|a, b| a.full_path.cmp(&b.full_path));

    // Deduplicate dir nodes
    nodes.dedup_by(|a, b| a.is_dir && b.is_dir && a.full_path == b.full_path);

    let prefixes = compute_tree_prefixes(&nodes);
    (nodes, prefixes)
}

fn compute_tree_prefixes(nodes: &[ResultTreeNode]) -> Vec<String> {
    if nodes.is_empty() {
        return Vec::new();
    }

    let max_depth = nodes.iter().map(|n| n.depth).max().unwrap_or(0);
    if max_depth == 0 {
        return vec![String::new(); nodes.len()];
    }

    let mut has_next_at_level = vec![false; max_depth + 1];
    let mut prefixes: Vec<String> = Vec::with_capacity(nodes.len());

    for node in nodes.iter().rev() {
        let depth = node.depth;

        if depth == 0 {
            has_next_at_level.fill(false);
            has_next_at_level[0] = true;
            prefixes.push(String::new());
            continue;
        }

        let mut prefix = String::with_capacity(depth * 3);
        for (lvl, has_next) in has_next_at_level[1..=depth].iter().enumerate() {
            let lvl = lvl + 1;
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

        for val in &mut has_next_at_level[(depth + 1)..] {
            *val = false;
        }
        has_next_at_level[depth] = true;
    }

    prefixes.reverse();
    prefixes
}

// ─── Background search functions ─────────────────────────────────────────

fn search_files(
    base_path: &Path,
    mask: &str,
    cancel: &AtomicBool,
    git_cache: Option<&GitStatusCache>,
) -> Vec<FileResult> {
    use ignore::WalkBuilder;

    let has_path_sep = mask.contains('/') || mask.contains('\\');
    let has_wildcards = mask.contains('*') || mask.contains('?');

    let glob_pattern = glob::Pattern::new(mask).ok();

    let mask_lower = mask.to_lowercase();
    let mut results = Vec::new();

    let walker = WalkBuilder::new(base_path)
        .hidden(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .build();

    for entry in walker {
        if cancel.load(Ordering::Relaxed) {
            break;
        }

        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let path = entry.path();
        if path == base_path {
            continue;
        }

        let relative_path = path
            .strip_prefix(base_path)
            .map(|r| r.display().to_string())
            .unwrap_or_default();

        let matches = if has_path_sep {
            glob_pattern
                .as_ref()
                .map(|g| g.matches(&relative_path))
                .unwrap_or(false)
        } else if has_wildcards {
            let name = match path.file_name() {
                Some(n) => n.to_string_lossy(),
                None => continue,
            };
            glob_pattern
                .as_ref()
                .map(|g| g.matches(&name))
                .unwrap_or(false)
        } else {
            let name = match path.file_name() {
                Some(n) => n.to_string_lossy(),
                None => continue,
            };
            name.to_lowercase().contains(&mask_lower)
        };

        if !matches {
            continue;
        }

        let is_dir = path.is_dir();
        let git_status = git_cache
            .map(|cache| {
                if is_dir {
                    cache.get_directory_status(&relative_path)
                } else {
                    cache.get_status(&relative_path)
                }
            })
            .unwrap_or(GitStatus::Unmodified);

        results.push(FileResult {
            full_path: path.to_path_buf(),
            relative_path,
            git_status,
            is_dir,
        });

        if results.len() >= MAX_RESULTS {
            break;
        }
    }

    results.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    results
}

fn search_content(
    base_path: &Path,
    mask: &str,
    content_pattern: &str,
    case_sensitive: bool,
    cancel: &AtomicBool,
    git_cache: Option<&GitStatusCache>,
    max_file_size: u64,
) -> Vec<ContentResult> {
    use ignore::WalkBuilder;

    let regex = match RegexBuilder::new(content_pattern)
        .case_insensitive(!case_sensitive)
        .build()
    {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    let has_path_sep = mask.contains('/') || mask.contains('\\');
    let has_wildcards = mask.contains('*') || mask.contains('?');
    let glob_pattern = glob::Pattern::new(mask).ok();
    let mask_lower = mask.to_lowercase();

    let min_size = content_pattern.len() as u64;
    let mut results = Vec::new();

    let walker = WalkBuilder::new(base_path)
        .hidden(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .build();

    for entry in walker {
        if cancel.load(Ordering::Relaxed) {
            break;
        }

        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let path = entry.path();
        if path.is_dir() {
            continue;
        }

        let relative_path = path
            .strip_prefix(base_path)
            .map(|r| r.display().to_string())
            .unwrap_or_default();

        let name_matches = if has_path_sep {
            glob_pattern
                .as_ref()
                .map(|g| g.matches(&relative_path))
                .unwrap_or(false)
        } else if has_wildcards {
            let name = match path.file_name() {
                Some(n) => n.to_string_lossy(),
                None => continue,
            };
            glob_pattern
                .as_ref()
                .map(|g| g.matches(&name))
                .unwrap_or(false)
        } else {
            let name = match path.file_name() {
                Some(n) => n.to_string_lossy(),
                None => continue,
            };
            name.to_lowercase().contains(&mask_lower)
        };

        if !name_matches {
            continue;
        }

        if should_skip_file(path, max_file_size, min_size) {
            continue;
        }

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let lines: Vec<&str> = content.lines().collect();
        let git_status = git_cache
            .map(|cache| cache.get_status(&relative_path))
            .unwrap_or(GitStatus::Unmodified);

        for (line_idx, line) in lines.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                break;
            }

            if let Some(m) = regex.find(line) {
                results.push(ContentResult {
                    full_path: path.to_path_buf(),
                    relative_path: relative_path.clone(),
                    line_number: line_idx + 1,
                    matched_line: line.to_string(),
                    match_start: m.start(),
                    match_end: m.end(),
                    git_status,
                });

                if results.len() >= MAX_RESULTS {
                    return results;
                }
            }
        }

        if results.len() >= MAX_RESULTS {
            break;
        }
    }

    results
}

fn should_skip_file(path: &Path, max_size: u64, min_size: u64) -> bool {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return true,
    };
    let file_size = meta.len();
    if file_size < min_size {
        return true;
    }
    if max_size > 0 && file_size > max_size {
        return true;
    }
    is_binary_file(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(path: PathBuf) -> ResultTreeNode {
        ResultTreeNode {
            name: path.display().to_string(),
            full_path: path,
            depth: 0,
            is_dir: false,
            git_status: GitStatus::Unmodified,
            content_match: None,
            is_file_header: true,
            match_count: 1,
            collapsed: false,
        }
    }

    #[test]
    fn replace_all_literal_rewrites_matched_files() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.txt");
        std::fs::write(&f, "foo bar foo\nbaz\n").unwrap();

        let mut state = FileSearchState::new_content(dir.path().to_path_buf(), 1 << 20);
        state.tree_nodes = vec![header(f.clone())];
        state.search_pattern = regex::escape("foo");
        state.search_use_regex = false;
        state.search_case_sensitive = true;

        assert_eq!(state.replace_all("X"), (1, 2));
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "X bar X\nbaz\n");
    }

    #[test]
    fn replace_all_regex_expands_capture_groups() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("b.rs");
        std::fs::write(&f, "get_user(id)\n").unwrap();

        let mut state = FileSearchState::new_content(dir.path().to_path_buf(), 1 << 20);
        state.tree_nodes = vec![header(f.clone())];
        state.search_pattern = r"get_(\w+)".to_string();
        state.search_use_regex = true;
        state.search_case_sensitive = true;

        assert_eq!(state.replace_all("fetch_$1"), (1, 1));
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "fetch_user(id)\n");
    }

    #[test]
    fn literal_replace_treats_dollar_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("c.txt");
        std::fs::write(&f, "a.b\n").unwrap();

        let mut state = FileSearchState::new_content(dir.path().to_path_buf(), 1 << 20);
        state.tree_nodes = vec![header(f.clone())];
        state.search_pattern = regex::escape(".");
        state.search_use_regex = false;
        state.search_case_sensitive = true;

        assert_eq!(state.replace_all("$1"), (1, 1));
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "a$1b\n");
    }

    #[test]
    fn preview_replacement_builds_new_line() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = FileSearchState::new_content(dir.path().to_path_buf(), 1 << 20);
        state.search_pattern = regex::escape("foo");
        state.search_use_regex = false;
        state.search_case_sensitive = true;
        state.set_replace_text(Some("bar".to_string()));
        assert_eq!(
            state.preview_replacement("foo x foo").as_deref(),
            Some("bar x bar")
        );
        // No preview when replacement is empty.
        state.set_replace_text(Some(String::new()));
        assert!(state.preview_replacement("foo").is_none());
    }
}
