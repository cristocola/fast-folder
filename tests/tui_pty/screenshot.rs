//! Look at the app. Not a test: an ignored case that drives the real binary
//! through the pty with the keys you name and prints the frame it left on
//! screen — the terminal's equivalent of a browser screenshot, for a person
//! or an agent building a screen to *see* it rather than infer it.
//!
//! ```bash
//! FASTF_SHOT_KEYS="down down enter" cargo test --test tui_pty screenshot -- --ignored --nocapture
//! FASTF_SHOT_KEYS="/ type:lulla" FASTF_SHOT_PROJECTS=30 cargo test --test tui_pty screenshot -- --ignored --nocapture
//! FASTF_SHOT_REAL=1 FASTF_SHOT_KEYS="c type:open" cargo test --test tui_pty screenshot -- --ignored --nocapture
//! FASTF_SHOT_ARGS="copy shared" FASTF_SHOT_KEYS="down" cargo test --test tui_pty screenshot -- --ignored --nocapture
//! ```
//!
//! `FASTF_SHOT_ARGS` drives a *subcommand* instead of the guided app, which is
//! how the command line's inline prompts — the ambiguity picker, a confirm, a
//! text field — are looked at. They draw where the cursor is rather than on the
//! alternate screen, so the frame includes whatever was printed above them.
//!
//! Tokens, whitespace-separated: `enter` `esc` `up` `down` `left` `right`
//! `pgup` `pgdn` `home` `end` `tab` `space` `backspace` `delete` `f1` `f5`,
//! `ctrl-<letter>` for any control chord (`ctrl-c` `ctrl-s` `ctrl-n` `ctrl-t`
//! `ctrl-u` `ctrl-k` `ctrl-r` `ctrl-z`), `alt-enter`, `wait:<ms>`,
//! `type:<text>` (typed as-is, no Enter), and any other token is sent as the
//! keys it spells (`q`, `/`, `?`, `c`). The frame is taken after the last
//! token, before the script ends the app.
//!
//! `FASTF_SHOT_SIZE=80x24` runs the app in that window instead of the suite's
//! 120×40, which is how the compact layout is looked at.
//!
//! `FASTF_SHOT_SVG=docs/img/dashboard.svg` also writes the frame as an SVG in
//! the app's truecolor palette — the README's screenshot, taken from the real
//! binary. Sandbox only: the repository is public.
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
    let args: Vec<String> = std::env::var("FASTF_SHOT_ARGS")
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let real = std::env::var("FASTF_SHOT_REAL").is_ok_and(|v| v == "1");
    let projects: usize = std::env::var("FASTF_SHOT_PROJECTS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(8);

    let (cols, rows) = std::env::var("FASTF_SHOT_SIZE")
        .ok()
        .and_then(|size| {
            let (c, r) = size.split_once('x')?;
            Some((c.parse().ok()?, r.parse().ok()?))
        })
        .unwrap_or((pty::PTY_COLS, pty::PTY_ROWS));

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
            "backspace" => script.key("\x7f"),
            "delete" => script.key("\x1b[3~"),
            "f1" => script.key("\x1bOP"),
            "f5" => script.key("\x1b[15~"),
            "alt-enter" => script.key("\x1b\r"),
            "ctrl-c" => script.ctrl_c(),
            other => match other.split_once(':') {
                Some(("wait", ms)) => script.pause(ms.parse().unwrap_or(500)),
                Some(("type", text)) => script.key(text),
                _ => match other.strip_prefix("ctrl-") {
                    // A control chord is the letter's position in the
                    // alphabet: Ctrl-A is 0x01, Ctrl-Z 0x1a.
                    Some(letter)
                        if letter.len() == 1 && letter.as_bytes()[0].is_ascii_lowercase() =>
                    {
                        script.key(&((letter.as_bytes()[0] - b'a' + 1) as char).to_string())
                    }
                    _ => script.key(other),
                },
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
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    let svg_path = std::env::var("FASTF_SHOT_SVG")
        .ok()
        .filter(|p| !p.is_empty());
    if svg_path.is_some() {
        assert!(
            !real,
            "FASTF_SHOT_SVG renders the sandbox only — the repository is public"
        );
    }
    let mut env = env;
    // An SVG wants the truecolor palette whatever this terminal announces.
    if svg_path.is_some() {
        env.push(("COLORTERM", std::path::Path::new("truecolor")));
        env.push(("FASTF_THEME", std::path::Path::new("rich")));
    }
    let (chunks, code) =
        pty::run_chunked_sized(cols, rows, common::FASTF, &argv, &env, &script, DEADLINE);
    let screen = screen_at_sized(&chunks, taken, cols, rows);
    if let Some(path) = svg_path {
        let parser = parser_at_sized(&chunks, taken, cols, rows);
        let svg = super::svg::render(parser.screen(), &fastf::tui::theme::Theme::rich());
        let sandbox = sb.tmp.path().display().to_string();
        assert!(
            !svg.contains(&sandbox),
            "the picture must not name the sandbox path"
        );
        fs::write(&path, svg).expect("writing the SVG");
        println!("── wrote {path} ──");
    }

    println!();
    println!("── screen {cols}×{rows} after `{keys}` (exit {code}) ──");

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
