//! DoSwitch: one key per Dofus window.
//!
//! The whole program is a tray icon, a panel, and a keyboard hook, all on
//! one thread. It reads nothing but the titles of the windows that are
//! already open, writes nothing but its own settings file, and touches no
//! other program: switching is Windows' own "bring this window forward".
//!
//! Nothing here runs when nothing is pressed. There is no timer, no poll
//! and no background work: the message loop sleeps, the hook is called by
//! the keyboard, and the window list is asked for only when the panel is
//! opened or a key names an account that was not in the last look.

#![windows_subsystem = "windows"]

mod app;
mod clients;
mod crypto;
mod http;
mod draw;
mod hook;
mod i18n;
mod keys;
mod menu;
mod panel;
mod store;
mod update;
mod theme;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use draw::wide;

const TRAY_CLASS: &str = "DoSwitchTray";
const WM_TRAY: u32 = WM_APP + 1;
const TRAY_ID: u32 = 1;
const MENU_ACCOUNTS: usize = 1;
const MENU_STARTUP: usize = 2;
const MENU_QUIT: usize = 3;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "DoSwitch";

fn main() {
    unsafe {
        // One copy at a time. Two would fight over the same keys, and the
        // second would look like the first having stopped working.
        let mutex = windows_sys::Win32::System::Threading::CreateMutexW(
            std::ptr::null(),
            1,
            wide("DoSwitch.single").as_ptr(),
        );
        if !mutex.is_null()
            && windows_sys::Win32::Foundation::GetLastError()
                == windows_sys::Win32::Foundation::ERROR_ALREADY_EXISTS
        {
            // Wake the copy that is already running instead of starting a
            // second one: the user pressed the shortcut for a reason.
            let existing = FindWindowW(wide(TRAY_CLASS).as_ptr(), std::ptr::null());
            if !existing.is_null() {
                PostMessageW(existing, WM_APP + 3, 0, 0);
            }
            return;
        }

        windows_sys::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );

        let settings = store::load(i18n::system_language());
        app::start(settings);

        let tray = create_tray_window();
        if tray.is_null() {
            return;
        }
        add_tray_icon(tray);
        // Update to a newer free build in the background, verified before
        // it is ever run. Never blocks startup.
        update::check_in_background();
        let panel = panel::create();
        app::with(|state| state.panel = panel);

        if !hook::install(tray) {
            // Without the hook the keys cannot work, and a switcher whose
            // keys do nothing should say so rather than sit there.
            MessageBoxW(
                std::ptr::null_mut(),
                wide("DoSwitch could not listen for keys. Another program may already be doing it.").as_ptr(),
                wide("DoSwitch").as_ptr(),
                MB_ICONWARNING,
            );
        }

        // The panel opens on the first run, when there is nothing saved to
        // act on yet, and stays out of the way on every run after.
        if app::with(|state| state.settings.accounts.is_empty()) {
            panel::show(panel);
        }

        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        hook::remove();
        remove_tray_icon(tray);
    }
}

unsafe fn create_tray_window() -> HWND {
    let instance = GetModuleHandleW(std::ptr::null());
    let class = wide(TRAY_CLASS);
    let mut window_class: WNDCLASSW = std::mem::zeroed();
    window_class.lpfnWndProc = Some(tray_proc);
    window_class.hInstance = instance;
    window_class.lpszClassName = class.as_ptr();
    RegisterClassW(&window_class);
    CreateWindowExW(
        0,
        class.as_ptr(),
        wide("DoSwitch").as_ptr(),
        WS_POPUP,
        0,
        0,
        0,
        0,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        instance,
        std::ptr::null(),
    )
}

unsafe fn tray_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut data: NOTIFYICONDATAW = std::mem::zeroed();
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = TRAY_ID;
    data
}

unsafe fn add_tray_icon(hwnd: HWND) {
    let instance = GetModuleHandleW(std::ptr::null());
    let mut data = tray_data(hwnd);
    data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    data.uCallbackMessage = WM_TRAY;
    data.hIcon = LoadIconW(instance, 1 as *const u16);
    let tip = wide(app::language().tray_tip());
    let count = tip.len().min(data.szTip.len());
    data.szTip[..count].copy_from_slice(&tip[..count]);
    Shell_NotifyIconW(NIM_ADD, &data);
}

unsafe fn remove_tray_icon(hwnd: HWND) {
    let data = tray_data(hwnd);
    Shell_NotifyIconW(NIM_DELETE, &data);
}

