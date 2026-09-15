//! The keys, taken from the keyboard itself.
//!
//! WHY A HOOK AND NOT RegisterHotKey. A registered hotkey belongs to this
//! program everywhere: bind F1 and F1 stops working in the browser, in
//! chat, in every other program on the machine. A low level hook sees the
//! key first and decides, so the bind can be swallowed while the game is
//! in front and passed straight through everywhere else. That is the
//! difference between a key you set once and a key you have to remember
//! not to press.
//!
//! The hook has to live on a thread with a message loop, and the callback
//! runs on that thread, which is why everything here shares one thread
//! with the panel and the tray.
//!
//! A hook that takes too long is silently removed by Windows, so the
//! callback does the least it can: decide, and post the work back to our
//! own window. Focusing a window from inside the callback would be work
//! done on the keyboard's own timing.
//!
//! "The least it can" is a rule the callback broke for a long time, and
//! the bill came in as binds that worked most of the time. Windows gives a
//! low level hook callback LowLevelHooksTimeout - 300ms by default - and
//! if it has not returned by then the key is passed on WITHOUT it. No
//! error, no log: the key reaches the game, the switch does not happen,
//! and the next press works fine. The callback was calling
//! clients::game_has_focus on every keystroke on the machine, and that
//! read the foreground window's TITLE - which for another process's window
//! is SendMessage(WM_GETTEXT), blocking until that process pumps. A Dofus
//! client loading a map does not pump. The keys that went missing were the
//! ones pressed while the game was busy, which is exactly when a player is
//! pressing them.
//!
//! So the callback now touches nothing that can block. It compares the
//! foreground window against a verdict this program already reached off
//! the hook path (clients::cached_verdict), and if it has never seen that
//! window it says so and asks for it to be looked at, rather than looking
//! itself.

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::app::{self, Capture};
use crate::clients;
use crate::keys::Bind;
use crate::store::NEXT;

/// Posted to the tray window when a bind was pressed. The payload is an
/// index into the list the callback built, so the message carries no
/// allocation.
pub const WM_SWITCH: u32 = WM_APP + 2;

/// Posted when the callback met a window it has no verdict for. The
/// wparam is the window; the handler decides about it and remembers.
pub const WM_JUDGE: u32 = WM_APP + 7;

static mut HOOK: HHOOK = std::ptr::null_mut();
/// The mouse's own low-level hook. Separate from the keyboard's because
/// Windows keeps them on different chains, and a mouse button is not a
/// key: it arrives as WM_XBUTTONDOWN with the button number in the high
/// word of mouseData, not as a virtual key code.
static mut MOUSE: HHOOK = std::ptr::null_mut();
thread_local! {
    static PENDING: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

pub fn install(owner: HWND) -> bool {
    unsafe {
        OWNER = owner;
        HOOK = SetWindowsHookExW(WH_KEYBOARD_LL, Some(callback), std::ptr::null_mut(), 0);
        // Best effort, and deliberately not part of the return value: if
        // the mouse hook cannot be installed the keyboard binds still
        // work, and a switcher whose keys work is not a switcher that
        // failed to start.
        MOUSE = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_callback), std::ptr::null_mut(), 0);
        !HOOK.is_null()
    }
}

pub fn remove() {
    unsafe {
        if !HOOK.is_null() {
            UnhookWindowsHookEx(HOOK);
            HOOK = std::ptr::null_mut();
        }
        if !MOUSE.is_null() {
            UnhookWindowsHookEx(MOUSE);
            MOUSE = std::ptr::null_mut();
        }
    }
}

static mut OWNER: HWND = std::ptr::null_mut();

/// The character whose key was pressed, taken by the window that handles
/// WM_SWITCH. Empty when the message arrived without one, which should
/// not happen and is simply ignored if it does.
pub fn take_pending() -> Option<String> {
    PENDING.with(|pending| {
        let mut queue = pending.borrow_mut();
        if queue.is_empty() {
            None
        } else {
            Some(queue.remove(0))
        }
    })
}

