use std::path::PathBuf;

/// Returns the directory where the fastf binary lives.
/// All config, templates, and counters are resolved relative to this path.
/// Uses canonicalize() so symlinks resolve to the real binary location.
///
/// Tests can override the install directory by setting `FASTF_INSTALL_DIR`.
/// This is the single escape hatch that lets integration tests run hermetically
/// against a temp directory without spawning a real fastf binary.
pub fn install_dir() -> PathBuf {
    if let Ok(override_dir) = std::env::var("FASTF_INSTALL_DIR")
        && !override_dir.is_empty()
    {
        return PathBuf::from(override_dir);
    }
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

pub fn projects_index_path() -> PathBuf {
    install_dir().join("projects.jsonl")
}
