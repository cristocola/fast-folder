//! The build script. It exists for one link argument.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // Windows gives a program's main thread 1 MB of stack unless the exe's
    // header asks for more; Linux gives 8. clap builds the whole command tree
    // in one derived function whose frame, unoptimized, is over 1 MB
    // (measured: a debug build needs between 1 and 1.5 MB to start, a
    // release build under 128 KB), so a debug `fastf.exe` overflows before
    // it has read its arguments. The header asks for what Linux gives, and
    // one stack budget holds on both. For the binary alone: a test
    // executable runs its cases on threads of their own.
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if os == "windows" {
        let stack = if env == "msvc" {
            "/STACK:8388608"
        } else {
            "-Wl,--stack,8388608"
        };
        println!("cargo:rustc-link-arg-bin=fastf={stack}");
    }
}
