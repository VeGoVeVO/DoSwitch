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
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SetCapture, ReleaseCapture};

// Not in the crate's tables, and a missing constant in a match arm binds
// a name instead of failing, which silently swallows every later arm.
const TME_LEAVE: u32 = 0x0000_0002;
const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_MOUSEWHEEL: u32 = 0x020A;
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
// The list shows at most this many accounts and scrolls the rest, so
// the window is a fixed, sensible size whether a player runs two
// clients or twenty. Six is what fits without the panel feeling tall.
const MAX_VISIBLE_ROWS: usize = 6;
// The scrollbar's gutter on the right of the list, present only when
// there is something to scroll.
const SCROLLBAR_GUTTER: i32 = 16;
const SCROLLBAR_WIDTH: i32 = 6;
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

/// Where the scrollbar is and how far it can go, when the account
/// list is longer than MAX_VISIBLE_ROWS. None when everything fits.
#[derive(Clone, Copy)]
pub struct Scroll {
    pub track: R,
    pub thumb: R,
    pub max: usize,
}

/// Where everything is, in logical pixels, plus the height it all needs
/// and the scrollbar when the list is longer than the panel shows.
///
/// The account rows live in a fixed viewport of at most MAX_VISIBLE_ROWS;
/// `scroll` (a whole-row offset) chooses which slice is drawn. Everything
/// below the viewport - the Next account row, the note, the footer - is
/// pinned, so the window is one fixed size no matter how many accounts
/// are open. This is also why toggling to a wordier language cannot push
/// the footer off the bottom the way it once did.
fn layout(lang: Lang, rows: &[(String, String)], text: Text, scroll: usize)
          -> (Vec<Item>, i32, Option<Scroll>) {
    let mut items = Vec::new();
    let overflowing = rows.len() > MAX_VISIBLE_ROWS;
    let gutter = if overflowing { SCROLLBAR_GUTTER } else { 0 };
    let inner = WIDTH - PAD * 2 - gutter;
    let key_x = WIDTH - PAD - 14 - gutter - KEY_WIDTH;
    let order_x = key_x - 20 - ORDER_WIDTH;

    items.push(Item {
        area: R::new(WIDTH - PAD - 102, 24, 102, 28),
        part: Part::Language,
        live: true,
    });

    let list_top = text.header + 36;
    let visible = rows.len().min(MAX_VISIBLE_ROWS);
    let scroll = scroll.min(max_scroll(rows.len()));
    let mut y = list_top;
    if rows.is_empty() {
        y += 54; // the "nothing open" line sits where the first row would
    }
    for slot in 0..visible {
        let (character, breed) = &rows[scroll + slot];
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

    // The scrollbar spans exactly the rows on screen, so its length reads
    // as "this much of the list is showing" at a glance.
    let scrollbar = if overflowing {
        let span = visible as i32 * (ROW_HEIGHT + ROW_GAP) - ROW_GAP;
        let track = R::new(WIDTH - PAD - SCROLLBAR_WIDTH, list_top, SCROLLBAR_WIDTH, span);
        let thumb_h = (span * visible as i32 / rows.len() as i32).max(28);
        let steps = max_scroll(rows.len()).max(1) as i32;
        let thumb_y = track.t + (span - thumb_h) * scroll as i32 / steps;
        let thumb = R::new(track.l, thumb_y, SCROLLBAR_WIDTH, thumb_h);
        Some(Scroll { track, thumb, max: max_scroll(rows.len()) })
    } else {
        None
    };

    let next_area = R::new(PAD, y, WIDTH - PAD * 2 - gutter, ROW_HEIGHT);
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
    // Room under the buttons for the build line, which paint draws bottom
    // right on its own baseline rather than over the Done button.
    (items, y + 12 + 36 + 30, scrollbar)
}

/// The furthest the list can be scrolled, in whole rows.
fn max_scroll(row_count: usize) -> usize {
    row_count.saturating_sub(MAX_VISIBLE_ROWS)
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

/// The window's pixel size for the current language and account count.
/// Deterministic, so show and resize always agree.
unsafe fn window_size(hwnd: HWND) -> (i32, i32) {
    let scale = scale_of(hwnd);
    let lang = app::language();
    let scroll = app::with(|state| state.scroll);
    let (_, height, _) = layout(lang, &rows_now(), measure_text(scale, lang), scroll);
    (
        (WIDTH as f32 * scale).round() as i32,
        (height as f32 * scale).round() as i32,
    )
}

/// Resize the window to fit its current contents WITHOUT moving it, then
/// repaint. Called whenever the contents change height under the panel -
/// a language toggle, a refresh - so the footer can never be pushed off
/// the bottom the way a wordier language once did.
pub fn resize(hwnd: HWND) {
    unsafe {
        let (width, height) = window_size(hwnd);
        SetWindowPos(
            hwnd, std::ptr::null_mut(), 0, 0, width, height,
            SWP_NOMOVE | SWP_NOZORDER,
        );
        InvalidateRect(hwnd, std::ptr::null(), 0);
    }
}

/// Size the window to its contents, centre it, and show it.
pub fn show(hwnd: HWND) {
    unsafe {
        app::refresh();
        app::with(|state| state.scroll = state.scroll.min(max_scroll(state.clients.len())));
        let (width, height) = window_size(hwnd);

        let mut work: RECT = std::mem::zeroed();
        SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut work as *mut RECT as *mut _, 0);
        let x = work.left + ((work.right - work.left) - width) / 2;
        let y = work.top + ((work.bottom - work.top) - height) / 2;

        // Raise to the top (no SWP_NOZORDER) and force the foreground through
        // the lock, so a panel shown while the player is in another window - a
        // second launch posting "show the panel", a tray click - actually
        // appears instead of opening behind that window and looking like
        // nothing happened.
        SetWindowPos(hwnd, std::ptr::null_mut(), x, y, width, height, SWP_SHOWWINDOW);
        ShowWindow(hwnd, SW_SHOW);
        crate::clients::to_foreground(hwnd);
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
    let scroll = app::with(|state| state.scroll);
    let (items, logical_height, scrollbar) = layout(lang, &rows, text, scroll);
    // Paint the WHOLE client rect, not the height the layout worked out.
    // The window is sized to that height elsewhere, but the two paths round
    // the DPI scale independently and a mid-animation resize can arrive
    // before the size settles - so the client can be a pixel or two taller
    // than the layout says. The class brush erases nothing (WM_ERASEBKGND is
    // a no-op on the promise that paint covers everything), so any strip the
    // canvas fails to cover is left showing whatever was behind the window:
    // the desktop, a terminal, the account it just left. Sizing the canvas
    // to the real client rect keeps that promise true - clear(INK) fills
    // every pixel the window owns and the blit copies every one back.
    let mut rc: RECT = std::mem::zeroed();
    GetClientRect(hwnd, &mut rc);
    let width = (rc.right - rc.left).max(1);
    let height = (rc.bottom - rc.top).max(1);

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
    // The card outline, a faint leaf line the whole way round.
    canvas.outline(R::new(0, 0, width, height), at(14), at(1), line(), INK);
    // The header: a vertical raised-to-panel gradient with a leaf glow
    // spilling from the top-left corner, exactly the web panel's header.
    let head = R::new(1, 1, width - 2, at(text.header) - 1);
    canvas.vgradient(head, RAISED, INK);
    canvas.glow(at(2), at(2), at(text.header) + at(40), LEAF, 0.14);
    canvas.round(R::new(0, at(text.header - 1), width, at(1)), 0, line_soft());

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
        LEAF_DIM,
        DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
    );
    canvas.text(
        lang.column_order(),
        scaled(R::new(order_x, columns_top, ORDER_WIDTH, 20), scale),
        fonts.column,
        LEAF_DIM,
        DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX,
    );
    canvas.text(
        lang.column_key(),
        scaled(R::new(key_x, columns_top, KEY_WIDTH, 20), scale),
        fonts.column,
        LEAF_DIM,
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
                canvas.outline(area, at(10), at(1), line_live(),
                               if hovered { RAISED } else { SURFACE });
                canvas.lit_dot(area.l + at(20), area.t + area.height() / 2, at(4), LEAF);
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
                canvas.outline(area, at(10), at(1), line_gold(),
                               if hovered { RAISED } else { SURFACE });
                canvas.lit_dot(area.l + at(20), area.t + area.height() / 2, at(4), GOLD);
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
                let edge = if capturing { GOLD } else { line_soft() };
                canvas.outline(area, at(8), at(1), edge,
                               if hovered || capturing { RAISED } else { PILL });
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
                let has_key = colour == LEAF || colour == GOLD;
                let edge = if capturing {
                    GOLD
                } else if has_key {
                    mix(SURFACE, accent, 0.30)
                } else {
                    line_soft()
                };
                canvas.outline(area, at(8), at(1), edge,
                               if hovered || capturing { RAISED } else { PILL });
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
                canvas.outline(area, at(8), at(1), line_soft(), if hovered { RAISED } else { PILL });
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
                let edge = if done { mix(SURFACE, LEAF, 0.3) } else { line_soft() };
                let fill = if hovered { RAISED } else { PILL };
                canvas.outline(area, at(8), at(1), edge, fill);
                canvas.text(
                    if done { lang.done() } else { lang.refresh() },
                    area,
                    fonts.button,
                    if done { LEAF } else { MUTED },
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

    // The scrollbar, drawn by hand in the palette so it belongs to the
    // panel rather than to Windows: a faint track, a leaf thumb that is
    // brighter while the pointer is over it.
    if let Some(bar) = scrollbar {
        let track = scaled(bar.track, scale);
        canvas.round(track, track.width() / 2, mix(INK, LEAF, 0.16));
        let thumb = scaled(bar.thumb, scale);
        let over = hover == Some(usize::MAX);
        canvas.round(thumb, thumb.width() / 2, if over { LEAF } else { mix(MOSS, LEAF, 0.5) });
    }

    // The build, small and dim in the corner.
    // The build, small and dim, bottom right on its own baseline BELOW the
    // buttons - not over the Done button, where it used to collide.
    let version = concat!("v", env!("DOSWITCH_VERSION"));
    canvas.text(
        version,
        scaled(R::new(WIDTH - PAD - 160, note_top + 42, 156, 18), scale),
        fonts.small,
        DIM,
        DT_SINGLELINE | DT_VCENTER | DT_RIGHT | DT_NOPREFIX,
    );

    canvas.blit_to(dc, 0, 0);
    EndPaint(hwnd, &paint_struct);
}

/// The scrollbar for the panel as it is right now, or None when the list
/// fits. Recomputed rather than cached so it always matches what is drawn.
fn scrollbar_now(hwnd: HWND) -> Option<Scroll> {
    let scale = scale_of(hwnd);
    let lang = app::language();
    let scroll = app::with(|state| state.scroll);
    let (_, _, bar) = layout(lang, &rows_now(), measure_text(scale, lang), scroll);
    let _ = scale;
    bar
}

/// Turn a pointer y (device pixels) into a scroll offset while dragging.
fn drag_scroll_to(hwnd: HWND, y: i32) {
    let scale = scale_of(hwnd);
    let Some(bar) = scrollbar_now(hwnd) else { return };
    let track = scaled(bar.track, scale);
    let thumb_h = scaled(bar.thumb, scale).height();
    let grab = app::with(|state| state.drag_grab);
    let span = (track.height() - thumb_h).max(1);
    let offset = (y - track.t - grab).clamp(0, span);
    let target = (offset as i64 * bar.max as i64 / span as i64) as usize;
    let changed = app::with(|state| {
        let was = state.scroll;
        state.scroll = target.min(bar.max);
        was != state.scroll
    });
    if changed {
        repaint(hwnd);
    }
}

fn hit(hwnd: HWND, x: i32, y: i32) -> Option<usize> {
    let scale = scale_of(hwnd);
    let lang = app::language();
    let scroll = app::with(|state| state.scroll);
    let (items, _, _) = layout(lang, &rows_now(), measure_text(scale, lang), scroll);
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
    let scroll = app::with(|state| state.scroll);
    let (items, _, _) = layout(lang, &rows_now(), measure_text(scale, lang), scroll);
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
        Part::Refresh => {
            app::refresh();
            app::with(|state| state.scroll = state.scroll.min(max_scroll(state.clients.len())));
            resize(hwnd);
            return;
        }
        Part::Done => hide(hwnd),
        Part::Language => {
            app::with(|state| {
                state.settings.lang = state.settings.lang.other();
            });
            app::save();
            // The subtitle and note are longer in some languages, so the
            // window's height changes with the language.
            resize(hwnd);
            return;
        }
    }
    repaint(hwnd);
}

/// Write a mouse button down as the bind being captured.
///
/// True when it was taken. A button is a virtual key code like any other -
/// Windows gives the five it reports their own - so this is the same
/// recording typed() does, without the modifier-only question that only a
/// keyboard can ask.
fn bind_pressed(hwnd: HWND, button: u32) -> bool {
    let capture = app::with(|state| state.capture.clone());
    let Capture::Key(owner) = capture else {
        return false;
    };
    let bind = Bind {
        code: button,
        ctrl: (unsafe { GetKeyState(VK_CONTROL as i32) } as u16 & 0x8000) != 0,
        alt: (unsafe { GetKeyState(VK_MENU as i32) } as u16 & 0x8000) != 0,
        shift: (unsafe { GetKeyState(VK_SHIFT as i32) } as u16 & 0x8000) != 0,
    };
    app::set_key(&owner, bind);
    app::with(|state| state.capture = Capture::Nothing);
    repaint(hwnd);
    true
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
        WM_MOUSEWHEEL => {
            // One notch, one row. The list is the only thing that scrolls,
            // and it scrolls in whole rows so nothing is ever half shown.
            let delta = ((wparam >> 16) & 0xFFFF) as i16;
            let rows = rows_now().len();
            app::with(|state| {
                let ceiling = max_scroll(rows);
                if delta > 0 {
                    state.scroll = state.scroll.saturating_sub(1);
                } else if delta < 0 {
                    state.scroll = (state.scroll + 1).min(ceiling);
                }
            });
            repaint(hwnd);
            0
        }
        WM_MOUSEMOVE => {
            let x = (lparam & 0xFFFF) as i16 as i32;
            let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
            // Dragging the thumb: the pointer's position in the track maps
            // straight to a row offset.
            if app::with(|state| state.dragging) {
                drag_scroll_to(hwnd, y);
                return 0;
            }
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
        // A mouse button IS a bind, so while the panel is waiting for one
        // these press messages are the answer rather than a click on the
        // window. Left is the exception: it is also how the cell being
        // bound was opened in the first place, so it only counts when it
        // lands back inside that same cell - anywhere else it stays an
        // ordinary click, which is what cancels the capture.
        WM_MBUTTONDOWN => {
            bind_pressed(hwnd, VK_MBUTTON as u32);
            0
        }
        WM_RBUTTONDOWN => {
            bind_pressed(hwnd, VK_RBUTTON as u32);
            0
        }
        WM_XBUTTONDOWN => {
            let button = match ((wparam >> 16) & 0xFFFF) as u16 {
                XBUTTON1 => VK_XBUTTON1,
                XBUTTON2 => VK_XBUTTON2,
                _ => return 0,
            };
            bind_pressed(hwnd, button as u32);
            // TRUE, because a program that handles WM_XBUTTONDOWN says so.
            1
        }

        WM_LBUTTONDOWN => {
            let x = (lparam & 0xFFFF) as i16 as i32;
            let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
            if let Capture::Key(owner) = app::with(|state| state.capture.clone()) {
                let on_its_own_pill = hit(hwnd, x, y)
                    .and_then(|index| part_at(hwnd, index))
                    .is_some_and(|part| part == Part::KeyPill { owner: owner.clone() });
                if on_its_own_pill && bind_pressed(hwnd, VK_LBUTTON as u32) {
                    return 0;
                }
            }
            // The scrollbar first: a press on the thumb starts a drag, a
            // press on the track above or below it pages towards the click.
            if let Some(bar) = scrollbar_now(hwnd) {
                let scale = scale_of(hwnd);
                let thumb = scaled(bar.thumb, scale);
                let track = scaled(bar.track, scale);
                if thumb.holds(x, y) {
                    let grab = y - thumb.t;
                    app::with(|state| { state.dragging = true; state.drag_grab = grab; });
                    SetCapture(hwnd);
                    return 0;
                }
                if track.holds(x, y) {
                    let rows = rows_now().len();
                    app::with(|state| {
                        let ceiling = max_scroll(rows);
                        if y < thumb.t {
                            state.scroll = state.scroll.saturating_sub(MAX_VISIBLE_ROWS);
                        } else {
                            state.scroll = (state.scroll + MAX_VISIBLE_ROWS).min(ceiling);
                        }
                    });
                    repaint(hwnd);
                    return 0;
                }
            }
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
        WM_LBUTTONUP => {
            if app::with(|state| state.dragging) {
                app::with(|state| state.dragging = false);
                ReleaseCapture();
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
