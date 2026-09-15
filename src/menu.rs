//! The tray menu, drawn in the panel's palette rather than Windows' grey.
//!
//! A right click on the tray icon should not drop the one piece of default
//! Windows chrome into an app that is otherwise entirely its own. This is
//! a small borderless window placed at the cursor, painted with the same
//! Canvas the panel uses, that runs its own short message loop and returns
//! the item chosen - the same thing TrackPopupMenu does, wearing the same
//! clothes as everything else.

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, SetCapture, ReleaseCapture};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::draw::{font, wide, Canvas, R};
use crate::theme::*;

const CLASS: &str = "DoSwitchMenu";
const WIDTH: i32 = 236;
const ITEM_H: i32 = 40;
const PAD: i32 = 8;
const SEP_H: i32 = 9;

/// One menu entry: its label, whether it shows a tick, and whether a thin
/// separator sits above it.
pub struct Entry {
    pub label: String,
    pub checked: bool,
    pub separator_above: bool,
    pub warn: bool,
}

struct State {
    entries: Vec<Entry>,
    hover: i32,
    chosen: i32,
    scale: f32,
    done: bool,
}

thread_local! {
    static STATE: std::cell::RefCell<Option<State>> = const { std::cell::RefCell::new(None) };
}

fn item_tops(entries: &[Entry]) -> Vec<i32> {
    let mut tops = Vec::with_capacity(entries.len());
    let mut y = PAD;
    for entry in entries {
        if entry.separator_above {
            y += SEP_H;
        }
        tops.push(y);
        y += ITEM_H;
    }
    tops
}

fn total_height(entries: &[Entry]) -> i32 {
    item_tops(entries)
        .last()
        .copied()
        .map(|t| t + ITEM_H + PAD)
        .unwrap_or(PAD * 2)
}

/// Show the menu with its lower right corner at the cursor, run it, and
/// return the index of the chosen entry, or None if dismissed.
pub fn show(owner: HWND, entries: Vec<Entry>) -> Option<usize> {
    unsafe {
        let scale = scale_for(owner);
        let logical_h = total_height(&entries);
        let width = (WIDTH as f32 * scale).round() as i32;
        let height = (logical_h as f32 * scale).round() as i32;

        STATE.with(|cell| {
            *cell.borrow_mut() = Some(State {
                entries,
                hover: -1,
                chosen: -1,
                scale,
                done: false,
            })
        });

        let instance = GetModuleHandleW(std::ptr::null());
        let class = wide(CLASS);
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.lpfnWndProc = Some(wndproc);
        wc.hInstance = instance;
        wc.lpszClassName = class.as_ptr();
        wc.hCursor = LoadCursorW(std::ptr::null_mut(), IDC_ARROW);
        RegisterClassW(&wc);

        let mut cursor: POINT = std::mem::zeroed();
        GetCursorPos(&mut cursor);
        let mut work: RECT = std::mem::zeroed();
        SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut work as *mut RECT as *mut _, 0);
        // The tray sits at the bottom right, so the menu grows up and left
        // from the cursor, then is nudged back inside the work area.
        let mut x = cursor.x - width;
        let mut y = cursor.y - height;
        if x < work.left {
            x = cursor.x;
        }
        if y < work.top {
            y = cursor.y;
        }
        x = x.min(work.right - width).max(work.left);
        y = y.min(work.bottom - height).max(work.top);

        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            class.as_ptr(),
            wide("").as_ptr(),
            WS_POPUP,
            x,
            y,
            width,
            height,
            owner,
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            STATE.with(|cell| *cell.borrow_mut() = None);
            return None;
        }
        let round: u32 = 2;
        DwmSetWindowAttribute(hwnd, 33, &round as *const u32 as *const core::ffi::c_void, 4);
        ShowWindow(hwnd, SW_SHOWNA);
        SetForegroundWindow(hwnd);
        SetCapture(hwnd);

        // A short message loop of its own, the way a menu runs: it ends
        // when something is chosen or the menu is dismissed.
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            let done = STATE.with(|cell| {
                cell.borrow().as_ref().map(|s| s.done).unwrap_or(true)
            });
            if done {
                break;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        let chosen = STATE.with(|cell| {
            cell.borrow().as_ref().map(|s| s.chosen).unwrap_or(-1)
        });
        ReleaseCapture();
        DestroyWindow(hwnd);
        STATE.with(|cell| *cell.borrow_mut() = None);
        if chosen >= 0 {
            Some(chosen as usize)
        } else {
            None
        }
    }
}

