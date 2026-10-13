//! The desk: every project as one line, grouped by who its next open todo is
//! assigned to. `fastf desk` prints it and the app's desk tab shows it.
//!
//! The **next todo** is the project's first open todo in file order. The
//! **assignee** is who that todo is assigned to: the `@owner` at the end of
//! its line, nobody when it has none, and no open todo at all when every todo
//! is done. Each project is one line under its assignee, with how long since
//! anything in the project was last written. Nothing here is inferred: the
//! owner is on the todo's line, the age is the file's and the folder's own
//! modification time, and "active" means the operating system reports a
//! program whose working folder is inside the project.
//!
//! Pure: this module reads nothing and prints nothing. The command line and
//! the app read the todos, the process table and the times and hand them in,
//! so both group the same way.

use std::cmp::Ordering;

use crate::core::body::Todo;
use crate::core::library::Project;

/// A project's first open todo, and where it sits in the file's list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NextTodo {
    pub todo: Todo,
    /// Its position among the file's todos, from zero. `fastf todo done`
    /// numbers the same list from one.
    pub index: usize,
}

/// Who a project's next open todo is assigned to.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Assignee {
    /// The owner the todo names, without its `@`.
    Owner(String),
    /// The next open todo has no owner.
    NoOwner,
    /// Every todo is done, or there are none.
    NoOpenTodo,
}

impl Assignee {
    /// The heading a group is listed under: `yours` for the hat, `@name` for
    /// any other owner, `no owner`, `no open todos`.
    pub fn label(&self, hat: Option<&str>) -> String {
        match self {
            Assignee::Owner(owner) if hat == Some(owner.as_str()) => "yours".to_string(),
            Assignee::Owner(owner) => format!("@{owner}"),
            Assignee::NoOwner => "no owner".to_string(),
            Assignee::NoOpenTodo => "no open todos".to_string(),
        }
    }

    /// The word `--json` writes in the `group` field.
    pub fn kind(&self) -> &'static str {
        match self {
            Assignee::Owner(_) => "owner",
            Assignee::NoOwner => "no-owner",
            Assignee::NoOpenTodo => "no-open-todos",
        }
    }

    /// The owner, when there is one.
    pub fn owner(&self) -> Option<&str> {
        match self {
            Assignee::Owner(owner) => Some(owner),
            _ => None,
        }
    }

    /// The text the app's session file and its search bar use for one
    /// assignee: `@claude`, `no owner`, `no open todos`.
    pub fn key(&self) -> String {
        match self {
            Assignee::Owner(owner) => format!("@{owner}"),
            Assignee::NoOwner => "no owner".to_string(),
            Assignee::NoOpenTodo => "no open todos".to_string(),
        }
    }

    /// The assignee `key` names, or `None` when it names none.
    pub fn parse_key(key: &str) -> Option<Assignee> {
        let key = key.trim();
        if let Some(owner) = key.strip_prefix('@') {
            return (!owner.is_empty()).then(|| Assignee::Owner(owner.to_string()));
        }
        match key {
            "no owner" | "no-owner" => Some(Assignee::NoOwner),
            "no open todos" | "no-open-todos" => Some(Assignee::NoOpenTodo),
            _ => None,
        }
    }
}

/// One project on the desk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    pub project: Project,
    pub next_todo: Option<NextTodo>,
    /// How many todos are open, and how many done.
    pub open: usize,
    pub done: usize,
    /// When anything in the project was last written, as a timestamp: the
    /// newer of `PROJECT_INFO.md` and the folder, or the newest file under
    /// the folder when a walk has found one. `None` where nothing could be
    /// read.
    pub quiet_since: Option<String>,
    /// Programs the operating system reports as working in the folder, by
    /// name, each once.
    pub active: Vec<String>,
}

impl Card {
    /// A card from what the callers read: the project, its todos in file
    /// order, who is working in it, and when it was last written.
    pub fn of(
        project: Project,
        todos: &[Todo],
        active: Vec<String>,
        quiet_since: Option<String>,
    ) -> Self {
        let next_todo =
            todos
                .iter()
                .enumerate()
                .find(|(_, todo)| !todo.done)
                .map(|(index, todo)| NextTodo {
                    todo: todo.clone(),
                    index,
                });
        Self {
            project,
            next_todo,
            open: todos.iter().filter(|todo| !todo.done).count(),
            done: todos.iter().filter(|todo| todo.done).count(),
            quiet_since,
            active,
        }
    }

    /// Who the next open todo is assigned to.
    pub fn assignee(&self) -> Assignee {
        match &self.next_todo {
            None => Assignee::NoOpenTodo,
            Some(next) => match &next.todo.owner {
                Some(owner) => Assignee::Owner(owner.clone()),
                None => Assignee::NoOwner,
            },
        }
    }

    /// The next todo as one line: `Post · colour grade`, the phase before
    /// the text when it has one; empty when there is no open todo.
    pub fn next_todo_line(&self, sep: &str) -> String {
        match &self.next_todo {
            None => String::new(),
            Some(next) => match &next.todo.phase {
                Some(phase) => format!("{phase}{sep}{}", next.todo.text),
                None => next.todo.text.clone(),
            },
        }
    }
}

