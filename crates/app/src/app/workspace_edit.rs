//! Apply LSP WorkspaceEdit to files on disk and reload open editors.

use std::path::PathBuf;

use anyhow::Result;
use lsp_types::{TextEdit, WorkspaceEdit};

use crate::PanelExt;

use super::App;

impl App {
    /// Apply an LSP WorkspaceEdit: write changed files to disk and reload open editors.
    ///
    /// Returns the number of files that were modified.
    pub(super) fn apply_workspace_edit(&mut self, edit: WorkspaceEdit) -> Result<usize> {
        // Collect (uri_string, text_edits) pairs from either changes or document_changes
        let mut edits_by_file: Vec<(String, Vec<TextEdit>)> = Vec::new();

        if let Some(changes) = edit.changes {
            for (uri, text_edits) in changes {
                edits_by_file.push((uri.to_string(), text_edits));
            }
        } else if let Some(doc_changes) = edit.document_changes {
            use lsp_types::DocumentChanges;
            if let DocumentChanges::Edits(annotated_edits) = doc_changes {
                for te in annotated_edits {
                    let uri_str = te.text_document.uri.to_string();
                    let text_edits: Vec<TextEdit> = te
                        .edits
                        .into_iter()
                        .filter_map(|e| match e {
                            lsp_types::OneOf::Left(te) => Some(te),
                            lsp_types::OneOf::Right(_) => None,
                        })
                        .collect();
                    edits_by_file.push((uri_str, text_edits));
                }
            }
        }

        if edits_by_file.is_empty() {
            return Ok(0);
        }

        let mut file_count = 0;
        let mut modified_paths: Vec<PathBuf> = Vec::new();

        for (uri_str, text_edits) in edits_by_file {
            if !uri_str.starts_with("file://") {
                continue;
            }
            let path_str = &uri_str[7..];
            #[cfg(unix)]
            let path = PathBuf::from(path_str);
            #[cfg(windows)]
            let path = PathBuf::from(path_str.trim_start_matches('/'));

            let content = std::fs::read_to_string(&path)?;
            let new_content = apply_text_edits(&content, &text_edits);
            std::fs::write(&path, &new_content)?;
            modified_paths.push(path);
            file_count += 1;
        }

        // Reload any open editors that were modified
        for path in &modified_paths {
            self.reload_editor_if_open(path);
        }

        Ok(file_count)
    }

    /// Reload the editor that has `path` open, if any.
    fn reload_editor_if_open(&mut self, path: &std::path::Path) {
        for panel in self.layout_manager.iter_all_panels_mut() {
            if let Some(editor) = panel.as_editor_mut() {
                if editor.file_path().map(|p| p == path).unwrap_or(false) {
                    if let Err(e) = editor.reload_from_disk() {
                        log::warn!("Failed to reload editor after rename: {}", e);
                    }
                    break;
                }
            }
        }
    }
}

/// Apply a list of TextEdits to a string (content of a file).
///
/// Edits are sorted in reverse order (bottom-to-top) so offsets don't shift.
fn apply_text_edits(content: &str, edits: &[TextEdit]) -> String {
    // Collect lines preserving original line endings
    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();

    let mut sorted = edits.to_vec();
    sorted.sort_by(|a, b| {
        b.range
            .start
            .line
            .cmp(&a.range.start.line)
            .then(b.range.start.character.cmp(&a.range.start.character))
    });

    for edit in sorted {
        let sl = edit.range.start.line as usize;
        let sc = edit.range.start.character as usize;
        let el = edit.range.end.line as usize;
        let ec = edit.range.end.character as usize;

        if sl == el {
            // Single-line edit
            if sl < lines.len() {
                let chars: Vec<char> = lines[sl].chars().collect();
                let before: String = chars[..sc.min(chars.len())].iter().collect();
                let after: String = chars[ec.min(chars.len())..].iter().collect();
                lines[sl] = format!("{}{}{}", before, edit.new_text, after);
            }
        } else {
            // Multi-line edit: join affected lines then replace range
            let first_chars: Vec<char> = lines
                .get(sl)
                .map(|l| l.chars().collect())
                .unwrap_or_default();
            let last_chars: Vec<char> = lines
                .get(el)
                .map(|l| l.chars().collect())
                .unwrap_or_default();
            let before: String = first_chars[..sc.min(first_chars.len())].iter().collect();
            let after: String = last_chars[ec.min(last_chars.len())..].iter().collect();
            let new_line = format!("{}{}{}", before, edit.new_text, after);
            // Replace lines sl..=el with new_line
            let end = el.min(lines.len().saturating_sub(1));
            lines[sl] = new_line;
            for _ in sl + 1..=end {
                if sl + 1 < lines.len() {
                    lines.remove(sl + 1);
                }
            }
        }
    }

    let result = lines.join("\n");
    // Preserve trailing newline
    if content.ends_with('\n') && !result.ends_with('\n') {
        format!("{}\n", result)
    } else {
        result
    }
}
