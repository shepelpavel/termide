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

        // Keep the language server's in-memory document in sync with the edit it
        // just asked us to apply. Without this, a server that performed the edit
        // via executeCommand (e.g. phpactor "Import class") still believes the
        // file is unchanged and re-applies the same edit next time — duplicating
        // the `use` statement on a second invocation.
        if let Some(lsp_manager) = self.state.lsp_manager.as_ref() {
            for panel in self.layout_manager.iter_all_panels_mut() {
                if let Some(editor) = panel.as_editor_mut() {
                    let is_modified = editor
                        .file_path()
                        .map(|p| modified_paths.iter().any(|m| m == p))
                        .unwrap_or(false);
                    if is_modified {
                        editor.notify_lsp_change(lsp_manager);
                    }
                }
            }
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

/// Apply a list of `TextEdit`s to a string (content of a file).
///
/// Edits address the original document, so we resolve every `(line, character)`
/// position to an absolute character offset once, then splice from the end of
/// the document toward the start — that way earlier offsets stay valid as we go.
/// Edits that share a start offset (e.g. phpactor's "Import class" inserts both
/// a blank line and a `use` line at the same point) must end up in array order;
/// since we apply back-to-front, the later array entry is spliced first, hence
/// the descending original-index tie-break. The whole content (including line
/// endings) is preserved verbatim except where an edit replaces it.
fn apply_text_edits(content: &str, edits: &[TextEdit]) -> String {
    let chars: Vec<char> = content.chars().collect();

    // line_start[i] = index into `chars` where line `i` begins.
    let mut line_start = vec![0usize];
    for (i, &c) in chars.iter().enumerate() {
        if c == '\n' {
            line_start.push(i + 1);
        }
    }

    let pos_to_offset = |line: u32, character: u32| -> usize {
        let l = line as usize;
        if l >= line_start.len() {
            return chars.len();
        }
        let base = line_start[l];
        // Don't let a character offset spill past this line into the next one.
        let line_end = line_start.get(l + 1).copied().unwrap_or(chars.len());
        (base + character as usize).min(line_end)
    };

    // (start_offset, end_offset, new_text, original_index)
    let mut spans: Vec<(usize, usize, &str, usize)> = edits
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let start = pos_to_offset(e.range.start.line, e.range.start.character);
            let end = pos_to_offset(e.range.end.line, e.range.end.character);
            (start, end.max(start), e.new_text.as_str(), i)
        })
        .collect();
    spans.sort_by(|a, b| b.0.cmp(&a.0).then(b.3.cmp(&a.3)));

    let mut out = chars;
    for (start, end, new_text, _) in spans {
        let start = start.min(out.len());
        let end = end.min(out.len());
        out.splice(start..end, new_text.chars());
    }
    out.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::apply_text_edits;
    use lsp_types::{Position, Range, TextEdit};

    fn insert(line: u32, character: u32, new_text: &str) -> TextEdit {
        let pos = Position { line, character };
        TextEdit {
            range: Range {
                start: pos,
                end: pos,
            },
            new_text: new_text.to_string(),
        }
    }

    // phpactor's "Import class" delivers two zero-width inserts at the SAME
    // position. The line-based predecessor mis-ordered/duplicated them; the
    // offset splice must reproduce the server's intent and leave the usage line
    // (and every other line) untouched. Payloads below are captured verbatim
    // from phpactor 2025.12.21.1.
    #[test]
    fn phpactor_import_with_namespace() {
        let content = "<?php\n\nnamespace App;\n\nclass Controller\n{\n    public function index()\n    {\n        return Order::where('id', 1)->get();\n    }\n}\n";
        // Both inserts land at end of `namespace App;` (line 2, char 14).
        let edits = vec![
            insert(2, 14, "\n"),
            insert(2, 14, "\nuse App\\Models\\Order\\Order;"),
        ];
        let result = apply_text_edits(content, &edits);
        assert!(
            result.contains("namespace App;\n\nuse App\\Models\\Order\\Order;\n\nclass Controller"),
            "import not placed correctly:\n{result}"
        );
        // The class usage must be preserved exactly.
        assert!(result.contains("        return Order::where('id', 1)->get();\n"));
    }

    #[test]
    fn phpactor_import_without_namespace() {
        let content = "<?php\n\nclass Top\n{\n    public function run()\n    {\n        return Order::where('id', 1)->get();\n    }\n}\n";
        // Both inserts land at the start of line 1; note the array order here is
        // the reverse of the namespaced case.
        let edits = vec![
            insert(1, 0, "\nuse App\\Models\\Order\\Order;"),
            insert(1, 0, "\n"),
        ];
        let result = apply_text_edits(content, &edits);
        assert!(
            result.starts_with("<?php\n\nuse App\\Models\\Order\\Order;\n\nclass Top"),
            "import not placed correctly:\n{result}"
        );
        assert!(result.contains("        return Order::where('id', 1)->get();\n"));
    }

    #[test]
    fn multiline_replacement_spans_and_collapses_lines() {
        let content = "alpha\nbeta\ngamma\ndelta\n";
        // Replace from line 1 col 0 through line 2 col 0 with "BETA\n".
        let edit = TextEdit {
            range: Range {
                start: Position {
                    line: 1,
                    character: 0,
                },
                end: Position {
                    line: 2,
                    character: 0,
                },
            },
            new_text: "BETA\n".to_string(),
        };
        assert_eq!(
            apply_text_edits(content, &[edit]),
            "alpha\nBETA\ngamma\ndelta\n"
        );
    }

    #[test]
    fn single_line_replacement_keeps_surrounding_text() {
        let content = "let foo = oldName(x);\n";
        let edit = TextEdit {
            range: Range {
                start: Position {
                    line: 0,
                    character: 10,
                },
                end: Position {
                    line: 0,
                    character: 17,
                },
            },
            new_text: "newName".to_string(),
        };
        assert_eq!(
            apply_text_edits(content, &[edit]),
            "let foo = newName(x);\n"
        );
    }
}
