//! Silent self-update, the same shape as the Pro app's.
//!
//! On startup, in the background, ask the licence server for the current
//! free build (GET /api/v1/version?product=free). If it is newer, download
//! the installer, and only if the bytes match BOTH the sha256 the server
//! named AND an Ed25519 signature over them, run it silently. The installer
//! is per-user, so no admin prompt, and it closes and relaunches the app
//! itself. Any failure just leaves the app on the version it has.

use std::os::windows::process::CommandExt;

use crate::crypto;
use crate::http;

const API: &str = match option_env!("DOSWITCH_API") {
    Some(api) => api,
    None => "https://api.doswitchpro.com",
};
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn check_in_background() {
    std::thread::spawn(|| {
        if let Err(reason) = try_update() {
            log(&format!("update check: {reason}"));
        }
    });
}

fn try_update() -> Result<(), String> {
    let response = http::get(&format!("{API}/api/v1/version?product=free"))?;
    if response.status != 200 {
        return Err(format!("version endpoint answered {}", response.status));
    }
    let json = String::from_utf8_lossy(&response.body);
    let latest = field(&json, "version").ok_or("no version in the reply")?;
    let current = env!("DOSWITCH_VERSION");
    if !is_newer(&latest, current) {
        log(&format!("update check: {current} is current (latest {latest})"));
        return Ok(());
    }
    let url = field(&json, "url").ok_or("no url in the reply")?;
    let want_sha = field(&json, "sha256").ok_or("no sha256 in the reply")?;
    let signature_b64 = field(&json, "signature").ok_or("no signature in the reply")?;

    log(&format!("update check: {latest} available, downloading"));
    let file = http::get(&url)?;
    if file.status != 200 {
        return Err(format!("download answered {}", file.status));
    }
    if crypto::sha256_hex(&file.body) != want_sha.to_ascii_lowercase() {
        return Err("the download did not match its sha256".into());
    }
    let signature = crypto::base64_decode(&signature_b64).ok_or("signature was not base64")?;
    crypto::verify_release(&file.body, &signature)?;

    let mut path = std::env::temp_dir();
    path.push(format!("DoSwitch-Setup-{latest}.exe"));
    std::fs::write(&path, &file.body).map_err(|e| format!("could not save the installer: {e}"))?;

    log(&format!("update check: verified {latest}, launching the installer"));
    std::process::Command::new(&path)
        .args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("could not launch the installer: {e}"))?;
    Ok(())
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

fn log(line: &str) {
    use std::io::Write;
    let Some(base) = std::env::var_os("LOCALAPPDATA") else { return };
    let dir = std::path::Path::new(&base).join("DoSwitch");
    let _ = std::fs::create_dir_all(&dir);
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
