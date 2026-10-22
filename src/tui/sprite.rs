//! Tenax's face: the small figure at the left of his box, and what it does.
//!
//! **Pure, like `motion.rs`**: no clock, no environment, no I/O. Every
//! function takes the milliseconds it reasons about, so `update` starts an
//! animation and a test asserts the frame it makes at a chosen moment.
//!
//! **He moves when he speaks, and only then** — a reply or a brief landing
//! plays one animation for [`PLAY_MS`] and lets go; a request in flight loops
//! [`Animation::Think`] until it answers. At rest, with `motion` off and on a
//! mono theme (so in every snapshot) he is [`rest`], and an idle app asks for
//! no wake on his account.
//!
//! **Which animation is a fact about the projects, picked at random among the
//! ones that fit** ([`choose`]): the deadpan look at the camera when a todo has
//! sat stuck, typing while someone works in a project, a little celebration
//! when something was ticked today, juggling a long list, dozing over a quiet
//! library, an alert when something needs a person. Two plain ones — a wave
//! and a nod — are always in the draw.
//!
//! Every frame is [`WIDTH`] display columns by [`HEIGHT`] rows, in both
//! alphabets, so the text beside him never moves.

/// The figure's size, in display columns and rows.
pub const WIDTH: u16 = 7;
pub const HEIGHT: u16 = 4;
/// One frame lasts this long.
pub const FRAME_MS: u64 = 200;
/// How long one animation plays when he speaks: twelve frames.
pub const PLAY_MS: u64 = 12 * FRAME_MS;

/// What he can do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Animation {
    Wave,
    Nod,
    /// The flat look at the camera: a todo has sat with its owner for days.
    Look,
    /// Somebody is working in a project.
    Typing,
    /// Something was ticked today.
    Celebrate,
    /// A long list of open todos.
    Juggle,
    /// Nothing has moved anywhere for a week.
    Doze,
    /// Something needs a person.
    Alert,
    /// A request is in flight. Loops for as long as it is.
    Think,
}

/// What [`choose`] weighs. Counted in `update` from what the app already
/// holds; nothing is read for it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    /// Todos given to someone in projects quiet for `STUCK_AFTER_DAYS`.
    pub stuck: usize,
    /// Projects a program is working in.
    pub working: usize,
    /// Todos ticked today.
    pub done_today: usize,
    /// Open todos across the library.
    pub open_todos: usize,
    /// Every project quiet for a week or more.
    pub all_quiet: bool,
    /// Unfinished work that needs a person.
    pub needs_you: usize,
}

/// A long list, for [`Animation::Juggle`].
const MANY_TODOS: usize = 10;

/// One animation, at random among the ones the facts call for — each fact
/// that holds weighs three, the two plain ones one each. `seed` is anything
/// that changes between calls (the clock and a count, as `DeskRandom`'s is),
/// so a test that fixes it gets the same answer every time.
pub fn choose(facts: &Facts, seed: u64) -> Animation {
    let mut draw: Vec<(Animation, u64)> = vec![(Animation::Wave, 1), (Animation::Nod, 1)];
    let mut fits = |holds: bool, animation: Animation| {
        if holds {
            draw.push((animation, 3));
        }
    };
    fits(facts.stuck > 0, Animation::Look);
    fits(facts.working > 0, Animation::Typing);
    fits(facts.done_today > 0, Animation::Celebrate);
    fits(facts.open_todos >= MANY_TODOS, Animation::Juggle);
    fits(facts.all_quiet, Animation::Doze);
    fits(facts.needs_you > 0, Animation::Alert);
    let total: u64 = draw.iter().map(|(_, weight)| weight).sum();
    let mut ticket = mix(seed) % total;
    for (animation, weight) in draw {
        if ticket < weight {
            return animation;
        }
        ticket -= weight;
    }
    Animation::Wave
}

