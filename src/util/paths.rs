use std::path::PathBuf;

/// Returns the directory where the fastf binary lives.
/// All config, templates, and counters are resolved relative to this path.
/// Uses canonicalize() so symlinks resolve to the real binary location.
pub fn install_dir() -> PathBuf {
    std::env::current_exe()
        .expect("cannot resolve binary path")
        .canonicalize()
        .expect("cannot canonicalize binary path")
        .parent()
        .expect("binary has no parent directory")
        .to_path_buf()
}

pub fn config_path() -> PathBuf {
    install_dir().join("config.toml")
}

pub fn counters_path() -> PathBuf {
    install_dir().join("counters.toml")
}

pub fn templates_dir() -> PathBuf {
    install_dir().join("templates")
}