/// Is a modifier physically down right now.
///
/// GetAsyncKeyState, not GetKeyState. GetKeyState reports the state as of
/// the last input message the CALLING thread removed from its own queue -
/// and this thread is a hook thread, which does not receive the keyboard's
/// messages in the ordinary way, so what it reports can be stale. A stale
/// "control is down" turns F2 into Ctrl+F2, which matches no bind, and the
/// press is passed through as if it were not a bind at all. Another of the
/// ways a key could simply do nothing and leave nothing behind.
fn held(key: u16) -> bool {
    (unsafe { GetAsyncKeyState(key as i32) } as u16 & 0x8000) != 0
}

unsafe extern "system" fn callback(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    }
    let pressed = wparam as u32 == WM_KEYDOWN || wparam as u32 == WM_SYSKEYDOWN;
    if !pressed {
        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    }

    census(Seen::Key);
    if !gate_open() {
        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    }
    let event = &*(lparam as *const KBDLLHOOKSTRUCT);
    let bind = Bind {
        code: event.vkCode,
        ctrl: held(VK_CONTROL),
        alt: held(VK_MENU),
        shift: held(VK_SHIFT),
    };
    if Bind::is_modifier_only(bind.code) {
        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    }

    if claim(bind) {
        // Swallowed: the press belongs to the switch now, and letting it
        // reach the game as well would cast a spell every time.
        1
    } else {
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }
}

/// The mouse's half of the same job.
///
/// A mouse button is bound and matched exactly like a key, because a Bind
/// is a virtual key code and Windows gives the buttons their own: left is
/// VK_LBUTTON, right VK_RBUTTON, middle VK_MBUTTON, and the two thumb
/// buttons VK_XBUTTON1 and VK_XBUTTON2. What is different is how they
/// arrive - a message per button rather than a code in a field - and that
/// the side buttons share one message and are told apart by the high word
/// of mouseData.
///
/// Five is what Windows reports, and a mouse with twelve buttons does not
/// change that: the rest are sent by the mouse's own software as keys or
/// macros, which this program already binds, and they never reach a mouse
/// hook as buttons at all.
///
/// Injected presses are ignored. Another program moving the mouse - or
/// this one, some day - would otherwise be able to trigger a switch, and
/// a bind that fires without anybody pressing anything is indistinguishable
/// from a bug.
unsafe extern "system" fn mouse_callback(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    }
    let event = &*(lparam as *const MSLLHOOKSTRUCT);
    if event.flags & LLMHF_INJECTED != 0 {
        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    }
    let button = match wparam as u32 {
        WM_LBUTTONDOWN => VK_LBUTTON,
        WM_RBUTTONDOWN => VK_RBUTTON,
        WM_MBUTTONDOWN => VK_MBUTTON,
        WM_XBUTTONDOWN => match (event.mouseData >> 16) as u16 {
            XBUTTON1 => VK_XBUTTON1,
            XBUTTON2 => VK_XBUTTON2,
            _ => return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam),
        },
        _ => return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam),
    };

    census(Seen::Key);
    if !gate_open() {
        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    }
    let bind = Bind {
        code: button as u32,
        ctrl: held(VK_CONTROL),
        alt: held(VK_MENU),
        shift: held(VK_SHIFT),
    };
    if claim(bind) {
        1
    } else {
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }
}

/// Is this program willing to take a press at all right now?
///
/// Shared by both hooks, so a mouse button is gated on exactly what a key
/// is gated on: not while the panel is waiting to write a bind down, and
/// only when the window in front is one this program already decided was
/// a game client. Asking Windows that question here is what used to lose
/// keystrokes; the answer is read from a verdict reached off this path.
unsafe fn gate_open() -> bool {
    // try_with, not with: this callback can be entered while the thread is
    // inside a Win32 call that pumps messages, and a panic on a double
    // borrow would end the program over a keystroke. Unreadable state is
    // treated as "not capturing", which is the state it is in almost all
    // of the time.
    if app::try_with(|state| state.capture != Capture::Nothing).unwrap_or(false) {
        return false;
    }
    // Cached only. See the note at the top of this file: asking Windows
    // this question here is what was losing keys.
    let front = GetForegroundWindow();
    let (known, restale) = clients::cached_verdict(front);
    if restale {
        // Ask for a fresh look off the hook path. The answer below is the
        // one already recorded, so this costs the keystroke nothing - it
        // only means the next one is decided on something newer.
        PostMessageW(OWNER, WM_JUDGE, front as WPARAM, 0);
    }
    match known {
        Some(true) => true,
        // A switch we asked for is still on its way, so the foreground is
        // mid-change and answering for the window we are leaving, for the
        // shell, or for nothing. Pressing another bind during that gap is
        // what spamming IS, and it was being read as "not in the game".
        _ if clients::switch_in_flight() => true,
        Some(false) => {
            census(Seen::NotGame);
            false
        }
        None => {
            // Never seen this window. The refresh was already asked for
            // above; let this one press through, and by the next one the
            // answer is in the cache.
            census(Seen::Unjudged);
            false
        }
    }
}

