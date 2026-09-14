//! The window with the accounts in it.
//!
//! One layout function builds a list of parts with their rectangles; the
//! painter draws that list and the mouse reads the same list back. There
//! is no second copy of the geometry, so a button cannot drift away from
//! the place that reacts to it - which is the way panels like this
//! usually break, and it breaks silently.
//!
//! Everything is laid out in logical pixels and multiplied by the
//! monitor's scale at the last moment, so the panel is the same size on a
//! 4K laptop as on a 1080p screen.

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TRACKMOUSEEVENT};

// Not in the crate's tables, and a missing constant in a match arm binds
// a name instead of failing, which silently swallows every later arm.
const TME_LEAVE: u32 = 0x0000_0002;
const WM_MOUSELEAVE: u32 = 0x02A3;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::app::{self, Capture};
use crate::draw::{font, wide, Canvas, Logo, R};
use crate::i18n::Lang;
use crate::keys::Bind;
use crate::store::NEXT;
use crate::theme::*;

const CLASS: &str = "DoSwitchPanel";
const WIDTH: i32 = 620;
const PAD: i32 = 22;
const ROW_HEIGHT: i32 = 62;
const ROW_GAP: i32 = 8;
const PILL_HEIGHT: i32 = 34;
const KEY_WIDTH: i32 = 160;
const ORDER_WIDTH: i32 = 90;
const SUBTITLE_TOP: i32 = 48;
const SUBTITLE_WIDTH: i32 = WIDTH - PAD * 2 - 66 - 114;

#[derive(Clone, PartialEq)]
pub enum Part {
    Row { character: String, breed: String },
    NextRow,
    KeyPill { owner: String },
    OrderPill { owner: String },
    Refresh,
    Done,
    Language,
}

pub struct Item {
    pub area: R,
    pub part: Part,
    /// Rows and pills are only live while the thing they name exists.
    pub live: bool,
}

struct Fonts {
    title: HFONT,
    subtitle: HFONT,
    column: HFONT,
    name: HFONT,
    small: HFONT,
    pill: HFONT,
    note: HFONT,
    button: HFONT,
}

impl Fonts {
    fn new(scale: f32) -> Fonts {
        let at = |size: f32| (size * scale).round() as i32;
        Fonts {
            title: font(at(18.0), FW_SEMIBOLD as i32),
            subtitle: font(at(12.0), FW_NORMAL as i32),
            column: font(at(11.0), FW_BOLD as i32),
            name: font(at(15.0), FW_SEMIBOLD as i32),
            small: font(at(12.0), FW_NORMAL as i32),
            pill: font(at(13.0), FW_SEMIBOLD as i32),
            note: font(at(12.0), FW_NORMAL as i32),
            button: font(at(13.0), FW_SEMIBOLD as i32),
        }
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        for handle in [
            self.title, self.subtitle, self.column, self.name, self.small,
            self.pill, self.note, self.button,
        ] {
            unsafe { DeleteObject(handle as HGDIOBJ) };
        }
    }
}

/// The two pieces of text that decide the panel's height. Both are
/// sentences, both wrap, and both are longer in French than in English,
/// so the window is sized from what they actually measure rather than
/// from a number that happened to fit the language it was written in.
#[derive(Clone, Copy)]
pub struct Text {
    pub header: i32,
    pub note: i32,
}

