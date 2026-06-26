//! `fastf ui` — launch the local browser UI.
//!
//! Thin command layer over [`crate::ui`]: decide whether a server is already
//! running, open the browser, and (if not) serve in the foreground. The HTTP
//! server and all API logic live in `crate::ui`; this module only handles
//! process orchestration and browser launching.

use anyhow::Result;
use colored::Colorize;
use std::thread;
use std::time::Duration;

use crate::ui;

pub struct UiArgs {
    pub address: String,
    pub no_open: bool,
    /// Open a dedicated Chromium/Chrome app window instead of the default browser.
    pub app: bool,
}

pub fn run(args: UiArgs) -> Result<()> {
    let url = format!("http://{}", args.address);

    // A server is already answering on this address — just open the browser
    // (this makes `fastf ui` and the desktop launcher idempotent).
    if ui::health_check(&args.address) {
        println!("Fast Folder UI already running on {url}");
        if !args.no_open {
            open_browser(&url, args.app);
        }
        return Ok(());
    }

    // Open the browser once the server is up. Serve blocks, so the opener runs
    // on a background thread that waits for the health check to pass.
    if !args.no_open {
        let address = args.address.clone();
        let url = url.clone();
        let app = args.app;
        thread::spawn(move || {
            for _ in 0..100 {
                if ui::health_check(&address) {
                    open_browser(&url, app);
                    return;
                }
                thread::sleep(Duration::from_millis(50));
            }
            eprintln!(
                "{} server did not come up in time; open {url} manually",
                "warning:".yellow().bold()
            );
        });
    }

    println!("Stop the server with Ctrl-C.");
    ui::serve(&args.address)
}

/// Open `url`, preferring a dedicated app window when `app` is set and a
/// Chromium-family browser is available. Failures are reported but never fatal —
/// the server keeps running and the user can open the URL manually.
fn open_browser(url: &str, app: bool) {
    if app && let Some(result) = open_app_window(url) {
        match result {
            Ok(()) => return,
            Err(error) => eprintln!(
                "{} could not open app window ({error}); falling back to default browser",
                "warning:".yellow().bold()
            ),
        }
    }
    if let Err(error) = open_default(url) {
        eprintln!(
            "{} could not open browser ({error}); open {url} manually",
            "warning:".yellow().bold()
        );
    }
}

/// Launch a Chromium/Chrome app window with a dedicated profile, mirroring the
/// old launcher's polished window experience. Returns `None` when no
/// Chromium-family browser is installed (so the caller falls back).
fn open_app_window(url: &str) -> Option<Result<(), std::io::Error>> {
    let profile = chromium_profile_dir();
    for browser in [
        "chromium",
        "google-chrome",
        "google-chrome-stable",
        "chromium-browser",
    ] {
        if which(browser) {
            let spawn = std::process::Command::new(browser)
                .arg(format!("--app={url}"))
                .arg("--class=FastFolderUI")
                .arg("--name=FastFolderUI")
                .arg(format!("--user-data-dir={}", profile.display()))
                .arg("--window-size=1440,940")
                .arg("--no-first-run")
                .spawn()
                .map(|_| ());
            return Some(spawn);
        }
    }
    None
}

fn chromium_profile_dir() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache")))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    base.join("fast-folder-ui").join("chromium")
}

fn which(binary: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(binary).is_file())
}

#[cfg(target_os = "windows")]
fn open_default(url: &str) -> Result<(), std::io::Error> {
    std::process::Command::new("cmd")
        .args(["/c", "start", "", url])
        .spawn()
        .map(|_| ())
}

#[cfg(target_os = "macos")]
fn open_default(url: &str) -> Result<(), std::io::Error> {
    std::process::Command::new("open")
        .arg(url)
        .spawn()
        .map(|_| ())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_default(url: &str) -> Result<(), std::io::Error> {
    std::process::Command::new("xdg-open")
        .arg(url)
        .spawn()
        .map(|_| ())
}
