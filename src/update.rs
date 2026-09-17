//! Seamless self-update: stage while running, apply on the way out.
//!
//! Nothing is applied while the app is running. On startup, in the
//! background, it asks the licence server for the current free build
//! (GET /api/v1/version?product=free). If it is newer it downloads the
//! installer, checks it, and STAGES it - saved to disk beside a marker
//! naming its version - without running anything. The app is never touched
//! and never interrupted.
//!
//! When the player quits, `apply_staged` runs the staged installer silently:
//! the exe is free to replace because the app is closing, and it does NOT
//! relaunch. The next time they open the app it is already the new version,
//! the way Chrome and the rest update, no interruption ever.
//!
//! Nothing unverified is ever staged: the download must match BOTH the sha256
//! the server named AND an Ed25519 signature over its bytes before it is
//! written to the staging path. The private half of that key lives only in
//! CI, so a swapped installer cannot be forged.

use std::os::windows::process::CommandExt;

use crate::crypto;
use crate::http;

const API: &str = match option_env!("DOSWITCH_API") {
    Some(api) => api,
    None => "https://api.doswitchpro.com",
};
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const DETACHED_PROCESS: u32 = 0x0000_0008;

/// `auto_update` is the player's toggle; a forced floor from the server
/// overrides it inside try_stage, so a major fix still reaches everyone.
pub fn check_in_background(auto_update: bool) {
    std::thread::spawn(move || {
        if let Err(reason) = try_stage(auto_update) {
            log(&format!("stage check: {reason}"));
        }
    });
}

fn try_stage(auto_update: bool) -> Result<(), String> {
    let response = http::get(&format!("{API}/api/v1/version?product=free"))?;
    if response.status != 200 {
        return Err(format!("version endpoint answered {}", response.status));
    }
    let json = String::from_utf8_lossy(&response.body);
    let latest = field(&json, "version").ok_or("no version in the reply")?;
    let current = env!("DOSWITCH_VERSION");
    // The forced floor, if the admin set one: the version an app must reach
    // even with auto-update off. "Forced" means the running build is BELOW it.
    let forced = field(&json, "min_version").is_some_and(|min| is_newer(&min, current));
    if !auto_update && !forced {
        // The player turned updates off and nothing is being forced. Make
        // sure a staged update from an earlier "on" session is not waiting,
        // then leave the build exactly as it is.
        clear_staged();
        log(&format!("stage check: auto-update off, {current} kept (latest {latest})"));
        return Ok(());
    }
    if !is_newer(&latest, current) {
        // Up to date. Sweep away a staged installer left from an update that
        // has since been applied, so it is never run twice.
        clear_staged();
        log(&format!("stage check: {current} is current (latest {latest})"));
        return Ok(());
    }
    if staged_version().as_deref() == Some(latest.as_str()) {
        // Already downloaded and waiting for the next quit.
        log(&format!("stage check: {latest} already staged, waiting for quit"));
        return Ok(());
    }
    let url = field(&json, "url").ok_or("no url in the reply")?;
    let want_sha = field(&json, "sha256").ok_or("no sha256 in the reply")?;
    let signature_b64 = field(&json, "signature").ok_or("no signature in the reply")?;

    log(&format!("stage check: {latest} available, downloading"));
    let file = http::get(&url)?;
    if file.status != 200 {
        return Err(format!("download answered {}", file.status));
    }
    if crypto::sha256_hex(&file.body) != want_sha.to_ascii_lowercase() {
        return Err("the download did not match its sha256".into());
    }
    let signature = crypto::base64_decode(&signature_b64).ok_or("signature was not base64")?;
    crypto::verify_release(&file.body, &signature)?;

    let path = staged_path().ok_or("no staging path")?;
    std::fs::write(&path, &file.body).map_err(|e| format!("could not stage the installer: {e}"))?;
    set_staged_version(&latest);
    // Remember whether this was forced, so apply_staged can respect the
    // toggle: a forced update still applies with auto-update off, a normal
    // one does not.
    set_staged_forced(forced);
    log(&format!("stage check: staged {latest}{}, will apply on quit", if forced { " (forced)" } else { "" }));
    Ok(())
}

