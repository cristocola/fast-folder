//! The Anthropic API key: where Tenax keeps it, and who may use which one.
//!
//! **The key is not configuration.** `config.toml` is printed by `config show`,
//! echoed by `config set` into the status line and both logs, and diffed and
//! committed by people who keep their dotfiles in git. The key lives in its own
//! file beside it, `tenax.key`, written owner-only (`atomic::write_private`),
//! and nothing but this module and the request that carries it ever holds the
//! text: the app is told a [`KeySource`], never a [`Secret`].
//!
//! **An environment key is not consent.** `ANTHROPIC_API_KEY` is set for many
//! tools, so it answers only [`Ask::Interactive`] — a person who pressed the
//! key to talk to Tenax. The brief fastf asks for on its own needs a key saved
//! in fastf, or upgrading would read and send a library nobody offered.

use std::fmt;
use std::path::PathBuf;

use anyhow::{Context, Result};

/// The file beside `config.toml` that holds the key, one line.
pub const KEY_FILE: &str = "tenax.key";
/// The variable every Anthropic SDK reads.
pub const ENV_KEY: &str = "ANTHROPIC_API_KEY";

/// The key's text. `Debug` never shows it, so an `Action` or `Effect` that
/// carries one can be printed by a failing test or a log line.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// A key as typed or pasted: trimmed, one printable ASCII word. Anything
    /// else is not a key — a paste that caught a newline or a second word.
    pub fn new(text: &str) -> Option<Self> {
        let text = text.trim();
        let printable = text.bytes().all(|b| b.is_ascii_graphic());
        (!text.is_empty() && text.len() <= MAX_LEN && printable).then(|| Self(text.to_string()))
    }

    /// The text, for the one header that sends it.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// The last four characters, which is how a key is recognised on a
    /// dashboard without being shown.
    pub fn last4(&self) -> String {
        let start = self.0.len().saturating_sub(4);
        self.0[start..].to_string()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(redacted)")
    }
}

/// Longer than any key the API issues; a paste longer than this is not one.
const MAX_LEN: usize = 512;

/// Where the key Tenax would use comes from — all the app is ever told.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum KeySource {
    /// None anywhere.
    #[default]
    None,
    /// Saved in fastf, with its last four characters.
    Stored { last4: String },
    /// Only `ANTHROPIC_API_KEY`, which answers a person who asked.
    Env,
}

impl KeySource {
    /// Whether a person who asks can be answered.
    pub fn any(&self) -> bool {
        !matches!(self, KeySource::None)
    }

    /// Whether fastf may ask on its own (the brief).
    pub fn stored(&self) -> bool {
        matches!(self, KeySource::Stored { .. })
    }
}

/// Who is asking for the key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    /// fastf, on its own: the stored key only.
    Automatic,
    /// A person who pressed a key: the stored key, else the environment's.
    Interactive,
}

/// `<data dir>/tenax.key`.
pub fn key_path() -> PathBuf {
    crate::util::paths::install_dir().join(KEY_FILE)
}

/// Save `secret`, replacing any saved before.
pub fn store(secret: &Secret) -> Result<()> {
    let path = key_path();
    crate::util::atomic::write_private(&path, format!("{}\n", secret.expose()))
        .context("saving the Tenax key")
}

/// Forget the saved key. `Ok(false)` when there was none.
pub fn clear() -> Result<bool> {
    let path = key_path();
    match crate::util::fs_retry::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err).context("removing the saved Tenax key"),
    }
}

/// The saved key, if one is saved and reads as one.
pub fn stored() -> Option<Secret> {
    let text = std::fs::read_to_string(key_path()).ok()?;
    Secret::new(&text)
}

fn from_env() -> Option<Secret> {
    std::env::var(ENV_KEY)
        .ok()
        .and_then(|text| Secret::new(&text))
}

/// Where the key would come from, for the app and `config show`.
pub fn source() -> KeySource {
    if let Some(secret) = stored() {
        return KeySource::Stored {
            last4: secret.last4(),
        };
    }
    if from_env().is_some() {
        return KeySource::Env;
    }
    KeySource::None
}

/// The key to send, for whoever is asking.
pub fn resolve(ask: Ask) -> Option<Secret> {
    stored().or_else(|| match ask {
        Ask::Interactive => from_env(),
        Ask::Automatic => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_env::EnvGuard;

    #[test]
    fn a_key_never_prints() {
        let secret = Secret::new("  test-key-0000-wxyz \n").unwrap();
        assert_eq!(secret.expose(), "test-key-0000-wxyz");
        assert_eq!(format!("{secret:?}"), "Secret(redacted)");
        assert_eq!(secret.last4(), "wxyz");
        assert!(!format!("{:?}", Some(secret)).contains("wxyz"));
    }

    #[test]
    fn a_paste_that_is_not_one_word_is_not_a_key() {
        assert!(Secret::new("").is_none());
        assert!(Secret::new("   ").is_none());
        assert!(Secret::new("two words").is_none());
        assert!(Secret::new("line\nbreak").is_none());
        assert!(Secret::new(&"x".repeat(MAX_LEN + 1)).is_none());
        assert!(Secret::new("ключ").is_none());
    }

    /// The environment's key answers a person who asked, never fastf on its
    /// own; a saved key answers both, and wins over the environment's.
    #[test]
    fn the_environment_key_answers_only_a_person() {
        let (mut guard, _dir) = EnvGuard::sandbox();
        guard.also_set(ENV_KEY, "env-key-1234");
        assert_eq!(source(), KeySource::Env);
        assert_eq!(resolve(Ask::Automatic), None);
        assert_eq!(resolve(Ask::Interactive).unwrap().expose(), "env-key-1234");

        store(&Secret::new("saved-key-9876").unwrap()).unwrap();
        assert_eq!(
            source(),
            KeySource::Stored {
                last4: "9876".into()
            }
        );
        assert_eq!(resolve(Ask::Automatic).unwrap().expose(), "saved-key-9876");
        assert_eq!(
            resolve(Ask::Interactive).unwrap().expose(),
            "saved-key-9876"
        );

        assert!(clear().unwrap());
        assert!(!clear().unwrap());
        assert_eq!(source(), KeySource::Env);
    }

    #[cfg(unix)]
    #[test]
    fn the_saved_key_is_its_owners_alone() {
        use std::os::unix::fs::PermissionsExt;
        let (mut guard, _dir) = EnvGuard::sandbox();
        guard.also_remove(ENV_KEY);
        assert_eq!(source(), KeySource::None);
        store(&Secret::new("saved-key-5555").unwrap()).unwrap();
        let mode = std::fs::metadata(key_path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