/// Where everything is, in logical pixels, plus the height it all needs.
fn layout(lang: Lang, rows: &[(String, String)], text: Text) -> (Vec<Item>, i32) {
    let mut items = Vec::new();
    let inner = WIDTH - PAD * 2;
    let key_x = WIDTH - PAD - 14 - KEY_WIDTH;
    let order_x = key_x - 20 - ORDER_WIDTH;

    items.push(Item {
        area: R::new(WIDTH - PAD - 102, 24, 102, 28),
        part: Part::Language,
        live: true,
    });

    let mut y = text.header + 36;
    if rows.is_empty() {
        y += 54; // the "nothing open" line sits where the first row would
    }
    for (character, breed) in rows {
        let area = R::new(PAD, y, inner, ROW_HEIGHT);
        items.push(Item {
            area,
            part: Part::Row {
                character: character.clone(),
                breed: breed.clone(),
            },
            live: true,
        });
        items.push(Item {
            area: R::new(order_x, y + (ROW_HEIGHT - PILL_HEIGHT) / 2, ORDER_WIDTH, PILL_HEIGHT),
            part: Part::OrderPill { owner: character.clone() },
            live: true,
        });
        items.push(Item {
            area: R::new(key_x, y + (ROW_HEIGHT - PILL_HEIGHT) / 2, KEY_WIDTH, PILL_HEIGHT),
            part: Part::KeyPill { owner: character.clone() },
            live: true,
        });
        y += ROW_HEIGHT + ROW_GAP;
    }

    let next_area = R::new(PAD, y, inner, ROW_HEIGHT);
    items.push(Item { area: next_area, part: Part::NextRow, live: true });
    items.push(Item {
        area: R::new(key_x, y + (ROW_HEIGHT - PILL_HEIGHT) / 2, KEY_WIDTH, PILL_HEIGHT),
        part: Part::KeyPill { owner: NEXT.to_string() },
        live: true,
    });
    y += ROW_HEIGHT + 16;

    // The note, then the line above the footer, then the footer itself.
    y += text.note + 18;
    items.push(Item {
        area: R::new(WIDTH - PAD - 96, y + 12, 96, 36),
        part: Part::Done,
        live: true,
    });
    items.push(Item {
        area: R::new(WIDTH - PAD - 96 - 12 - 108, y + 12, 108, 36),
        part: Part::Refresh,
        live: true,
    });
    let _ = lang;
    (items, y + 12 + 36 + 18)
}

fn scale_of(hwnd: HWND) -> f32 {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 {
        1.0
    } else {
        dpi as f32 / 96.0
    }
}

fn scaled(area: R, scale: f32) -> R {
    let at = |value: i32| (value as f32 * scale).round() as i32;
    R { l: at(area.l), t: at(area.t), r: at(area.r), b: at(area.b) }
}

/// Both wrapped blocks measured, in logical pixels. Measuring needs a
/// device context, so it happens against a throwaway one.
fn measure_text(scale: f32, lang: Lang) -> Text {
    let screen = unsafe { GetDC(std::ptr::null_mut()) };
    let canvas = Canvas::new(screen, 8, 8);
    let text = match canvas {
        Some(canvas) => {
            let fonts = Fonts::new(scale);
            let back = |value: i32| (value as f32 / scale).round() as i32;
            let forward = |value: i32| (value as f32 * scale).round() as i32;
            let note = back(canvas.wrapped_height(
                lang.note(),
                forward(WIDTH - PAD * 2 - 8),
                fonts.note,
            ));
            let subtitle = back(canvas.wrapped_height(
                lang.subtitle(),
                forward(SUBTITLE_WIDTH),
                fonts.subtitle,
            ));
            Text { header: (SUBTITLE_TOP + subtitle + 16).max(96), note }
        }
        None => Text { header: 96, note: 48 },
    };
    unsafe { ReleaseDC(std::ptr::null_mut(), screen) };
    text
}

fn rows_now() -> Vec<(String, String)> {
    app::with(|state| {
        app::ordered(state)
            .iter()
            .map(|client| (client.character.clone(), client.breed.clone()))
            .collect()
    })
}

