//! The desk: every project as one line, grouped by whose court its ball is
//! in — what `fastf desk` prints and the app's desk tab shows.
//!
//! **The ball** is a project's first open todo in file order; **the court**
//! is who holds it: the todo's `@owner`, nobody when it has none, and no
//! court at all when nothing is planned. Grouped under their courts, with how
//! long each project has been quiet, N projects are N lines — never every
//! todo. Nothing here is inferred: the owner is on the line, the age is the
//! file's and the folder's own time, and "active" is a program the operating
//! system says is working in the folder.
//!
//! Pure: this module reads nothing and prints nothing. The command line and
//! the app read the todos, the holders and the times and hand them in, so
//! both group the same way.

use std::cmp::Ordering;

use crate::core::body::Todo;
use crate::core::library::Project;

/// A project's first open todo, and where it sits in the file's list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ball {
    pub todo: Todo,
    /// Its ordinal among the file's todos — what `fastf todo done` numbers
    /// from one, so a reader can act on it.
    pub index: usize,
}

/// Whose court a project's ball is in.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Court {
    /// Somebody's: the owner the ball names, without its `@`.
    Owner(String),
    /// A ball nobody was named for — the one on top's, until handed off.
    Unowned,
    /// No open todo at all: add one, or close the project.
    NothingPlanned,
}

impl Court {
    /// The heading a group wears: `your court` for the hat worn, `with
    /// @claude` for anybody else's, and the two piles by name.
    pub fn label(&self, hat: Option<&str>) -> String {
        match self {
            Court::Owner(owner) if hat == Some(owner.as_str()) => "your court".to_string(),
            Court::Owner(owner) => format!("with @{owner}"),
            Court::Unowned => "unowned".to_string(),
            Court::NothingPlanned => "nothing planned".to_string(),
        }
    }

    /// The word `--json` says for the kind of court.
    pub fn kind(&self) -> &'static str {
        match self {
            Court::Owner(_) => "owner",
            Court::Unowned => "unowned",
            Court::NothingPlanned => "nothing-planned",
        }
    }

    /// The owner, for a court that has one.
    pub fn owner(&self) -> Option<&str> {
        match self {
            Court::Owner(owner) => Some(owner),
            _ => None,
        }
    }
}

/// One project on the desk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    pub project: Project,
    pub ball: Option<Ball>,
    /// How many todos are open, and how many done.
    pub open: usize,
    pub done: usize,
    /// When anything last wrote in the project, as a stamp: `touched`, or
    /// the newest file under the folder when a walk has found it. `None`
    /// where nothing could be read.
    pub quiet_since: Option<String>,
    /// Programs the operating system says are working in the folder, by
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
        let ball = todos
            .iter()
            .enumerate()
            .find(|(_, todo)| !todo.done)
            .map(|(index, todo)| Ball {
                todo: todo.clone(),
                index,
            });
        Self {
            project,
            ball,
            open: todos.iter().filter(|todo| !todo.done).count(),
            done: todos.iter().filter(|todo| todo.done).count(),
            quiet_since,
            active,
        }
    }

    /// Whose court the ball is in.
    pub fn court(&self) -> Court {
        match &self.ball {
            None => Court::NothingPlanned,
            Some(ball) => match &ball.todo.owner {
                Some(owner) => Court::Owner(owner.clone()),
                None => Court::Unowned,
            },
        }
    }

    /// The ball as one line: `Post · colour grade`, the phase before the
    /// text when it has one; empty with nothing planned.
    pub fn ball_line(&self, sep: &str) -> String {
        match &self.ball {
            None => String::new(),
            Some(ball) => match &ball.todo.phase {
                Some(phase) => format!("{phase}{sep}{}", ball.todo.text),
                None => ball.todo.text.clone(),
            },
        }
    }
}

/// How the cards under one court are ordered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Order {
    /// Stalest first: the longest since anything wrote in the project, which
    /// is what makes a decision. A project with no time at all comes last.
    #[default]
    Quiet,
    /// Most recently written first.
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

/// The cards under one court, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub court: Court,
    pub cards: Vec<Card>,
}

/// Every card under its court, the courts in the order the desk shows
/// them: the hat's own first when one is worn, then `unowned` — the one on
/// top's pile until handed off, and first of all when no hat is worn — then
/// the other owners by how many projects each holds, then what has nothing
/// planned. A court with no card is not listed.
pub fn group(cards: Vec<Card>, hat: Option<&str>, order: Order) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for card in cards {
        let court = card.court();
        match groups.iter_mut().find(|group| group.court == court) {
            Some(group) => group.cards.push(card),
            None => groups.push(Group {
                court,
                cards: vec![card],
            }),
        }
    }
    for group in &mut groups {
        group.cards.sort_by(|a, b| order.compare(a, b));
    }
    let rank = |group: &Group| -> (u8, usize, String) {
        match &group.court {
            Court::Owner(owner) if hat == Some(owner.as_str()) => (0, 0, String::new()),
            Court::Unowned => (1, 0, String::new()),
            Court::Owner(owner) => (2, usize::MAX - group.cards.len(), owner.clone()),
            Court::NothingPlanned => (3, 0, String::new()),
        }
    };
    groups.sort_by_key(rank);
    groups
}

/// How long since `since`, as a person reads it beside a line: `20 min`,
/// `6 h`, `12 d`, `3 mo`; `now` is handed in so a test can fix it, and a
/// stamp that will not parse reads as its first ten characters, the day.
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
/// cut. A card with no time at all is not quiet; it is unknown.
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

    /// The ball is the first open todo in file order, whatever is ticked
    /// above it; a list with every todo done has nothing planned.
    #[test]
    fn the_ball_is_the_first_open_todo() {
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
            c.ball.as_ref().map(|b| (b.index, b.todo.text.as_str())),
            Some((1, "grade"))
        );
        assert_eq!(c.court(), Court::Owner("claude".to_string()));
        assert_eq!((c.open, c.done), (2, 1));
        let c = card("ID0002", &[todo(true, "shoot", None)], None);
        assert_eq!(c.court(), Court::NothingPlanned);
        assert_eq!(c.ball_line(" · "), "");
        let c = card("ID0003", &[todo(false, "deliver", None)], None);
        assert_eq!(c.court(), Court::Unowned);
    }

    /// Yours first when a hat is worn, then unowned, then the other owners
    /// by how many projects each holds, then nothing planned; without a
    /// hat the unowned pile leads. Within a court the stalest comes first,
    /// and a card with no time at all last.
    #[test]
    fn courts_are_ordered_for_the_one_on_top() {
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
            .map(|g| g.court.label(Some("sales")))
            .collect();
        assert_eq!(
            labels,
            [
                "your court",
                "unowned",
                "with @claude",
                "with @acme",
                "nothing planned"
            ]
        );
        let claude: Vec<&str> = groups[2]
            .cards
            .iter()
            .map(|c| c.project.id.as_str())
            .collect();
        assert_eq!(claude, ["ID0004", "ID0001"], "stalest first");

        let groups = group(cards, None, Order::Quiet);
        let labels: Vec<String> = groups.iter().map(|g| g.court.label(None)).collect();
        assert_eq!(
            labels,
            [
                "unowned",
                "with @claude",
                "with @acme",
                "with @sales",
                "nothing planned"
            ]
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