/// How the cards under one assignee are ordered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Order {
    /// The project nothing has written in for the longest comes first. A
    /// project with no known time comes last.
    #[default]
    Quiet,
    /// The most recently written project comes first.
    Recent,
    /// By folder name.
    Name,
    /// By id number, highest first.
    Id,
}

impl Order {
    pub const ALL: [Order; 4] = [Order::Quiet, Order::Recent, Order::Name, Order::Id];

    pub fn label(self) -> &'static str {
        match self {
            Order::Quiet => "quiet",
            Order::Recent => "recent",
            Order::Name => "name",
            Order::Id => "id",
        }
    }

    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|order| order.label().eq_ignore_ascii_case(word.trim()))
    }

    /// What the order means, for a picker's second column.
    pub fn detail(self) -> &'static str {
        match self {
            Order::Quiet => "the project nothing has written in for the longest first",
            Order::Recent => "the most recently written project first",
            Order::Name => "by folder name",
            Order::Id => "the highest ID first",
        }
    }

    /// The next order round, for a key that cycles them.
    pub fn next(self) -> Self {
        let at = Self::ALL.iter().position(|o| *o == self).unwrap_or(0);
        Self::ALL[(at + 1) % Self::ALL.len()]
    }

    fn compare(self, a: &Card, b: &Card) -> Ordering {
        match self {
            Order::Quiet => match (&a.quiet_since, &b.quiet_since) {
                (Some(x), Some(y)) => x.cmp(y),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            },
            Order::Recent => Order::Quiet.compare(b, a),
            Order::Name => a.project.name.cmp(&b.project.name),
            Order::Id => b.project.number().cmp(&a.project.number()),
        }
        // The tie-break never reverses, or groups of equals would shuffle.
        .then_with(|| a.project.id.cmp(&b.project.id))
        .then_with(|| a.project.path.cmp(&b.project.path))
    }
}

/// The cards under one assignee, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub assignee: Assignee,
    pub cards: Vec<Card>,
}

/// Every card under its assignee, the groups in the order the desk shows
/// them: the hat's own group first when a hat is set, then the todos with no
/// owner (first of all when no hat is set), then the other owners by how
/// many projects each has, then the projects with no open todo. A group
/// with no card is not listed.
pub fn group(cards: Vec<Card>, hat: Option<&str>, order: Order) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for card in cards {
        let assignee = card.assignee();
        match groups.iter_mut().find(|group| group.assignee == assignee) {
            Some(group) => group.cards.push(card),
            None => groups.push(Group {
                assignee,
                cards: vec![card],
            }),
        }
    }
    for group in &mut groups {
        group.cards.sort_by(|a, b| order.compare(a, b));
    }
    let rank = |group: &Group| -> (u8, usize, String) {
        match &group.assignee {
            Assignee::Owner(owner) if hat == Some(owner.as_str()) => (0, 0, String::new()),
            Assignee::NoOwner => (1, 0, String::new()),
            Assignee::Owner(owner) => (2, usize::MAX - group.cards.len(), owner.clone()),
            Assignee::NoOpenTodo => (3, 0, String::new()),
        }
    };
    groups.sort_by_key(rank);
    groups
}

/// How long since `since`, as a person reads it beside a line: `20 min`,
/// `6 h`, `12 d`, `3 mo`. `now` is handed in so a test can fix it, and a
/// timestamp that will not parse reads as its first ten characters, the day.
pub fn age_label(since: &str, now: &str) -> String {
    let (Ok(since), Ok(now)) = (
        chrono::DateTime::parse_from_rfc3339(since),
        chrono::DateTime::parse_from_rfc3339(now),
    ) else {
        return since.get(..10).unwrap_or(since).to_string();
    };
    let minutes = (now - since).num_minutes().max(0);
    if minutes < 60 {
        format!("{minutes} min")
    } else if minutes < 60 * 24 {
        format!("{} h", minutes / 60)
    } else if minutes < 60 * 24 * 60 {
        format!("{} d", minutes / (60 * 24))
    } else {
        format!("{} mo", minutes / (60 * 24 * 30))
    }
}

/// Whether `since` is at least `days` days before `now`: the `--quiet`
/// filter. A card with no known time does not count as quiet.
pub fn quiet_for_days(since: Option<&str>, now: &str, days: u64) -> bool {
    let (Some(since), Ok(now)) = (
        since.and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()),
        chrono::DateTime::parse_from_rfc3339(now),
    ) else {
        return false;
    };
    (now - since).num_days().max(0) as u64 >= days
}

#[cfg(test)]
mod tests {
    /// An assignee written down reads back as itself, and a word that names
    /// none is nothing rather than a guess.
    #[test]
    fn an_assignee_round_trips_through_its_key() {
        use super::Assignee;
        for assignee in [
            Assignee::Owner("claude".to_string()),
            Assignee::NoOwner,
            Assignee::NoOpenTodo,
        ] {
            assert_eq!(Assignee::parse_key(&assignee.key()), Some(assignee));
        }
        assert_eq!(Assignee::parse_key("@"), None);
        assert_eq!(Assignee::parse_key("somewhere"), None);
    }

