//! Whole-binary guard for syntax-grammar loading.
//!
//! Integration tests in the root binary crate link the *entire* workspace
//! dependency graph — unlike `cargo test -p <crate>`, which links only that
//! crate's subtree. That distinction matters: tree-sitter grammars export a
//! C symbol per language (e.g. `tree_sitter_php`), so if two crates pull
//! different major-incompatible versions of the same grammar, both copies link
//! and the symbol collides — the wrong ABI can win and silently disable that
//! language. A per-crate test can't see this because it never links the second
//! copy. This test does, so it catches a recurrence of the PHP 0.23/0.24 split.

/// Every language the highlighter advertises must actually load when the full
/// binary is linked. A `None` here means a grammar was dropped at startup
/// (ABI mismatch or a duplicate-version symbol collision).
#[test]
fn all_supported_grammars_load_in_full_binary() {
    let highlighter = termide_highlight::global_highlighter();
    let missing: Vec<&str> = termide_highlight::SUPPORTED_LANGUAGES
        .iter()
        .filter(|lang| highlighter.get_config(lang).is_none())
        .copied()
        .collect();
    assert!(
        missing.is_empty(),
        "grammars failed to load in the full binary (likely a tree-sitter \
         version/ABI collision across crates): {missing:?}"
    );
}
