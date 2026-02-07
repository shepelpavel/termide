//! Regex fallback for symbol extraction from markdown and HTML files.

use regex::Regex;
use std::sync::OnceLock;

use crate::symbols::{SymbolInfo, SymbolKind};

static MD_HEADING_RE: OnceLock<Regex> = OnceLock::new();
static HTML_HEADING_RE: OnceLock<Regex> = OnceLock::new();
static TOML_SECTION_RE: OnceLock<Regex> = OnceLock::new();

fn md_heading_regex() -> &'static Regex {
    MD_HEADING_RE.get_or_init(|| Regex::new(r"^(#{1,6})\s+(.+)$").expect("valid regex"))
}

fn html_heading_regex() -> &'static Regex {
    HTML_HEADING_RE.get_or_init(|| Regex::new(r"<h([1-6])[^>]*>([^<]+)").expect("valid regex"))
}

fn toml_section_regex() -> &'static Regex {
    // Matches [section], [section.sub], [[array]], [[array.sub]]
    // Only bare TOML keys: alphanumeric, '_', '-', joined by '.'
    TOML_SECTION_RE.get_or_init(|| {
        Regex::new(r"^\s*\[(\[?)\s*([A-Za-z0-9_-]+(?:\.[A-Za-z0-9_-]+)*)\s*\]?\]")
            .expect("valid regex")
    })
}

/// Extract symbols using regex patterns for languages without tree-sitter support.
pub fn extract_symbols_regex(source: &str, language: &str) -> Vec<SymbolInfo> {
    match language {
        "markdown" => extract_markdown_headings(source),
        "html" => extract_html_headings(source),
        "toml" => extract_toml_sections(source),
        _ => Vec::new(),
    }
}

fn extract_markdown_headings(source: &str) -> Vec<SymbolInfo> {
    let re = md_heading_regex();
    let mut symbols = Vec::new();

    for (line_idx, line) in source.lines().enumerate() {
        if let Some(caps) = re.captures(line) {
            let hashes = caps.get(1).map(|m| m.as_str().len()).unwrap_or(1);
            let name = caps.get(2).map(|m| m.as_str().trim()).unwrap_or("");
            if name.is_empty() {
                continue;
            }
            let kind = match hashes {
                1 => SymbolKind::Heading1,
                2 => SymbolKind::Heading2,
                3 => SymbolKind::Heading3,
                4 => SymbolKind::Heading4,
                5 => SymbolKind::Heading5,
                _ => SymbolKind::Heading6,
            };
            symbols.push(SymbolInfo {
                name: name.to_string(),
                full_name: None,
                kind,
                line: line_idx,
                column: 0,
                depth: hashes.saturating_sub(1),
            });
        }
    }

    symbols
}

fn extract_toml_sections(source: &str) -> Vec<SymbolInfo> {
    let re = toml_section_regex();
    let mut symbols = Vec::new();
    let mut emitted: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for (line_idx, line) in source.lines().enumerate() {
        if let Some(caps) = re.captures(line) {
            let is_array = caps.get(1).is_some_and(|m| !m.as_str().is_empty());
            if is_array {
                continue;
            }
            let full_name = caps.get(2).map(|m| m.as_str().trim()).unwrap_or("");
            if full_name.is_empty() {
                continue;
            }

            // Find the longest emitted ancestor prefix
            let segments: Vec<&str> = full_name.split('.').collect();
            let mut ancestor_len = 0;
            let mut ancestor_depth = 0;
            for i in (1..segments.len()).rev() {
                let prefix = segments[..i].join(".");
                if let Some(&d) = emitted.get(&prefix) {
                    ancestor_len = i;
                    ancestor_depth = d;
                    break;
                }
            }

            let depth = if ancestor_len > 0 {
                ancestor_depth + 1
            } else {
                0
            };
            let display_name = segments[ancestor_len..].join(".");

            emitted.insert(full_name.to_string(), depth);
            symbols.push(SymbolInfo {
                name: display_name,
                full_name: if full_name.contains('.') {
                    Some(full_name.to_string())
                } else {
                    None
                },
                kind: SymbolKind::Section,
                line: line_idx,
                column: 0,
                depth,
            });
        }
    }

    symbols
}

