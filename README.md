# Mercury NA Plugin Template (Rust)

A Rust template for building dynamically-loaded Mercury Network Abstraction
(NA) plugins. Produces a `libna_plugin_abc.so` that Mercury discovers at
runtime via `NA_PLUGIN_PATH`, identical in interface to a C plugin but with
the implementation written in Rust.

## Prerequisites

| Tool | Version | Purpose |
|------|---------|---------|
| Rust (rustc, cargo) | stable >= 1.77 | Compile the plugin (`c""` literals require 1.77+) |
| libclang-dev | >= 13 | Required by the `bindgen` crate to parse C headers |
| CMake | >= 3.15 | Build wrapper and C test suite |
| Mercury | >= 2.4 | Provides `libna.so`, headers, and cmake config |

Cargo automatically fetches the `bindgen` and `pkg-config` build-dependencies.

## Project structure

```
Cargo.toml              # cdylib crate config
build.rs                # Runs bindgen, emits link flags
wrapper.h               # Thin #include <na_plugin.h> for bindgen
src/
  lib.rs                # Exports na_abc_class_ops_g ops table
  plugin.rs             # All ~34 extern "C" callback stubs
CMakeLists.txt          # Finds Mercury, invokes cargo, builds C tests
test/                   # C test suite (language-agnostic, from Mercury)
rename-plugin.sh        # Rename "abc" to your plugin name
```

## Building

### Option A: Cargo only (plugin .so)

If Mercury is installed and discoverable via `pkg-config`:

```sh
PKG_CONFIG_PATH=/path/to/mercury/lib/pkgconfig cargo build --release
```

The plugin is at `target/release/libna_plugin_abc.so`.

If Mercury is not in `pkg-config`, set the paths explicitly:

```sh
MERCURY_INCLUDE_DIR=/path/to/mercury/include \
MERCURY_LIB_DIR=/path/to/mercury/lib \
cargo build --release
```

Multiple include directories can be colon-separated:

```sh
MERCURY_INCLUDE_DIR=/path/to/include:/path/to/extra/headers ...
```

### Option B: CMake (plugin + tests)

```sh
mkdir build && cd build
cmake .. -Dmercury_DIR=/path/to/mercury/share/cmake/mercury
make -j$(nproc)
```

This builds both the Rust plugin (via cargo) and the C test executables.
The plugin `.so` is placed in `build/cargo-build/release/`.

## Testing

From the CMake build directory:

```sh
LD_LIBRARY_PATH=/path/to/mercury/lib ctest --output-on-failure
```

The test suite exercises the plugin through Mercury's public NA API. Since
the template stubs return `NA_PROTOCOL_ERROR` from `initialize()`, only the
`abc_proc` test (which does not require initialization) will pass. The
remaining tests will pass once you implement the callbacks.

### Expected results for the unmodified template

| Test | Result | Reason |
|------|--------|--------|
| `abc_proc` | Pass | Standalone serialization test, no plugin init needed |
| `abc_init` | Fail | `initialize()` returns `NA_PROTOCOL_ERROR` |
| `abc_msg` | Fail | Cannot initialize plugin |
| `abc_lookup` | Fail | Cannot initialize plugin |
| `abc_rpc` | Fail | Cannot initialize plugin |
| `abc_bulk` | Fail | Cannot initialize plugin |

## Renaming the plugin

To rename from "abc" to your own plugin name (e.g. "xyz"):

```sh
./rename-plugin.sh xyz
```

This updates `Cargo.toml`, `CMakeLists.txt`, `src/lib.rs`, `src/plugin.rs`,
and all test files. The output library becomes `libna_plugin_xyz.so` with
exported symbol `na_xyz_class_ops_g`.

Remove any old `build/` and `target/` directories before rebuilding.

## Implementing your plugin

1. **Rename**: `./rename-plugin.sh <name>`

2. **Define state structs** in `src/plugin.rs` for your addresses, memory
   handles, operation IDs, and class-level state.

3. **Implement callbacks** in `src/plugin.rs`. Each function has a doc-comment
   explaining the expected behaviour and a `// TODO` marker. Key FFI patterns:

   - **Storing Rust state in C structs**: Use `Box::into_raw()` to store a
     boxed struct in `na_class.plugin_class` during `initialize`, and
     `Box::from_raw()` to reclaim it during `finalize`.

   - **Completing async operations**: Fill in an `na_cb_completion_data`
     struct and call `na_cb_completion_add(context, &mut completion_data)`.

   - **Nullable function pointers**: `Some(f)` = non-NULL callback,
     `None` = NULL (optional callback). Set unused optional callbacks to
     `None` in the ops table in `lib.rs`.

4. **Run tests**: `cd build && cmake .. && make && ctest`

## Runtime usage

Point Mercury at your plugin directory:

```sh
NA_PLUGIN_PATH=/path/to/dir/containing/libna_plugin_xyz.so  your_application
```

Or register the plugin programmatically from C:

```c
extern const struct na_class_ops na_xyz_class_ops_g;
NA_Register_plugin(&na_xyz_class_ops_g);
```
