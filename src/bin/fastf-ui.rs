//! Windowless launcher for the browser UI. Not a second server — it links the
//! same `fastf` library and runs the exact `fastf ui --app` flow; it exists so
//! the Windows Start Menu shortcut can open the app window without flashing a
//! console (`windows_subsystem = "windows"` detaches from the console; std
//! silently discards the flow's println! output there).
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    if let Err(error) = run() {
        report(&format!("{error:#}"));
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    // Same startup order as src/main.rs: resolve the data dir first for a
    // readable error, then bootstrap before serving.
    fastf::util::paths::try_install_dir()?;
    fastf::bootstrap::ensure_bootstrapped()?;
    fastf::cli::ui::run(fastf::cli::ui::UiArgs {
        address: fastf::ui::DEFAULT_ADDRESS.to_string(),
        no_open: false,
        app: true,
    })
}

/// With no console attached there is nowhere to print — raise a native
/// message box instead (raw user32 link, no extra dependency).
#[cfg(windows)]
fn report(message: &str) {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(hwnd: isize, text: *const u16, caption: *const u16, utype: u32) -> i32;
    }
    let wide = |s: &str| -> Vec<u16> {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    };
    let text = wide(message);
    let caption = wide("Fast Folder");
    const MB_ICONERROR: u32 = 0x0000_0010;
    unsafe {
        MessageBoxW(0, text.as_ptr(), caption.as_ptr(), MB_ICONERROR);
    }
}

#[cfg(not(windows))]
fn report(message: &str) {
    eprintln!("error: {message}");
}
