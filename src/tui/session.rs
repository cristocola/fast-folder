//! What the app remembers between runs: the sort order, whether the detail
//! pane was open, the row the cursor was on, whether the template guide has
//! been shown, whether the editor's explanation panel is open, and which
//! bases the list shows — every one, or only those not marked inactive.
//!
//! A few keystrokes' worth, kept in `state.toml` beside `config.toml` — the
//! data directory is the one place that is this machine's own — and never
//! anything a project holds. It is read once before the first frame and written
//! after the screen is given back — and the moment the bases' view changes
//! (`Effect::SaveSession`); `update` never touches it. A file that
//! is missing, unreadable or garbage starts the app with the defaults and says
//! so once, because a lost convenience is not an error.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::tui::app::App;
use crate::tui::app::library::{BasesView, Order, Sort};
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
    /// Whether the template guide has ever been shown. It offers itself once,
    /// unasked — the first time somebody reaches the templates tab or opens the
    /// editor, whichever happens first — and one flag for both doors is what
    /// keeps it from appearing twice on one afternoon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guide_seen: Option<bool>,
    /// Whether the editor's explanation panel is open. On until it is turned
    /// off, which is the opposite default from every other pane here and
    /// deliberate: somebody meeting the template editor has more to gain from
    /// the panel than from the width.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explain_open: Option<bool>,
    /// Which bases the list shows (`BasesView::label`); absent is every base.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bases_view: Option<String>,
    /// The bases marked inactive, as the configuration spells them: the name
    /// a base has whether or not it is mounted when the app starts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inactive_bases: Vec<String>,
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
    /// the selection; the pane's state and the bases' view are everyone's.
    pub fn capture(app: &App, previous: &Session) -> Self {
        let mut session = previous.clone();
        session.detail_open = Some(app.detail_open);
        session.explain_open = Some(app.explain_open);
        session.bases_view =
            (app.library.view == BasesView::Active).then(|| BasesView::Active.label().to_string());
        session.inactive_bases = app.inactive_to_remember();
        if app.guide_seen {
            session.guide_seen = Some(true);
        }
        if app.is_menu {
            session.sort = app.library.explicit_sort.map(|sort| sort.label());
            session.selected = app.library.selected().map(|project| project.id.clone());
        }
        session
    }

    /// The bases' view this session names; anything it cannot read is every
    /// base.
    pub fn bases_view(&self) -> BasesView {
        self.bases_view
            .as_deref()
            .map(BasesView::from_label)
            .unwrap_or_default()
    }

    /// The sort order this session names, if it names a real one. `newest` is
    /// the default and reads as no explicit choice, so a query still sorts by
    /// relevance after a restart, exactly as it does before `s` was ever
    /// pressed.
    pub fn sort_order(&self) -> Option<Sort> {
        self.sort
            .as_deref()
            .and_then(Sort::from_label)
            .filter(|sort| sort.order != Order::Newest || sort.reversed)
    }
}

#[cfg(test)]
mod tests {
    use super::Session;
    use crate::tui::app::library::{BasesView, Order, Sort};
    use crate::util::test_env::EnvGuard;

    #[test]
    fn a_session_round_trips_through_toml() {
        let session = Session {
            sort: Some("name".to_string()),
            detail_open: Some(false),
            selected: Some("ID0240".to_string()),
            guide_seen: Some(true),
            explain_open: Some(false),
            bases_view: Some("active".to_string()),
            inactive_bases: vec!["/mnt/projects/archive".to_string()],
        };
        let text = toml::to_string(&session).unwrap();
        assert_eq!(Session::parse(&text).unwrap(), session);
        assert_eq!(session.sort_order(), Some(Sort::new(Order::Name)));
        assert_eq!(Session::parse("").unwrap(), Session::default());
        // A key from a later version is not a reason to forget the rest.
        let newer = Session::parse("sort = \"size\"\nfuture = 1\n").unwrap();
        assert_eq!(newer.sort_order(), Some(Sort::new(Order::Size)));
    }

    /// **A label written before there was a direction still names an order**,
    /// and one written with a direction reads back with it. `state.toml` is a
    /// convenience, but a convenience that silently forgets the order you
    /// chose is worse than none.
    #[test]
    fn a_direction_round_trips_and_the_old_spelling_still_parses() {
        let reversed = Session {
            sort: Some(
                Sort {
                    order: Order::Size,
                    reversed: true,
                }
                .label(),
            ),
            ..Session::default()
        };
        assert_eq!(reversed.sort, Some("size reversed".to_string()));
        assert_eq!(
            reversed.sort_order(),
            Some(Sort {
                order: Order::Size,
                reversed: true
            })
        );
        // An order with only one direction cannot be reversed into one.
        let odd = Session {
            sort: Some("newest reversed".to_string()),
            ..Session::default()
        };
        assert_eq!(odd.sort_order(), None);
    }

    /// **The bases' view reads back as it was left, and anything else is
    /// every base**: a word this version does not know must not hide a base,
    /// and a file from before there was a view still loads, showing every
    /// base, as it did then.
    #[test]
    fn the_bases_view_round_trips_and_anything_else_is_every_base() {
        let active = Session::parse(
            "bases_view = \"active\"\ninactive_bases = [\"/mnt/projects/archive\"]\n",
        )
        .unwrap();
        assert_eq!(active.bases_view(), BasesView::Active);
        assert_eq!(active.inactive_bases, vec!["/mnt/projects/archive"]);
        for word in ["every", "Active ", "sideways", ""] {
            let session = Session {
                bases_view: Some(word.to_string()),
                ..Session::default()
            };
            let expected = if word.trim().eq_ignore_ascii_case("active") {
                BasesView::Active
            } else {
                BasesView::Every
            };
            assert_eq!(session.bases_view(), expected, "{word:?}");
        }
        let older = Session::parse("sort = \"id\"\ndetail_open = true\n").unwrap();
        assert_eq!(older.bases_view(), BasesView::Every);
        assert!(older.inactive_bases.is_empty());
        // Nothing is written for the default, so a file that never chose
        // stays as short as it was.
        let text = toml::to_string(&Session::default()).unwrap();
        assert!(!text.contains("bases"), "{text}");
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
            guide_seen: None,
            explain_open: Some(true),
            ..Session::default()
        };
        session.save_to(&path).unwrap();
        assert_eq!(Session::load_from(&path), session);
        assert_eq!(super::path(), path, "it lives beside the config");
    }
}
