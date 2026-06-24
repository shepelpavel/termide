//! Mermaid parsing and text-pseudographics layout — a UI-agnostic library.
//!
//! Returns plain `Vec<String>` canvases so it can be reused both by the
//! standalone Mermaid viewer panel and by the Markdown renderer for embedded
//! ```` ```mermaid ```` blocks. No terminal/UI dependencies.

mod canvas;
pub mod chart;
pub mod flowchart;
pub mod gitgraph;
pub mod parser;
pub mod relational;
pub mod render;

pub use parser::{
    detect_kind, parse_class, parse_er, parse_flowchart, parse_gantt, parse_gitgraph,
    parse_journey, parse_mindmap, parse_pie, parse_quadrant, parse_sequence, parse_state,
    parse_timeline, ClassDiagram, DiagramKind, ErDiagram, Flowchart, Gantt, GitGraph, Journey,
    Mindmap, Pie, Quadrant, Sequence, Timeline,
};
pub use render::render_sequence;

pub use chart::{
    render_gantt, render_journey, render_mindmap, render_pie, render_quadrant, render_timeline,
};
pub use flowchart::render_flowchart;
pub use gitgraph::render_gitgraph;
pub use relational::{render_class, render_er};

/// Render any supported Mermaid source to canvas lines. Unsupported kinds
/// return `None` so the caller can fall back (e.g. show the source).
pub fn render_to_lines(src: &str) -> Option<Vec<String>> {
    match detect_kind(src) {
        DiagramKind::Sequence => Some(render_sequence(&parse_sequence(src))),
        DiagramKind::Flowchart => Some(render_flowchart(&parse_flowchart(src))),
        DiagramKind::State => Some(render_flowchart(&parse_state(src))),
        DiagramKind::Pie => Some(render_pie(&parse_pie(src))),
        DiagramKind::Class => Some(render_class(&parse_class(src))),
        DiagramKind::Er => Some(render_er(&parse_er(src))),
        DiagramKind::Gantt => Some(render_gantt(&parse_gantt(src))),
        DiagramKind::Journey => Some(render_journey(&parse_journey(src))),
        DiagramKind::Mindmap => Some(render_mindmap(&parse_mindmap(src))),
        DiagramKind::Timeline => Some(render_timeline(&parse_timeline(src))),
        DiagramKind::GitGraph => Some(render_gitgraph(&parse_gitgraph(src))),
        DiagramKind::Quadrant => Some(render_quadrant(&parse_quadrant(src))),
        DiagramKind::Other | DiagramKind::Unknown => None,
    }
}
