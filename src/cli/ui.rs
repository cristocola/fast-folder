//! `fastf ui` — launch the local browser UI.
//!
//! Thin command layer over [`crate::ui`]: decide whether a server is already
//! running, open the browser, and (if not) serve in the foreground. The HTTP
//! server and all API logic live in `crate::ui`; this module only handles
//! process orchestration and browser launching.

use anyhow::Result;
use colored::Colorize;
use std::thread;
use std::time::{Duration, Instant};

use crate::ui;

/// How long closing the app window waits for an in-flight background copy
/// before exiting anyway. Any copy still running past this is recoverable via
/// `fastf reconcile`, so refusing to exit buys nothing and costs a stranded
/// process.
const JOB_DRAIN_TIMEOUT: Duration = Duration::from_secs(60);

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

    // `--app` with a Chromium-family browser ties the server's lifetime to the
    // app window: serve in the background, wait for the window process to
    // exit, then shut down. Closing the window fully stops fastf, so the next
    // launcher click always starts fresh — no lingering background server and
    // no race against a half-closed window. Every other mode (terminal
    // `fastf ui`, `--no-open`, default-browser fallback) serves in the
    // foreground until Ctrl-C, since a browser tab can't be waited on.
    if args.app && !args.no_open {
        let server = {
            let address = args.address.clone();
            thread::spawn(move || ui::serve(&address))
        };
        let mut up = false;
        for _ in 0..100 {
            if server.is_finished() {
                // The server died before answering (e.g. bind failure) —
                // surface its error instead of a vague timeout.
                return match server.join() {
                    Ok(result) => result,
                    Err(_) => anyhow::bail!("UI server thread panicked"),
                };
            }
            if ui::health_check(&args.address) {
                up = true;
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        if !up {
            eprintln!(
                "{} server did not come up in time; open {url} manually",
                "warning:".yellow().bold()
            );
        }
        if let Some(spawned) = open_app_window(&url) {
            match spawned {
                Ok(mut child) => {
                    println!("Close the app window to stop the server (or Ctrl-C).");
                    let _ = child.wait();
                    // Don't strand an in-flight background copy: let it land
                    // before exiting (it would otherwise wait for reconcile).
                    //
                    // Bounded on purpose. This loop used to be unbounded, so a
                    // job that never reached a terminal status kept `fastf.exe`
                    // alive forever after the window closed — and the next
                    // launcher click health-checked successfully against that
                    // zombie and attached a window to a dead server. Giving up
                    // is safe: the provisioning marker means an unfinished copy
                    // is recoverable by `fastf reconcile`, which is exactly the
                    // design's promise.
                    if ui::jobs_active() {
                        println!("Waiting for a background copy to finish…");
                        let deadline = Instant::now() + JOB_DRAIN_TIMEOUT;
                        while ui::jobs_active() {
                            if Instant::now() >= deadline {
                                eprintln!(
                                    "{} a background copy is still running after {}s; exiting anyway — run `fastf reconcile` to finish or roll it back",
                                    "warning:".yellow().bold(),
                                    JOB_DRAIN_TIMEOUT.as_secs()
                                );
                                break;
                            }
                            thread::sleep(Duration::from_millis(500));
                        }
                    }
                    println!("App window closed — Fast Folder UI stopped.");
                    return Ok(());
                }
                Err(error) => eprintln!(
                    "{} could not open app window ({error}); falling back to default browser",
                    "warning:".yellow().bold()
                ),
            }
        }
        // No Chromium (or the app window failed): default browser + serve
        // until Ctrl-C, like the non-app path.
        if let Err(error) = open_default(&url) {
            eprintln!(
                "{} could not open browser ({error}); open {url} manually",
                "warning:".yellow().bold()
            );
        }
        println!("Stop the server with Ctrl-C.");
        return match server.join() {
            Ok(result) => result,
            Err(_) => anyhow::bail!("UI server thread panicked"),
        };
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
            // The window process is intentionally not waited on here — this
            // path runs when the server belongs to another fastf process.
            Ok(_child) => return,
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
/// Chromium-family browser is installed (so the caller falls back). On success
/// hands back the window's [`Child`](std::process::Child) so `run` can tie the
/// server's lifetime to it.
fn open_app_window(url: &str) -> Option<Result<std::process::Child, std::io::Error>> {
    let browser = find_app_browser()?;
    let profile = chromium_profile_dir();
    let spawn = std::process::Command::new(browser)
        .arg(format!("--app={url}"))
        // X11 window-class hints; harmless no-ops on other platforms.
        .arg("--class=FastFolderUI")
        .arg("--name=FastFolderUI")
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("--window-size=1440,940")
        .arg("--no-first-run")
        // Keep the renderer alive while the window sits occluded or idle for
        // hours — deep background throttling is what left long-running app
        // windows frozen with corrupted (white) surfaces on resume.
        .arg("--disable-backgrounding-occluded-windows")
        .arg("--disable-renderer-backgrounding")
        // Chromium's native occlusion tracking on Windows decides a window is
        // hidden when other windows cover it, and can leave the window
        // permanently unresponsive to input after a long stretch covered — the
        // "left it open for hours and it stopped taking clicks" report. Nothing
        // in-page can recover from that: a wedged renderer cannot run the health
        // tick either. Turning the calculation off costs a little idle CPU on a
        // window that is already meant to stay live (see the two flags above).
        .arg("--disable-features=CalculateNativeWinOcclusion")
        .spawn();
    Some(spawn)
}

#[cfg(not(windows))]
fn find_app_browser() -> Option<std::path::PathBuf> {
    [
        "chromium",
        "google-chrome",
        "google-chrome-stable",
        "chromium-browser",
    ]
    .iter()
    .find_map(|name| which(name))
}

/// Chrome before Edge: if the user installed Chrome they chose it; Edge is the
/// always-present fallback on Windows 10/11 and supports the same `--app=` /
/// `--user-data-dir` flags. PATH is probed first (respects custom setups),
/// then the well-known install locations (browsers aren't normally on PATH).
#[cfg(windows)]
fn find_app_browser() -> Option<std::path::PathBuf> {
    for name in ["chrome.exe", "msedge.exe", "chromium.exe"] {
        if let Some(path) = which(name) {
            return Some(path);
        }
    }
    let candidates: [(&str, &str); 5] = [
        ("ProgramFiles", r"Google\Chrome\Application\chrome.exe"),
        ("ProgramFiles(x86)", r"Google\Chrome\Application\chrome.exe"),
        ("LOCALAPPDATA", r"Google\Chrome\Application\chrome.exe"),
        // Edge installs under the x86 Program Files even on 64-bit Windows.
        (
            "ProgramFiles(x86)",
            r"Microsoft\Edge\Application\msedge.exe",
        ),
        ("ProgramFiles", r"Microsoft\Edge\Application\msedge.exe"),
    ];
    candidates.iter().find_map(|(env, rel)| {
        let base = std::env::var_os(env)?;
        let path = std::path::PathBuf::from(base).join(rel);
        path.is_file().then_some(path)
    })
}

#[cfg(windows)]
fn chromium_profile_dir() -> std::path::PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("fast-folder-ui")
        .join("chromium")
}

#[cfg(not(windows))]
fn chromium_profile_dir() -> std::path::PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir)
        .join("fast-folder-ui")
        .join("chromium")
}

fn which(binary: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(binary))
        .find(|candidate| candidate.is_file())
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
