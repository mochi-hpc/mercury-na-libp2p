cmake_minimum_required(VERSION 3.15)
project(na_plugin_libp2p_rust)

# Find Mercury's NA library (provides include/lib paths)
find_package(mercury REQUIRED)

# na_plugin.h and na_error.h are required to build an NA plugin.  If the
# installed Mercury headers already contain them (Mercury >= 2.4), we use
# those directly.  Otherwise, download them from the matching release tag.
get_target_property(_na_inc_dirs na INTERFACE_INCLUDE_DIRECTORIES)

# Filter out generator expressions -- we only need the concrete paths
set(_concrete_inc_dirs)
foreach(_d ${_na_inc_dirs})
    if(NOT _d MATCHES "^\\$<")
        list(APPEND _concrete_inc_dirs "${_d}")
    endif()
endforeach()

# Also try mercury_util for additional concrete include paths
get_target_property(_util_inc_dirs mercury_util INTERFACE_INCLUDE_DIRECTORIES)
if(_util_inc_dirs)
    foreach(_d ${_util_inc_dirs})
        if(NOT _d MATCHES "^\\$<")
            list(APPEND _concrete_inc_dirs "${_d}")
        endif()
    endforeach()
endif()

list(REMOVE_DUPLICATES _concrete_inc_dirs)

set(_have_plugin_h FALSE)
foreach(_d ${_concrete_inc_dirs})
    if(EXISTS "${_d}/na_plugin.h")
        set(_have_plugin_h TRUE)
        break()
    endif()
endforeach()

if(NOT _have_plugin_h)
    set(_na_hdr_dir "${CMAKE_CURRENT_BINARY_DIR}/na_headers")
    set(_na_hdr_base_url
        "https://raw.githubusercontent.com/mercury-hpc/mercury/v${MERCURY_VERSION_FULL}/src/na")
    foreach(_hdr na_plugin.h na_error.h)
        if(NOT EXISTS "${_na_hdr_dir}/${_hdr}")
            message(STATUS "Downloading ${_hdr} for Mercury ${MERCURY_VERSION_FULL}")
            file(DOWNLOAD
                "${_na_hdr_base_url}/${_hdr}"
                "${_na_hdr_dir}/${_hdr}"
                STATUS _dl_status
            )
            list(GET _dl_status 0 _dl_code)
            if(NOT _dl_code EQUAL 0)
                list(GET _dl_status 1 _dl_msg)
                message(FATAL_ERROR "Failed to download ${_hdr}: ${_dl_msg}")
            endif()
        endif()
    endforeach()
    list(APPEND _concrete_inc_dirs "${_na_hdr_dir}")
endif()

# Build a colon-separated include path for the Rust build script
string(REPLACE ";" ":" _mercury_include_path "${_concrete_inc_dirs}")

# Locate the library directory
get_target_property(_na_loc na IMPORTED_LOCATION_RELWITHDEBINFO)
if(NOT _na_loc)
    get_target_property(_na_loc na IMPORTED_LOCATION)
endif()
if(NOT _na_loc)
    get_target_property(_na_loc na IMPORTED_LOCATION_RELEASE)
endif()
get_filename_component(_mercury_lib_dir "${_na_loc}" DIRECTORY)

# Cargo output directory inside the build tree
set(_cargo_build_dir "${CMAKE_CURRENT_BINARY_DIR}/cargo-build")
set(_cargo_target_dir "${_cargo_build_dir}")
set(_plugin_so "${_cargo_build_dir}/release/libna_plugin_libp2p.so")

# Build the Rust plugin via cargo
add_custom_command(
    OUTPUT "${_plugin_so}"
    COMMAND ${CMAKE_COMMAND} -E env
        "MERCURY_INCLUDE_DIR=${_mercury_include_path}"
        "MERCURY_LIB_DIR=${_mercury_lib_dir}"
        "CARGO_TARGET_DIR=${_cargo_target_dir}"
        cargo build --release
        --manifest-path "${CMAKE_CURRENT_SOURCE_DIR}/Cargo.toml"
    DEPENDS
        "${CMAKE_CURRENT_SOURCE_DIR}/Cargo.toml"
        "${CMAKE_CURRENT_SOURCE_DIR}/build.rs"
        "${CMAKE_CURRENT_SOURCE_DIR}/wrapper.h"
        "${CMAKE_CURRENT_SOURCE_DIR}/src/lib.rs"
        "${CMAKE_CURRENT_SOURCE_DIR}/src/plugin.rs"
        "${CMAKE_CURRENT_SOURCE_DIR}/src/state.rs"
        "${CMAKE_CURRENT_SOURCE_DIR}/src/runtime.rs"
        "${CMAKE_CURRENT_SOURCE_DIR}/src/protocol.rs"
    WORKING_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}"
    COMMENT "Building Rust NA plugin (cargo build --release)"
    VERBATIM
)
add_custom_target(na_plugin_libp2p ALL DEPENDS "${_plugin_so}")

# Installation -----------------------------------------------------------------
if(NOT NA_PLUGIN_INSTALL_DIR)
    include(GNUInstallDirs)
    set(NA_PLUGIN_INSTALL_DIR ${CMAKE_INSTALL_LIBDIR})
endif()

install(FILES "${_plugin_so}"
    DESTINATION ${NA_PLUGIN_INSTALL_DIR}
)

# Tests
enable_testing()
add_subdirectory(test)
