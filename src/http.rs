//! The little HTTP client the licence half talks through.
//!
//! WinHTTP rather than a Rust HTTP stack, for three reasons that matter
//! more than the convenience of a crate:
//!
//!   - It uses the SYSTEM's TLS and the system's root store, so a
//!     corporate machine that trusts an inspecting proxy keeps working
//!     and nobody has to ship a certificate bundle that goes stale.
//!   - It reads the machine's proxy settings for free. A player behind a
//!     company proxy is a support ticket nobody can debug remotely.
//!   - It adds nothing to the download. The loader is meant to be small.
//!
//! Only what the protocol needs: a POST and a GET, both returning the
//! status and the body. Errors come back as a string because every one
//! of them ends up in the same place - a line in the panel saying the
//! licence could not be checked.

use std::ffi::c_void;

use windows_sys::Win32::Networking::WinHttp::*;

const AGENT: &str = "DoSwitchPro";

/// One response: the HTTP status and the body bytes.
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Response {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// A fresh nonce for the protocol header: sixteen random bytes as hex,
/// from the OS, so no two requests carry the same one and none of it
/// depends on a clock or a counter this program would have to keep.
fn nonce() -> String {
    use rand_core::{OsRng, RngCore};
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// A POST with a JSON body, and the cookie header the session needs.
pub fn post_json(url: &str, body: &str) -> Result<Response, String> {
    request("POST", url, Some(body))
}

pub fn get(url: &str) -> Result<Response, String> {
    request("GET", url, None)
}

fn request(method: &str, url: &str, body: Option<&str>) -> Result<Response, String> {
    // https://host/path -> the three pieces WinHttp wants separately.
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| "only https is allowed".to_string())?;
    let (host, path) = match rest.find('/') {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, "/"),
    };

    unsafe {
        let session = WinHttpOpen(
            wide(AGENT).as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            std::ptr::null(),
            std::ptr::null(),
            0,
        );
        if session.is_null() {
            return Err("could not start a connection".into());
        }
        let guard = Handles { session, connect: std::ptr::null_mut(), request: std::ptr::null_mut() };
        let mut guard = guard;

        // Ten seconds each. A licence check that hangs is a program that
        // looks frozen, which is worse than one that says it could not
        // reach the server.
        WinHttpSetTimeouts(session, 10_000, 10_000, 10_000, 10_000);

        let connect = WinHttpConnect(session, wide(host).as_ptr(), INTERNET_DEFAULT_HTTPS_PORT as u16, 0);
        if connect.is_null() {
            return Err(format!("could not reach {host}"));
        }
        guard.connect = connect;

        let request = WinHttpOpenRequest(
            connect,
            wide(method).as_ptr(),
            wide(path).as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null_mut(),
            WINHTTP_FLAG_SECURE,
        );
        if request.is_null() {
            return Err("could not build the request".into());
        }
        guard.request = request;

        // Every request on the licence protocol carries its version and a
        // nonce; without them the server answers 400
        // missing_protocol_headers, which is what activation had been
        // failing with. The version ties an event in the server log to a
        // client build without trusting the body to say so, and the nonce
        // makes each request distinguishable from a byte-for-byte replay
        // of an earlier one. This is NOT the cryptographic nonce the
        // session body carries - that one is 32 bytes and feeds HKDF.
        let headers = wide(&format!(
            "Content-Type: application/json
X-DoSwitch-Version: {}
X-DoSwitch-Nonce: {}
",
            // DOSWITCH_VERSION, not CARGO_PKG_VERSION. The crate version is
            // the same string for every build in a release line, so this
            // header - the one thing that ties an event in the server log
            // to a client build - said "1.0.0" for build 1 and for build
            // 46 alike, and the admin's Version column sat on it and never
            // moved. A version that cannot change is worse than no version:
            // it reads as a customer who never updates.
            env!("DOSWITCH_VERSION"),
            nonce(),
        ));
        let (body_ptr, body_len) = match body {
            Some(text) => (text.as_ptr() as *const c_void, text.len() as u32),
            None => (std::ptr::null(), 0),
        };
        let sent = WinHttpSendRequest(
            request,
            headers.as_ptr(),
            u32::MAX, // count the headers for me
            body_ptr as *const c_void,
            body_len,
            body_len,
            0,
        );
        if sent == 0 {
            return Err("the request could not be sent".into());
        }
        if WinHttpReceiveResponse(request, std::ptr::null_mut()) == 0 {
            return Err("no answer from the server".into());
        }

        let mut status: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            request,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            std::ptr::null(),
            &mut status as *mut u32 as *mut c_void,
            &mut size,
            std::ptr::null_mut(),
        );

        let mut out = Vec::new();
        loop {
            let mut available: u32 = 0;
            if WinHttpQueryDataAvailable(request, &mut available) == 0 || available == 0 {
                break;
            }
            let mut chunk = vec![0u8; available as usize];
            let mut read: u32 = 0;
            if WinHttpReadData(
                request,
                chunk.as_mut_ptr() as *mut c_void,
                available,
                &mut read,
            ) == 0
            {
                break;
            }
            chunk.truncate(read as usize);
            out.extend_from_slice(&chunk);
            // A server that keeps promising bytes it never sends would
            // spin here forever otherwise.
            if read == 0 {
                break;
            }
        }

        Ok(Response { status: status as u16, body: out })
    }
}

/// Closes the three WinHttp handles however the function leaves, which a
/// chain of early returns otherwise gets wrong.
struct Handles {
    session: *mut c_void,
    connect: *mut c_void,
    request: *mut c_void,
}

impl Drop for Handles {
    fn drop(&mut self) {
        unsafe {
            for handle in [self.request, self.connect, self.session] {
                if !handle.is_null() {
                    WinHttpCloseHandle(handle);
                }
            }
        }
    }
}