unsafe fn refresh_tray_tip(hwnd: HWND) {
    let mut data = tray_data(hwnd);
    data.uFlags = NIF_TIP;
    let tip = wide(app::language().tray_tip());
    let count = tip.len().min(data.szTip.len());
    data.szTip[..count].copy_from_slice(&tip[..count]);
    Shell_NotifyIconW(NIM_MODIFY, &data);
}

/// Whether Windows is set to start this program at login.
fn starts_with_windows() -> bool {
    unsafe {
        let mut size = 0u32;
        let status = RegGetValueW(
            HKEY_CURRENT_USER,
            wide(RUN_KEY).as_ptr(),
            wide(RUN_VALUE).as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        );
        status == 0
    }
}

fn set_starts_with_windows(on: bool) {
    unsafe {
        if on {
            let Ok(exe) = std::env::current_exe() else { return };
            let value = format!("\"{}\"", exe.display());
            let wide_value = wide(&value);
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                wide(RUN_KEY).as_ptr(),
                wide(RUN_VALUE).as_ptr(),
                REG_SZ,
                wide_value.as_ptr() as *const core::ffi::c_void,
                (wide_value.len() * 2) as u32,
            );
        } else {
            RegDeleteKeyValueW(
                HKEY_CURRENT_USER,
                wide(RUN_KEY).as_ptr(),
                wide(RUN_VALUE).as_ptr(),
            );
        }
    }
}

unsafe fn show_menu(hwnd: HWND) {
    // The tray menu is drawn in the app's own palette (src/menu.rs), not
    // the grey Windows one, so nothing about the program looks borrowed.
    let lang = app::language();
    let entries = vec![
        menu::Entry {
            label: lang.menu_accounts().to_string(),
            checked: false,
            separator_above: false,
            warn: false,
        },
        menu::Entry {
            label: lang.menu_startup().to_string(),
            checked: starts_with_windows(),
            separator_above: false,
            warn: false,
        },
        menu::Entry {
            label: lang.menu_quit().to_string(),
            checked: false,
            separator_above: true,
            warn: true,
        },
    ];
    // The foreground has to be ours or the menu will not keep focus.
    SetForegroundWindow(hwnd);
    match menu::show(hwnd, entries) {
        Some(0) => { let panel = app::with(|state| state.panel); panel::show(panel); }
        Some(1) => {
            let wanted = !starts_with_windows();
            set_starts_with_windows(wanted);
            app::with(|state| state.settings.startup = wanted);
            app::save();
        }
        Some(2) => { PostQuitMessage(0); }
        _ => {}
    }
}

unsafe extern "system" fn tray_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_TRAY => {
            match lparam as u32 {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => {
                    let panel = app::with(|state| state.panel);
                    if panel::is_open(panel) {
                        panel::hide(panel);
                    } else {
                        panel::show(panel);
                    }
                }
                WM_RBUTTONUP => show_menu(hwnd),
                _ => {}
            }
            0
        }
        hook::WM_SWITCH => {
            hook::act();
            0
        }
        // A window the hook had never met. Deciding about it means reading
        // the process image and the title, both of which can block, which
        // is the whole reason it is decided HERE and not in the callback.
        hook::WM_JUDGE => {
            clients::judge(wparam as HWND);
            0
        }
        WM_COMMAND => {
            match (wparam & 0xFFFF) as usize {
                MENU_ACCOUNTS => {
                    let panel = app::with(|state| state.panel);
                    panel::show(panel);
                }
                MENU_STARTUP => {
                    let wanted = !starts_with_windows();
                    set_starts_with_windows(wanted);
                    app::with(|state| state.settings.startup = wanted);
                    app::save();
                }
                MENU_QUIT => {
                    PostQuitMessage(0);
                }
                _ => {}
            }
            0
        }
        // A second copy was started: show the panel rather than nothing.
        WM_APP_SHOW => {
            let panel = app::with(|state| state.panel);
            panel::show(panel);
            refresh_tray_tip(hwnd);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => {
            // The shell restarts sometimes; the icon has to be put back or
            // it simply disappears and the program looks like it stopped.
            if message == taskbar_created() {
                add_tray_icon(hwnd);
                return 0;
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
    }
}

const WM_APP_SHOW: u32 = WM_APP + 3;

fn taskbar_created() -> u32 {
    use std::sync::OnceLock;
    static ID: OnceLock<u32> = OnceLock::new();
    *ID.get_or_init(|| unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) })
}
