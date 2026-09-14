//! English and French, chosen with the toggle and remembered.
//!
//! Every string the program shows is here, in both languages, as one
//! table. A language is not a lookup that can miss: the compiler will not
//! let a string exist in one language and not the other.
//!
//! The first run picks from the user's Windows language, so a French
//! machine opens in French without being asked.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Fr,
}

impl Lang {
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Fr => "fr",
        }
    }

    pub fn from_code(code: &str) -> Option<Lang> {
        match code {
            "en" => Some(Lang::En),
            "fr" => Some(Lang::Fr),
            _ => None,
        }
    }

    pub fn other(self) -> Lang {
        match self {
            Lang::En => Lang::Fr,
            Lang::Fr => Lang::En,
        }
    }

    fn pick(self, en: &'static str, fr: &'static str) -> &'static str {
        match self {
            Lang::En => en,
            Lang::Fr => fr,
        }
    }

    pub fn title(self) -> &'static str {
        self.pick("Your accounts", "Vos comptes")
    }

    pub fn subtitle(self) -> &'static str {
        self.pick(
            "One key each. They work while Dofus has the focus, for as long as this app is running.",
            "Une touche chacun. Elles fonctionnent quand Dofus est au premier plan, tant que cette application tourne.",
        )
    }

    pub fn column_character(self) -> &'static str {
        self.pick("CHARACTER", "PERSONNAGE")
    }

    pub fn column_order(self) -> &'static str {
        self.pick("ORDER", "ORDRE")
    }

    pub fn column_key(self) -> &'static str {
        self.pick("KEY", "TOUCHE")
    }

    pub fn next_account(self) -> &'static str {
        self.pick("Next account", "Compte suivant")
    }

    pub fn next_account_hint(self) -> &'static str {
        self.pick(
            "one key for the whole team, in the order above",
            "une seule touche pour toute l'\u{e9}quipe, dans l'ordre ci-dessus",
        )
    }

    pub fn note(self) -> &'static str {
        self.pick(
            "Order is yours to set: lowest number plays first, and the Next account key walks it. Click a key to change it, click an order number and type a new one.",
            "L'ordre est le v\u{f4}tre: le plus petit num\u{e9}ro joue en premier, et la touche Compte suivant le parcourt. Cliquez une touche pour la changer, cliquez un num\u{e9}ro d'ordre et tapez-en un autre.",
        )
    }

    pub fn press_a_key(self) -> &'static str {
        self.pick("press a key", "appuyez sur une touche")
    }

    pub fn type_a_number(self) -> &'static str {
        self.pick("type 1 to 9", "tapez 1 \u{e0} 9")
    }

    pub fn unbound(self) -> &'static str {
        self.pick("not set", "non d\u{e9}finie")
    }

    pub fn refresh(self) -> &'static str {
        self.pick("Refresh", "Actualiser")
    }

    pub fn done(self) -> &'static str {
        self.pick("Done", "Fermer")
    }

    pub fn nothing_open(self) -> &'static str {
        self.pick(
            "No Dofus window is open. Log in, then press Refresh.",
            "Aucune fen\u{ea}tre Dofus n'est ouverte. Connectez-vous, puis Actualiser.",
        )
    }

    /// "3 open, 2 bound", in the language's own word order.
    pub fn counts(self, open: usize, bound: usize) -> String {
        match self {
            Lang::En => format!("{open} open, {bound} bound"),
            Lang::Fr => format!("{open} ouverte(s), {bound} li\u{e9}e(s)"),
        }
    }

    pub fn menu_accounts(self) -> &'static str {
        self.pick("Your accounts", "Vos comptes")
    }

    pub fn menu_startup(self) -> &'static str {
        self.pick("Start with Windows", "D\u{e9}marrer avec Windows")
    }

    pub fn menu_quit(self) -> &'static str {
        self.pick("Quit", "Quitter")
    }

    pub fn tray_tip(self) -> &'static str {
        self.pick(
            "DoSwitch - one key per account",
            "DoSwitch - une touche par compte",
        )
    }
}

/// What Windows itself is set to, for a first run with nothing saved.
pub fn system_language() -> Lang {
    let mut buffer = [0u16; 16];
    let taken = unsafe {
        windows_sys::Win32::Globalization::GetUserDefaultLocaleName(
            buffer.as_mut_ptr(),
            buffer.len() as i32,
        )
    };
    if taken > 1 {
        let name = String::from_utf16_lossy(&buffer[..(taken - 1) as usize]);
        if name.to_ascii_lowercase().starts_with("fr") {
            return Lang::Fr;
        }
    }
    Lang::En
}
