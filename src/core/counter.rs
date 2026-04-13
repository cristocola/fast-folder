use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;

use crate::util::paths;

const GLOBAL_KEY: &str = "global";

/// Single global counter shared across all templates.
/// counters.toml contains one line: global = 47
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Counters {
    #[serde(default)]
    pub global: u64,
}

impl Counters {
    pub fn load() -> Result<Self> {
        let path = paths::counters_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let c: Self = toml::from_str(&raw)
            .with_context(|| format!("parsing {}", path.display()))?;
        Ok(c)
    }

    pub fn save(&self) -> Result<()> {
        let path = paths::counters_path();
        let raw = toml::to_string_pretty(self)
            .context("serializing counters")?;
        fs::write(&path, raw)
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    /// Current global counter value (last used ID).
    pub fn get(&self) -> u64 {
        self.global
    }

    /// Increment the global counter and return the NEW value.
    pub fn increment(&mut self) -> u64 {
        self.global += 1;
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
