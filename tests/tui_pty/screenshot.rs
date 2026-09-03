//! Look at the app. Not a test: an ignored case that drives the real binary
//! through the pty with the keys you name and prints the frame it left on
//! screen — the terminal's equivalent of a browser screenshot, for a person
//! or an agent building a screen to *see* it rather than infer it.
//!
//! ```bash
//! FASTF_SHOT_KEYS="down down enter" cargo test --test tui_pty screenshot -- --ignored --nocapture
//! FASTF_SHOT_KEYS="/ type:lulla" FASTF_SHOT_PROJECTS=30 cargo test --test tui_pty screenshot -- --ignored --nocapture
//! FASTF_SHOT_REAL=1 FASTF_SHOT_KEYS="c type:open" cargo test --test tui_pty screenshot -- --ignored --nocapture
//! ```
//!
//! Tokens, whitespace-separated: `enter` `esc` `up` `down` `left` `right`
//! `pgup` `pgdn` `home` `end` `tab` `space` `ctrl-c`, `wait:<ms>`,
//! `type:<text>` (typed as-is, no Enter), and any other token is sent as the
//! keys it spells (`q`, `/`, `?`, `c`). The frame is taken after the last
//! token, before the script ends the app.
//!
//! The library is a sandbox of `FASTF_SHOT_PROJECTS` planted projects (eight
//! by default) unless `FASTF_SHOT_REAL=1`, which runs against **your own**
//! configuration and library — read-only keys only, please.

use super::common::{self, Sandbox, pty};
use super::harness::*;
use std::fs;

#[test]
#[ignore = "a tool, not a check: run it with --ignored --nocapture and FASTF_SHOT_KEYS"]
fn screenshot() {
    let keys = std::env::var("FASTF_SHOT_KEYS").unwrap_or_default();
    let real = std::env::var("FASTF_SHOT_REAL").is_ok_and(|v| v == "1");
    let projects: usize = std::env::var("FASTF_SHOT_PROJECTS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(8);

    let mut script = pty::Script::new().pause(1200);
    for token in keys.split_whitespace() {
        script = match token {
            "enter" => script.enter(),
            "esc" => script.esc(),
            "up" => script.key("\x1b[A"),
            "down" => script.down(1),
            "left" => script.key("\x1b[D"),
            "right" => script.key("\x1b[C"),
            "pgup" => script.page_up(),
            "pgdn" => script.page_down(),
            "home" => script.home(),
            "end" => script.key("\x1b[F"),
            "tab" => script.key("\t"),
            "space" => script.key(" "),
            "ctrl-c" => script.ctrl_c(),
            other => match other.split_once(':') {
                Some(("wait", ms)) => script.pause(ms.parse().unwrap_or(500)),
                Some(("type", text)) => script.key(text),
                _ => script.key(other),
            },
        };
    }
    // Let the last key land and the frame settle, then take the picture and
    // end the app: two Ctrl-C, since the first may only close a dialog.
    script = script.pause(700);
    let taken = script.elapsed();
    let script = script.ctrl_c().ctrl_c().build();

    let sb = Sandbox::new();
    if !real {
        plant_showcase(&sb, projects);
    }
    let env: Vec<(&str, &std::path::Path)> = if real {
        Vec::new()
    } else {
        vec![
            ("FASTF_INSTALL_DIR", sb.install.as_path()),
            ("HOME", sb.tmp.path()),
        ]
    };
    let (chunks, code) = pty::run_chunked(common::FASTF, &[], &env, &script, DEADLINE);
    let screen = screen_at(&chunks, taken);

    println!();
    println!(
        "── screen {}×{} after `{keys}` (exit {code}) ──",
        pty::PTY_COLS,
        pty::PTY_ROWS
    );
    for line in screen.lines() {
        println!("{line}");
    }
    println!("── end ──");
}

/// A library with something in every column: several templates, tags, dates
/// spread across months, and payloads of different sizes.
fn plant_showcase(sb: &Sandbox, n: usize) {
    let templates = [
        ("music-video", "Music video"),
        ("general", "General"),
        ("client-project", "Client project"),
        ("photography", "Photography"),
    ];
    let names = [
        "Lullaby_Remix",
        "Client_Onboarding_Acme",
        "Old_Shoot",
        "Spring_Campaign",
        "Live_Session",
        "Portfolio_Site",
        "Wedding_Highlights",
        "Podcast_Intro",
    ];
    for i in 0..n {
        let (slug, name) = templates[i % templates.len()];
        let id = format!("ID{:04}", 200 + n - i);
        let month = 1 + (i % 8) as u32;
        let day = 1 + (i % 27) as u32;
        let folder = format!("2026-{month:02}-{day:02}_{}_{id}", names[i % names.len()]);
        let root = plant_dated_project(
            sb,
            &folder,
            &id,
            &format!("2026-{month:02}-{day:02}T10:00:00Z"),
            (i + 1) * 40_000,
        );
        let pinfo = root.join("PROJECT_INFO.md");
        let raw = fs::read_to_string(&pinfo).unwrap();
        let tags = match i % 3 {
            0 => "tags:\n  - draft",
            1 => "tags:\n  - client/Acme\n  - draft",
            _ => "tags: []",
        };
        let raw = raw
            .replace("template: general", &format!("template: {slug}"))
            .replace("template_name: General", &format!("template_name: {name}"))
            .replace("tags: []", tags);
        fs::write(&pinfo, raw).unwrap();
    }
    // The index the header reads before discovery answers.
    sb.ok(&["reindex"]);
}
