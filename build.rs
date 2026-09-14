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
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/icon.ico");
        resource.set("FileDescription", "DoSwitch");
        resource.set("ProductName", "DoSwitch");
        resource.set("LegalCopyright", "MIT licensed");
        if let Err(error) = resource.compile() {
            // Not fatal: without it the program runs and simply wears the
            // default icon, and saying so beats failing the build on a
            // machine that has no resource compiler.
            println!("cargo:warning=icon not attached: {error}");
        }
    }
}