/// Apply a staged update at STARTUP, before this instance does anything else.
/// Returns true if it launched the installer, in which case main MUST return
/// at once so the exe is free to be replaced: the installer closes this
/// just-started instance, swaps the exe, and relaunches the app (the [Run]
/// entry in the .iss).
///
/// On startup, NOT on exit. An exit is not guaranteed to run any code - a
/// killed or crashed process runs no cleanup, so a stage-on-exit update never
/// lands. A startup always happens, so the update lands on the next launch no
/// matter how the last session ended.
///
/// Guarded against a loop: if the installer was already run for this exact
/// staged version and the running build is STILL older, the install did not
/// take, so it is not tried again. Even if the relaunch never happens the exe
/// is replaced, so the next manual launch is simply the new version.
pub fn apply_staged_on_startup(auto_update: bool) -> bool {
    let Some(version) = staged_version() else { return false };
    let current = env!("DOSWITCH_VERSION");
    if !is_newer(&version, current) {
        clear_staged();
        clear_attempt();
        return false;
    }
    // Respect the toggle at apply time too: a staged update applies only if
    // auto-update is on, or it was staged because the server forced it. The
    // held file is swept by try_stage this same session.
    if !auto_update && !staged_forced() {
        log(&format!("staged {version} held: auto-update off and not forced"));
        return false;
    }
    if attempt_version().as_deref() == Some(version.as_str()) {
        log(&format!("staged {version} did not apply last time; running {current}"));
        return false;
    }
    let Some(path) = staged_path() else { return false };
    if !path.exists() {
        return false;
    }
    set_attempt(&version);
    log(&format!("applying staged {version} on startup"));
    std::process::Command::new(&path)
        .args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"])
        .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
        .spawn()
        .is_ok()
}

/// Compared field by field as numbers, so 1.0.0.9 is below 1.0.0.10.
fn is_newer(latest: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> { v.split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    let a = parse(latest);
    let b = parse(current);
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    false
}

/// The one JSON field reader this app needs - a flat string value.
fn field(json: &str, name: &str) -> Option<String> {
    let needle = format!("\"{name}\"");
    let at = json.find(&needle)? + needle.len();
    let rest = &json[at..];
    let colon = rest.find(':')?;
    let after = rest[colon + 1..].trim_start();
    let after = after.strip_prefix('"')?;
    let end = after.find('"')?;
    Some(after[..end].to_string())
}

/// The DoSwitch directory beside the diary, created if need be.
fn state_dir() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    let dir = std::path::Path::new(&base).join("DoSwitch");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir)
}

/// Where a downloaded-but-not-yet-applied installer waits.
fn staged_path() -> Option<std::path::PathBuf> {
    Some(state_dir()?.join("staged-free-update.exe"))
}

/// The marker naming which version is staged.
fn staged_marker() -> Option<std::path::PathBuf> {
    Some(state_dir()?.join("staged-free-version"))
}

fn staged_version() -> Option<String> {
    let text = std::fs::read_to_string(staged_marker()?).ok()?;
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn set_staged_version(version: &str) {
    if let Some(marker) = staged_marker() {
        let _ = std::fs::write(marker, version);
    }
}

/// Forget a staged update, so a spent installer is never run twice.
fn clear_staged() {
    if let Some(path) = staged_path() {
        let _ = std::fs::remove_file(path);
    }
    if let Some(marker) = staged_marker() {
        let _ = std::fs::remove_file(marker);
    }
    if let Some(marker) = staged_forced_marker() {
        let _ = std::fs::remove_file(marker);
    }
}

/// Whether the staged update was staged because the server forced it, rather
/// than a normal newer release. Lets apply_staged land a forced update even
/// when the player has auto-update off.
fn staged_forced_marker() -> Option<std::path::PathBuf> {
    Some(state_dir()?.join("staged-free-forced"))
}

fn staged_forced() -> bool {
    staged_forced_marker().is_some_and(|p| p.exists())
}

fn set_staged_forced(forced: bool) {
    let Some(marker) = staged_forced_marker() else { return };
    if forced {
        let _ = std::fs::write(marker, "1");
    } else {
        let _ = std::fs::remove_file(marker);
    }
}

/// The version the startup-apply last ran the installer for - the loop guard,
/// so a staged update that fails to install is not retried every launch.
fn attempt_marker() -> Option<std::path::PathBuf> {
    Some(state_dir()?.join("staged-free-attempt"))
}

fn attempt_version() -> Option<String> {
    let text = std::fs::read_to_string(attempt_marker()?).ok()?;
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn set_attempt(version: &str) {
    if let Some(marker) = attempt_marker() {
        let _ = std::fs::write(marker, version);
    }
}

fn clear_attempt() {
    if let Some(marker) = attempt_marker() {
        let _ = std::fs::remove_file(marker);
    }
}

fn log(line: &str) {
    use std::io::Write;
    let Some(dir) = state_dir() else { return };
    let path = dir.join("update.log");
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > 128 * 1024 {
            let _ = std::fs::write(&path, b"");
        }
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{stamp} {line}");
    }
}

#[cfg(test)]
mod tests {
    use super::{field, is_newer};

    #[test]
    fn version_compare_is_numeric() {
        assert!(is_newer("1.0.0.10", "1.0.0.9"));
        assert!(!is_newer("1.0.0.9", "1.0.0.10"));
        assert!(!is_newer("1.0.0.5", "1.0.0.5"));
    }

    #[test]
    fn reads_a_flat_field() {
        let j = r#"{"version":"1.0.0.7","url":"https://x/y","sha256":"ab"}"#;
        assert_eq!(field(j, "version").as_deref(), Some("1.0.0.7"));
        assert_eq!(field(j, "url").as_deref(), Some("https://x/y"));
    }
}
