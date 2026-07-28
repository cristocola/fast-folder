use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::core::config::Config;
use crate::util::paths;

/// The counter's high-water mark as kept **inside a base**, as
/// `.fastf-counter.toml` next to that base's `.fastf-index.json`.
///
/// This exists because of where the number has to be *readable* from. The
/// counter used to live only in the data directory — `%APPDATA%\fastf` on
/// Windows, `~/.config/fastf` on Linux — so a dual-boot machine had two of them
/// and no way to keep them in step. The only workaround was to symlink one home
/// into the other, which breaks the moment either is encrypted. The projects
/// never had that problem: they already sit on a drive both systems mount, so
/// the number that indexes them sits there too.
///
/// It does **not** replace the data-directory counter — see [`Counters::load`].
/// The two cover different failures, and both are written on every create.
pub const BASE_COUNTER_FILE: &str = ".fastf-counter.toml";

/// Single global counter shared across all templates.
/// The file contains one line: `global = 47`
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Counters {
    #[serde(default)]
    pub global: u64,
}

impl Counters {
    /// Read this machine's counter from the data directory.
    ///
    /// Still written, and still needed, even though the base file is the shared
    /// record — the two cover different failures:
    ///
    /// - the **base** file is visible to every OS that mounts the drive, which
    ///   is what removed the symlink;
    /// - this one spans **every base the machine has ever written to**, which is
    ///   what survives a base being unplugged.
    ///
    /// Without it, working in an archive base up to ID0005, unplugging it, then
    /// creating in another base restarts at ID0001 — and plugging the archive
    /// back in gives two projects the same ID. Keeping it also means upgrading
    /// needs no migration step.
    pub fn load() -> Result<Self> {
        let path = paths::counters_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let c: Self =
            toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        Ok(c)
    }

    /// Persist this machine's counter atomically. A truncated file would reset
    /// ID allocation, so the write must never be observable half-done.
    pub fn save(&self) -> Result<()> {
        let path = paths::counters_path();
        let raw = toml::to_string_pretty(self).context("serializing counters")?;
        crate::util::atomic::write(&path, raw)
            .with_context(|| format!("writing {}", path.display()))
    }

    /// Record `value` in both places a create must update: the base (shared
    /// between operating systems) and the data directory (spans every base this
    /// machine has seen). Best-effort — [`Counters::floor`] also reads the
    /// highest ID present in the projects themselves, so a failed write costs
    /// tidiness, not correctness.
    ///
    /// Only the target base is written, never every mounted one: creating a file
    /// in a directory changes its mtime, which would invalidate that base's
    /// `.fastf-index.json` staleness gate and force a full rescan. The target
    /// base's cache is being rewritten by this create anyway.
    pub fn record(base: &Path, value: u64) {
        if let Err(err) = Self::save_base(base, value) {
            eprintln!(
                "warning: could not record the ID counter in {} ({err})",
                base.display()
            );
        }
        let mut local = Self::load().unwrap_or_default();
        if value > local.get() {
            local.set_value(value);
            let _ = local.save();
        }
    }

    /// Where a base keeps its counter.
    pub fn base_path(base: &Path) -> PathBuf {
        base.join(BASE_COUNTER_FILE)
    }

    /// One base's recorded high-water mark. A missing, unreadable or malformed
    /// file reads as `0` — the same "degrade, never panic" rule the index cache
    /// follows, and safe because [`Counters::floor`] also consults the projects
    /// actually on disk.
    pub fn load_base(base: &Path) -> u64 {
        let path = Self::base_path(base);
        fs::read_to_string(&path)
            .ok()
            .and_then(|raw| toml::from_str::<Self>(&raw).ok())
            .map(|c| c.global)
            .unwrap_or(0)
    }

    /// Record `value` in `base`, but only ever upward.
    ///
    /// Monotonic on purpose: two machines writing the same base must not be able
    /// to walk the number backwards, and a base that has seen higher IDs than
    /// this create knows about keeps its mark.
    pub fn save_base(base: &Path, value: u64) -> Result<()> {
        if Self::load_base(base) >= value {
            return Ok(());
        }
        let path = Self::base_path(base);
        let raw =
            toml::to_string_pretty(&Self { global: value }).context("serializing counters")?;
        crate::util::atomic::write(&path, raw)
            .with_context(|| format!("writing {}", path.display()))
    }

    /// The highest value recorded by any configured base.
    pub fn base_floor(cfg: &Config) -> u64 {
        cfg.effective_bases()
            .iter()
            .filter(|base| base.is_dir())
            .map(|base| Self::load_base(base))
            .max()
            .unwrap_or(0)
    }

    /// The authoritative floor for the next ID: the highest value seen anywhere.
    ///
    /// Three inputs, because each covers a hole the others leave:
    /// - every mounted base's counter file — shared across operating systems;
    /// - this machine's data-directory counter — spans every base it has
    ///   written to, so an unplugged drive cannot restart numbering;
    /// - the highest ID actually present in project metadata — so a deleted or
    ///   hand-edited counter file can never hand out an ID that already exists.
    ///
    /// The last one is why losing a counter file is untidy rather than harmful.
    pub fn floor(cfg: &Config) -> u64 {
        let local = Self::load().map(|c| c.get()).unwrap_or(0);
        local
            .max(Self::base_floor(cfg))
            .max(crate::core::library::max_id(cfg))
    }

    /// Current global counter value (last used ID).
    pub fn get(&self) -> u64 {
        self.global
    }

    /// Set the global counter to a specific value.
    pub fn set_value(&mut self, value: u64) {
        self.global = value;
    }

    /// Reset the global counter to 0.
    pub fn reset(&mut self) {
        self.global = 0;
    }

    /// Format a counter value: prefix + zero-padded number.
    pub fn format_id(prefix: &str, digits: usize, value: u64) -> String {
        format!("{}{:0>width$}", prefix, value, width = digits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_base_with_no_counter_file_reads_as_zero() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(Counters::load_base(tmp.path()), 0);
    }

    #[test]
    fn a_corrupt_counter_file_reads_as_zero_rather_than_failing() {
        let tmp = tempfile::tempdir().unwrap();
        for garbage in ["", "global = ", "not toml at all", "global = -1"] {
            fs::write(Counters::base_path(tmp.path()), garbage).unwrap();
            assert_eq!(
                Counters::load_base(tmp.path()),
                0,
                "counter {garbage:?} should degrade to 0, not panic"
            );
        }
    }

    #[test]
    fn saving_a_base_counter_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        Counters::save_base(tmp.path(), 47).unwrap();
        assert_eq!(Counters::load_base(tmp.path()), 47);
    }

    /// Two machines share a base. Neither may drag the number backwards.
    #[test]
    fn a_base_counter_only_ever_moves_up() {
        let tmp = tempfile::tempdir().unwrap();
        Counters::save_base(tmp.path(), 47).unwrap();
        Counters::save_base(tmp.path(), 12).unwrap();
        assert_eq!(
            Counters::load_base(tmp.path()),
            47,
            "a lower write must not lower the mark"
        );
        Counters::save_base(tmp.path(), 48).unwrap();
        assert_eq!(Counters::load_base(tmp.path()), 48);
    }
}
