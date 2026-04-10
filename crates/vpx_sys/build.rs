use std::env;
use std::path::PathBuf;

fn main() {
    // Try pkg-config first, fall back to manual search
    if let Ok(lib) = pkg_config::probe_library("vpx") {
        // pkg-config found it — link flags are already emitted.
        // Use the first include path for bindgen.
        let include = lib
            .include_paths
            .first()
            .cloned()
            .unwrap_or_else(|| PathBuf::from("/opt/homebrew/include"));

        generate_bindings(&include);
        return;
    }

    // Manual fallback: search common paths
    let search_paths = [
        "/opt/homebrew/lib",
        "/usr/local/lib",
        "/usr/lib",
        "/usr/lib/x86_64-linux-gnu",
    ];
    let include_paths = [
        "/opt/homebrew/include",
        "/usr/local/include",
        "/usr/include",
    ];

    for path in &search_paths {
        println!("cargo:rustc-link-search=native={}", path);
    }
    println!("cargo:rustc-link-lib=vpx");

    let include = include_paths
        .iter()
        .map(PathBuf::from)
        .find(|p| p.join("vpx/vpx_codec.h").exists())
        .expect("Could not find vpx headers. Install libvpx-dev or libvpx.");

    generate_bindings(&include);
}

fn generate_bindings(include_path: &std::path::Path) {
    let wrapper = "\
#include <vpx/vpx_codec.h>
#include <vpx/vpx_decoder.h>
#include <vpx/vpx_encoder.h>
#include <vpx/vpx_image.h>
#include <vpx/vp8.h>
#include <vpx/vp8cx.h>
#include <vpx/vp8dx.h>
";

    // Write wrapper to a temp file
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let wrapper_path = out_dir.join("wrapper.h");
    std::fs::write(&wrapper_path, wrapper).expect("Failed to write wrapper.h");

    let bindings = bindgen::Builder::default()
        .header(wrapper_path.to_str().unwrap())
        .clang_arg(format!("-I{}", include_path.display()))
        .allowlist_function("vpx_.*")
        .allowlist_type("vpx_.*")
        .allowlist_type("vp8e_enc_control_id")
        .allowlist_type("vp9e_enc_control_id")
        .allowlist_var("VPX_.*")
        .allowlist_var("VP8.*")
        .allowlist_var("VP9.*")
        // Derive useful traits
        .derive_debug(true)
        .derive_default(true)
        .derive_copy(true)
        .generate()
        .expect("Failed to generate vpx bindings");

    bindings
        .write_to_file(out_dir.join("bindings.rs"))
        .expect("Failed to write bindings");
}
