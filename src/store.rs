//! What is remembered between runs, and where.
//!
//! `%APPDATA%\DoSwitch\settings.json`, keyed on the CHARACTER rather than
//! on the window or the process. A window handle is gone the moment the
//! client closes and a process id is different every launch, so either
//! would mean setting the keys again every session. A character name is
//! the one thing that is the same tomorrow.
//!
//! Written whole and renamed into place. A reader that catches a half
//! written file sees broken settings and starts blank, and losing an
//! evening of binds to a crash at the wrong instant is exactly the kind
//! of failure nobody would think to look for.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{json, Value};

use crate::i18n::Lang;
use crate::keys::Bind;

#[derive(Clone, Default)]
pub struct Account {
    pub key: Option<Bind>,
    pub order: Option<u32>,
}

#[derive(Clone)]
pub struct Settings {
    pub lang: Lang,
    pub next: Option<Bind>,
    pub accounts: BTreeMap<String, Account>,
    pub startup: bool,
}

impl Settings {
    // Clean settings for a language, never touching the saved file - used by
    // the marketing snapshot (main.rs --panel-snapshot) so its picture is the
    // same on a machine that has used the app and one that never has.
    pub(crate) fn empty(lang: Lang) -> Settings {
        Settings {
            lang,
            next: None,
            accounts: BTreeMap::new(),
            startup: false,
        }
    }

    pub fn account(&mut self, character: &str) -> &mut Account {
        self.accounts.entry(character.to_string()).or_default()
    }

    /// The bind for a character, if it has one.
    pub fn key_of(&self, character: &str) -> Option<Bind> {
        self.accounts.get(character).and_then(|a| a.key)
    }

    /// Whoever else holds this key loses it. One key, one account: two
    /// accounts on one key is a switch with no answer.
    pub fn take_key(&mut self, bind: Bind, keep: Option<&str>) {
        if self.next == Some(bind) && keep != Some(NEXT) {
            self.next = None;
        }
        for (name, account) in self.accounts.iter_mut() {
            if account.key == Some(bind) && Some(name.as_str()) != keep {
                account.key = None;
            }
        }
    }
}

/// The name reserved for the cycle key, so one function can clear a bind
/// from wherever it sits.
pub const NEXT: &str = "\u{0}next";

pub fn folder() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(base).join("DoSwitch"))
}

fn file() -> Option<PathBuf> {
    Some(folder()?.join("settings.json"))
}

pub fn load(fallback: Lang) -> Settings {
    let Some(path) = file() else {
        return Settings::empty(fallback);
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Settings::empty(fallback);
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return Settings::empty(fallback);
    };

    let mut settings = Settings::empty(
        value["language"]
            .as_str()
            .and_then(Lang::from_code)
            .unwrap_or(fallback),
    );
    settings.next = value["next"].as_str().and_then(Bind::parse);
    settings.startup = value["startup"].as_bool().unwrap_or(false);
    if let Some(table) = value["accounts"].as_object() {
        for (name, entry) in table {
            settings.accounts.insert(
                name.clone(),
                Account {
                    key: entry["key"].as_str().and_then(Bind::parse),
                    order: entry["order"].as_u64().map(|n| n as u32),
                },
            );
        }
    }
    settings
}

pub fn save(settings: &Settings) {
    let Some(path) = file() else { return };
    let Some(parent) = path.parent() else { return };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }

    let mut accounts = serde_json::Map::new();
    for (name, account) in &settings.accounts {
        if account.key.is_none() && account.order.is_none() {
            continue;
        }
        accounts.insert(
            name.clone(),
            json!({
                "key": account.key.map(|b| b.name()),
                "order": account.order,
            }),
        );
    }
    let body = json!({
        "language": settings.lang.code(),
        "next": settings.next.map(|b| b.name()),
        "startup": settings.startup,
        "accounts": Value::Object(accounts),
    });
    let Ok(text) = serde_json::to_string_pretty(&body) else { return };

    let temporary = path.with_extension("json.new");
    if std::fs::write(&temporary, text).is_ok() {
        let _ = std::fs::rename(&temporary, &path);
    }
}
