//! A folder tree as lines of text.
//!
//! The same shape `cli::render::print_tree` prints, produced as data so a
//! frame can scroll it, colour it or measure it. Read-only by design: the
//! preview shows what a template *will* create, and nothing there is picked or
//! folded, so this is a formatter and not a widget with state.

use crate::core::template::FolderNode;

/// One row of the drawn tree: the connectors and the name, already assembled.
pub fn lines(nodes: &[FolderNode], ascii: bool) -> Vec<String> {
    let mut out = Vec::new();
    walk(nodes, "", ascii, &mut out);
    out
}

fn walk(nodes: &[FolderNode], indent: &str, ascii: bool, out: &mut Vec<String>) {
    let (branch, last, pipe) = if ascii {
        ("|-- ", "`-- ", "|")
    } else {
        ("├── ", "└── ", "│")
    };
    for (i, node) in nodes.iter().enumerate() {
        let is_last = i + 1 == nodes.len();
        out.push(format!(
            "{indent}{}{}/",
            if is_last { last } else { branch },
            node.name
        ));
        if !node.children.is_empty() {
            let child = format!("{indent}{}   ", if is_last { " " } else { pipe });
            walk(&node.children, &child, ascii, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::lines;
    use crate::core::template::FolderNode;

    fn node(name: &str, children: Vec<FolderNode>) -> FolderNode {
        FolderNode {
            name: name.to_string(),
            children,
        }
    }

    #[test]
    fn the_last_child_closes_its_branch() {
        let tree = vec![
            node("01_Assets", vec![node("raw", Vec::new())]),
            node("02_Edit", Vec::new()),
        ];
        assert_eq!(
            lines(&tree, false),
            vec![
                "├── 01_Assets/".to_string(),
                "│   └── raw/".to_string(),
                "└── 02_Edit/".to_string(),
            ]
        );
    }

    #[test]
    fn the_ascii_form_uses_no_box_drawing() {
        let tree = vec![node("a", vec![node("b", Vec::new())])];
        let drawn = lines(&tree, true);
        assert_eq!(drawn, vec!["`-- a/".to_string(), "    `-- b/".to_string()]);
        assert!(drawn.iter().all(|line| line.is_ascii()));
    }
}
