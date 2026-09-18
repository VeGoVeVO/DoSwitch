//! The state the whole program shares, on one thread.
//!
//! The tray, the panel and the keyboard hook all run on the same thread -
//! the one with the message loop, because a hook has to - so the state
//! they share needs no lock and cannot deadlock. A RefCell says exactly
//! that: borrowed twice at once would be a bug in the code, not a race
//! against another thread.

use std::cell::RefCell;

use windows_sys::Win32::Foundation::HWND;

use crate::clients::{self, Client};
use crate::i18n::Lang;
use crate::keys::Bind;
use crate::store::{self, Settings, NEXT};

/// What the panel is waiting for, when it is waiting for something.
#[derive(Clone, PartialEq)]
pub enum Capture {
    Nothing,
    /// A key for this character, or for the cycle when it is NEXT.
    Key(String),
    /// A number for this character's place in the order.
    Order(String),
    /// This character's initiative, as it is being typed. Order takes one
    /// digit and is done; an initiative runs to four, so the digits are
    /// gathered here and only reach the settings on Enter - which is also
    /// what makes an empty entry a way to clear it again.
    Initiative { owner: String, typed: String },
}

pub struct App {
    pub settings: Settings,
    pub clients: Vec<Client>,
    pub panel: HWND,
    pub capture: Capture,
    pub hover: Option<usize>,
    /// Index of the first account row shown when the list is longer than
    /// the panel cares to grow. Scrolling is by whole rows, so there is
    /// never a half row at the top or bottom to misjudge.
    pub scroll: usize,
    /// True while the scrollbar thumb is being dragged, with the grab
    /// offset from the top of the thumb so it does not jump on the first
    /// move.
    pub dragging: bool,
    pub drag_grab: i32,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

pub fn start(settings: Settings) {
    APP.with(|cell| {
        *cell.borrow_mut() = Some(App {
            settings,
            clients: clients::list(),
            panel: std::ptr::null_mut(),
            capture: Capture::Nothing,
            hover: None,
            scroll: 0,
            dragging: false,
            drag_grab: 0,
        });
    });
}

/// The same, refusing rather than panicking when the state is already
/// borrowed or not started yet. The keyboard hook uses this one: its
/// callback can be entered while the thread is inside a Win32 call that
/// pumps messages, and a panic there takes the whole program down over a
/// keystroke.
pub fn try_with<T>(f: impl FnOnce(&mut App) -> T) -> Option<T> {
    APP.with(|cell| match cell.try_borrow_mut() {
        Ok(mut borrow) => borrow.as_mut().map(f),
        Err(_) => None,
    })
}

pub fn with<T>(f: impl FnOnce(&mut App) -> T) -> T {
    APP.with(|cell| {
        let mut borrow = cell.borrow_mut();
        f(borrow.as_mut().expect("the app state is not started yet"))
    })
}

pub fn language() -> Lang {
    with(|app| app.settings.lang)
}

pub fn refresh() {
    clients::forget_dead_windows();
    let fresh = clients::list();
    with(|app| app.clients = fresh);

    // The account list has just changed, and the panel's height is a
    // function of how many accounts are in it - so re-fit the window HERE,
    // at the one place the list can change, instead of trusting every
    // caller to remember afterwards.
    //
    // hook::act() is the caller that proves the point. When a bind misses
    // because the window handle went stale, it refreshes and switches
    // again - and it never touches the panel at all, because switching is
    // all it is there to do. A client opened since the panel was last
    // sized therefore landed in the list with nothing asking the window to
    // grow for it, and the next repaint drew the new row into a window
    // still sized for the old count, with the footer past the bottom edge.
    //
    // Both `with` blocks above are closed before this runs: resize() reads
    // the same state through window_size(), and borrowing it twice at once
    // would panic rather than merely look wrong.
    let panel = with(|app| app.panel);
    if crate::panel::is_open(panel) {
        crate::panel::resize(panel);
    }
}

pub fn save() {
    with(|app| store::save(&app.settings));
}

/// The accounts in the order the player set, the unnumbered ones after,
/// each in the order Windows listed it. This is the order the cycle key
/// walks and the order the panel draws.
pub fn ordered(app: &App) -> Vec<Client> {
    let mut rows = app.clients.clone();
    rows.sort_by_key(|client| {
        let place = app
            .settings
            .accounts
            .get(&client.character)
            .and_then(|account| account.order)
            .unwrap_or(u32::MAX);
        (place, client.character.clone())
    });
    rows
}

/// Give a character a place in the order, and close the gap the move
/// leaves behind: the numbers the player sees are always 1, 2, 3 with
/// nothing missing, whatever they typed.
pub fn set_order(character: &str, wanted: u32) {
    with(|app| {
        let mut names: Vec<String> =
            ordered(app).iter().map(|c| c.character.clone()).collect();
        let Some(from) = names.iter().position(|n| n == character) else {
            return;
        };
        let to = (wanted.max(1) as usize - 1).min(names.len() - 1);
        let moved = names.remove(from);
        names.insert(to, moved);
        for (index, name) in names.iter().enumerate() {
            app.settings.account(name).order = Some(index as u32 + 1);
        }
        store::save(&app.settings);
    });
}

/// Remember a character's initiative, or forget it when `value` is None.
/// It changes nothing else on its own: the order is only rewritten when
/// the player asks for it, in `order_by_initiative`.
pub fn set_initiative(character: &str, value: Option<u32>) {
    with(|app| {
        app.settings.account(character).initiative = value;
        store::save(&app.settings);
    });
}

/// The order the initiatives ask for, from the order the list is in now.
/// Highest initiative plays first.
///
/// Only the characters that HAVE an initiative move, and they are dealt
/// back into the places they already occupied between them. A character
/// with no initiative is not slow, it is unknown - sweeping the unknown
/// ones to the back would be inventing an answer out of a blank field,
/// and the player would have no way to tell that from a real reading.
///
/// Pure, and takes its numbers through a closure, so the rule can be
/// tested without a window, a settings file or a running game.
fn by_initiative(names: &[String], initiative: impl Fn(&str) -> Option<u32>) -> Vec<String> {
    let slots: Vec<usize> = (0..names.len())
        .filter(|&index| initiative(&names[index]).is_some())
        .collect();
    // Ties keep the order they were already in - sort_by_key is stable, and
    // two characters on the same initiative really are in either order.
    let mut movers: Vec<String> = slots.iter().map(|&index| names[index].clone()).collect();
    movers.sort_by_key(|name| std::cmp::Reverse(initiative(name).unwrap_or(0)));

    let mut sorted = names.to_vec();
    for (slot, name) in slots.iter().zip(movers) {
        sorted[*slot] = name;
    }
    sorted
}

/// Put the team in turn order from the initiatives, and save it.
///
/// True when the order actually changed, so a click that would do nothing
/// neither rewrites the settings file nor claims to have done something.
pub fn order_by_initiative() -> bool {
    with(|app| {
        let names: Vec<String> = ordered(app).iter().map(|c| c.character.clone()).collect();
        let sorted = by_initiative(&names, |name| {
            app.settings
                .accounts
                .get(name)
                .and_then(|account| account.initiative)
        });
        if sorted == names {
            return false;
        }
        for (index, name) in sorted.iter().enumerate() {
            app.settings.account(name).order = Some(index as u32 + 1);
        }
        store::save(&app.settings);
        true
    })
}

pub fn set_key(character: &str, bind: Bind) {
    with(|app| {
        app.settings.take_key(bind, Some(character));
        if character == NEXT {
            app.settings.next = Some(bind);
        } else {
            app.settings.account(character).key = Some(bind);
        }
        store::save(&app.settings);
    });
}

pub fn clear_key(character: &str) {
    with(|app| {
        if character == NEXT {
            app.settings.next = None;
        } else {
            app.settings.account(character).key = None;
        }
        store::save(&app.settings);
    });
}

/// The account a key press means, or None when it means nothing.
pub fn account_for(bind: Bind) -> Option<String> {
    with(|app| {
        if app.settings.next == Some(bind) {
            return Some(NEXT.to_string());
        }
        app.settings
            .accounts
            .iter()
            .find(|(_, account)| account.key == Some(bind))
            .map(|(name, _)| name.clone())
    })
}

/// Bring one character's window forward. The window list is refreshed on
/// a miss rather than trusted: a client that logged in since the panel
/// was last opened is exactly the one being asked for.
pub fn switch_to(character: &str) -> bool {
    let found = with(|app| {
        app.clients
            .iter()
            .find(|client| client.character == character)
            .map(|client| client.hwnd)
    });
    let hwnd = match found {
        Some(hwnd) => hwnd,
        None => {
            refresh();
            match with(|app| {
                app.clients
                    .iter()
                    .find(|client| client.character == character)
                    .map(|client| client.hwnd)
            }) {
                Some(hwnd) => hwnd,
                None => return false,
            }
        }
    };
    clients::focus(hwnd)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| name.to_string()).collect()
    }

    fn table(pairs: &[(&'static str, u32)]) -> impl Fn(&str) -> Option<u32> {
        let pairs = pairs.to_vec();
        move |name: &str| {
            pairs
                .iter()
                .find(|(who, _)| *who == name)
                .map(|(_, value)| *value)
        }
    }

    #[test]
    fn the_fastest_plays_first() {
        let sorted = by_initiative(
            &names(&["slow", "fast", "middling"]),
            table(&[("slow", 700), ("fast", 1200), ("middling", 900)]),
        );
        assert_eq!(sorted, names(&["fast", "middling", "slow"]));
    }

    /// The point of the whole rule: a character nobody typed a number for
    /// is unknown, not slow. It keeps the slot it had, and the ones that
    /// ARE known sort among the slots they had between them.
    #[test]
    fn a_character_with_no_number_does_not_move() {
        let sorted = by_initiative(
            &names(&["slow", "unknown", "fast"]),
            table(&[("slow", 700), ("fast", 1200)]),
        );
        assert_eq!(sorted, names(&["fast", "unknown", "slow"]));
    }

    #[test]
    fn equal_initiatives_keep_the_order_they_had() {
        let sorted = by_initiative(
            &names(&["first", "second"]),
            table(&[("first", 900), ("second", 900)]),
        );
        assert_eq!(sorted, names(&["first", "second"]));
    }

    /// Nothing to sort is not an error, and it is not a reordering either -
    /// the caller reads "unchanged" and leaves the settings file alone.
    #[test]
    fn nothing_to_go_on_changes_nothing() {
        let list = names(&["a", "b", "c"]);
        assert_eq!(by_initiative(&list, table(&[])), list);
        assert_eq!(by_initiative(&[], table(&[("a", 1)])), Vec::<String>::new());
    }
}

/// The next window in the player's order, starting from whichever is in
/// front right now. Not from a counter of our own: the player switches by
/// hand too, and a counter would send them somewhere they did not expect.
pub fn switch_next() -> bool {
    refresh();
    let (rows, front) = with(|app| (ordered(app), clients::focused_window()));
    if rows.is_empty() {
        return false;
    }
    let here = front
        .and_then(|hwnd| rows.iter().position(|client| client.hwnd == hwnd));
    let next = match here {
        Some(index) => (index + 1) % rows.len(),
        None => 0,
    };
    clients::focus(rows[next].hwnd)
}
