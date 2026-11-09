//! Build the Qt 6 WebEngine shim (Linux only; elsewhere the binary reports
//! that it is unavailable, so a workspace build never needs Qt).

#[cfg(target_os = "linux")]
fn main() {
    const MODULES: &[&str] = &[
        "Qt6Core",
        "Qt6Gui",
        "Qt6Widgets",
        "Qt6WebEngineCore",
        "Qt6WebEngineWidgets",
    ];
    let mut includes = Vec::new();
    for m in MODULES {
        match pkg_config::Config::new().cargo_metadata(true).probe(m) {
            Ok(lib) => includes.extend(lib.include_paths),
            Err(e) => {
                // No Qt dev packages: build a stub that explains itself.
                println!("cargo:warning=capture-render: {m} not found ({e}); building without the renderer");
                println!("cargo:rustc-cfg=no_qt");
                println!("cargo:rustc-check-cfg=cfg(no_qt)");
                return;
            }
        }
    }
    println!("cargo:rustc-check-cfg=cfg(no_qt)");
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .file("shim/render.cpp")
        .flag_if_supported("-std=c++17")
        .flag_if_supported("-fPIC");
    for i in &includes {
        build.include(i);
    }
    build.compile("capture_render_shim");
    println!("cargo:rustc-link-lib=stdc++");
    println!("cargo:rerun-if-changed=shim/render.cpp");
}

#[cfg(not(target_os = "linux"))]
fn main() {
    println!("cargo:rustc-cfg=no_qt");
    println!("cargo:rustc-check-cfg=cfg(no_qt)");
}
