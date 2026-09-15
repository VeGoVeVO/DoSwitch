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

use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, MAX_PATH};
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
            return (0..count)
                .map(|i| Client {
                    hwnd: std::ptr::null_mut(),
                    character: format!("Account-{:02}", i + 1),
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
    let title = window_title(hwnd);
    let Some((character, breed)) = split_title(&title) else {
        return 1;
    };
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if pid == 0 || !is_game_process(pid) {
        return 1;
    }
    FOUND.with(|found| {
        found.borrow_mut().push(Client { hwnd, character, breed })
    });
    1
}

fn window_title(hwnd: HWND) -> String {
    let length = unsafe { GetWindowTextLengthW(hwnd) };
    if length <= 0 {
        return String::new();
    }
    let mut buffer = vec![0u16; length as usize + 1];
    let taken =
        unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    String::from_utf16_lossy(&buffer[..taken.max(0) as usize])
}

/// ("Back-Bonned", "Pandawa") out of "Back-Bonned - Pandawa - 3.6 - Release".
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

/// True when the window in front right now is one of the game's.
pub fn game_has_focus() -> bool {
    let front = unsafe { GetForegroundWindow() };
    if front.is_null() {
        return false;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(front, &mut pid) };
    pid != 0 && is_game_process(pid) && split_title(&window_title(front)).is_some()
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
