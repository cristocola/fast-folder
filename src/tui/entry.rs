//! How the app was opened, and what it was handed.

use crate::core::library::Project;

/// The filters `fastf recent` was given. Shown as a chip before the search
/// query and applied on top of it, so the list means what the flags said.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preset {
    pub template: Option<String>,
    pub since: Option<String>,
    pub tag: Option<String>,
    pub limit: Option<usize>,
}

impl Preset {
    pub fn is_empty(&self) -> bool {
        self.template.is_none()
            && self.since.is_none()
            && self.tag.is_none()
            && self.limit.is_none()
    }

    /// The chip text: `recent: template=music-video since=2026-01-01 limit=20`.
    pub fn label(&self) -> String {
        let mut parts = Vec::new();
        if let Some(t) = &self.template {
            parts.push(format!("template={t}"));
        }
        if let Some(s) = &self.since {
            parts.push(format!("since={s}"));
        }
        if let Some(t) = &self.tag {
            parts.push(format!("tag={t}"));
        }
        if let Some(n) = self.limit {
            parts.push(format!("limit={n}"));
        }
        format!("recent: {}", parts.join(" "))
    }

    pub fn keeps(&self, project: &Project) -> bool {
        if let Some(slug) = &self.template
            && &project.template != slug
        {
            return false;
        }
        if let Some(since) = &self.since
            && project.created.as_str() < since.as_str()
        {
            return false;
        }
        if let Some(tag) = &self.tag
            && !project.tags.iter().any(|t| t == tag)
        {
            return false;
        }
        true
    }
}

/// The three doors into the one app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// `fastf` with no arguments.
    Menu,
    /// `fastf recent …`: the flags as a preset, the rows already read.
    Recent {
        preset: Preset,
        initial: Vec<Project>,
    },
    /// `fastf search …`: the terms in the search bar, the matches already read.
    Search {
        terms: Vec<String>,
        initial: Vec<Project>,
    },
}

impl Entry {
    /// Whether leaving prints `Goodbye.` — only the menu ever did.
    pub fn is_menu(&self) -> bool {
        matches!(self, Entry::Menu)
    }
}
