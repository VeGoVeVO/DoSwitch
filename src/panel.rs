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
// 720 rather than the 620 this was before the initiative column: a fourth
// column taken out of the old width left the character name about 150
// logical pixels, which is short enough to clip an ordinary Dofus name.
const WIDTH: i32 = 720;
const PAD: i32 = 22;
const ROW_HEIGHT: i32 = 62;
const ROW_GAP: i32 = 8;
const PILL_HEIGHT: i32 = 34;
const KEY_WIDTH: i32 = 160;
const ORDER_WIDTH: i32 = 90;
const INIT_WIDTH: i32 = 92;
/// The rounded tile at the head of every row. A 24px emblem hung on its
/// own beside two lines of text had nothing to sit on: it floated in the
/// gap between the name and the breed and read as something dropped on
/// the row rather than part of it. A badge gives it a surface, squares it
/// up against the two text lines, and - because the Next-account and
/// auto-switch rows get the same badge with their status dot inside it -
/// puts every row's text on one left rail instead of two.
const BADGE: i32 = 40;
/// Where a row's text starts: past the badge, plus a gap.
const ROW_TEXT: i32 = 11 + BADGE + 12;
// The list shows at most this many accounts and scrolls the rest, so
// the window is a fixed, sensible size whether a player runs two
// clients or twenty. Six is what fits without the panel feeling tall.
const MAX_VISIBLE_ROWS: usize = 6;
// The scrollbar's gutter on the right of the list, present only when
// there is something to scroll.
const SCROLLBAR_GUTTER: i32 = 16;
const SCROLLBAR_WIDTH: i32 = 6;
const SUBTITLE_TOP: i32 = 48;
const SUBTITLE_WIDTH: i32 = WIDTH - PAD * 2 - 122 - 114;

#[derive(Clone, PartialEq)]
pub enum Part {
    Row { character: String, breed: String },
    NextRow,
    KeyPill { owner: String },
    OrderPill { owner: String },
    /// The character's initiative, typed in by hand. Optional: unset is a
    /// dash and changes nothing.
    InitPill { owner: String },
    /// The INITIATIVE heading, which is also the button that rewrites the
    /// order from those numbers. A column of numbers answers "how fast is
    /// this one"; the question actually being asked is "who plays first",
    /// and only the list being IN that order answers it at a glance.
    SortInitiative,
    Refresh,
    Done,
    /// The header toggle for self-updates, where the language switcher used
    /// to be (the language is chosen in the installer now).
    AutoUpdate,
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
    let init_x = order_x - 20 - INIT_WIDTH;

    items.push(Item {
        area: R::new(WIDTH - PAD - 118, 24, 118, 28),
        part: Part::AutoUpdate,
        live: true,
    });

