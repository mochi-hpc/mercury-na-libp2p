//! Mercury NA plugin: "libp2p" (Rust)
//!
//! This crate produces a `cdylib` (`libna_plugin_libp2p.so`) that implements
//! a Mercury NA plugin using libp2p as the underlying transport.
//!
//! Transport stack: TCP + Noise (encryption) + Yamux (stream muxing) +
//! libp2p-stream protocol for messaging.

// Generated FFI bindings from Mercury's NA headers.
#[allow(non_upper_case_globals)]
#[allow(non_camel_case_types)]
#[allow(non_snake_case)]
#[allow(dead_code)]
pub(crate) mod bindings {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

    unsafe impl Sync for na_class_ops {}
}

mod plugin;
mod protocol;
mod runtime;
mod state;

use bindings::*;
use std::ffi::c_char;

// Return-code aliases
pub(crate) const NA_SUCCESS: na_return_t = na_return_NA_SUCCESS;
pub(crate) const NA_PROTOCOL_ERROR: na_return_t = na_return_NA_PROTOCOL_ERROR;
pub(crate) const NA_TIMEOUT: na_return_t = na_return_NA_TIMEOUT;
pub(crate) const NA_NOMEM: na_return_t = na_return_NA_NOMEM;
pub(crate) const NA_CANCELED: na_return_t = na_return_NA_CANCELED;
pub(crate) const NA_OVERFLOW: na_return_t = na_return_NA_OVERFLOW;
pub(crate) const NA_INVALID_ARG: na_return_t = na_return_NA_INVALID_ARG;


const CLASS_NAME: *const c_char = c"libp2p".as_ptr();

#[no_mangle]
pub static na_libp2p_class_ops_g: na_class_ops = na_class_ops {
    class_name: CLASS_NAME,

    get_protocol_info: Some(plugin::na_libp2p_get_protocol_info),
    check_protocol: Some(plugin::na_libp2p_check_protocol),

    initialize: Some(plugin::na_libp2p_initialize),
    finalize: Some(plugin::na_libp2p_finalize),
    cleanup: Some(plugin::na_libp2p_cleanup),
    has_opt_feature: None,

    context_create: Some(plugin::na_libp2p_context_create),
    context_destroy: Some(plugin::na_libp2p_context_destroy),

    op_create: Some(plugin::na_libp2p_op_create),
    op_destroy: Some(plugin::na_libp2p_op_destroy),

    addr_lookup: Some(plugin::na_libp2p_addr_lookup),
    addr_free: Some(plugin::na_libp2p_addr_free),
    addr_set_remove: None,
    addr_self: Some(plugin::na_libp2p_addr_self),
    addr_dup: Some(plugin::na_libp2p_addr_dup),
    addr_cmp: Some(plugin::na_libp2p_addr_cmp),
    addr_is_self: Some(plugin::na_libp2p_addr_is_self),
    addr_to_string: Some(plugin::na_libp2p_addr_to_string),
    addr_get_serialize_size: Some(plugin::na_libp2p_addr_get_serialize_size),
    addr_serialize: Some(plugin::na_libp2p_addr_serialize),
    addr_deserialize: Some(plugin::na_libp2p_addr_deserialize),

    msg_get_max_unexpected_size: Some(plugin::na_libp2p_msg_get_max_unexpected_size),
    msg_get_max_expected_size: Some(plugin::na_libp2p_msg_get_max_expected_size),
    msg_get_unexpected_header_size: None,
    msg_get_expected_header_size: None,
    msg_get_max_tag: Some(plugin::na_libp2p_msg_get_max_tag),

    msg_buf_alloc: None,
    msg_buf_free: None,

    msg_init_unexpected: None,
    msg_send_unexpected: Some(plugin::na_libp2p_msg_send_unexpected),
    msg_recv_unexpected: Some(plugin::na_libp2p_msg_recv_unexpected),
    msg_multi_recv_unexpected: None,

    msg_init_expected: None,
    msg_send_expected: Some(plugin::na_libp2p_msg_send_expected),
    msg_recv_expected: Some(plugin::na_libp2p_msg_recv_expected),

    mem_handle_create: Some(plugin::na_libp2p_mem_handle_create),
    mem_handle_create_segments: None,
    mem_handle_free: Some(plugin::na_libp2p_mem_handle_free),
    mem_handle_get_max_segments: None,
    mem_register: None,
    mem_deregister: None,
    mem_handle_get_serialize_size: Some(plugin::na_libp2p_mem_handle_get_serialize_size),
    mem_handle_serialize: Some(plugin::na_libp2p_mem_handle_serialize),
    mem_handle_deserialize: Some(plugin::na_libp2p_mem_handle_deserialize),

    put: Some(plugin::na_libp2p_put),
    get: Some(plugin::na_libp2p_get),

    poll_get_fd: Some(plugin::na_libp2p_poll_get_fd),
    poll_try_wait: Some(plugin::na_libp2p_poll_try_wait),
    poll: Some(plugin::na_libp2p_poll),
    poll_wait: None,
    cancel: Some(plugin::na_libp2p_cancel),
};
