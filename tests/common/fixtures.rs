//! Templates to test against, written the way fastf stores them.

use std::fs;
use std::path::Path;

/// Install a template in v0.8 folder form: `templates/<slug>/template.yaml`
/// plus a `files/` subtree.
///
/// For test convenience the fixture YAML may still carry an inline `files:`
/// block, as pre-v0.8 flat templates did; this splits it onto disk exactly like
/// the real conversion, so the copy engine — which walks `files/` and never the
/// manifest — sees the files. The `files:` key left in the manifest is an
/// unknown field that `Template`'s deserializer ignores.
pub fn write_template(install: &Path, slug: &str, yaml: &str) {
    #[derive(serde::Deserialize)]
    struct InlineFiles {
        #[serde(default)]
        files: Vec<InlineFile>,
    }
    #[derive(serde::Deserialize)]
    struct InlineFile {
        path: String,
        #[serde(default)]
        template: String,
        #[serde(default)]
        content: String,
    }

    let dir = install.join("templates").join(slug);
    fs::create_dir_all(dir.join("files")).unwrap();
    fs::write(dir.join("template.yaml"), yaml).unwrap();
    if let Ok(inline) = serde_yaml_ng::from_str::<InlineFiles>(yaml) {
        for file in inline.files {
            let body = if !file.template.is_empty() {
                file.template
            } else {
                file.content
            };
            let dest = dir.join("files").join(&file.path);
            fs::create_dir_all(dest.parent().unwrap()).unwrap();
            fs::write(dest, body).unwrap();
        }
    }
}

/// A minimal valid template: one text variable, one nested folder, one
/// templated file.
pub fn minimal_template_yaml(slug: &str) -> String {
    format!(
        r#"name: Test
slug: {slug}
description: fixture
naming_pattern: "{{id}}_{{name}}"
id:
  prefix: T
  digits: 3
variables:
  - slug: name
    label: Name
    type: text
    required: true
    transform: title_underscore
structure:
  - name: src
    children:
      - name: core
files:
  - path: README.md
    template: |
      # {{name}}
      id: {{id}}
"#
    )
}

/// `minimal_template_yaml` written to disk.
pub fn write_minimal_template(install: &Path, slug: &str) {
    write_template(install, slug, &minimal_template_yaml(slug));
}
