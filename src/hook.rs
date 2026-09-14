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

static mut HOOK: HHOOK = std::ptr::null_mut();
thread_local! {
    static PENDING: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

pub fn install(owner: HWND) -> bool {
    unsafe {
        OWNER = owner;
        HOOK = SetWindowsHookExW(WH_KEYBOARD_LL, Some(callback), std::ptr::null_mut(), 0);
        !HOOK.is_null()
    }
}

pub fn remove() {
    unsafe {
        if !HOOK.is_null() {
            UnhookWindowsHookEx(HOOK);
            HOOK = std::ptr::null_mut();
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

fn held(key: u16) -> bool {
    (unsafe { GetKeyState(key as i32) } as u16 & 0x8000) != 0
}

unsafe extern "system" fn callback(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    }
    let pressed = wparam as u32 == WM_KEYDOWN || wparam as u32 == WM_SYSKEYDOWN;
    if !pressed {
        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    }

    // The panel is waiting for a key to write down: it has the focus, so
    // it will be told by Windows in the ordinary way, and taking the key
    // here would mean it never arrives.
    if app::with(|state| state.capture != Capture::Nothing) {
        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    }
    if !clients::game_has_focus() {
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

    match app::account_for(bind) {
        Some(character) => {
            PENDING.with(|pending| pending.borrow_mut().push(character));
            PostMessageW(OWNER, WM_SWITCH, 0, 0);
            // Swallowed: the key belongs to the switch now, and letting
            // it reach the game as well would cast a spell every time.
            1
        }
        None => CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam),
    }
}

/// Act on one WM_SWITCH. Kept out of the callback on purpose.
pub fn act() {
    while let Some(character) = take_pending() {
        if character == NEXT {
            app::switch_next();
        } else {
            app::switch_to(&character);
        }
    }
}
