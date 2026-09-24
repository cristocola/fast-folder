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
//! `pgup` `pgdn` `home` `end` `tab` `space` `backspace` `delete` `f1` `f2`
//! `f5`, `ctrl-<letter>` for any control chord (`ctrl-c` `ctrl-s` `ctrl-n`
//! `ctrl-t` `ctrl-u` `ctrl-k` `ctrl-r` `ctrl-z`), `alt-enter`, `wait:<ms>`,
//! `type:<text>` (typed as-is, no Enter), `text:<words>` (the same, `_` typed
//! as a space), `paste:<line>|<line>` (a bracketed
//! paste, `|` between its lines), and any other token is sent as the
//! keys it spells (`q`, `/`, `?`, `c`, `+`, `<`, `>`). The frame is taken after
//! the last token, before the script ends the app.
//!
//! `FASTF_SHOT_SIZE=80x24` runs the app in that window instead of the suite's
//! 120×40, which is how the compact layout is looked at.
//!
//! `FASTF_SHOT_THEME=rich` picks the SVG's palette (default `doom-one`, the
//! app's own default look).
//!
//! `FASTF_SHOT_SVG=docs/img/dashboard.svg` also writes the frame as an SVG in
//! the app's truecolor palette — the README's screenshot, taken from the real
//! binary. Sandbox only: the repository is public.
//!
//! The library is a sandbox of `FASTF_SHOT_PROJECTS` planted projects (eight
//! by default); `FASTF_SHOT_LONG=1` gives every other one a folder name of
//! about ninety characters, the shape a library of client handles and song
//! titles has, where the table's names claim most of the window.
//!
//! `FASTF_SHOT_REAL=1` runs against **your own** library instead — read-only
//! keys only, please. It reads your configuration from a private copy of the
//! data directory, because the app remembers the cursor's row, the sort and
//! whether the pane is open when it exits, and a picture must not change what
//! you see the next time you open it.

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
    let long = std::env::var("FASTF_SHOT_LONG").is_ok_and(|v| v == "1");
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
            "f2" => script.key("\x1bOQ"),
            "f5" => script.key("\x1b[15~"),
            "alt-enter" => script.key("\x1b\r"),
            "ctrl-c" => script.ctrl_c(),
            other => match other.split_once(':') {
                Some(("wait", ms)) => script.pause(ms.parse().unwrap_or(500)),
                Some(("type", text)) => script.key(text),
                // Words with spaces in them: `_` stands for the space.
                Some(("text", text)) => script.key(&text.replace('_', " ")),
                // A bracketed paste, as a terminal sends one: `|` between
                // lines, since a token cannot hold a space or a newline.
                Some(("paste", text)) => {
                    script.key(&format!("\x1b[200~{}\x1b[201~", text.replace('|', "\r")))
                }
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
    // A tool for looking at a named screen: it starts where the keys aim,
    // not behind the one-time guide. `G` still opens it on purpose.
    sb.guide_seen();
    let real_data = sb.tmp.path().join("real-data");
    if real {
        copy_real_data_dir(&real_data);
    } else {
        plant_showcase(&sb, projects, long);
    }
    // A real run keeps the real `HOME`, so a `~` in the configuration still
    // names your folders; only the data directory is the copy.
    let env: Vec<(&str, &std::path::Path)> = if real {
        vec![("FASTF_INSTALL_DIR", real_data.as_path())]
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
    // An SVG is drawn in one palette whatever this terminal announces: the
    // default look unless `FASTF_SHOT_THEME` names another.
    let theme_name = std::env::var("FASTF_SHOT_THEME")
        .ok()
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "doom-one".to_string());
    let theme = match fastf::tui::theme::ThemeChoice::parse(&theme_name) {
        Some(fastf::tui::theme::ThemeChoice::Kind(kind)) => {
            fastf::tui::theme::Theme::from_kind(kind)
        }
        _ => panic!("FASTF_SHOT_THEME is doom-one, rich, ansi or mono, not {theme_name:?}"),
    };
    if svg_path.is_some() {
        env.push(("COLORTERM", std::path::Path::new("truecolor")));
        env.push(("FASTF_THEME", std::path::Path::new(theme.kind.name())));
    }
    let (chunks, code) =
        pty::run_chunked_sized(cols, rows, common::FASTF, &argv, &env, &script, DEADLINE);
    let screen = screen_at_sized(&chunks, taken, cols, rows);
    if let Some(path) = svg_path {
        let parser = parser_at_sized(&chunks, taken, cols, rows);
        let svg = super::svg::render(parser.screen(), &theme);
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

/// Copy the real data directory's configuration, session, counter and
/// templates into `to`, so a `FASTF_SHOT_REAL` run reads your library as it is
/// and writes its session somewhere you never look. The lock and the logs stay
/// behind: the copy takes its own lock, and a picture has nothing to log.
fn copy_real_data_dir(to: &std::path::Path) {
    let (from, _) = fastf::util::paths::try_install_dir().expect("finding the real data directory");
    fs::create_dir_all(to).expect("creating the copy");
    for file in ["config.toml", "state.toml", "counters.toml"] {
        let source = from.join(file);
        if source.is_file() {
            fs::copy(&source, to.join(file)).expect("copying the data directory");
        }
    }
    copy_tree(&from.join("templates"), &to.join("templates"));
    // Past the one-time guide, as the sandbox is.
    let state = to.join("state.toml");
    let mut session = fs::read_to_string(&state).unwrap_or_default();
    if !session.contains("guide_seen") {
        if !session.is_empty() && !session.ends_with('\n') {
            session.push('\n');
        }
        session.push_str("guide_seen = true\n");
        fs::write(&state, session).expect("writing the copy's state.toml");
    }
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    let Ok(entries) = fs::read_dir(from) else {
        return;
    };
    fs::create_dir_all(to).expect("creating a folder in the copy");
    for entry in entries.flatten() {
        let path = entry.path();
        let target = to.join(entry.file_name());
        if path.is_dir() {
            copy_tree(&path, &target);
        } else if path.is_file() {
            fs::copy(&path, &target).expect("copying a template file");
        }
    }
}

/// A library with something in every column: several templates, tags, dates
/// spread across months, distinct project names, and sizes across every
/// magnitude the size column can render.
///
/// **Every name is different and every size is different.** This library is
/// what the README's picture shows, and a column of repeated names or a column
/// of numbers that all read `xxx KB` demonstrates nothing about either column.
/// The payloads are sparse files: `tree_size` reads `metadata.len()`, so a
/// forty gigabyte project costs one `set_len` call and no disk.
///
/// `long` gives every other folder a client handle and a long title, about
/// ninety characters in all, as a library of commissioned work reads.
fn plant_showcase(sb: &Sandbox, n: usize, long: bool) {
    // name, template slug, template display name, size in bytes.
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    struct Row {
        name: &'static str,
        slug: &'static str,
        display: &'static str,
        bytes: u64,
        client: &'static str,
        folders: &'static [&'static str],
    }
    const VIDEO: &[&str] = &["01_Footage", "02_Audio", "03_Project_Files", "04_Export"];
    const CLIENT: &[&str] = &["00_Brief", "01_Working", "02_Delivery"];
    const PHOTO: &[&str] = &["01_RAW", "02_Selects", "03_Retouched", "04_Delivery"];
    const PLAIN: &[&str] = &["00_Inbox", "01_Work"];
    let rows: [Row; 16] = [
        Row {
            name: "Spring_Campaign",
            slug: "client-project",
            display: "Client project",
            bytes: 41 * GB,
            client: "Acme",
            folders: CLIENT,
        },
        Row {
            name: "Documentary_Rough_Cut",
            slug: "video-production",
            display: "Video production",
            bytes: 22 * GB,
            client: "Northwind",
            folders: VIDEO,
        },
        Row {
            name: "Lullaby_Remix",
            slug: "music-video",
            display: "Music video",
            bytes: 12 * GB,
            client: "Aria_Vance",
            folders: VIDEO,
        },
        Row {
            name: "Wedding_Highlights",
            slug: "photography",
            display: "Photography",
            bytes: 6 * GB + 400 * MB,
            client: "Bell",
            folders: PHOTO,
        },
        Row {
            name: "Product_Launch_Reel",
            slug: "video-production",
            display: "Video production",
            bytes: 3 * GB + 200 * MB,
            client: "Acme",
            folders: VIDEO,
        },
        Row {
            name: "Live_Session",
            slug: "music-video",
            display: "Music video",
            bytes: GB + 300 * MB,
            client: "Aria_Vance",
            folders: VIDEO,
        },
        Row {
            name: "Autumn_Lookbook",
            slug: "photography",
            display: "Photography",
            bytes: 880 * MB,
            client: "Meridian",
            folders: PHOTO,
        },
        Row {
            name: "Album_Artwork",
            slug: "music-video",
            display: "Music video",
            bytes: 340 * MB,
            client: "Aria_Vance",
            folders: PLAIN,
        },
        Row {
            name: "Client_Onboarding_Acme",
            slug: "client-project",
            display: "Client project",
            bytes: 96 * MB,
            client: "Acme",
            folders: CLIENT,
        },
        Row {
            name: "Motion_Graphics_Pack",
            slug: "general",
            display: "General",
            bytes: 47 * MB,
            client: "Meridian",
            folders: PLAIN,
        },
        Row {
            name: "Interview_Series",
            slug: "video-production",
            display: "Video production",
            bytes: 12 * MB,
            client: "Northwind",
            folders: VIDEO,
        },
        Row {
            name: "Portfolio_Site",
            slug: "general",
            display: "General",
            bytes: 4 * MB + 200 * KB,
            client: "Meridian",
            folders: PLAIN,
        },
        Row {
            name: "Studio_Website",
            slug: "general",
            display: "General",
            bytes: 760 * KB,
            client: "Bell",
            folders: PLAIN,
        },
        Row {
            name: "Brand_Refresh",
            slug: "client-project",
            display: "Client project",
            bytes: 220 * KB,
            client: "Northwind",
            folders: CLIENT,
        },
        Row {
            name: "Podcast_Intro",
            slug: "general",
            display: "General",
            bytes: 48 * KB,
            client: "Bell",
            folders: PLAIN,
        },
        Row {
            name: "Archive_Restoration",
            slug: "photography",
            display: "Photography",
            bytes: 3 * KB,
            client: "Meridian",
            folders: PHOTO,
        },
    ];
    // **Two bases**, because one of the things worth showing is that a library
    // can span drives: a working one and an archive. The sandbox's own base is
    // called `base`, which says nothing, so the showcase configures its own.
    let projects = sb.tmp.path().join("projects");
    let archive = sb.tmp.path().join("archive");
    fs::create_dir_all(&projects).unwrap();
    fs::create_dir_all(&archive).unwrap();
    sb.ok(&["config", "set", "base-dir", &projects.display().to_string()]);
    sb.ok(&["config", "set", "bases", &archive.display().to_string()]);

    for i in 0..n {
        let row = &rows[i % rows.len()];
        // Ascending with the row, so an id sort reads as the order the
        // projects were made in and a date sort reads as something else.
        let id = format!("ID{:04}", 201 + i);
        let month = 1 + (i % 9) as u32;
        let day = 2 + (i % 26) as u32;
        let folder = if long && i % 2 == 0 {
            format!(
                "2026-{month:02}-{day:02}_{}_studio_{}-Extended_Directors_Cut_Alternate_Endings_Final_{id}",
                row.client.to_lowercase(),
                row.name
            )
        } else {
            format!("2026-{month:02}-{day:02}_{}_{id}", row.name)
        };
        let base = if i % 4 == 1 { &archive } else { &projects };
        let root = sb.plant_project(base, &folder, &id);
        for sub in row.folders {
            fs::create_dir_all(root.join(sub)).unwrap();
        }
        let payload =
            fs::File::create(root.join(row.folders[0]).join("payload.bin")).expect("the payload");
        payload.set_len(row.bytes).expect("a sparse payload");
        drop(payload);
        // A file at the top level as well as folders, so the detail pane's
        // "inside" reads like a project and not like a directory listing.
        let note = match row.slug {
            "client-project" => "BRIEF.md",
            "photography" => "SHOTLIST.md",
            "general" => "NOTES.md",
            _ => "TREATMENT.md",
        };
        fs::write(root.join(note), "#\n").unwrap();

        let pinfo = root.join("PROJECT_INFO.md");
        let raw = fs::read_to_string(&pinfo).unwrap();
        let tags = match i % 3 {
            0 => format!("tags:\n  - draft\n  - client/{}", row.client),
            1 => format!("tags:\n  - client/{}", row.client),
            _ => format!("tags:\n  - delivered\n  - client/{}", row.client),
        };
        let title = row.name.replace('_', " ");
        let raw = raw
            .replace("template: general", &format!("template: {}", row.slug))
            .replace(
                "template_name: General",
                &format!("template_name: {}", row.display),
            )
            .replace(
                "created: 2026-01-01T00:00:00Z",
                &format!("created: 2026-{month:02}-{day:02}T10:00:00Z"),
            )
            .replace(
                "variables: {}",
                &format!(
                    "variables:\n  client: {}\n  project: {title}\n  year: '2026'",
                    row.client
                ),
            )
            .replace("tags: []", &tags);
        // The first project carries notes and todos, so the pane shows what a
        // project's record looks like: a note of more than one line, and a
        // list in two phases, the first of them finished.
        let raw = if i == 0 {
            raw.replace(
                "## Notes\n",
                "## Notes\n\n\
                 - 2026-01-05T09:12:00Z — brief signed off, shoot booked\n\
                 - 2026-01-16T18:40:00Z — first cut sent to Acme\n  \
                   hold the logo two seconds longer\n  \
                   and a quieter music bed\n\n\
                 ## Todo\n\n\
                 ### Shoot\n\n\
                 - [x] shoot the product close-ups\n\
                 - [x] rough cut\n\n\
                 ### Deliver\n\n\
                 - [ ] colour and sound mix\n\
                 - [ ] deliver the 16:9 and 9:16 masters\n",
            )
        } else {
            raw
        };
        fs::write(&pinfo, raw).unwrap();
    }
    // The index the header reads before discovery answers.
    sb.ok(&["reindex"]);
}
