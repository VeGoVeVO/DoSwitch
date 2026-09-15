//! The open Dofus windows, and putting one of them in front.
//!
//! A window is recognised by two things together: the process it belongs
//! to is a Dofus one, and its title has the shape the game gives a logged
//! in character, "Name - Class - version - Release". Either alone is not
//! enough - the launcher is the same process family, and another program
//! could have a similar title - and the pair has never been wrong.
//!
//! BRINGING A WINDOW TO THE FRONT IS NOT SetForegroundWindow. Windows
//! refuses it unless the calling process already owns the foreground, and
//! it refuses SILENTLY: the call returns, nothing moves, and there is no
//! error to read. The way through is to make Windows believe we are the
//! foreground thread for the length of the call - AttachThreadInput to
//! whichever thread owns it right now, raise the window, detach again -
//! with the steps around it each there for a reason:
//!
//!   ShowWindow(RESTORE)   a minimised window cannot take the foreground
//!                         at all, and a window parked in the taskbar is
//!                         exactly the one being switched to.
//!   SwitchToThisWindow    what the shell itself uses for Alt+Tab, and it
//!                         often still works when the attach is refused.
//!   BringWindowToTop      orders the stack once the foreground is ours.
//!
//! `focus` reports whether the window really ended up in front, rather
//! than claiming success: a switcher that lies is worse than one that
//! says it could not.

use std::cell::RefCell;

use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, MAX_PATH, WPARAM};
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, Debug)]
pub struct Client {
    pub hwnd: HWND,
    pub character: String,
    pub breed: String,
}

thread_local! {
    static FOUND: RefCell<Vec<Client>> = const { RefCell::new(Vec::new()) };
}

/// Every logged in Dofus window, in the order Windows lists them.
pub fn list() -> Vec<Client> {
    // A test hatch, never reached in a normal run: set DOSWITCH_FAKE to a
    // count and the list is that many invented accounts, so the panel can
    // be seen under twenty windows without opening twenty clients. It is
    // read fresh each call, costs nothing when unset, and cannot fire by
    // accident because the variable does not exist on a player's machine.
    if let Ok(raw) = std::env::var("DOSWITCH_FAKE") {
        if let Ok(count) = raw.trim().parse::<usize>() {
            let breeds = [
                "Iop", "Cra", "Eniripsa", "Sacrieur", "Pandawa", "Feca",
                "Sram", "Xelor", "Ecaflip", "Enutrof", "Osamodas", "Sadida",
            ];
            // Invented names, so a screenshot taken from this never
            // carries somebody's real characters into a public README.
            let names = [
                "Aurelia", "Kaelin-Vex", "Morwen", "Tovak", "Elandre",
                "Sylnae", "Brannoc", "Ysolde", "Faelan", "Ombrelle",
                "Cendrik", "Nivelle",
            ];
            return (0..count)
                .map(|i| Client {
                    hwnd: std::ptr::null_mut(),
                    character: if i < names.len() {
                        names[i].to_string()
                    } else {
                        format!("{}-{}", names[i % names.len()], i / names.len() + 1)
                    },
                    breed: breeds[i % breeds.len()].to_string(),
                })
                .collect();
        }
    }
    FOUND.with(|found| found.borrow_mut().clear());
    unsafe {
        EnumWindows(Some(each), 0);
    }
    FOUND.with(|found| found.borrow().clone())
}

unsafe extern "system" fn each(hwnd: HWND, _param: LPARAM) -> BOOL {
    if IsWindowVisible(hwnd) == 0 {
        return 1;
    }
    // The verdict is recorded on the way past, either way. This walk is
    // already paying for the title and the process image, so the cache the
    // keyboard hook reads is filled by the refresh that was happening
    // anyway rather than by a second pass of its own.
    let Some(title) = window_title(hwnd) else {
        // Could not read it. Leave whatever verdict it already has alone.
        return 1;
    };
    let Some((character, breed)) = split_title(&title) else {
        remember(hwnd, false);
        return 1;
    };
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if pid == 0 || !is_game_process(pid) {
        remember(hwnd, false);
        return 1;
    }
    remember(hwnd, true);
    FOUND.with(|found| {
        found.borrow_mut().push(Client { hwnd, character, breed })
    });
    1
}

