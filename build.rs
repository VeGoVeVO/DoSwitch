//! Turns the artwork into something the program can draw directly.
//!
//! The logo is decoded here, at build time, into premultiplied BGRA - the
//! exact layout a 32bpp DIB section wants and AlphaBlend expects - so the
//! executable carries pixels rather than a PNG and no image decoder.
//!
//! The icon is attached as a Windows resource, which is what the taskbar,
//! Alt+Tab and the file itself in Explorer all read.

use std::env;
use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=assets/logo.png");
    println!("cargo:rerun-if-changed=assets/icon.ico");

    let out = env::var("OUT_DIR").expect("no OUT_DIR");
    let decoder = png::Decoder::new(
        fs::File::open("assets/logo.png").expect("assets/logo.png is missing"),
    );
    let mut reader = decoder.read_info().expect("logo.png is not a PNG");
    let mut raw = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut raw).expect("logo.png would not decode");
    let (width, height) = (info.width as usize, info.height as usize);
    assert_eq!(info.color_type, png::ColorType::Rgba,
               "the logo needs an alpha channel");

    let mut bgra = Vec::with_capacity(width * height * 4);
    for pixel in raw.chunks_exact(4) {
        let (r, g, b, a) = (pixel[0] as u32, pixel[1] as u32,
                            pixel[2] as u32, pixel[3] as u32);
        // Premultiplied, because AlphaBlend with AC_SRC_ALPHA reads it
        // that way; handing it straight colours haloes every edge.
        bgra.push((b * a / 255) as u8);
        bgra.push((g * a / 255) as u8);
        bgra.push((r * a / 255) as u8);
        bgra.push(a as u8);
    }
    fs::write(Path::new(&out).join("logo.bgra"), &bgra).expect("logo.bgra");
    fs::write(
        Path::new(&out).join("logo.rs"),
        format!("pub const LOGO_WIDTH: i32 = {width};\n\
                 pub const LOGO_HEIGHT: i32 = {height};\n"),
    )
    .expect("logo.rs");

    if env::var("CARGO_CFG_WINDOWS").is_ok() {
        println!("cargo:rerun-if-env-changed=DOSWITCH_BUILD");

        // The four-part version the release is published under, so the
        // number Explorer's Details tab shows is the number in the
        // filename the server hands over. A local build has no build
        // number and reports x.y.z.0.
        let build: u64 = env::var("DOSWITCH_BUILD")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        let crate_version = env::var("CARGO_PKG_VERSION").expect("no CARGO_PKG_VERSION");
        let version = format!("{crate_version}.{build}");

        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/icon.ico");
        // Filled in honestly, and for a reason beyond tidiness: an
        // executable that declares no publisher, no product and no version
        // is indistinguishable from one with something to hide, and is
        // scored that way by everything that looks at a fresh download.
        resource.set("CompanyName", "VeGoVeVO");
        resource.set("FileDescription", "DoSwitch - Dofus window switcher");
        resource.set("ProductName", "DoSwitch");
        resource.set("OriginalFilename", "DoSwitch.exe");
        resource.set("InternalName", "DoSwitch");
        resource.set("LegalCopyright", "Copyright (c) 2026 VeGoVeVO - MIT licensed");
        resource.set("FileVersion", &version);
        resource.set("ProductVersion", &version);

        // The binary FILEVERSION record as well as the strings above: only
        // the strings show in Explorer, so leaving this at x.y.z.0 would
        // make the inventory-facing version disagree with the one a person
        // reads - a split nobody notices until a build cannot be named.
        let mut parts = crate_version.split('.').map(|p| p.parse::<u64>().unwrap_or(0));
        let (major, minor, patch) = (
            parts.next().unwrap_or(0),
            parts.next().unwrap_or(0),
            parts.next().unwrap_or(0),
        );
        let packed = (major << 48) | (minor << 32) | (patch << 16) | build;
        resource.set_version_info(winresource::VersionInfo::FILEVERSION, packed);
        resource.set_version_info(winresource::VersionInfo::PRODUCTVERSION, packed);
        if let Err(error) = resource.compile() {
            // Not fatal: without it the program runs and simply wears the
            // default icon, and saying so beats failing the build on a
            // machine that has no resource compiler.
            println!("cargo:warning=icon not attached: {error}");
        }
    }
}