pub fn create() -> HWND {
    unsafe {
        let instance =
            windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null());
        let class = wide(CLASS);
        let mut window_class: WNDCLASSW = std::mem::zeroed();
        window_class.lpfnWndProc = Some(wndproc);
        window_class.hInstance = instance;
        window_class.lpszClassName = class.as_ptr();
        window_class.hCursor = LoadCursorW(std::ptr::null_mut(), IDC_ARROW);
        window_class.hIcon = LoadIconW(instance, 1 as *const u16);
        RegisterClassW(&window_class);

        let hwnd = CreateWindowExW(
            WS_EX_APPWINDOW,
            class.as_ptr(),
            wide("DoSwitch").as_ptr(),
            WS_POPUP,
            0,
            0,
            100,
            100,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if !hwnd.is_null() {
            // Windows 11 rounds the corners for us when asked; on anything
            // older the attribute is simply refused and the panel is square,
            // which is a difference nobody has to be told about.
            let round: u32 = 2;
            DwmSetWindowAttribute(
                hwnd,
                33,
                &round as *const u32 as *const core::ffi::c_void,
                4,
            );
        }
        hwnd
    }
}

/// Size the window to its contents, centre it, and show it.
pub fn show(hwnd: HWND) {
    unsafe {
        app::refresh();
        let scale = scale_of(hwnd);
        let lang = app::language();
        let (_, height) = layout(lang, &rows_now(), measure_text(scale, lang));
        let width = (WIDTH as f32 * scale).round() as i32;
        let height = (height as f32 * scale).round() as i32;

        let mut work: RECT = std::mem::zeroed();
        SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut work as *mut RECT as *mut _, 0);
        let x = work.left + ((work.right - work.left) - width) / 2;
        let y = work.top + ((work.bottom - work.top) - height) / 2;

        SetWindowPos(hwnd, std::ptr::null_mut(), x, y, width, height, SWP_NOZORDER);
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        InvalidateRect(hwnd, std::ptr::null(), 0);
    }
}

pub fn hide(hwnd: HWND) {
    app::with(|state| state.capture = Capture::Nothing);
    unsafe { ShowWindow(hwnd, SW_HIDE) };
}

fn repaint(hwnd: HWND) {
    unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
}

fn pill_label(owner: &str, lang: Lang, capturing: bool) -> (String, u32) {
    if capturing {
        return (lang.press_a_key().to_string(), GOLD);
    }
    let bind = app::with(|state| {
        if owner == NEXT {
            state.settings.next
        } else {
            state.settings.key_of(owner)
        }
    });
    match bind {
        Some(bind) => (bind.name(), LEAF),
        None => (lang.unbound().to_string(), DIM),
    }
}

