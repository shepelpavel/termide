//! A backend-agnostic cell value, decoded for display in the table view.

/// One table cell, normalised across engines. Rendering is the panel's job;
/// this just preserves enough structure to format/copy losslessly.
#[derive(Debug, Clone, PartialEq)]
pub enum DbValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    /// Raw bytes (BLOB / bytea); shown as a length + hex preview by the UI.
    Bytes(Vec<u8>),
}

impl DbValue {
    /// Plain-text rendering for the table grid. `NULL` is rendered by the UI
    /// (with a distinct style), so here it's an empty string.
    pub fn display(&self) -> String {
        match self {
            DbValue::Null => String::new(),
            DbValue::Bool(b) => b.to_string(),
            DbValue::Int(i) => i.to_string(),
            DbValue::Float(f) => f.to_string(),
            DbValue::Text(s) => s.clone(),
            DbValue::Bytes(b) => format!("0x{} ({} bytes)", hex_preview(b, 8), b.len()),
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, DbValue::Null)
    }
}

fn hex_preview(bytes: &[u8], max: usize) -> String {
    let mut s = String::with_capacity(max * 2 + 1);
    for b in bytes.iter().take(max) {
        s.push_str(&format!("{b:02x}"));
    }
    if bytes.len() > max {
        s.push('…');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_formats() {
        assert_eq!(DbValue::Null.display(), "");
        assert_eq!(DbValue::Int(42).display(), "42");
        assert_eq!(DbValue::Text("hi".into()).display(), "hi");
        assert!(DbValue::Bytes(vec![0xde, 0xad])
            .display()
            .starts_with("0xdead ("));
    }
}