/// A window's title, with a deadline.
///
/// GetWindowTextW against a window owned by ANOTHER process is not a read
/// of anything: it is SendMessage(WM_GETTEXT), and it blocks until that
/// process pumps its message queue. A Dofus client loading a map, taking
/// a garbage collection pause, or waiting on the disk does not pump, and
/// the call sits there. This used to be called from inside the low level
/// keyboard hook, where Windows gives the whole callback 300ms before it
/// gives up on it and lets the key through unhandled - which is exactly
/// what "sometimes I press F2 and nothing happens" was.
///
/// It is off the hook path now, but a title read that can hang the UI
/// thread for seconds is not something to leave lying around either, so
/// the send carries a deadline and ABORTIFHUNG. A title we could not get
/// comes back empty, which every caller already treats as "not a client".
fn window_title(hwnd: HWND) -> Option<String> {
    const TITLE_TIMEOUT_MS: u32 = 120;
    let mut buffer = [0u16; 256];
    let mut taken: usize = 0;
    let ok = unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_GETTEXT,
            buffer.len() as WPARAM,
            buffer.as_mut_ptr() as LPARAM,
            SMTO_ABORTIFHUNG | SMTO_BLOCK,
            TITLE_TIMEOUT_MS,
            &mut taken as *mut usize as *mut usize,
        )
    };
    // None is "we could not read it", which is NOT the same as "it has no
    // title" and must not be cached as a decision. A client that was busy
    // when we happened to ask is exactly the one whose key has to keep
    // working; recording a no for it would switch that key off until
    // something else happened to refresh the list.
    if ok == 0 {
        return None;
    }
    let taken = taken.min(buffer.len() - 1);
    Some(String::from_utf16_lossy(&buffer[..taken]))
}

/// ("Morwen", "Pandawa") out of "Morwen - Pandawa - 3.6 - Release".
///
/// Split on the separator WITH its spaces. A character name may contain a
/// hyphen, and one that does was read as half a name for as long as the
/// separator was just the character.
fn split_title(title: &str) -> Option<(String, String)> {
    let pieces: Vec<&str> = title.split(" - ").collect();
    if pieces.len() < 3 {
        return None;
    }
    let name = pieces[0].trim();
    let breed = pieces[1].trim();
    if name.is_empty() || breed.is_empty() {
        return None;
    }
    Some((name.to_string(), breed.to_string()))
}

fn is_game_process(pid: u32) -> bool {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut buffer = [0u16; MAX_PATH as usize];
        let mut size = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            0,
            buffer.as_mut_ptr(),
            &mut size,
        );
        windows_sys::Win32::Foundation::CloseHandle(handle);
        if ok == 0 {
            return false;
        }
        let path = String::from_utf16_lossy(&buffer[..size as usize]);
        let file = path.rsplit(['\\', '/']).next().unwrap_or("");
        file.to_ascii_lowercase().starts_with("dofus")
    }
}

thread_local! {
    /// Windows we have already decided about: true for a logged in game
    /// client, false for everything else. The keyboard hook reads this and
    /// NOTHING else, because every way of asking Windows the question -
    /// the title, the process image - is a call that can block, and a low
    /// level hook callback that blocks for 300ms has its key taken away
    /// from it and passed through unhandled.
    static VERDICTS: RefCell<std::collections::HashMap<isize, (bool, std::time::Instant)>> =
        RefCell::new(std::collections::HashMap::new());
}

/// How long a decision about a window is used before it is looked at
/// again. The old answer keeps being used while the new one is worked out
/// off the hook path, so going stale costs no keystroke - it just means
/// the answer is at most this old.
const VERDICT_TTL: std::time::Duration = std::time::Duration::from_secs(3);

thread_local! {
    /// The window one of OUR switches is currently heading for, and when
    /// it set off. Nothing else sets this.
    static SWITCHING: RefCell<(isize, Option<std::time::Instant>)> =
        const { RefCell::new((0, None)) };
}

/// How long after a switch sets off the foreground still counts as the
/// game's. A switch is not instant: between asking for a window and that
/// window being in front there is a gap in which GetForegroundWindow
/// answers with the old window, with the shell, or with nothing at all.
const IN_FLIGHT: std::time::Duration = std::time::Duration::from_millis(450);

