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

// Where the installer records the language the player chose (installer.iss
// [Registry]). It is the app's language now that the in-app switcher is
// gone; read once at startup as the fallback for store::load.
const LANG_KEY: &str = r"Software\DoSwitch";
const LANG_VALUE: &str = "Language";

fn main() {
    // The accounts panel's own snapshot, for the README screenshots rather
    // than a golden check. It renders the panel filled with INVENTED accounts
    // (DOSWITCH_FAKE) so the pictures the repo ships never carry a real
    // character's name - the failure this replaces was hand-taken captures of
    // the author's own team going out on a public page.
    let args: Vec<String> = std::env::args().collect();
    if let Some(at) = args.iter().position(|a| a == "--panel-snapshot") {
        let Some(path) = args.get(at + 1) else {
            std::process::exit(2);
        };
        let lang = match args.iter().position(|a| a == "--lang").and_then(|i| args.get(i + 1)) {
            Some(code) if code == "fr" => i18n::Lang::Fr,
            Some(code) if code == "es" => i18n::Lang::Es,
            _ => i18n::Lang::En,
        };
        let count = args
            .iter()
            .position(|a| a == "--count")
            .and_then(|i| args.get(i + 1))
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|n| *n >= 1 && *n <= 11)
            .unwrap_or(3);
        // Invented clients, read fresh by clients::list() (see clients.rs).
        // Set before app::start, which fills the client list from it.
        std::env::set_var("DOSWITCH_FAKE", count.to_string());
        unsafe {
            windows_sys::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
                windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            );
        }
        // A clean settings (never the saved file), with each invented account
        // given a place in the order and an F-key so the picture shows the
        // tool in use rather than an empty first run.
        app::start(store::Settings::empty(lang));
        let names: Vec<String> =
            app::with(|state| state.clients.iter().map(|c| c.character.clone()).collect());
        app::with(|state| {
            for (index, name) in names.iter().enumerate() {
                let account = state.settings.account(name);
                account.order = Some(index as u32 + 1);
                account.key = keys::Bind::parse(&format!("F{}", index + 1));
            }
            state.settings.next = keys::Bind::parse(&format!("F{}", count + 1));
        });
        let ok = panel::snapshot(path);
        std::process::exit(if ok { 0 } else { 1 });
    }

    // Before anything else - before the single-instance mutex, before any
    // window - apply a staged update if one is waiting. Doing it at the start
    // of every launch is what makes the update reliable: it does not depend on
    // a clean exit, so however the last session ended the new build lands on
    // the next launch. If it launches the installer, this throwaway instance
    // must get out of the way at once so the exe can be replaced.
    if update::apply_staged_on_startup(store::auto_update_from_disk()) {
        return;
    }

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
                // Hand the running copy the right to take the foreground before
                // asking it to show the panel. Windows blocks a background
                // process from SetForegroundWindow, so without this the panel
                // opened BEHIND whatever the player was looking at - shown but
                // invisible. THIS process was just launched, so it can pass
                // that right on with AllowSetForegroundWindow.
                let mut pid: u32 = 0;
                GetWindowThreadProcessId(existing, &mut pid);
                if pid != 0 {
                    AllowSetForegroundWindow(pid);
                }
                PostMessageW(existing, WM_APP + 3, 0, 0);
            }
            return;
        }

        windows_sys::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );

        // The language is the install-time choice now (the panel's switcher
        // was replaced by the auto-update toggle), read from the registry the
        // installer wrote; a build run without an installer (dev, portable)
        // falls back to the system language.
        let settings = store::load(install_language());
        let auto_update = settings.auto_update;
        app::start(settings);

        let tray = create_tray_window();
        if tray.is_null() {
            return;
        }
        add_tray_icon(tray);
        // Update to a newer free build in the background, verified before it is
        // ever run - unless the player turned auto-update off, in which case
        // only a server-forced floor still applies. Never blocks startup.
        update::check_in_background(auto_update);
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

        // Open the panel on EVERY launch, not just the first. Opening
        // straight to the tray with no window made people think the app had
        // not started; the panel comes up front, and Minimize sends it back
        // to the tray where it keeps running.
        panel::show(panel);

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
/// The language the installer recorded, or the system language when there is
/// none - a build run without the installer (dev, a portable copy) has no
/// registry value to read, so it still comes up in something sensible.
fn install_language() -> i18n::Lang {
    unsafe {
        let mut buf = [0u16; 16];
        let mut size = (buf.len() * 2) as u32;
        let status = RegGetValueW(
            HKEY_CURRENT_USER,
            wide(LANG_KEY).as_ptr(),
            wide(LANG_VALUE).as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr() as *mut core::ffi::c_void,
            &mut size,
        );
        if status == 0 {
            // size is the byte count including the trailing NUL.
            let chars = (size as usize / 2).saturating_sub(1).min(buf.len());
            let code = String::from_utf16_lossy(&buf[..chars]);
            if let Some(lang) = i18n::Lang::from_code(code.trim()) {
                return lang;
            }
        }
    }
    i18n::system_language()
}

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