/// A seed's bits, spread: consecutive clocks would otherwise pick neighbours.
fn mix(seed: u64) -> u64 {
    let mut x = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Whether an animation started at `started` is still playing at `now`.
/// `Think` plays for as long as it is asked to.
pub fn playing(animation: Animation, started: u64, now: u64) -> bool {
    animation == Animation::Think || now.saturating_sub(started) < PLAY_MS
}

/// One frame: four rows of [`WIDTH`] columns.
pub type Frame = [&'static str; 4];

/// The figure at rest.
pub fn rest(ascii: bool) -> Frame {
    art(ascii).rest
}

/// The frame `animation`, started at `started`, shows at `now` — the rest
/// frame once a one-shot has played out.
pub fn frame(animation: Animation, started: u64, now: u64, ascii: bool) -> Frame {
    let art = art(ascii);
    let step = (now.saturating_sub(started) / FRAME_MS) as usize;
    let (frames, script) = art.of(animation);
    if animation != Animation::Think && step >= script.len() {
        return art.rest;
    }
    frames[script[step % script.len()] as usize]
}

struct Art {
    rest: Frame,
    wave: &'static [Frame],
    nod: &'static [Frame],
    look: &'static [Frame],
    typing: &'static [Frame],
    celebrate: &'static [Frame],
    juggle: &'static [Frame],
    doze: &'static [Frame],
    alert: &'static [Frame],
    think: &'static [Frame],
}

// Which frame each of the twelve steps shows; `Think`'s loops.
const WAVE: &[u8] = &[1, 2, 1, 2, 1, 2, 0, 0, 3, 0, 0, 0];
const NOD: &[u8] = &[0, 1, 0, 0, 1, 2, 2, 2, 0, 0, 0, 0];
const LOOK: &[u8] = &[0, 0, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0];
const TYPING: &[u8] = &[0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 2, 2];
const CELEBRATE: &[u8] = &[0, 1, 2, 1, 2, 1, 2, 1, 2, 0, 0, 0];
const JUGGLE: &[u8] = &[0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3];
const DOZE: &[u8] = &[0, 1, 2, 3, 1, 2, 3, 1, 2, 3, 0, 0];
const ALERT: &[u8] = &[0, 1, 0, 1, 0, 1, 0, 1, 2, 2, 2, 2];
const THINK: &[u8] = &[0, 1, 2, 3];

impl Art {
    fn of(&self, animation: Animation) -> (&'static [Frame], &'static [u8]) {
        match animation {
            Animation::Wave => (self.wave, WAVE),
            Animation::Nod => (self.nod, NOD),
            Animation::Look => (self.look, LOOK),
            Animation::Typing => (self.typing, TYPING),
            Animation::Celebrate => (self.celebrate, CELEBRATE),
            Animation::Juggle => (self.juggle, JUGGLE),
            Animation::Doze => (self.doze, DOZE),
            Animation::Alert => (self.alert, ALERT),
            Animation::Think => (self.think, THINK),
        }
    }
}

fn art(ascii: bool) -> &'static Art {
    if ascii { &ASCII } else { &UNICODE }
}

// The top row is air, for what is above his head; the three under it are
// his head. Every row is seven columns.
const U_TOP: &str = " ╭───╮ ";
const U_BOTTOM: &str = " ╰───╯ ";
const U_SMILE: &str = " ╰─‿─╯ ";
const AIR: &str = "       ";

static UNICODE: Art = Art {
    rest: [AIR, U_TOP, " │• •│ ", U_BOTTOM],
    wave: &[
        [AIR, U_TOP, " │• •│ ", U_BOTTOM],
        [AIR, " ╭───╮╱", " │• •│ ", U_SMILE],
        [AIR, " ╭───╮─", " │• •│ ", U_SMILE],
        [AIR, U_TOP, " │- -│ ", U_BOTTOM],
    ],
    nod: &[
        [AIR, U_TOP, " │• •│ ", U_BOTTOM],
        [AIR, U_TOP, " │- -│ ", U_BOTTOM],
        [AIR, U_TOP, " │• •│ ", U_SMILE],
    ],
    look: &[
        [AIR, U_TOP, " │• •│ ", U_BOTTOM],
        [AIR, U_TOP, " │ ••│ ", U_BOTTOM],
    ],
    typing: &[
        ["  ▁▂▁  ", U_TOP, " │. .│ ", U_BOTTOM],
        ["  ▂▁▂  ", U_TOP, " │. .│ ", U_BOTTOM],
        [AIR, U_TOP, " │• •│ ", U_SMILE],
    ],
    celebrate: &[
        [AIR, U_TOP, " │^ ^│ ", U_SMILE],
        [" *   · ", U_TOP, "*│^ ^│ ", U_SMILE],
        [" ·   * ", U_TOP, " │^ ^│*", U_SMILE],
    ],
    juggle: &[
        ["  °    ", U_TOP, " │° °│ ", U_BOTTOM],
        ["   °   ", U_TOP, " │° °│ ", U_BOTTOM],
        ["    °  ", U_TOP, " │° °│ ", U_BOTTOM],
        ["   °   ", U_TOP, " │° °│ ", U_BOTTOM],
    ],
    doze: &[
        [AIR, U_TOP, " │- -│ ", U_BOTTOM],
        ["     z ", U_TOP, " │- -│ ", U_BOTTOM],
        ["    zZ ", U_TOP, " │- -│ ", U_BOTTOM],
        ["   zZz ", U_TOP, " │- -│ ", U_BOTTOM],
    ],
    alert: &[
        [AIR, U_TOP, " │o o│ ", U_BOTTOM],
        ["   !   ", U_TOP, " │o o│ ", U_BOTTOM],
        [AIR, U_TOP, " │• •│ ", U_BOTTOM],
    ],
    think: &[
        [AIR, U_TOP, " │ °°│ ", U_BOTTOM],
        ["     . ", U_TOP, " │ °°│ ", U_BOTTOM],
        ["    .. ", U_TOP, " │ °°│ ", U_BOTTOM],
        ["   ... ", U_TOP, " │ °°│ ", U_BOTTOM],
    ],
};

const A_TOP: &str = " .---. ";
const A_BOTTOM: &str = " '---' ";
const A_SMILE: &str = " '-u-' ";

static ASCII: Art = Art {
    rest: [AIR, A_TOP, " |o o| ", A_BOTTOM],
    wave: &[
        [AIR, A_TOP, " |o o| ", A_BOTTOM],
        [AIR, " .---./", " |o o| ", A_SMILE],
        [AIR, " .---.-", " |o o| ", A_SMILE],
        [AIR, A_TOP, " |- -| ", A_BOTTOM],
    ],
    nod: &[
        [AIR, A_TOP, " |o o| ", A_BOTTOM],
        [AIR, A_TOP, " |- -| ", A_BOTTOM],
        [AIR, A_TOP, " |o o| ", A_SMILE],
    ],
    look: &[
        [AIR, A_TOP, " |o o| ", A_BOTTOM],
        [AIR, A_TOP, " | oo| ", A_BOTTOM],
    ],
    typing: &[
        ["  _-_  ", A_TOP, " |. .| ", A_BOTTOM],
        ["  -_-  ", A_TOP, " |. .| ", A_BOTTOM],
        [AIR, A_TOP, " |o o| ", A_SMILE],
    ],
    celebrate: &[
        [AIR, A_TOP, " |^ ^| ", A_SMILE],
        [" *   . ", A_TOP, "*|^ ^| ", A_SMILE],
        [" .   * ", A_TOP, " |^ ^|*", A_SMILE],
    ],
    juggle: &[
        ["  o    ", A_TOP, " |o o| ", A_BOTTOM],
        ["   o   ", A_TOP, " |o o| ", A_BOTTOM],
        ["    o  ", A_TOP, " |o o| ", A_BOTTOM],
        ["   o   ", A_TOP, " |o o| ", A_BOTTOM],
    ],
    doze: &[
        [AIR, A_TOP, " |- -| ", A_BOTTOM],
        ["     z ", A_TOP, " |- -| ", A_BOTTOM],
        ["    zZ ", A_TOP, " |- -| ", A_BOTTOM],
        ["   zZz ", A_TOP, " |- -| ", A_BOTTOM],
    ],
    alert: &[
        [AIR, A_TOP, " |O O| ", A_BOTTOM],
        ["   !   ", A_TOP, " |O O| ", A_BOTTOM],
        [AIR, A_TOP, " |o o| ", A_BOTTOM],
    ],
    think: &[
        [AIR, A_TOP, " | oo| ", A_BOTTOM],
        ["     . ", A_TOP, " | oo| ", A_BOTTOM],
        ["    .. ", A_TOP, " | oo| ", A_BOTTOM],
        ["   ... ", A_TOP, " | oo| ", A_BOTTOM],
    ],
};

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    const ALL: [Animation; 9] = [
        Animation::Wave,
        Animation::Nod,
        Animation::Look,
        Animation::Typing,
        Animation::Celebrate,
        Animation::Juggle,
        Animation::Doze,
        Animation::Alert,
        Animation::Think,
    ];

    /// Every row of every frame is seven columns, so the text beside him
    /// never shifts, and the ASCII alphabet's frames hold nothing else.
    #[test]
    fn every_frame_is_seven_columns_and_ascii_is_ascii() {
        for ascii in [false, true] {
            let art = art(ascii);
            let mut frames: Vec<Frame> = vec![art.rest];
            for animation in ALL {
                let (set, script) = art.of(animation);
                frames.extend_from_slice(set);
                assert!(
                    script.iter().all(|i| (*i as usize) < set.len()),
                    "{animation:?}"
                );
            }
            for frame in frames {
                for row in frame {
                    assert_eq!(row.width(), WIDTH as usize, "{row:?}");
                    if ascii {
                        assert!(row.is_ascii(), "{row:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_one_shot_plays_then_rests_and_thinking_loops() {
        assert!(playing(Animation::Wave, 1_000, 1_000 + PLAY_MS - 1));
        assert!(!playing(Animation::Wave, 1_000, 1_000 + PLAY_MS));
        assert_eq!(frame(Animation::Wave, 0, PLAY_MS, false), rest(false));
        assert_ne!(frame(Animation::Wave, 0, FRAME_MS, false), rest(false));
        assert!(playing(Animation::Think, 0, 60_000));
        assert_eq!(
            frame(Animation::Think, 0, 4 * FRAME_MS, true),
            frame(Animation::Think, 0, 0, true)
        );
    }

    /// With nothing to go on he only waves or nods; a fact that holds is
    /// three times as likely as each of those.
    #[test]
    fn a_fact_weighs_three_and_the_plain_ones_one() {
        let plain = Facts::default();
        for seed in 0..200 {
            assert!(matches!(
                choose(&plain, seed),
                Animation::Wave | Animation::Nod
            ));
        }
        let stuck = Facts {
            stuck: 1,
            ..Facts::default()
        };
        let looks = (0..1_000)
            .filter(|seed| choose(&stuck, *seed) == Animation::Look)
            .count();
        assert!((500..700).contains(&looks), "{looks} of 1000");
        assert_eq!(choose(&stuck, 7), choose(&stuck, 7));
    }
}