/// Remember that a switch to this window has set off.
pub fn note_switch(hwnd: HWND) {
    SWITCHING.with(|s| *s.borrow_mut() = (hwnd as isize, Some(std::time::Instant::now())));
}

/// True while one of our own switches is still in flight.
///
/// This exists because of a hole the counters found: spam the binds and
/// one press in three was being read as "the game is not in front" and
/// handed to whatever was. It was not - the switch it had just asked for
/// had not landed yet, and the foreground was mid-change. Without this a
/// player pressing faster than Windows can raise a window loses the press
/// AND sends the key into the game, which is the worse half.
pub fn switch_in_flight() -> bool {
    SWITCHING.with(|s| {
        let (_, at) = *s.borrow();
        at.is_some_and(|at| at.elapsed() < IN_FLIGHT)
    })
}

/// What we already know about this window, without asking Windows.
///
/// `None` means we have never looked at it. The hook treats that as "not
/// mine" for this one keystroke and asks for it to be looked at off the
/// hook path, so the answer is there by the next one. A key going to the
/// game once, the first time a brand new client is focused, is a far
/// smaller thing than a key being dropped because the callback was busy.
/// What we know, and whether it wants looking at again.
///
/// The verdict is ALWAYS the one to act on, even when stale. The first
/// version of this returned None for a stale entry, which meant one key in
/// every few seconds was passed through while the answer was recomputed -
/// trading a rare failure for a regular one.
pub fn cached_verdict(hwnd: HWND) -> (Option<bool>, bool) {
    if hwnd.is_null() {
        return (Some(false), false);
    }
    VERDICTS.with(|v| match v.borrow().get(&(hwnd as isize)) {
        Some((verdict, at)) => (Some(*verdict), at.elapsed() > VERDICT_TTL),
        None => (None, true),
    })
}

/// Decide about one window and remember it. Safe to call from anywhere
/// that is not the keyboard hook: it reads the process image and the
/// title, both of which can block, the title now with a deadline.
pub fn judge(hwnd: HWND) -> bool {
    if hwnd.is_null() {
        return false;
    }
    let Some(title) = window_title(hwnd) else {
        // Busy, or gone. Whatever is already recorded stands; we do not
        // turn "could not ask" into "no".
        return VERDICTS
            .with(|v| v.borrow().get(&(hwnd as isize)).map(|(verdict, _)| *verdict))
            .unwrap_or(false);
    };
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    let verdict = pid != 0 && is_game_process(pid) && split_title(&title).is_some();
    remember(hwnd, verdict);
    verdict
}

fn remember(hwnd: HWND, verdict: bool) {
    VERDICTS.with(|v| {
        v.borrow_mut()
            .insert(hwnd as isize, (verdict, std::time::Instant::now()))
    });
}

/// Forget windows that no longer exist, so the map cannot grow for the
/// life of the process. Called from the same place the client list is
/// refreshed, which is often enough and never on the hook path.
pub fn forget_dead_windows() {
    VERDICTS.with(|v| {
        v.borrow_mut()
            .retain(|handle, _| unsafe { IsWindow(*handle as HWND) != 0 })
    });
}

/// The window in front right now, if it is a game window.
pub fn focused_window() -> Option<HWND> {
    let front = unsafe { GetForegroundWindow() };
    if front.is_null() {
        None
    } else {
        Some(front)
    }
}

/// Put a window in front. Returns whether it actually got there.
pub fn focus(hwnd: HWND) -> bool {
    unsafe {
        if IsWindow(hwnd) == 0 {
            return false;
        }
        note_switch(hwnd);
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        let front = GetForegroundWindow();
        if front == hwnd {
            return true;
        }
        let ours = GetCurrentThreadId();
        let theirs = if front.is_null() {
            0
        } else {
            GetWindowThreadProcessId(front, std::ptr::null_mut())
        };
        let attached = theirs != 0
            && theirs != ours
            && AttachThreadInput(ours, theirs, 1) != 0;

        SetForegroundWindow(hwnd);
        BringWindowToTop(hwnd);
        SetFocus(hwnd);

        if attached {
            AttachThreadInput(ours, theirs, 0);
        }
        if GetForegroundWindow() != hwnd {
            // The documented-enough fallback, and the one that survives a
            // refusal of the attach trick.
            SwitchToThisWindow(hwnd, 1);
        }
        GetForegroundWindow() == hwnd
    }
}
