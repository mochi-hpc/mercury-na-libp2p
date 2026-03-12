//! Mercury NA plugin template: "abc" (Rust)
//!
//! This crate produces a `cdylib` (`libna_plugin_abc.so`) that implements a
//! skeleton NA plugin.  Every callback that appears in `struct na_class_ops`
//! (defined in `na.h`) is listed in [`plugin`] with a brief doc-comment
//! explaining what the callback is expected to do.
//!
//! The exported symbol [`na_abc_class_ops_g`] is the ops table that NA's
//! dynamic plugin loader looks up via `dlsym()`.
//!
//! # Turning this into a real plugin
//!
//! 1. Run `./rename-plugin.sh <name>` to rename "abc" / "myprotocol" to your
//!    class / protocol names throughout the project.
//! 2. Define private data structures for addresses, memory handles, op IDs,
//!    and plugin-level class state.  Store class state in
//!    `na_class.plugin_class` during `initialize` (use `Box::into_raw()`),
//!    and reclaim it during `finalize` (use `Box::from_raw()`).
//! 3. Implement each callback in `src/plugin.rs` according to its doc-comment.

// Generated FFI bindings from Mercury's NA headers.
#[allow(non_upper_case_globals)]
#[allow(non_camel_case_types)]
#[allow(non_snake_case)]
#[allow(dead_code)]
pub(crate) mod bindings {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

    // na_class_ops is a table of function pointers and a `*const c_char`
    // class name.  All pointers are to static/const data, so the struct is
    // safe to share across threads.
    unsafe impl Sync for na_class_ops {}
}

mod plugin;

use bindings::*;
use std::ffi::c_char;

// ---------------------------------------------------------------------------
//  Return-code aliases for readability (matching the C enum values)
// ---------------------------------------------------------------------------

pub(crate) const NA_SUCCESS: na_return_t = na_return_NA_SUCCESS;
pub(crate) const NA_PROTOCOL_ERROR: na_return_t = na_return_NA_PROTOCOL_ERROR;
pub(crate) const NA_TIMEOUT: na_return_t = na_return_NA_TIMEOUT;

// ---------------------------------------------------------------------------
//  Plugin ops table
// ---------------------------------------------------------------------------

/// The class name as a NUL-terminated C string.
///
/// `c"abc"` is Rust's C string literal syntax (stable since 1.77).  It
/// produces a `&'static CStr` whose `.as_ptr()` gives `*const c_char`.
const CLASS_NAME: *const c_char = c"abc".as_ptr();

/// Exported symbol that NA's dynamic plugin loader looks up via `dlsym()`
/// when loading `libna_plugin_abc.so`.  The symbol name **must** follow the
/// pattern `na_<name>_class_ops_g` where `<name>` matches the file-name
/// pattern `libna_plugin_<name>.so`.
///
/// Callbacks set to `None` are optional; NA provides default behaviour or
/// simply skips the call.
#[no_mangle]
pub static na_abc_class_ops_g: na_class_ops = na_class_ops {
    // --- Identification ---
    class_name: CLASS_NAME,

    // --- Protocol discovery ---
    get_protocol_info: Some(plugin::na_abc_get_protocol_info),
    check_protocol: Some(plugin::na_abc_check_protocol),

    // --- Lifecycle ---
    initialize: Some(plugin::na_abc_initialize),
    finalize: Some(plugin::na_abc_finalize),
    cleanup: Some(plugin::na_abc_cleanup),
    has_opt_feature: None, // optional

    // --- Contexts ---
    context_create: Some(plugin::na_abc_context_create),
    context_destroy: Some(plugin::na_abc_context_destroy),

    // --- Operation IDs ---
    op_create: Some(plugin::na_abc_op_create),
    op_destroy: Some(plugin::na_abc_op_destroy),

    // --- Addressing ---
    addr_lookup: Some(plugin::na_abc_addr_lookup),
    addr_free: Some(plugin::na_abc_addr_free),
    addr_set_remove: None, // optional
    addr_self: Some(plugin::na_abc_addr_self),
    addr_dup: Some(plugin::na_abc_addr_dup),
    addr_cmp: Some(plugin::na_abc_addr_cmp),
    addr_is_self: Some(plugin::na_abc_addr_is_self),
    addr_to_string: Some(plugin::na_abc_addr_to_string),
    addr_get_serialize_size: Some(plugin::na_abc_addr_get_serialize_size),
    addr_serialize: Some(plugin::na_abc_addr_serialize),
    addr_deserialize: Some(plugin::na_abc_addr_deserialize),

    // --- Message sizes & tags ---
    msg_get_max_unexpected_size: Some(plugin::na_abc_msg_get_max_unexpected_size),
    msg_get_max_expected_size: Some(plugin::na_abc_msg_get_max_expected_size),
    msg_get_unexpected_header_size: None, // optional
    msg_get_expected_header_size: None,   // optional
    msg_get_max_tag: Some(plugin::na_abc_msg_get_max_tag),

    // --- Message buffers ---
    msg_buf_alloc: None, // optional -- NA provides default
    msg_buf_free: None,  // optional -- NA provides default

    // --- Unexpected messages ---
    msg_init_unexpected: None, // optional
    msg_send_unexpected: Some(plugin::na_abc_msg_send_unexpected),
    msg_recv_unexpected: Some(plugin::na_abc_msg_recv_unexpected),
    msg_multi_recv_unexpected: None, // optional

    // --- Expected messages ---
    msg_init_expected: None, // optional
    msg_send_expected: Some(plugin::na_abc_msg_send_expected),
    msg_recv_expected: Some(plugin::na_abc_msg_recv_expected),

    // --- Memory handles ---
    mem_handle_create: Some(plugin::na_abc_mem_handle_create),
    mem_handle_create_segments: None, // optional
    mem_handle_free: Some(plugin::na_abc_mem_handle_free),
    mem_handle_get_max_segments: None, // optional
    mem_register: None,                // optional
    mem_deregister: None,              // optional
    mem_handle_get_serialize_size: Some(plugin::na_abc_mem_handle_get_serialize_size),
    mem_handle_serialize: Some(plugin::na_abc_mem_handle_serialize),
    mem_handle_deserialize: Some(plugin::na_abc_mem_handle_deserialize),

    // --- RMA ---
    put: Some(plugin::na_abc_put),
    get: Some(plugin::na_abc_get),

    // --- Progress ---
    poll_get_fd: Some(plugin::na_abc_poll_get_fd),
    poll_try_wait: Some(plugin::na_abc_poll_try_wait),
    poll: Some(plugin::na_abc_poll),
    poll_wait: None, // optional -- NA busy-polls if NULL
    cancel: Some(plugin::na_abc_cancel),
};