fn extract_html_headings(source: &str) -> Vec<SymbolInfo> {
    let re = html_heading_regex();
    let mut symbols = Vec::new();

    for (line_idx, line) in source.lines().enumerate() {
        for caps in re.captures_iter(line) {
            let level: usize = caps
                .get(1)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(1);
            let name = caps.get(2).map(|m| m.as_str().trim()).unwrap_or("");
            if name.is_empty() {
                continue;
            }
            let kind = match level {
                1 => SymbolKind::Heading1,
                2 => SymbolKind::Heading2,
                3 => SymbolKind::Heading3,
                4 => SymbolKind::Heading4,
                5 => SymbolKind::Heading5,
                _ => SymbolKind::Heading6,
            };
            symbols.push(SymbolInfo {
                name: name.to_string(),
                full_name: None,
                kind,
                line: line_idx,
                column: 0,
                depth: level.saturating_sub(1),
            });
        }
    }

    symbols
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_toml_sections_skip_arrays() {
        let source = "[package]\nname = \"foo\"\n\n[[bin]]\nname = \"bar\"\n\n[dependencies]\n";
        let symbols = extract_toml_sections(source);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["package", "dependencies"]);
    }

    #[test]
    fn test_toml_nested_sections() {
        let source = "[a]\n[a.b]\n[[a.b.c]]\n";
        let symbols = extract_toml_sections(source);
        assert_eq!(symbols.len(), 2);
        assert_eq!(symbols[0].name, "a");
        assert_eq!(symbols[0].depth, 0);
        assert_eq!(symbols[1].name, "b");
        assert_eq!(symbols[1].depth, 1);
    }

    #[test]
    fn test_toml_skipped_ancestors_show_relative_path() {
        // [package] exists, but [package.metadata] and [package.metadata.docs] do not
        let source = "[package]\n[package.metadata.docs.rs]\n[package.metadata.deb]\n";
        let symbols = extract_toml_sections(source);
        assert_eq!(symbols.len(), 3);
        assert_eq!(symbols[0].name, "package");
        assert_eq!(symbols[0].depth, 0);
        // metadata.docs.rs collapsed under package
        assert_eq!(symbols[1].name, "metadata.docs.rs");
        assert_eq!(symbols[1].depth, 1);
        // metadata.deb — now package.metadata is emitted, so deb is under it
        assert_eq!(symbols[2].name, "metadata.deb");
        assert_eq!(symbols[2].depth, 1);
    }

    #[test]
    fn test_toml_inline_arrays_not_matched() {
        // Inline array values like ["target/release/termide", "usr/bin/", "755"]
        // must not be matched as section headers
        let source = "[package]\nassets = [\n  [\"target/release/termide\", \"usr/bin/\", \"755\"],\n]\n[dependencies]\n";
        let symbols = extract_toml_sections(source);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["package", "dependencies"]);
    }

    #[test]
    fn test_toml_full_name_for_dotted_sections() {
        let source = "[package]\n[package.metadata.docs.rs]\n[dependencies]\n";
        let symbols = extract_toml_sections(source);
        assert!(symbols[0].full_name.is_none()); // "package" — no dots
        assert_eq!(
            symbols[1].full_name.as_deref(),
            Some("package.metadata.docs.rs")
        );
        assert!(symbols[2].full_name.is_none()); // "dependencies" — no dots
    }

    #[test]
    fn test_toml_deep_chain_with_intermediate() {
        // [package.metadata.docs.rs] then [package.metadata.generate-rpm.requires]
        let source = "[package]\n[package.metadata.docs.rs]\n[package.metadata.generate-rpm]\n[package.metadata.generate-rpm.requires]\n";
        let symbols = extract_toml_sections(source);
        assert_eq!(symbols.len(), 4);
        assert_eq!(symbols[2].name, "metadata.generate-rpm");
        assert_eq!(symbols[2].depth, 1);
        // requires is a child of the emitted generate-rpm
        assert_eq!(symbols[3].name, "requires");
        assert_eq!(symbols[3].depth, 2);
    }
}
