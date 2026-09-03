//! What the app remembers between runs: the sort order, whether the detail
//! pane was open, and the row the cursor was on.
//!
//! A few keystrokes' worth, kept in `state.toml` beside `config.toml` — the
//! data directory is the one place that is this machine's own — and never
//! anything a project holds. It is read once before the first frame and written
//! once after the screen is given back; `update` never touches it. A file that
//! is missing, unreadable or garbage starts the app with the defaults and says
//! so once, because a lost convenience is not an error.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::tui::app::App;
use crate::tui::app::library::Order;
use crate::util::paths::display_path;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    /// The sort order chosen with `s`/`S`, by its label; absent means the
    /// default (newest, or relevance while the query has bare words).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail_open: Option<bool>,
    /// The id of the project the cursor was on. An id, not a path: a rename
    /// or a move between runs must not lose it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<String>,
}

/// `state.toml` in the data directory.
pub fn path() -> PathBuf {
    crate::util::paths::install_dir().join("state.toml")
}

impl Session {
    /// Read the file, or the defaults when there is none or it cannot be read
    /// — with a note in the second case, so a file that went wrong is noticed
    /// without stopping anything.
    pub fn load() -> Self {
        Self::load_from(&path())
    }

    fn load_from(path: &std::path::Path) -> Self {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(err) => {
                crate::util::diag::note(format!(
                    "{} could not be read — starting with the defaults: {err}",
                    display_path(path)
                ));
                return Self::default();
            }
        };
        match Self::parse(&text) {
            Ok(session) => session,
            Err(err) => {
                crate::util::diag::note(format!(
                    "{} could not be read — starting with the defaults: {err:#}",
                    display_path(path)
                ));
                Self::default()
            }
        }
    }

    pub fn parse(text: &str) -> Result<Self> {
        toml::from_str(text).context("parsing the session state")
    }

    /// Write the file atomically. Best effort at the call site: a session that
    /// cannot be remembered is reported, not fatal.
    pub fn save(&self) -> Result<()> {
        self.save_to(&path())
    }

    fn save_to(&self, path: &std::path::Path) -> Result<()> {
        let text = toml::to_string(self).context("encoding the session state")?;
        crate::util::atomic::write(path, text)
            .with_context(|| format!("writing {}", display_path(path)))
    }

    /// What this run leaves behind. `fastf recent`/`search` own their order
    /// and their rows, so only the guided app (`fastf`) updates the sort and
    /// the selection; the pane's state is everyone's.
    pub fn capture(app: &App, previous: &Session) -> Self {
        let mut session = previous.clone();
        session.detail_open = Some(app.detail_open);
        if app.is_menu {
            session.sort = app
                .library
                .explicit_sort
                .map(|order| order.label().to_string());
            session.selected = app.library.selected().map(|project| project.id.clone());
        }
        session
    }

    /// The sort order this session names, if it names a real one. `newest` is
    /// the default and reads as no explicit choice, so a query still sorts by
    /// relevance after a restart, exactly as it does before `s` was ever
    /// pressed.
    pub fn sort_order(&self) -> Option<Order> {
        self.sort
            .as_deref()
            .and_then(Order::from_label)
            .filter(|order| *order != Order::Newest)
    }
}

#[cfg(test)]
mod tests {
    use super::Session;
    use crate::tui::app::library::Order;
    use crate::util::test_env::EnvGuard;

    #[test]
    fn a_session_round_trips_through_toml() {
        let session = Session {
            sort: Some("name".to_string()),
            detail_open: Some(false),
            selected: Some("ID0240".to_string()),
        };
        let text = toml::to_string(&session).unwrap();
        assert_eq!(Session::parse(&text).unwrap(), session);
        assert_eq!(session.sort_order(), Some(Order::Name));
        assert_eq!(Session::parse("").unwrap(), Session::default());
        // A key from a later version is not a reason to forget the rest.
        let newer = Session::parse("sort = \"size\"\nfuture = 1\n").unwrap();
        assert_eq!(newer.sort_order(), Some(Order::Size));
    }

    #[test]
    fn newest_and_nonsense_read_as_no_explicit_sort() {
        let newest = Session {
            sort: Some("newest".to_string()),
            ..Session::default()
        };
        assert_eq!(newest.sort_order(), None);
        let nonsense = Session {
            sort: Some("sideways".to_string()),
            ..Session::default()
        };
        assert_eq!(nonsense.sort_order(), None);
        assert!(Session::parse("sort = [1, 2]").is_err());
    }

    #[test]
    fn a_missing_or_broken_file_starts_with_the_defaults_and_a_good_one_is_kept() {
        let (_guard, dir) = EnvGuard::sandbox();
        let path = dir.path().join("state.toml");
        assert_eq!(Session::load_from(&path), Session::default());
        std::fs::write(&path, "this is not toml = = =").unwrap();
        assert_eq!(Session::load_from(&path), Session::default());
        let session = Session {
            sort: Some("id".to_string()),
            detail_open: Some(true),
            selected: None,
        };
        session.save_to(&path).unwrap();
        assert_eq!(Session::load_from(&path), session);
        assert_eq!(super::path(), path, "it lives beside the config");
    }
}
