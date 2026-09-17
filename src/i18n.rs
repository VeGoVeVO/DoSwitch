//! English, French and Spanish, chosen with the toggle and remembered.
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
    Es,
}

impl Lang {
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Fr => "fr",
            Lang::Es => "es",
        }
    }

    pub fn from_code(code: &str) -> Option<Lang> {
        match code {
            "en" => Some(Lang::En),
            "fr" => Some(Lang::Fr),
            "es" => Some(Lang::Es),
            _ => None,
        }
    }

    fn pick(self, en: &'static str, fr: &'static str, es: &'static str)
            -> &'static str {
        match self {
            Lang::En => en,
            Lang::Fr => fr,
            Lang::Es => es,
        }
    }

    pub fn title(self) -> &'static str {
        self.pick("Your accounts", "Vos comptes", "Tus cuentas")
    }

    pub fn subtitle(self) -> &'static str {
        self.pick(
            "One key each. They work while Dofus has the focus, for as long as this app is running.",
            "Une touche chacun. Elles fonctionnent quand Dofus est au premier plan, tant que cette application tourne.",
            "Una tecla para cada una. Funcionan mientras Dofus est\u{e1} en primer plano, mientras esta aplicaci\u{f3}n siga abierta.",
        )
    }

    pub fn column_character(self) -> &'static str {
        self.pick("CHARACTER", "PERSONNAGE", "PERSONAJE")
    }

    pub fn column_order(self) -> &'static str {
        self.pick("ORDER", "ORDRE", "ORDEN")
    }

    /// Also the button that sorts by it, which is why it is a heading the
    /// pointer lights up rather than a label.
    pub fn column_initiative(self) -> &'static str {
        self.pick("INITIATIVE", "INITIATIVE", "INICIATIVA")
    }

    pub fn column_key(self) -> &'static str {
        self.pick("KEY", "TOUCHE", "TECLA")
    }

    pub fn next_account(self) -> &'static str {
        self.pick("Next account", "Compte suivant", "Cuenta siguiente")
    }

    pub fn next_account_hint(self) -> &'static str {
        self.pick(
            "one key for the whole team, in the order above",
            "une seule touche pour toute l'\u{e9}quipe, dans l'ordre ci-dessus",
            "una sola tecla para todo el equipo, en el orden de arriba",
        )
    }

    pub fn note(self) -> &'static str {
        self.pick(
            "Order is yours to set: lowest number plays first, and the Next account key walks it. Click a key to change it, click an order number and type a new one. Initiative is optional - type each one once, then click INITIATIVE to put the team in turn order.",
            "L'ordre est le v\u{f4}tre: le plus petit num\u{e9}ro joue en premier, et la touche Compte suivant le parcourt. Cliquez une touche pour la changer, cliquez un num\u{e9}ro d'ordre et tapez-en un autre. L'initiative est facultative: saisissez-la une fois par personnage, puis cliquez INITIATIVE pour ranger l'\u{e9}quipe dans l'ordre des tours.",
            "El orden lo decides t\u{fa}: el n\u{fa}mero m\u{e1}s bajo juega primero, y la tecla Cuenta siguiente lo recorre. Haz clic en una tecla para cambiarla, haz clic en un n\u{fa}mero de orden y escribe otro. La iniciativa es opcional: escr\u{ed}bela una vez por personaje y haz clic en INICIATIVA para ordenar al equipo por turnos.",
        )
    }

    pub fn press_a_key(self) -> &'static str {
        self.pick("press a key", "appuyez sur une touche", "pulsa una tecla")
    }

    pub fn type_a_number(self) -> &'static str {
        self.pick("type 1 to 9", "tapez 1 \u{e0} 9", "escribe 1 a 9")
    }

    /// The initiative pill while it is waiting and nothing has been typed.
    /// It has to name the key that COMMITS: an order number is one digit
    /// and finishes itself, an initiative is not, so a player who typed
    /// their number and clicked away would otherwise lose it without ever
    /// being told there was a step left.
    pub fn type_initiative(self) -> &'static str {
        self.pick("type, Enter", "tapez, Entr\u{e9}e", "escribe, Intro")
    }

    pub fn unbound(self) -> &'static str {
        self.pick("not set", "non d\u{e9}finie", "sin asignar")
    }

    pub fn refresh(self) -> &'static str {
        self.pick("Refresh", "Actualiser", "Actualizar")
    }

    pub fn done(self) -> &'static str {
        self.pick("Minimize", "Réduire", "Minimizar")
    }

    /// The header's auto-update toggle. Short on purpose - it sits where the
    /// language switcher used to, and the language is an install choice now.
    pub fn auto_update_label(self) -> &'static str {
        self.pick("Auto-update", "M\u{e0}j auto", "Auto-act.")
    }

    pub fn nothing_open(self) -> &'static str {
        self.pick(
            "No Dofus window is open. Log in, then press Refresh.",
            "Aucune fen\u{ea}tre Dofus n'est ouverte. Connectez-vous, puis Actualiser.",
            "No hay ninguna ventana de Dofus abierta. Con\u{e9}ctate y pulsa Actualizar.",
        )
    }

    /// "3 open, 2 bound", in the language's own word order.
    pub fn counts(self, open: usize, bound: usize) -> String {
        match self {
            Lang::En => format!("{open} open, {bound} bound"),
            Lang::Fr => format!("{open} ouverte(s), {bound} li\u{e9}e(s)"),
            Lang::Es => format!("{open} abierta(s), {bound} asignada(s)"),
        }
    }

    pub fn menu_accounts(self) -> &'static str {
        self.pick("Your accounts", "Vos comptes", "Tus cuentas")
    }

    pub fn menu_startup(self) -> &'static str {
        self.pick("Start with Windows", "D\u{e9}marrer avec Windows",
                  "Iniciar con Windows")
    }

    pub fn menu_quit(self) -> &'static str {
        self.pick("Quit", "Quitter", "Salir")
    }

    pub fn tray_tip(self) -> &'static str {
        self.pick(
            "DoSwitch - one key per account",
            "DoSwitch - une touche par compte",
            "DoSwitch - una tecla por cuenta",
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
        let name = name.to_ascii_lowercase();
        if name.starts_with("fr") {
            return Lang::Fr;
        }
        if name.starts_with("es") {
            return Lang::Es;
        }
    }
    Lang::En
}
