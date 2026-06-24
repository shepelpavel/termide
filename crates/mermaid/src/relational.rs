//! Class and entity-relationship rendering.
//!
//! Both are relationship graphs of named boxes, so they reuse the flowchart
//! layered layout: class/entity names become nodes, relationships become
//! labelled edges. Member/attribute detail is listed below the diagram (a
//! compact, readable form for a terminal).

use crate::flowchart::render_flowchart;
use crate::parser::{
    ClassDiagram, Direction, EdgeLine, ErDiagram, FlowEdge, FlowNode, Flowchart, NodeShape,
};

/// Build a flowchart graph whose nodes carry compartment bodies, get-or-
/// inserting nodes (so a relation endpoint without its own declaration still
/// appears as a box).
fn graph(nodes: &[(&str, &[String])], rels: &[(&str, &str, &str)]) -> Flowchart {
    let mut fc = Flowchart {
        direction: Direction::Down,
        nodes: Vec::new(),
        edges: Vec::new(),
    };
    let index = |fc: &mut Flowchart, name: &str| -> usize {
        if let Some(i) = fc.nodes.iter().position(|n| n.id == name) {
            i
        } else {
            fc.nodes.push(FlowNode {
                id: name.to_string(),
                label: name.to_string(),
                shape: NodeShape::Rect,
                body: Vec::new(),
            });
            fc.nodes.len() - 1
        }
    };
    for (name, body) in nodes {
        let i = index(&mut fc, name);
        fc.nodes[i].body = body.to_vec();
    }
    for (from, to, label) in rels {
        let f = index(&mut fc, from);
        let t = index(&mut fc, to);
        fc.edges.push(FlowEdge {
            from: f,
            to: t,
            label: label.to_string(),
            line: EdgeLine::Solid,
            arrow: true,
        });
    }
    fc
}

/// Lay out a relationship graph (shared by class and ER diagrams): the box
/// nodes (name + compartment body) and labelled relations, or an empty-state
/// line when there are no nodes.
fn render_relation_graph(
    nodes: &[(&str, &[String])],
    rels: &[(&str, &str, &str)],
    empty: &str,
) -> Vec<String> {
    if nodes.is_empty() {
        return vec![empty.to_string()];
    }
    render_flowchart(&graph(nodes, rels))
}

/// Render a class diagram: a relationship graph with members inside each box.
#[must_use]
pub fn render_class(d: &ClassDiagram) -> Vec<String> {
    let nodes: Vec<(&str, &[String])> = d
        .entries
        .iter()
        .map(|e| (e.name.as_str(), e.members.as_slice()))
        .collect();
    let rels: Vec<(&str, &str, &str)> = d
        .rels
        .iter()
        .map(|r| (r.from.as_str(), r.to.as_str(), r.label.as_str()))
        .collect();
    render_relation_graph(&nodes, &rels, "(empty class diagram)")
}

/// Render an ER diagram: a relationship graph with attributes inside each box.
#[must_use]
pub fn render_er(d: &ErDiagram) -> Vec<String> {
    let nodes: Vec<(&str, &[String])> = d
        .entries
        .iter()
        .map(|e| (e.name.as_str(), e.attrs.as_slice()))
        .collect();
    let rels: Vec<(&str, &str, &str)> = d
        .rels
        .iter()
        .map(|r| (r.from.as_str(), r.to.as_str(), r.label.as_str()))
        .collect();
    render_relation_graph(&nodes, &rels, "(empty ER diagram)")
}

#[cfg(test)]
mod tests {
    use crate::parser::{parse_class, parse_er};

    use super::*;

    #[test]
    fn class_renders_graph_and_members() {
        let src =
            "classDiagram\nclass Animal {\n+String name\n+makeSound() void\n}\nAnimal <|-- Dog";
        let out = render_class(&parse_class(src)).join("\n");
        assert!(out.contains("Animal") && out.contains("Dog"), "{out}");
        assert!(out.contains("makeSound() void"), "members listed: {out}");
        assert!(out.contains("inherits"), "relation label: {out}");
    }

    #[test]
    fn er_renders_graph_and_cardinality() {
        let src = "erDiagram\nCUSTOMER ||--o{ ORDER : places\nCUSTOMER {\nstring name\n}";
        let out = render_er(&parse_er(src)).join("\n");
        assert!(out.contains("CUSTOMER") && out.contains("ORDER"), "{out}");
        assert!(out.contains("places"), "verb: {out}");
        assert!(
            out.contains("1") && out.contains("0..N"),
            "cardinality: {out}"
        );
        assert!(out.contains("string name"), "attrs listed: {out}");
    }
}
