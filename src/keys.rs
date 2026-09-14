//! A key as a name you can read, and as the code Windows sends.
//!
//! The name is what goes in the settings file and on the button, so it
//! has to survive being written down: "F3", "Ctrl+A", "Num 5". The code
//! is what arrives from the keyboard.
//!
//! Only the physical key is stored, never the character it would type.
//! A French keyboard sends the same code for the key an English one calls
//! A, and a bind set on one layout has to keep working on the other.

use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Bind {
    pub code: u32,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Bind {
    /// True when this is a modifier on its own, which is never a bind:
    /// holding Shift to reach a key must not set the bind to Shift.
    pub fn is_modifier_only(code: u32) -> bool {
        matches!(
            code as u16,
            VK_SHIFT | VK_LSHIFT | VK_RSHIFT | VK_CONTROL | VK_LCONTROL
                | VK_RCONTROL | VK_MENU | VK_LMENU | VK_RMENU | VK_LWIN
                | VK_RWIN | VK_CAPITAL
        )
    }

    pub fn name(self) -> String {
        let mut out = String::new();
        if self.ctrl {
            out.push_str("Ctrl+");
        }
        if self.alt {
            out.push_str("Alt+");
        }
        if self.shift {
            out.push_str("Shift+");
        }
        out.push_str(&key_name(self.code));
        out
    }

    pub fn parse(text: &str) -> Option<Bind> {
        let mut bind = Bind { code: 0, ctrl: false, alt: false, shift: false };
        for piece in text.split('+') {
            match piece.trim().to_ascii_lowercase().as_str() {
                "ctrl" => bind.ctrl = true,
                "alt" => bind.alt = true,
                "shift" => bind.shift = true,
                other => bind.code = key_code(other)?,
            }
        }
        if bind.code == 0 {
            None
        } else {
            Some(bind)
        }
    }
}

/// The name of one key, without its modifiers.
fn key_name(code: u32) -> String {
    let code16 = code as u16;
    if (VK_F1..=VK_F24).contains(&code16) {
        return format!("F{}", code16 - VK_F1 + 1);
    }
    if (VK_NUMPAD0..=VK_NUMPAD9).contains(&code16) {
        return format!("Num {}", code16 - VK_NUMPAD0);
    }
    if (0x30..=0x39).contains(&code16) || (0x41..=0x5A).contains(&code16) {
        return ((code16 as u8) as char).to_string();
    }
    match code16 {
        VK_SPACE => "Space".into(),
        VK_TAB => "Tab".into(),
        VK_INSERT => "Insert".into(),
        VK_DELETE => "Delete".into(),
        VK_HOME => "Home".into(),
        VK_END => "End".into(),
        VK_PRIOR => "Page Up".into(),
        VK_NEXT => "Page Down".into(),
        VK_LEFT => "Left".into(),
        VK_RIGHT => "Right".into(),
        VK_UP => "Up".into(),
        VK_DOWN => "Down".into(),
        VK_MULTIPLY => "Num *".into(),
        VK_ADD => "Num +".into(),
        VK_SUBTRACT => "Num -".into(),
        VK_DIVIDE => "Num /".into(),
        VK_DECIMAL => "Num .".into(),
        VK_OEM_3 => "`".into(),
        other => format!("Key {other}"),
    }
}

fn key_code(name: &str) -> Option<u32> {
    if let Some(number) = name.strip_prefix('f') {
        if let Ok(index) = number.parse::<u16>() {
            if (1..=24).contains(&index) {
                return Some((VK_F1 + index - 1) as u32);
            }
        }
    }
    if let Some(number) = name.strip_prefix("num ") {
        if let Ok(digit) = number.parse::<u16>() {
            if digit <= 9 {
                return Some((VK_NUMPAD0 + digit) as u32);
            }
        }
        return match number {
            "*" => Some(VK_MULTIPLY as u32),
            "+" => Some(VK_ADD as u32),
            "-" => Some(VK_SUBTRACT as u32),
            "/" => Some(VK_DIVIDE as u32),
            "." => Some(VK_DECIMAL as u32),
            _ => None,
        };
    }
    let upper = name.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    if bytes.len() == 1 && (bytes[0].is_ascii_alphanumeric()) {
        return Some(bytes[0] as u32);
    }
    Some(match upper.as_str() {
        "SPACE" => VK_SPACE as u32,
        "TAB" => VK_TAB as u32,
        "INSERT" => VK_INSERT as u32,
        "DELETE" => VK_DELETE as u32,
        "HOME" => VK_HOME as u32,
        "END" => VK_END as u32,
        "PAGE UP" => VK_PRIOR as u32,
        "PAGE DOWN" => VK_NEXT as u32,
        "LEFT" => VK_LEFT as u32,
        "RIGHT" => VK_RIGHT as u32,
        "UP" => VK_UP as u32,
        "DOWN" => VK_DOWN as u32,
        "`" => VK_OEM_3 as u32,
        _ => return None,
    })
}
