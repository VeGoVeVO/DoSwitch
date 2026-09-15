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
