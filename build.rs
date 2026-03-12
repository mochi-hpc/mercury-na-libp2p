//! Build script for the Mercury NA plugin.
//!
//! 1. Locates Mercury include/lib paths via `MERCURY_INCLUDE_DIR` /
//!    `MERCURY_LIB_DIR` env vars (set by the CMake wrapper) or falls back to
//!    `pkg-config`.
//! 2. Runs bindgen on `wrapper.h` to generate Rust FFI types from the
//!    installed Mercury headers.
//! 3. Emits the correct `rustc-link-search` and `rustc-link-lib` directives.

use std::env;
use std::path::PathBuf;

fn main() {
    // --- Locate Mercury include & lib directories ---

    // MERCURY_INCLUDE_DIR may be colon-separated (set by the CMake wrapper),
    // e.g. "/path/to/include:/path/to/na_headers".
    let (include_dirs, lib_dir) = match (
        env::var("MERCURY_INCLUDE_DIR"),
        env::var("MERCURY_LIB_DIR"),
    ) {
        (Ok(inc), Ok(lib)) => {
            let dirs: Vec<String> = inc.split(':').map(|s| s.to_owned()).collect();
            (dirs, lib)
        }
        _ => {
            // Fall back to pkg-config
            let lib = pkg_config::Config::new()
                .probe("na")
                .expect("Could not find Mercury NA library via pkg-config or env vars");
            let dirs: Vec<String> = lib
                .include_paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            let libdir = lib
                .link_paths
                .first()
                .expect("pkg-config did not return any link paths")
                .to_string_lossy()
                .into_owned();
            (dirs, libdir)
        }
    };

    println!("cargo:rerun-if-env-changed=MERCURY_INCLUDE_DIR");
    println!("cargo:rerun-if-env-changed=MERCURY_LIB_DIR");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");
    println!("cargo:rerun-if-changed=wrapper.h");

    // --- Link flags ---
    println!("cargo:rustc-link-search=native={lib_dir}");
    println!("cargo:rustc-link-lib=dylib=na");

    // --- Bindgen ---
    let mut builder = bindgen::Builder::default()
        .header("wrapper.h");
    for dir in &include_dirs {
        builder = builder.clang_arg(format!("-I{dir}"));
    }
    let bindings = builder
        // Allowlist the types and functions we need.
        .allowlist_type("na_class_ops")
        .allowlist_type("na_class")
        .allowlist_type("na_context")
        .allowlist_type("na_addr")
        .allowlist_type("na_op_id")
        .allowlist_type("na_mem_handle")
        .allowlist_type("na_segment")
        .allowlist_type("na_info")
        .allowlist_type("na_init_info")
        .allowlist_type("na_protocol_info")
        .allowlist_type("na_cb_info")
        .allowlist_type("na_cb_completion_data")
        .allowlist_type("na_return")
        .allowlist_type("na_return_t")
        .allowlist_type("na_cb_type")
        .allowlist_type("na_cb_type_t")
        .allowlist_type("na_tag_t")
        .allowlist_type("na_offset_t")
        .allowlist_type("na_cb_t")
        .allowlist_type("na_mem_type")
        .allowlist_type("na_plugin_cb_t")
        .allowlist_function("na_cb_completion_add")
        .allowlist_function("na_protocol_info_alloc")
        .allowlist_function("na_protocol_info_free")
        // Derive common traits for generated types.
        .derive_default(true)
        .derive_debug(true)
        // Use core types (no std dependency needed for the bindings themselves).
        .use_core()
        // Generate the bindings.
        .generate()
        .expect("Failed to generate bindings from Mercury NA headers");

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Failed to write bindings.rs");
}
