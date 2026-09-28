//! Folder paths typed as a list and the tree they make, and a slug made from a
//! name.

use super::*;

// ---------------------------------------------------------------------------
// Folder paths ⇄ tree
// ---------------------------------------------------------------------------

/// Parse flat path strings into a nested tree.
/// `01_Assets/01_Audio` → `01_Assets` with a child `01_Audio`.
pub fn parse_paths_to_tree(paths: &[String]) -> Vec<FolderNode> {
    let mut roots: Vec<FolderNode> = Vec::new();
    for path in paths {
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        insert_path(&mut roots, &parts);
    }
    roots
}

fn insert_path(nodes: &mut Vec<FolderNode>, parts: &[&str]) {
    let Some((head, rest)) = parts.split_first() else {
        return;
    };
    if let Some(node) = nodes.iter_mut().find(|n| n.name == *head) {
        insert_path(&mut node.children, rest);
        return;
    }
    let mut node = FolderNode {
        name: (*head).to_string(),
        children: Vec::new(),
    };
    insert_path(&mut node.children, rest);
    nodes.push(node);
}

/// Flatten a nested tree back into path strings, parents before children.
pub fn flatten_tree(nodes: &[FolderNode], prefix: &str) -> Vec<String> {
    let mut out = Vec::new();
    for node in nodes {
        let path = if prefix.is_empty() {
            node.name.clone()
        } else {
            format!("{prefix}/{}", node.name)
        };
        out.push(path.clone());
        out.extend(flatten_tree(&node.children, &path));
    }
    out
}

/// A name like `My Music Video` as the slug `my-music-video`.
pub fn slugify(name: &str) -> String {
    name.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

/// Keep the slug following the name until somebody types a slug of their own.
pub fn suggest_slug(form: &mut Form) {
    let suggestion = slugify(&form.value("name"));
    if let Some(field) = form.field_mut("slug")
        && !field.touched
        && matches!(field.kind, FieldKind::Text(_))
    {
        field.set_text(suggestion);
    }
}