unsafe fn paint(hwnd: HWND) {
    let mut paint_struct: PAINTSTRUCT = std::mem::zeroed();
    let dc = BeginPaint(hwnd, &mut paint_struct);
    let scale = scale_of(hwnd);
    let lang = app::language();
    let rows = rows_now();
    let text = measure_text(scale, lang);
    let (items, logical_height) = layout(lang, &rows, text);
    let width = (WIDTH as f32 * scale).round() as i32;
    let height = (logical_height as f32 * scale).round() as i32;

    let Some(canvas) = Canvas::new(dc, width, height) else {
        EndPaint(hwnd, &paint_struct);
        return;
    };
    let fonts = Fonts::new(scale);
    let logo = Logo::new(dc);
    let at = |value: i32| (value as f32 * scale).round() as i32;
    let capture = app::with(|state| state.capture.clone());
    let hover = app::with(|state| state.hover);

    canvas.clear(INK);
    canvas.round(R::new(0, 0, width, at(text.header)), at(14), mix(INK, SURFACE, 0.55));
    canvas.round(R::new(0, at(text.header - 1), width, at(1)), 0, BORDER);

    // The header: logo, name of the panel, one line saying what it does.
    if let Some(logo) = logo.as_ref() {
        logo.draw(&canvas, scaled(R::new(PAD - 4, 18, 62, 58), scale));
    }
    canvas.text(
        lang.title(),
        scaled(R::new(PAD + 66, 22, 400, 26), scale),
        fonts.title,
        CREAM,
        DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
    );
    canvas.text(
        lang.subtitle(),
        scaled(
            R::new(PAD + 66, SUBTITLE_TOP, SUBTITLE_WIDTH, text.header - SUBTITLE_TOP),
            scale,
        ),
        fonts.subtitle,
        MUTED,
        DT_WORDBREAK | DT_NOPREFIX,
    );

    // The column headings, each over the thing it names.
    let key_x = WIDTH - PAD - 14 - KEY_WIDTH;
    let order_x = key_x - 20 - ORDER_WIDTH;
    let columns_top = text.header + 8;
    canvas.text(
        lang.column_character(),
        scaled(R::new(PAD + 18, columns_top, 240, 20), scale),
        fonts.column,
        LEAF,
        DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
    );
    canvas.text(
        lang.column_order(),
        scaled(R::new(order_x, columns_top, ORDER_WIDTH, 20), scale),
        fonts.column,
        MOSS,
        DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX,
    );
    canvas.text(
        lang.column_key(),
        scaled(R::new(key_x, columns_top, KEY_WIDTH, 20), scale),
        fonts.column,
        MOSS,
        DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX,
    );

    if rows.is_empty() {
        canvas.text(
            lang.nothing_open(),
            scaled(
                R::new(PAD + 18, text.header + 44, WIDTH - PAD * 2 - 36, 40),
                scale,
            ),
            fonts.small,
            MUTED,
            DT_WORDBREAK | DT_NOPREFIX,
        );
    }

    for (index, item) in items.iter().enumerate() {
        let area = scaled(item.area, scale);
        let hovered = hover == Some(index);
        match &item.part {
            Part::Row { character, breed } => {
                canvas.outline(area, at(12), at(1), BORDER,
                               if hovered { RAISED } else { SURFACE });
                canvas.dot(area.l + at(20), area.t + area.height() / 2, at(4), LEAF);
                canvas.text(
                    character,
                    R { l: area.l + at(36), t: area.t + at(10), r: area.r, b: area.t + at(32) },
                    fonts.name,
                    CREAM,
                    DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
                );
                canvas.text(
                    breed,
                    R { l: area.l + at(36), t: area.t + at(32), r: area.r, b: area.b - at(8) },
                    fonts.small,
                    MUTED,
                    DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
                );
            }
            Part::NextRow => {
                canvas.outline(area, at(12), at(1), mix(BORDER, GOLD, 0.35),
                               if hovered { RAISED } else { SURFACE });
                canvas.dot(area.l + at(20), area.t + area.height() / 2, at(4), GOLD);
                canvas.text(
                    lang.next_account(),
                    R { l: area.l + at(36), t: area.t + at(10), r: area.r, b: area.t + at(32) },
                    fonts.name,
                    GOLD,
                    DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
                );
                canvas.text(
                    lang.next_account_hint(),
                    R { l: area.l + at(36), t: area.t + at(32), r: area.r, b: area.b - at(8) },
                    fonts.small,
                    MUTED,
                    DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
                );
            }
            Part::OrderPill { owner } => {
                let capturing = capture == Capture::Order(owner.clone());
                let place = app::with(|state| {
                    state.settings.accounts.get(owner).and_then(|a| a.order)
                });
                let label = if capturing {
                    lang.type_a_number().to_string()
                } else {
                    place.map(|n| n.to_string()).unwrap_or_else(|| "-".into())
                };
                let edge = if capturing { GOLD } else { BORDER };
                canvas.outline(area, at(9), at(1), edge,
                               if hovered || capturing { RAISED } else { INK });
                canvas.text(
                    &label,
                    area,
                    if capturing { fonts.small } else { fonts.pill },
                    if capturing { GOLD } else { CREAM },
                    DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX,
                );
            }
            Part::KeyPill { owner } => {
                let capturing = capture == Capture::Key(owner.clone());
                let (label, colour) = pill_label(owner, lang, capturing);
                let accent = if owner == NEXT { GOLD } else { LEAF };
                let edge = if capturing {
                    GOLD
                } else {
                    mix(BORDER, accent, 0.30)
                };
                canvas.outline(area, at(9), at(1), edge,
                               if hovered || capturing { RAISED } else { INK });
                canvas.text(
                    &label,
                    area,
                    if capturing { fonts.small } else { fonts.pill },
                    colour,
                    DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX,
                );
            }
            Part::Language => {
                // Three segments, the live one in leaf. Clicking anywhere
                // on it moves to the next language round the ring, which
                // is one target instead of three and reads the same.
                canvas.outline(area, at(8), at(1), BORDER, if hovered { RAISED } else { INK });
                let third = area.width() / 3;
                for (index, one) in Lang::ALL.iter().enumerate() {
                    let slot = R {
                        l: area.l + third * index as i32,
                        r: area.l + third * (index as i32 + 1),
                        ..area
                    };
                    canvas.text(one.label(), slot, fonts.column,
                                if lang == *one { LEAF } else { DIM },
                                DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX);
                }
            }
            Part::Refresh | Part::Done => {
                let done = item.part == Part::Done;
                let edge = if done { mix(BORDER, LEAF, 0.6) } else { BORDER };
                let fill = if hovered { RAISED } else if done { mix(INK, MOSS, 0.5) } else { INK };
                canvas.outline(area, at(9), at(1), edge, fill);
                canvas.text(
                    if done { lang.done() } else { lang.refresh() },
                    area,
                    fonts.button,
                    if done { CREAM } else { MUTED },
                    DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX,
                );
            }
        }
    }

    // The note and the counts sit between the last row and the buttons.
    let note_top = items
        .iter()
        .find(|item| item.part == Part::Done)
        .map(|item| item.area.t)
        .unwrap_or(logical_height);
    canvas.text(
        lang.note(),
        scaled(
            R::new(PAD + 4, note_top - 16 - text.note, WIDTH - PAD * 2 - 8, text.note + 4),
            scale,
        ),
        fonts.note,
        DIM,
        DT_WORDBREAK | DT_NOPREFIX,
    );
    let bound = app::with(|state| {
        state
            .clients
            .iter()
            .filter(|client| state.settings.key_of(&client.character).is_some())
            .count()
    });
    canvas.text(
        &lang.counts(rows.len(), bound),
        scaled(R::new(PAD + 4, note_top + 12, 320, 36), scale),
        fonts.small,
        MUTED,
        DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
    );

    canvas.blit_to(dc, 0, 0);
    EndPaint(hwnd, &paint_struct);
}