fn scale_for(hwnd: HWND) -> f32 {
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd) };
    if dpi == 0 {
        1.0
    } else {
        dpi as f32 / 96.0
    }
}

fn index_at(y_logical: i32, entries: &[Entry]) -> i32 {
    for (index, top) in item_tops(entries).iter().enumerate() {
        if y_logical >= *top && y_logical < top + ITEM_H {
            return index as i32;
        }
    }
    -1
}

unsafe fn paint(hwnd: HWND) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let dc = BeginPaint(hwnd, &mut ps);
    STATE.with(|cell| {
        let borrow = cell.borrow();
        let Some(state) = borrow.as_ref() else {
            EndPaint(hwnd, &ps);
            return;
        };
        let scale = state.scale;
        let at = |v: i32| (v as f32 * scale).round() as i32;
        let width = at(WIDTH);
        let height = at(total_height(&state.entries));
        let Some(canvas) = Canvas::new(dc, width, height) else {
            EndPaint(hwnd, &ps);
            return;
        };
        let text_font = font(at(14), FW_SEMIBOLD as i32);
        let tick_font = font(at(13), FW_BOLD as i32);

        canvas.clear(INK);
        // The same card as the panel: a leaf-lined surface with a faint
        // glow from the top so the menu belongs to the app, not Windows.
        canvas.round(R::new(0, 0, width, height), at(12), INK);
        canvas.outline(R::new(0, 0, width, height), at(12), at(1), line(), INK);
        canvas.glow(at(10), at(6), at(90), LEAF, 0.10);

        let tops = item_tops(&state.entries);
        for (index, entry) in state.entries.iter().enumerate() {
            let top = at(tops[index]);
            let item = R::new(at(PAD), top, width - at(PAD) * 2, at(ITEM_H));
            if entry.separator_above {
                canvas.round(
                    R::new(at(PAD) + at(6), top - at(SEP_H) / 2, item.width() - at(12), at(1)),
                    0,
                    line_soft(),
                );
            }
            if state.hover == index as i32 {
                canvas.round(item, at(8), RAISED);
            }
            let colour = if entry.warn { GOLD } else { CREAM };
            canvas.text(
                &entry.label,
                R { l: item.l + at(14), ..item },
                text_font,
                colour,
                DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
            );
            if entry.checked {
                canvas.text(
                    "on",
                    R { r: item.r - at(12), ..item },
                    tick_font,
                    LEAF,
                    DT_SINGLELINE | DT_VCENTER | DT_RIGHT | DT_NOPREFIX,
                );
            }
        }

        canvas.blit_to(dc, 0, 0);
        DeleteObject(text_font as HGDIOBJ);
        DeleteObject(tick_font as HGDIOBJ);
    });
    EndPaint(hwnd, &ps);
}

unsafe extern "system" fn wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_PAINT => {
            paint(hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_MOUSEMOVE => {
            let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
            STATE.with(|cell| {
                if let Some(state) = cell.borrow_mut().as_mut() {
                    let logical = (y as f32 / state.scale).round() as i32;
                    let found = index_at(logical, &state.entries);
                    if state.hover != found {
                        state.hover = found;
                        InvalidateRect(hwnd, std::ptr::null(), 0);
                    }
                }
            });
            0
        }
        WM_LBUTTONUP => {
            let x = (lparam & 0xFFFF) as i16 as i32;
            let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
            // With the mouse captured, a click is reported in this window's
            // coordinates wherever it lands. A click outside the menu
            // dismisses it; inside, it picks the item under the pointer.
            let mut rect: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut rect);
            let inside = x >= 0 && x < rect.right && y >= 0 && y < rect.bottom;
            STATE.with(|cell| {
                if let Some(state) = cell.borrow_mut().as_mut() {
                    if inside {
                        let logical = (y as f32 / state.scale).round() as i32;
                        state.chosen = index_at(logical, &state.entries);
                    }
                    state.done = true;
                }
            });
            0
        }
        WM_KEYDOWN if wparam as u16 == VK_ESCAPE => {
            STATE.with(|cell| {
                if let Some(state) = cell.borrow_mut().as_mut() {
                    state.done = true;
                }
            });
            0
        }
        WM_CAPTURECHANGED => {
            // The mouse capture was taken away - an alt-tab, another window
            // grabbing focus. The menu is done. This replaces a WM_ACTIVATE
            // dismiss, which fired before the menu had even appeared: a tool
            // window owned by the hidden tray window never truly activates,
            // so it read as "lost activation" on the first frame and closed
            // itself instantly.
            STATE.with(|cell| {
                if let Some(state) = cell.borrow_mut().as_mut() {
                    state.done = true;
                }
            });
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}
