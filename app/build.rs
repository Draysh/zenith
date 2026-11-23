//! Builds the app's QML module, `Zenith`: every file in `qml/`, the
//! Rust QObjects in `src/bridge/`, and the C++ in `cpp/` (the mpv video item
//! and a few Qt calls Rust has no binding for).

use cxx_qt_build::{CxxQtBuilder, QmlFile, QmlModule};

fn main() {
    // On Windows the icon is part of the program file: what the taskbar,
    // the Start Menu and the window's corner show.
    #[cfg(windows)]
    {
        println!("cargo::rerun-if-changed=icons/zenith.ico");
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("icons/zenith.ico");
        if let Err(e) = resource.compile() {
            println!("cargo::warning=the icon could not be put into the program: {e}");
        }
    }
    for dir in ["qml", "cpp", "src/bridge", "fonts"] {
        println!("cargo::rerun-if-changed={dir}");
    }
    let mut files: Vec<_> = std::fs::read_dir("qml")
        .expect("a qml directory")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "qml"))
        .collect();
    files.sort();
    let mut module = QmlModule::new("Zenith");
    for f in &files {
        let singleton = std::fs::read_to_string(f)
            .expect("a qml file")
            .lines()
            .map(str::trim)
            .take_while(|l| l.is_empty() || l.starts_with("pragma") || l.starts_with("//"))
            .any(|l| l == "pragma Singleton");
        module = module.qml_file(QmlFile::from(f.to_string_lossy().as_ref()).singleton(singleton));
    }
    let mut bridges: Vec<_> = std::fs::read_dir("src/bridge")
        .expect("src/bridge")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "rs") && p.file_name().is_some_and(|n| n != "mod.rs"))
        .collect();
    bridges.sort();

    // libmpv: found by itself where the system keeps libraries (Linux).
    // `MPV_DIR` names a folder holding `include/mpv/*.h` and the library to
    // link (`mpv.lib`), for where it is not (Windows).
    println!("cargo::rerun-if-env-changed=MPV_DIR");
    println!("cargo::rustc-link-lib=mpv");
    let mut builder = CxxQtBuilder::new_qml_module(module);
    if let Some(dir) = std::env::var_os("MPV_DIR").map(std::path::PathBuf::from) {
        println!("cargo::rustc-link-search=native={}", dir.display());
        builder = builder.include_dir(dir.join("include"));
    }
    builder
        .qt_module("Quick")
        .qt_module("OpenGL")
        .files(bridges)
        .cpp_files(["cpp/mpvitem.h", "cpp/mpvitem.cpp"])
        // The folder's own list of its singletons, beside the files: a file
        // finds its neighbours through the folder first.
        .qrc_resources([
            "qml/qmldir",
            // The typefaces, so the look does not depend on what is installed.
            "fonts/Anton-Regular.ttf",
            "fonts/Inter.ttf",
            "fonts/IBMPlexMono-Regular.ttf",
            "fonts/IBMPlexMono-Medium.ttf",
        ])
        .build();
}