    use super::*;
    use std::path::PathBuf;

    fn project(id: &str, name: &str) -> Project {
        Project {
            id: id.to_string(),
            id_number: crate::core::naming::id_value(id),
            template: "t".to_string(),
            template_name: "T".to_string(),
            name: name.to_string(),
            path: PathBuf::from("/mnt/projects").join(name),
            base: PathBuf::from("/mnt/projects"),
            created: "2026-01-01T00:00:00Z".to_string(),
            description: String::new(),
            tags: Vec::new(),
            exists: true,
        }
    }

    fn todo(done: bool, text: &str, owner: Option<&str>) -> Todo {
        Todo {
            done,
            text: text.to_string(),
            owner: owner.map(str::to_string),
            ..Default::default()
        }
    }

    fn card(id: &str, todos: &[Todo], quiet: Option<&str>) -> Card {
        Card::of(
            project(id, &format!("P_{id}")),
            todos,
            Vec::new(),
            quiet.map(str::to_string),
        )
    }

    /// The next todo is the first open todo in file order, whatever is
    /// ticked above it; a list with every todo done has no open todo.
    #[test]
    fn the_next_todo_is_the_first_open_todo() {
        let c = card(
            "ID0001",
            &[
                todo(true, "shoot", Some("sales")),
                todo(false, "grade", Some("claude")),
                todo(false, "deliver", None),
            ],
            None,
        );
        assert_eq!(
            c.next_todo
                .as_ref()
                .map(|n| (n.index, n.todo.text.as_str())),
            Some((1, "grade"))
        );
        assert_eq!(c.assignee(), Assignee::Owner("claude".to_string()));
        assert_eq!((c.open, c.done), (2, 1));
        let c = card("ID0002", &[todo(true, "shoot", None)], None);
        assert_eq!(c.assignee(), Assignee::NoOpenTodo);
        assert_eq!(c.next_todo_line(" · "), "");
        let c = card("ID0003", &[todo(false, "deliver", None)], None);
        assert_eq!(c.assignee(), Assignee::NoOwner);
    }

    /// The hat's group first when a hat is set, then the todos with no owner,
    /// then the other owners by how many projects each has, then the projects
    /// with no open todo; without a hat the todos with no owner come first.
    /// Within a group the project written in longest ago comes first, and a
    /// card with no known time last.
    #[test]
    fn groups_are_ordered_by_assignee() {
        let cards = vec![
            card(
                "ID0001",
                &[todo(false, "a", Some("claude"))],
                Some("2026-10-10T00:00:00Z"),
            ),
            card(
                "ID0002",
                &[todo(false, "b", None)],
                Some("2026-10-01T00:00:00Z"),
            ),
            card("ID0003", &[todo(false, "c", Some("sales"))], None),
            card(
                "ID0004",
                &[todo(false, "d", Some("claude"))],
                Some("2026-10-02T00:00:00Z"),
            ),
            card(
                "ID0005",
                &[todo(true, "e", None)],
                Some("2026-09-01T00:00:00Z"),
            ),
            card(
                "ID0006",
                &[todo(false, "f", Some("acme"))],
                Some("2026-10-05T00:00:00Z"),
            ),
        ];
        let groups = group(cards.clone(), Some("sales"), Order::Quiet);
        let labels: Vec<String> = groups
            .iter()
            .map(|g| g.assignee.label(Some("sales")))
            .collect();
        assert_eq!(
            labels,
            ["yours", "no owner", "@claude", "@acme", "no open todos"]
        );
        let claude: Vec<&str> = groups[2]
            .cards
            .iter()
            .map(|c| c.project.id.as_str())
            .collect();
        assert_eq!(claude, ["ID0004", "ID0001"], "written in longest ago first");

        let groups = group(cards, None, Order::Quiet);
        let labels: Vec<String> = groups.iter().map(|g| g.assignee.label(None)).collect();
        assert_eq!(
            labels,
            ["no owner", "@claude", "@acme", "@sales", "no open todos"]
        );
        assert_eq!(Order::Quiet.next(), Order::Recent);
        assert_eq!(Order::parse("Name"), Some(Order::Name));
    }

    #[test]
    fn an_age_reads_as_a_person_says_it() {
        let now = "2026-10-13T12:00:00Z";
        assert_eq!(age_label("2026-10-13T11:40:00Z", now), "20 min");
        assert_eq!(age_label("2026-10-13T06:00:00Z", now), "6 h");
        assert_eq!(age_label("2026-10-01T12:00:00Z", now), "12 d");
        assert_eq!(age_label("2026-07-01T12:00:00Z", now), "3 mo");
        assert_eq!(
            age_label("2026-10-15T00:00:00Z", now),
            "0 min",
            "a clock ahead of now"
        );
        assert_eq!(age_label("yesterday", now), "yesterday");
        assert!(quiet_for_days(Some("2026-10-01T12:00:00Z"), now, 7));
        assert!(!quiet_for_days(Some("2026-10-12T12:00:00Z"), now, 7));
        assert!(!quiet_for_days(None, now, 0));
    }
}
