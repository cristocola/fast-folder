//! The status line and the message log: what the app says, and the activity
//! screen that keeps it.

use super::*;

impl App {
    fn set_status(&mut self, level: StatusLevel, text: impl Into<String>) {
        let text = text.into();
        self.log.push_back(LogEntry {
            at: (self.clock)(),
            level,
            text: text.clone(),
        });
        while self.log.len() > LOG_CAP {
            self.log.pop_front();
        }
        self.outbox.push(crate::util::messages::Message {
            at: String::new(),
            level: match level {
                StatusLevel::Info => crate::util::messages::Level::Info,
                StatusLevel::Good => crate::util::messages::Level::Good,
                StatusLevel::Warn => crate::util::messages::Level::Warn,
                StatusLevel::Error => crate::util::messages::Level::Error,
            },
            source: "app".to_string(),
            text: text.clone(),
        });
        // A warning under a full-height dialog is a warning nobody saw.
        if matches!(level, StatusLevel::Warn | StatusLevel::Error) && !self.modals.is_empty() {
            self.unseen_warnings += 1;
        }
        self.status = Status {
            text,
            level,
            expires_at: Some(self.elapsed_ms + STATUS_MS),
            shown_at: Some(self.elapsed_ms),
        };
    }

    /// `L`: the activity screen — every session's messages, and the log.
    ///
    /// It goes up at once with this session's messages, from memory, and
    /// fills in from the data directory when the read lands; the runtime
    /// reads again while it stays open, so a move running in another fastf
    /// shows up in it.
    pub(super) fn open_log(&mut self) -> Vec<Effect> {
        self.unseen_warnings = 0;
        let g = self.theme.glyphs;
        let messages = if self.log.is_empty() {
            vec!["No messages yet.".to_string()]
        } else {
            self.log
                .iter()
                .rev()
                .map(|entry| {
                    let mark = match entry.level {
                        StatusLevel::Warn => format!("{} ", g.warn),
                        StatusLevel::Error => format!("{} ", g.cross),
                        StatusLevel::Good => format!("{} ", g.check),
                        StatusLevel::Info => String::new(),
                    };
                    format!("{}  {mark}{}", entry.at, entry.text)
                })
                .collect()
        };
        self.modals.push(Modal::Activity(Box::new(Activity {
            page: ActivityPage::Messages,
            messages,
            log: vec!["reading…".to_string()],
            jobs: Vec::new(),
            job_ids: Vec::new(),
            job_cursor: 0,
            scroll: [0, 0, 0],
            loaded: false,
        })));
        self.refresh_activity_jobs();
        vec![Effect::LoadActivity, Effect::WatchJobs]
    }

    /// Put the jobs the app knows of on the activity screen's jobs page, if
    /// it is open, keeping the cursor on the job it was on.
    pub(super) fn refresh_activity_jobs(&mut self) {
        let (rows, ids) = modal::job_rows(&self.background.jobs);
        if let Some(Modal::Activity(activity)) = self.modals.top_mut() {
            let was = activity.job_at_cursor().map(str::to_string);
            activity.jobs = rows;
            activity.job_cursor = was
                .and_then(|was| ids.iter().position(|id| *id == was))
                .unwrap_or(0);
            activity.job_ids = ids;
        }
    }

    /// Keys on the activity screen: on the jobs page the arrows move the
    /// cursor and Enter opens that job's own log; elsewhere it reads like any
    /// message.
    pub(super) fn on_activity_key(&mut self, key: Key) -> Vec<Effect> {
        let on_jobs = matches!(
            self.modals.top(),
            Some(Modal::Activity(activity)) if activity.page == ActivityPage::Jobs
        );
        if on_jobs && key == Key::plain(KeyCode::Enter) {
            let Some(Modal::Activity(activity)) = self.modals.top() else {
                return Vec::new();
            };
            let Some(id) = activity.job_at_cursor().map(str::to_string) else {
                return Vec::new();
            };
            let title = format!(
                "log · {}",
                self.background
                    .job(&id)
                    .map(|job| job.title())
                    .unwrap_or_else(|| id.clone())
            );
            self.modals.push(Modal::message(
                title.clone(),
                "reading…",
                MessageLevel::Info,
            ));
            return vec![Effect::LoadJobLog { id, title }];
        }
        self.on_scroll_modal_key(key)
    }

    /// Whether the activity screen is on top, for the runtime to keep it
    /// fresh.
    pub fn activity_open(&self) -> bool {
        matches!(self.modals.top(), Some(Modal::Activity(_)))
    }

    pub(super) fn info(&mut self, text: impl Into<String>) {
        self.set_status(StatusLevel::Info, text);
    }

    /// A success, with the theme's own tick in front of it.
    ///
    /// **The glyph belongs here and not in the message.** Twelve of
    /// `runtime::run_action`'s strings carried a literal `✓`, which
    /// `Glyphs::ascii` maps to `+` — so on a legacy Windows console, or under
    /// `FASTF_ASCII=1`, they drew a replacement box beside the app's own
    /// correctly-themed messages. `run_action` runs on a worker with no theme
    /// to ask, and this is the one place every one of its messages passes
    /// through.
    pub(super) fn good(&mut self, text: impl Into<String>) {
        let text = format!("{}  {}", self.theme.glyphs.check, text.into());
        self.set_status(StatusLevel::Good, text);
    }

    pub(super) fn warn(&mut self, text: impl Into<String>) {
        self.set_status(StatusLevel::Warn, text);
    }

    pub(super) fn error(&mut self, text: impl Into<String>) {
        self.set_status(StatusLevel::Error, text);
    }
}
