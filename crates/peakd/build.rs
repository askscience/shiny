//! Build the Qt 6 C++ shim and find the Qt modules through pkg-config.
//!
//! This validates that Debian's Qt 6.8 + QtWebEngine toolchain links from a
//! Cargo build. The shim is
//! deliberately tiny — window, main view, IPC bridge, one child view — and
//! grows into the real shell only after the spike passes.

#[cfg(target_os = "linux")]
fn main() {
    use std::path::PathBuf;
    use std::process::Command;

    // QtWebEngineWidgets pulls in Core/Gui/Network; WebChannel is explicit
    // because the IPC bridge needs it.
    const MODULES: &[&str] = &[
        "Qt6Core",
        "Qt6Gui",
        "Qt6Widgets",
        "Qt6Network",
        "Qt6WebChannel",
        "Qt6WebEngineWidgets",
    ];
    let mut includes = Vec::new();
    for module in MODULES {
        let library = pkg_config::Config::new()
            .cargo_metadata(true)
            .probe(module)
            .unwrap_or_else(|err| panic!("pkg-config could not find {module}: {err}"));
        includes.extend(library.include_paths);
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let moc = find_moc();
    let moc_output = out_dir.join("moc_ipc_bridge.cpp");
    let mut moc_cmd = Command::new(&moc);
    moc_cmd.arg("-I").arg("shim");
    for include in &includes {
        moc_cmd.arg("-I").arg(include);
    }
    moc_cmd
        .arg("shim/ipc_bridge.h")
        .arg("-o")
        .arg(&moc_output);
    let status = moc_cmd
        .status()
        .unwrap_or_else(|err| panic!("could not run {}: {err}", moc.display()));
    assert!(status.success(), "moc failed on shim/ipc_bridge.h");

    for file in [
        "shim/peakd.cpp",
        "shim/peakd.h",
        "shim/ipc_bridge.h",
        "build.rs",
    ] {
        println!("cargo:rerun-if-changed={file}");
    }

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .file("shim/peakd.cpp")
        .file(&moc_output)
        .include("shim")
        .include(&out_dir)
        .flag_if_supported("-fPIC")
        .flag_if_supported("-Wno-deprecated-declarations");
    for include in includes {
        build.include(include);
    }
    build.compile("peakd_qt_shim");
}

/// `moc` ships in qt6-base-dev-tools; ask qmake where it is, with fallbacks.
#[cfg(target_os = "linux")]
fn find_moc() -> std::path::PathBuf {
    use std::path::PathBuf;

    if let Ok(output) = std::process::Command::new("qmake6")
        .args(["-query", "QT_HOST_LIBEXECS"])
        .output()
    {
        if output.status.success() {
            let dir = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let candidate = PathBuf::from(dir).join("moc");
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    for candidate in ["/usr/lib/qt6/libexec/moc", "/usr/lib/qt6/bin/moc"] {
        let path = PathBuf::from(candidate);
        if path.is_file() {
            return path;
        }
    }
    panic!("moc not found; install qt6-base-dev-tools");
}

#[cfg(not(target_os = "linux"))]
fn main() {}