/// Take the press, if it names an account. True when it has been taken and
/// the caller should swallow it.
unsafe fn claim(bind: Bind) -> bool {
    match app::account_for(bind) {
        Some(character) => {
            PENDING.with(|pending| pending.borrow_mut().push(character));
            // If the post fails the press has been taken from the game and
            // given to nobody, so it is handed back rather than eaten.
            if PostMessageW(OWNER, WM_SWITCH, 0, 0) == 0 {
                let _ = take_pending();
                census(Seen::Lost);
                return false;
            }
            true
        }
        None => {
            census(Seen::NoBind);
            false
        }
    }
}

/// Act on one WM_SWITCH. Kept out of the callback on purpose.
///
/// The key has already been swallowed by the time this runs - the player
/// pressed a bind and the game did not get it - so if the switch does not
/// happen, nothing happens at all. `focus` reports honestly whether the
/// window reached the front, so a refusal is retried once against a
/// freshly read window list: the usual reason for a miss is a handle that
/// belongs to a client which has since been closed and reopened.
pub fn act() {
    while let Some(character) = take_pending() {
        let switched = if character == NEXT {
            app::switch_next()
        } else {
            app::switch_to(&character)
        };
        if !switched {
            app::refresh();
            let again = if character == NEXT {
                app::switch_next()
            } else {
                app::switch_to(&character)
            };
            census(if again { Seen::Retried } else { Seen::Lost });
        } else {
            census(Seen::Switched);
        }
        census_write();
    }
}

/// What the hook saw, counted.
///
/// A bind that does nothing leaves no trace by design - the key is eaten,
/// the switch does not happen, and the player is left saying "sometimes it
/// does not work" with nothing to look at. These counters are the smallest
/// thing that turns that into a number naming which step lost it. They are
/// written out only when DOSWITCH_HOOK_LOG names a file, so they cost a
/// few increments on a player's machine and nothing else.
#[derive(Clone, Copy)]
pub enum Seen {
    Key,
    Unjudged,
    NotGame,
    NoBind,
    Switched,
    Retried,
    Lost,
}

static COUNTS: [std::sync::atomic::AtomicU64; 7] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 7];

pub fn census(what: Seen) {
    COUNTS[what as usize].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// Append the counters to the file DOSWITCH_HOOK_LOG names, if it names
/// one. Called after every switch, so the file is a running tail of what
/// the hook has seen rather than a snapshot nobody was there to take.
///
/// This is the answer to "it does not always work": a player can be asked
/// to set one variable, play for ten minutes, and send back a line that
/// says which step lost the key. A LOST above zero is the hook eating a
/// key and nothing acting on it; an unjudged above zero with switches
/// missing is a window this program had not met yet; a no-bind climbing
/// while the player insists they pressed F2 is the modifier state being
/// read wrong.
pub fn census_write() {
    // Read fresh each time rather than cached at startup, so the log can
    // be turned on for a running app by nothing more than a restart.
    let Ok(path) = std::env::var("DOSWITCH_HOOK_LOG") else {
        return;
    };
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{}", census_line());
    }
}

/// The counters, as one line. Empty when nothing has been counted.
pub fn census_line() -> String {
    let n = |i: usize| COUNTS[i].load(std::sync::atomic::Ordering::Relaxed);
    format!(
        "keys {} unjudged {} not-game {} no-bind {} switched {} retried {} LOST {}",
        n(Seen::Key as usize),
        n(Seen::Unjudged as usize),
        n(Seen::NotGame as usize),
        n(Seen::NoBind as usize),
        n(Seen::Switched as usize),
        n(Seen::Retried as usize),
        n(Seen::Lost as usize),
    )
}