fn hit(hwnd: HWND, x: i32, y: i32) -> Option<usize> {
    let scale = scale_of(hwnd);
    let lang = app::language();
    let (items, _) = layout(lang, &rows_now(), measure_text(scale, lang));
    // BACKWARDS, because that is the order they were drawn in. A row's
    // rectangle runs the full width and the pills sit ON it, so a search
    // from the front answers "the row" for every click on a pill - which
    // looks like the pills not working at all.
    items
        .iter()
        .rposition(|item| item.live && scaled(item.area, scale).holds(x, y))
}

fn part_at(hwnd: HWND, index: usize) -> Option<Part> {
    let scale = scale_of(hwnd);
    let lang = app::language();
    let (items, _) = layout(lang, &rows_now(), measure_text(scale, lang));
    items.get(index).map(|item| item.part.clone())
}

fn clicked(hwnd: HWND, part: Part) {
    match part {
        Part::Row { character, .. } => {
            app::switch_to(&character);
        }
        Part::KeyPill { owner } => {
            app::with(|state| state.capture = Capture::Key(owner));
        }
        Part::OrderPill { owner } => {
            app::with(|state| state.capture = Capture::Order(owner));
        }
        Part::NextRow => {
            app::switch_next();
        }
        Part::Refresh => app::refresh(),
        Part::Done => hide(hwnd),
        Part::Language => {
            app::with(|state| {
                state.settings.lang = state.settings.lang.other();
            });
            app::save();
        }
    }
    repaint(hwnd);
}