    // The heading doubles as the sort button, so it is an item with a
    // rectangle rather than text paint puts wherever it likes - the click
    // and the word are then the same pixels by construction. Dead until
    // two of the characters on screen have a number, because sorting one
    // of them against nothing is a click that appears to do nothing.
    let numbered = rows
        .iter()
        .filter(|(character, _)| {
            app::with(|state| {
                state
                    .settings
                    .accounts
                    .get(character)
                    .and_then(|account| account.initiative)
                    .is_some()
            })
        })
        .count();
    items.push(Item {
        area: R::new(init_x, text.header + 8, INIT_WIDTH, 20),
        part: Part::SortInitiative,
        live: numbered >= 2,
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
            area: R::new(init_x, y + (ROW_HEIGHT - PILL_HEIGHT) / 2, INIT_WIDTH, PILL_HEIGHT),
            part: Part::InitPill { owner: character.clone() },
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

/// The badge rectangle at the head of a row, in device pixels.
fn badge_of(area: R, scale: f32) -> R {
    let at = |value: i32| (value as f32 * scale).round() as i32;
    let size = at(BADGE);
    R::new(area.l + at(11), area.t + (area.height() - size) / 2, size, size)
}

fn scale_of(hwnd: HWND) -> f32 {
    // A forced scale, for the marketing snapshot (--panel-snapshot): the
    // screenshots the README ships have to be the same picture on a 96-DPI CI
    // runner as on the author's desk, and DPI is the one input the machine
    // decides rather than the code. Nothing sets this in a normal run, so a
    // player's panel is still sized by their own monitor.
    if let Ok(forced) = std::env::var("DOSWITCH_UI_SCALE") {
        if let Ok(value) = forced.parse::<f32>() {
            if value > 0.0 {
                return value;
            }
        }
    }
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
            Text { header: (SUBTITLE_TOP + subtitle + 16).max(100), note }
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

/// Photograph the accounts panel into a bottom-up 32bpp BMP, for the README
/// screenshots. The panel is filled with invented accounts (DOSWITCH_FAKE)
/// and a seeded order/keys by the caller (main.rs --panel-snapshot), so a
/// shot taken here never carries a real character's name - which is the
/// whole reason it exists rather than a capture taken by hand. Same path a
/// PrintWindow snapshot always uses: create OUR window, pin it to the
/// top-left ON screen so PrintWindow has something composited to copy (an
/// off-screen window is not composited and copies out black), pump, then
/// read the DIB straight out. tools/panel_shots.py turns the BMP into the
/// PNG the README serves.
pub fn snapshot(path: &str) -> bool {
    unsafe {
        // Never photograph a panel owned by another copy of the app: the
        // class name is shared, so a running instance would be captured
        // instead of ours.
        let existing = FindWindowW(wide(CLASS).as_ptr(), std::ptr::null());
        if !existing.is_null() {
            let mut owner = 0u32;
            GetWindowThreadProcessId(existing, &mut owner);
            if owner != windows_sys::Win32::System::Threading::GetCurrentProcessId() {
                std::process::exit(6);
            }
        }

        let hwnd = create();
        if hwnd.is_null() {
            return false;
        }
        app::refresh();
        app::with(|state| state.scroll = state.scroll.min(max_scroll(state.clients.len())));
        let (width, height) = window_size(hwnd);
        SetWindowPos(hwnd, std::ptr::null_mut(), 0, 0, width, height, SWP_SHOWWINDOW);
        ShowWindow(hwnd, SW_SHOW);
        InvalidateRect(hwnd, std::ptr::null(), 0);

        // Pump so the window actually paints before it is captured.
        let mut msg: MSG = std::mem::zeroed();
        for _ in 0..24 {
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::thread::sleep(std::time::Duration::from_millis(8));
        }

        // The CLIENT rect, not the window rect: the painter fills the client
        // rect and promises nothing outside it (WM_ERASEBKGND is a no-op on
        // exactly that promise), so a capture sized to anything larger copies
        // rows nobody ever wrote - which come out pure black, because that is
        // what a fresh DIB section holds.
        //
        let mut rc: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut rc);
        let width = rc.right - rc.left;
        let height = rc.bottom - rc.top;
        if width <= 0 || height <= 0 {
            return false;
        }

        let screen = GetDC(std::ptr::null_mut());
        let memory = CreateCompatibleDC(screen);
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width;
        info.bmiHeader.biHeight = height;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB as u32;
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bitmap = CreateDIBSection(memory, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        if bitmap.is_null() || bits.is_null() {
            ReleaseDC(std::ptr::null_mut(), screen);
            DeleteDC(memory);
            return false;
        }
        let old = SelectObject(memory, bitmap as HGDIOBJ);
        // Ask the window to draw itself into our bitmap, rather than copying
        // it off the desktop. PrintWindow can only hand back pixels the
        // desktop actually holds, so a window TALLER THAN THE SCREEN loses
        // the part hanging off the bottom - it comes back as the black a
        // fresh DIB section starts as. That is not a theory: GitHub's Windows
        // runners are 1024x768, this panel is 788 tall, and the screenshots
        // it shipped to the site had exactly twenty black rows along their
        // bottom edge. The free panel is 690 tall, fit, and looked perfect,
        // which is why only one of the two repos was ever wrong.
        //
        // WM_PRINTCLIENT runs the panel's own painter straight into this DC.
        // It has no idea where the window is, whether anything covers it, or
        // how big the screen is.
        SendMessageW(hwnd, WM_PRINTCLIENT, memory as WPARAM, PRF_CLIENT as LPARAM);

        let size = (width * height * 4) as usize;
        let pixels = std::slice::from_raw_parts(bits as *const u8, size);

        // Refuse to write a picture with a strip nobody painted. The panel
        // clears to INK and paints over it, so no pixel it draws is pure
        // black; a whole row of it means the capture caught ground the
        // painter never covered. A snapshot like that once went out to the
        // site with twenty black rows along its bottom edge and stayed
        // there, because everything reported success and the corners were
        // even rounded - out of the black.
        let unpainted = (0..height).any(|row| {
            let from = (row * width * 4) as usize;
            pixels[from..from + (width * 4) as usize]
                .chunks_exact(4)
                .all(|px| px[0] == 0 && px[1] == 0 && px[2] == 0)
        });
        if unpainted {
            SelectObject(memory, old);
            DeleteObject(bitmap as HGDIOBJ);
            DeleteDC(memory);
            ReleaseDC(std::ptr::null_mut(), screen);
            return false;
        }

        let stride = (width * 4) as u32;
        let mut file: Vec<u8> = Vec::with_capacity(size + 54);
        file.extend_from_slice(b"BM");
        file.extend_from_slice(&((54 + size) as u32).to_le_bytes());
        file.extend_from_slice(&0u32.to_le_bytes());
        file.extend_from_slice(&54u32.to_le_bytes());
        file.extend_from_slice(&40u32.to_le_bytes());
        file.extend_from_slice(&width.to_le_bytes());
        file.extend_from_slice(&height.to_le_bytes());
        file.extend_from_slice(&1u16.to_le_bytes());
        file.extend_from_slice(&32u16.to_le_bytes());
        file.extend_from_slice(&0u32.to_le_bytes());
        file.extend_from_slice(&(size as u32).to_le_bytes());
        file.extend_from_slice(&stride.to_le_bytes());
        file.extend_from_slice(&stride.to_le_bytes());
        file.extend_from_slice(&0u32.to_le_bytes());
        file.extend_from_slice(&0u32.to_le_bytes());
        file.extend_from_slice(pixels);

        SelectObject(memory, old);
        DeleteObject(bitmap as HGDIOBJ);
        DeleteDC(memory);
        ReleaseDC(std::ptr::null_mut(), screen);

        std::fs::write(path, &file).is_ok()
    }
}

pub fn hide(hwnd: HWND) {
    end_capture();
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
    paint_to(hwnd, dc);
    EndPaint(hwnd, &paint_struct);
}

/// Draw the whole panel into ANY device context.
///
/// Split out of paint so a snapshot can ask for the picture directly
/// (WM_PRINTCLIENT) instead of photographing the screen. PrintWindow can
/// only copy what the desktop holds, and a window taller than the desktop
/// has no pixels for the part hanging off it: on a 1024x768 CI runner the
/// 788px Pro panel came back with its bottom twenty rows pure black, while
/// the shorter free panel fit and looked perfect. That is the whole reason
/// one repo's screenshots were fine and the other's had a black band.
unsafe fn paint_to(hwnd: HWND, dc: HDC) {
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
        return;
    };
    let fonts = Fonts::new(scale);
    let logo = Logo::new(dc);
    let at = |value: i32| (value as f32 * scale).round() as i32;
    let capture = app::with(|state| state.capture.clone());
    let hover = app::with(|state| state.hover);

    canvas.clear(INK);
    // Two faint lights behind the card - the site shader's resting palette as
    // two cheap radial glows: lime from the top-left, teal from the
    // bottom-right, with a cool spark low-left. Painted once per repaint,
    // nothing animated - the very-lightweight stand-in for the site's WebGL
    // background, dim enough that the rows and text keep their contrast.
    canvas.glow(width * 16 / 100, height * 10 / 100, (width * 95 / 100).max(1), GLOW_LIME, 0.16);
    canvas.glow(width * 88 / 100, height * 86 / 100, (width * 95 / 100).max(1), GLOW_TEAL, 0.22);
    canvas.glow(width * 8 / 100, height * 70 / 100, (width * 60 / 100).max(1), GLOW_TEAL, 0.12);
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
        logo.draw(&canvas, scaled(R::new(PAD - 6, 12, 118, 74), scale));
    }
    canvas.text(
        lang.title(),
        scaled(R::new(PAD + 122, 22, 300, 26), scale),
        fonts.title,
        CREAM,
        DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
    );
    canvas.text(
        lang.subtitle(),
        scaled(
            R::new(PAD + 122, SUBTITLE_TOP, SUBTITLE_WIDTH, text.header - SUBTITLE_TOP),
            scale,
        ),
        fonts.subtitle,
        MUTED,
        DT_WORDBREAK | DT_NOPREFIX,
    );

    // The column headings, each over the thing it names. The gutter is
    // subtracted here exactly as layout subtracts it: without it the
    // headings stayed put while the pills slid left to make room for the
    // scrollbar, so from the seventh account on, every heading sat a
    // scrollbar's width to the right of its own column.
    let gutter = if rows.len() > MAX_VISIBLE_ROWS { SCROLLBAR_GUTTER } else { 0 };
    let key_x = WIDTH - PAD - 14 - gutter - KEY_WIDTH;
    let order_x = key_x - 20 - ORDER_WIDTH;
    let init_x = order_x - 20 - INIT_WIDTH;
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
                // The class portrait, filling the badge, with a hairline
                // over it so it reads as a tile rather than as a picture
                // floating on the row. A breed the table does not know
                // gets the plain dot back in an empty badge: a wrong face
                // beside a name would look like a reading, and there is
                // nothing about it that looks wrong.
                let badge = badge_of(area, scale);
                if crate::emblem::draw(&canvas, badge, at(12), breed) {
                    canvas.ring(badge, at(12), at(1), line_soft());
                } else {
                    canvas.outline(badge, at(12), at(1), line_soft(), PILL);
                    canvas.lit_dot(
                        badge.l + badge.width() / 2,
                        badge.t + badge.height() / 2,
                        at(6),
                        LEAF,
                    );
                }
                // Stopped short of the first pill. The name is the game's
                // and can be long; running it under the numbers is how a
                // wide column quietly becomes an unreadable one.
                let text_right = at(init_x - 12);
                canvas.text(
                    character,
                    R { l: area.l + at(ROW_TEXT), t: area.t + at(10), r: text_right, b: area.t + at(32) },
                    fonts.name,
                    CREAM,
                    DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS,
                );
                canvas.text(
                    breed,
                    R { l: area.l + at(ROW_TEXT), t: area.t + at(32), r: text_right, b: area.b - at(8) },
                    fonts.small,
                    MUTED,
                    DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS,
                );
            }
            Part::NextRow => {
                canvas.outline(area, at(10), at(1), line_gold(),
                               if hovered { RAISED } else { SURFACE });
                // The same badge the account rows wear, with the gold dot
                // inside it - so the whole list reads down one left rail
                // and every row's text starts at the same x.
                let badge = badge_of(area, scale);
                canvas.outline(badge, at(12), at(1), mix(SURFACE, GOLD, 0.30), PILL);
                canvas.lit_dot(badge.l + badge.width() / 2, badge.t + badge.height() / 2, at(6), GOLD);
                canvas.text(
                    lang.next_account(),
                    R { l: area.l + at(ROW_TEXT), t: area.t + at(10), r: area.r, b: area.t + at(32) },
                    fonts.name,
                    GOLD,
                    DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
                );
                canvas.text(
                    lang.next_account_hint(),
                    R { l: area.l + at(ROW_TEXT), t: area.t + at(32), r: area.r, b: area.b - at(8) },
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
            Part::InitPill { owner } => {
                let typing = match &capture {
                    Capture::Initiative { owner: who, typed } if who == owner => Some(typed),
                    _ => None,
                };
                let saved = app::with(|state| {
                    state.settings.accounts.get(owner).and_then(|a| a.initiative)
                });
                // A caret after the digits, so a half-typed number reads as
                // half typed rather than as a number already taken.
                let label = match typing {
                    Some(typed) if typed.is_empty() => lang.type_initiative().to_string(),
                    Some(typed) => format!("{typed}_"),
                    None => saved.map(|n| n.to_string()).unwrap_or_else(|| "-".into()),
                };
                let edge = if typing.is_some() { GOLD } else { line_soft() };
                canvas.outline(area, at(8), at(1), edge,
                               if hovered || typing.is_some() { RAISED } else { PILL });
                let empty_hint = matches!(typing, Some(typed) if typed.is_empty());
                canvas.text(
                    &label,
                    area,
                    if empty_hint { fonts.small } else { fonts.pill },
                    if typing.is_some() {
                        GOLD
                    } else if saved.is_some() {
                        CREAM
                    } else {
                        DIM
                    },
                    DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX,
                );
            }
            Part::SortInitiative => {
                // Brighter than the headings beside it once it can actually
                // sort - a heading that has become a button should not look
                // like one that has not, and there is no room down here for
                // a word explaining the difference.
                if hovered {
                    canvas.round(area.inset(at(-4)), at(6), PILL);
                }
                canvas.text(
                    lang.column_initiative(),
                    area,
                    fonts.column,
                    if !item.live { LEAF_DIM } else if hovered { CREAM } else { LEAF },
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
            Part::AutoUpdate => {
                // A short label and a little sliding switch: on (leaf, knob
                // right) or off (dim, knob left). The whole pill is one
                // target and lights on hover, like Refresh beside it.
                let on = app::with(|state| state.settings.auto_update);
                canvas.outline(area, at(8), at(1), line_soft(), if hovered { RAISED } else { PILL });
                let tw = at(30);
                let th = at(16);
                let track = R::new(area.r - tw - at(8), area.t + (area.height() - th) / 2, tw, th);
                let label = R { r: track.l - at(8), ..area };
                canvas.text(
                    lang.auto_update_label(),
                    label,
                    fonts.column,
                    if on { CREAM } else { MUTED },
                    DT_SINGLELINE | DT_VCENTER | DT_RIGHT | DT_NOPREFIX,
                );
                canvas.round(track, th / 2, if on { mix(PILL, LEAF, 0.55) } else { PILL });
                let knob = th - at(4);
                let knob_x = if on { track.r - knob - at(2) } else { track.l + at(2) };
                canvas.round(
                    R::new(knob_x, track.t + at(2), knob, knob),
                    knob / 2,
                    if on { LEAF } else { DIM },
                );
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

/// Close whatever the panel was capturing, KEEPING a half-typed
/// initiative rather than dropping it.
///
/// A key bind is either pressed or it is not, so putting that capture away
/// loses nothing. An initiative is typed a digit at a time, and a player
/// who typed 1150 and then clicked the next row would have lost it with
/// nothing to say so - the pill would simply still read "-", which looks
/// exactly like never having typed it.
fn end_capture() {
    let pending = app::with(|state| {
        let pending = match &state.capture {
            Capture::Initiative { owner, typed } => Some((owner.clone(), typed.clone())),
            _ => None,
        };
        state.capture = Capture::Nothing;
        pending
    });
    if let Some((owner, typed)) = pending {
        // Nothing typed, or a zero, means "forget it": an initiative of
        // zero is not a reading anybody has.
        app::set_initiative(&owner, typed.parse().ok().filter(|n| *n > 0));
    }
}

fn clicked(hwnd: HWND, part: Part) {
    // Whatever was being typed is finished first, whichever part was
    // clicked - including this same pill being clicked again.
    end_capture();
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
        Part::InitPill { owner } => {
            // Opened on the number that is already there, so a correction
            // is a backspace rather than a retype from nothing.
            let typed = app::with(|state| {
                state.settings.accounts.get(&owner).and_then(|a| a.initiative)
            })
            .map(|value| value.to_string())
            .unwrap_or_default();
            app::with(|state| state.capture = Capture::Initiative { owner, typed });
        }
        Part::SortInitiative => {
            if app::order_by_initiative() {
                app::with(|state| state.scroll = 0);
            }
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
        Part::AutoUpdate => {
            // Flip and persist. The size does not change, so a repaint (below)
            // is enough - no resize. update.rs reads the flag from disk on the
            // next launch, which is when a staged update would apply anyway.
            app::with(|state| state.settings.auto_update = !state.settings.auto_update);
            app::save();
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
        Capture::Initiative { owner, mut typed } => {
            // Four digits is every initiative the game has, and a cap means
            // a key held down cannot grow the string without end.
            let digit = match code as u16 {
                0x30..=0x39 => Some(code as u16 - 0x30),
                VK_NUMPAD0..=VK_NUMPAD9 => Some(code as u16 - VK_NUMPAD0),
                _ => None,
            };
            if let Some(digit) = digit {
                if typed.len() < 4 {
                    typed.push(char::from(b'0' + digit as u8));
                }
                app::with(|state| state.capture = Capture::Initiative { owner, typed });
            } else if code == VK_RETURN as u32 {
                end_capture();
            } else if code == VK_BACK as u32 {
                typed.pop();
                app::with(|state| state.capture = Capture::Initiative { owner, typed });
            } else if code == VK_DELETE as u32 {
                app::with(|state| state.capture = Capture::Nothing);
                app::set_initiative(&owner, None);
            } else if code == VK_ESCAPE as u32 {
                // The one way out that keeps the saved number: Escape
                // abandons the edit, everything else commits it.
                app::with(|state| state.capture = Capture::Nothing);
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
        // The same painter, into a caller's DC. This is how the marketing
        // snapshot is taken, so the picture it ships is drawn by the code
        // the player sees rather than scraped off a desktop that may not
        // even be tall enough to hold the window.
        WM_PRINTCLIENT => {
            paint_to(hwnd, wparam as HDC);
            0
        }
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
                end_capture();
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
            end_capture();
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
