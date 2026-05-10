//! TOML diff/merge primitives used by the layered-config save/load flow.
//!
//! The two functions here form a duality: `diff_toml` produces a minimal
//! TOML value that, when fed into `merge_partial` on top of the same
//! baseline, reproduces the original `actual`.
//!
//! Tables merge structurally; arrays and scalars are treated as atomic
//! units (any difference triggers full replacement). Atomic arrays are a
//! deliberate simplification: keybinding `Vec<String>` lists are the only
//! non-trivial array shape in `Config`, and treating them as a single
//! unit is what users expect when they "set a binding".
//!
//! A `None` `Option<T>` field disappears entirely from the serialized
//! TOML — the diff therefore never emits a key to "clear" a value back
//! to its baseline. For termide this matches the existing `normalize()`
//! contract: missing keybinding entries are filled from defaults at load
//! time.

use toml::Value;

/// Compute the minimal `actual - baseline` diff.
///
/// Returns `None` when the two values are equal (nothing to record).
/// The returned `Value` is always a structural sub-tree of `actual`.
pub fn diff_toml(actual: &Value, baseline: &Value) -> Option<Value> {
    match (actual, baseline) {
        (Value::Table(a), Value::Table(b)) => {
            let mut out = toml::value::Table::new();
            for (key, av) in a {
                match b.get(key) {
                    Some(bv) => {
                        if let Some(diff) = diff_toml(av, bv) {
                            out.insert(key.clone(), diff);
                        }
                    }
                    None => {
                        out.insert(key.clone(), av.clone());
                    }
                }
            }
            if out.is_empty() {
                None
            } else {
                Some(Value::Table(out))
            }
        }
        _ if actual == baseline => None,
        _ => Some(actual.clone()),
    }
}

/// Recursively merge `partial` into `target` in-place.
///
/// Tables merge key-by-key; arrays and scalars in `partial` overwrite the
/// corresponding slot in `target`. Keys absent from `partial` are left
/// untouched, which is the property layered loading relies on.
pub fn merge_partial(target: &mut Value, partial: &Value) {
    match (target, partial) {
        (Value::Table(t), Value::Table(p)) => {
            for (key, pv) in p {
                match t.get_mut(key) {
                    Some(tv) => merge_partial(tv, pv),
                    None => {
                        t.insert(key.clone(), pv.clone());
                    }
                }
            }
        }
        (slot, other) => {
            *slot = other.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use toml::Value;

    fn parse(s: &str) -> Value {
        toml::from_str::<Value>(s).expect("test toml")
    }

    #[test]
    fn equal_scalars_diff_to_none() {
        assert!(diff_toml(&Value::Integer(4), &Value::Integer(4)).is_none());
        assert!(diff_toml(&Value::String("a".into()), &Value::String("a".into())).is_none());
    }

    #[test]
    fn differing_scalar_yields_actual() {
        let d = diff_toml(&Value::Integer(8), &Value::Integer(4)).unwrap();
        assert_eq!(d, Value::Integer(8));
    }

    #[test]
    fn equal_tables_diff_to_none() {
        let a = parse("[editor]\ntab_size = 4\n");
        let b = parse("[editor]\ntab_size = 4\n");
        assert!(diff_toml(&a, &b).is_none());
    }

    #[test]
    fn nested_diff_keeps_only_changed_field() {
        let a = parse("[editor]\ntab_size = 8\nword_wrap = true\n");
        let b = parse("[editor]\ntab_size = 4\nword_wrap = true\n");
        let d = diff_toml(&a, &b).unwrap();
        assert_eq!(d, parse("[editor]\ntab_size = 8\n"));
    }

    #[test]
    fn empty_section_after_filter_is_dropped() {
        let a = parse("[editor]\ntab_size = 4\n[general]\ntheme = \"dark\"\n");
        let b = parse("[editor]\ntab_size = 4\n[general]\ntheme = \"light\"\n");
        let d = diff_toml(&a, &b).unwrap();
        // [editor] section drops out entirely; only [general].theme remains
        assert_eq!(d, parse("[general]\ntheme = \"dark\"\n"));
    }

    #[test]
    fn arrays_are_atomic() {
        // Same array → drops out
        let a = parse("vals = [1, 2, 3]\n");
        let b = parse("vals = [1, 2, 3]\n");
        assert!(diff_toml(&a, &b).is_none());

        // One element differs → whole array emitted
        let a = parse("vals = [1, 2, 3]\n");
        let b = parse("vals = [1, 2, 4]\n");
        let d = diff_toml(&a, &b).unwrap();
        assert_eq!(d, parse("vals = [1, 2, 3]\n"));
    }

    #[test]
    fn missing_in_baseline_is_preserved() {
        let a = parse("[editor]\ntab_size = 4\nnew_field = \"hello\"\n");
        let b = parse("[editor]\ntab_size = 4\n");
        let d = diff_toml(&a, &b).unwrap();
        assert_eq!(d, parse("[editor]\nnew_field = \"hello\"\n"));
    }

    #[test]
    fn missing_in_actual_is_dropped() {
        // baseline has a field, actual does not (e.g. None Option)
        // → diff shouldn't try to "clear" it
        let a = parse("[editor]\ntab_size = 4\n");
        let b = parse("[editor]\ntab_size = 4\nbg_color = \"black\"\n");
        assert!(diff_toml(&a, &b).is_none());
    }

    #[test]
    fn merge_overlays_partial() {
        let mut target = parse("[editor]\ntab_size = 4\nword_wrap = true\n");
        let partial = parse("[editor]\ntab_size = 8\n");
        merge_partial(&mut target, &partial);
        assert_eq!(target, parse("[editor]\ntab_size = 8\nword_wrap = true\n"));
    }

    #[test]
    fn merge_inserts_missing_keys() {
        let mut target = parse("[editor]\ntab_size = 4\n");
        let partial = parse("[general]\ntheme = \"dark\"\n");
        merge_partial(&mut target, &partial);
        assert_eq!(
            target,
            parse("[editor]\ntab_size = 4\n[general]\ntheme = \"dark\"\n")
        );
    }

    #[test]
    fn merge_arrays_overwrite_atomically() {
        let mut target = parse("vals = [1, 2, 3]\n");
        let partial = parse("vals = [9]\n");
        merge_partial(&mut target, &partial);
        assert_eq!(target, parse("vals = [9]\n"));
    }

    #[test]
    fn diff_then_merge_roundtrip() {
        let actual = parse(
            r#"
[editor]
tab_size = 8
word_wrap = false
[general]
theme = "dark"
"#,
        );
        let baseline = parse(
            r#"
[editor]
tab_size = 4
word_wrap = true
[general]
theme = "default"
"#,
        );
        let diff = diff_toml(&actual, &baseline).unwrap();
        let mut reconstructed = baseline.clone();
        merge_partial(&mut reconstructed, &diff);
        assert_eq!(reconstructed, actual);
    }
}
