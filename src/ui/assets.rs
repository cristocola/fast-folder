//! Frontend assets for the Fast Folder UI.
//!
//! Assets are embedded in the binary by default, so `fastf` stays a portable
//! single-file distribution with no external web directory to locate. If the
//! `FASTF_UI_DIR` environment variable is set, the named file is read from that
//! directory instead — this preserves the frontend live-reload workflow during
//! development (edit `src/ui/web/*`, refresh the browser, no rebuild needed).

use anyhow::{Context, Result, bail};
use std::path::Path;

const INDEX_HTML: &str = include_str!("web/index.html");
const STYLES_CSS: &str = include_str!("web/styles.css");
const APP_JS: &str = include_str!("web/app.js");
const ICON_SVG: &str = include_str!("web/icon.svg");

/// Resolve a static GET route to its `(content_type, bytes)` pair.
///
/// Only the four known frontend files are served; anything else is a 404
/// (`not found:` prefix, which the caller maps to HTTP 404).
pub fn serve(route: &str) -> Result<(&'static str, Vec<u8>)> {
    let filename = match route.split('?').next().unwrap_or(route) {
        "/" | "/index.html" => "index.html",
        "/styles.css" => "styles.css",
        "/app.js" => "app.js",
        "/icon.svg" => "icon.svg",
        other => bail!("not found: GET {other}"),
    };
    let content_type = content_type_for(filename);

    // Dev override: serve from disk so frontend edits appear on a plain refresh.
    if let Ok(dir) = std::env::var("FASTF_UI_DIR") {
        let path = Path::new(&dir).join(filename);
        let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        return Ok((content_type, bytes));
    }

    let embedded = match filename {
        "index.html" => INDEX_HTML,
        "styles.css" => STYLES_CSS,
        "app.js" => APP_JS,
        "icon.svg" => ICON_SVG,
        _ => unreachable!("filename already validated above"),
    };
    Ok((content_type, embedded.as_bytes().to_vec()))
}

fn content_type_for(filename: &str) -> &'static str {
    match filename.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}