fn typed(hwnd: HWND, code: u32) {
    let capture = app::with(|state| state.capture.clone());
    match capture {
        Capture::Nothing => {}
        Capture::Key(owner) => {
            if code == VK_ESCAPE as u32 {
                app::with(|state| state.capture = Capture::Nothing);
            } else if code == VK_BACK as u32 || code == VK_DELETE as u32 {
                app::clear_key(&owner);
                app::with(|state| state.capture = Capture::Nothing);
            } else if !Bind::is_modifier_only(code) {
                let bind = Bind {
                    code,
                    ctrl: (unsafe { GetKeyState(VK_CONTROL as i32) } as u16 & 0x8000) != 0,
                    alt: (unsafe { GetKeyState(VK_MENU as i32) } as u16 & 0x8000) != 0,
                    shift: (unsafe { GetKeyState(VK_SHIFT as i32) } as u16 & 0x8000) != 0,
                };
                app::set_key(&owner, bind);
                app::with(|state| state.capture = Capture::Nothing);
            }
        }
        Capture::Order(owner) => {
            let digit = match code as u16 {
                0x31..=0x39 => Some(code as u16 - 0x30),
                VK_NUMPAD1..=VK_NUMPAD9 => Some(code as u16 - VK_NUMPAD0),
                _ => None,
            };
            match digit {
                Some(place) => {
                    app::set_order(&owner, place as u32);
                    app::with(|state| state.capture = Capture::Nothing);
                }
                None => {
                    if code == VK_ESCAPE as u32 {
                        app::with(|state| state.capture = Capture::Nothing);
                    }
                }
            }
        }
    }
    repaint(hwnd);
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
        WM_ERASEBKGND => 1, // the whole surface is painted every time
        WM_MOUSEMOVE => {
            let x = (lparam & 0xFFFF) as i16 as i32;
            let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
            let found = hit(hwnd, x, y);
            let changed = app::with(|state| {
                let changed = state.hover != found;
                state.hover = found;
                changed
            });
            if changed {
                let mut track: TRACKMOUSEEVENT = std::mem::zeroed();
                track.cbSize = std::mem::size_of::<TRACKMOUSEEVENT>() as u32;
                track.dwFlags = TME_LEAVE;
                track.hwndTrack = hwnd;
                TrackMouseEvent(&mut track);
                repaint(hwnd);
            }
            0
        }
        WM_MOUSELEAVE => {
            app::with(|state| state.hover = None);
            repaint(hwnd);
            0
        }
        WM_LBUTTONDOWN => {
            let x = (lparam & 0xFFFF) as i16 as i32;
            let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
            if let Some(index) = hit(hwnd, x, y) {
                if let Some(part) = part_at(hwnd, index) {
                    clicked(hwnd, part);
                }
            } else {
                app::with(|state| state.capture = Capture::Nothing);
                repaint(hwnd);
            }
            0
        }
        WM_NCHITTEST => {
            // The header is the handle: dragging it moves the panel, which
            // a window with no title bar otherwise cannot do.
            let scale = scale_of(hwnd);
            let mut point = POINT {
                x: (lparam & 0xFFFF) as i16 as i32,
                y: ((lparam >> 16) & 0xFFFF) as i16 as i32,
            };
            ScreenToClient(hwnd, &mut point);
            let header = (96.0 * scale) as i32;
            if point.y < header && hit(hwnd, point.x, point.y).is_none() {
                HTCAPTION as LRESULT
            } else {
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            typed(hwnd, wparam as u32);
            0
        }
        WM_KILLFOCUS => {
            app::with(|state| state.capture = Capture::Nothing);
            repaint(hwnd);
            0
        }
        WM_CLOSE => {
            hide(hwnd);
            0
        }
        WM_DPICHANGED => {
            show(hwnd);
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

/// True when this window is the panel, used by the tray to tell whether a
/// click should open or close it.
pub fn is_open(hwnd: HWND) -> bool {
    !hwnd.is_null() && unsafe { IsWindowVisible(hwnd) != 0 }
}
