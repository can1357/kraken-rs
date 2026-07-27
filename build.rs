use std::{env, fs, path::PathBuf};

const UI_SOURCE: &str = "ui/app.slab";

fn main() {
    println!("cargo:rerun-if-changed={UI_SOURCE}");
    println!("cargo:rerun-if-changed=assets/fonts/InstrumentSans.ttf");
    println!("cargo:rerun-if-changed=assets/fonts/JetBrainsMonoNerdFontMono-Regular.ttf");

    let source = fs::read_to_string(UI_SOURCE).expect("read Slab UI source");
    // Compile FONT tables from the real UI faces so glyph ids match the
    // byte-backed fonts registered by the renderer (src/gpu/slab.rs).
    let fonts = [
        ("Instrument Sans", "assets/fonts/InstrumentSans.ttf"),
        (
            "JetBrainsMono Nerd Font Mono",
            "assets/fonts/JetBrainsMonoNerdFontMono-Regular.ttf",
        ),
    ]
    .into_iter()
    .map(|(family, path)| (family.to_owned(), fs::read(path).expect("read UI font")))
    .collect();
    let options = slab_compile::Options {
        base_dir: PathBuf::from("ui"),
        fonts,
        ..slab_compile::Options::default()
    };
    let (module, diagnostics) = slab_compile::rustgen::generate(&source, &options, UI_SOURCE);
    for diagnostic in &diagnostics.0 {
        println!("cargo:warning={}", diagnostic.format(UI_SOURCE));
    }
    let module = module.expect("Slab UI source must compile").replacen(
        "#![allow(clippy::all, dead_code)]\n",
        "",
        1,
    );
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    fs::write(output.join("app.rs"), module).expect("write generated Slab UI binding");
}
