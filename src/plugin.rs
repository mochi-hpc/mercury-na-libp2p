//! All NA plugin callback stubs for the "abc" plugin.
//!
//! Each function corresponds to a field in `struct na_class_ops`.  Required
//! callbacks return an error (`NA_PROTOCOL_ERROR`) to indicate "not yet
//! implemented"; optional callbacks that can safely be left `NULL` are set to
//! `None` in the ops table in `lib.rs`.
//!
//! # FFI patterns used in this template
//!
//! - **Storing Rust state in C structs**: Use `Box::into_raw()` to convert a
//!   boxed Rust struct into a `*mut c_void` and store it in
//!   `na_class.plugin_class` during `initialize`.  Use `Box::from_raw()` to
//!   reclaim it during `finalize`.
//!
//! - **All callbacks are `unsafe extern "C"`** because they are called from C.
//!
//! - **`Option<unsafe extern "C" fn(...)>`** maps to nullable function
//!   pointers: `Some(f)` = non-NULL, `None` = NULL.

use std::ffi::c_void;

use crate::bindings::*;
use crate::{NA_PROTOCOL_ERROR, NA_SUCCESS, NA_TIMEOUT};

// ------------------------------------------------------------------ //
//  Protocol discovery                                                  //
// ------------------------------------------------------------------ //

/// Return protocol info entries for protocols this plugin supports.
///
/// Called by `NA_Get_protocol_info()`.  Allocate entries with
/// `na_protocol_info_alloc()` and chain them via the `->next` pointer.
pub(crate) unsafe extern "C" fn na_abc_get_protocol_info(
    _na_info: *const na_info,
    na_protocol_info_p: *mut *mut na_protocol_info,
) -> na_return_t {
    // TODO: Allocate one or more protocol info entries using
    // na_protocol_info_alloc("abc", "myprotocol", "device0") and chain
    // them via ->next.  Return the head of the list via *na_protocol_info_p.
    // If na_info is non-NULL, use it to filter results (e.g. matching a
    // requested protocol name).
    unsafe { *na_protocol_info_p = std::ptr::null_mut() };
    NA_SUCCESS
}

/// Return `true` if this plugin can handle the given protocol name.
///
/// For example, if your plugin handles `"myprotocol"`, return
/// `strcmp(protocol_name, "myprotocol") == 0`.
///
/// **REQUIRED** -- must not be `None`.
pub(crate) unsafe extern "C" fn na_abc_check_protocol(
    protocol_name: *const std::ffi::c_char,
) -> bool {
    // TODO: Return true if protocol_name matches a protocol this plugin
    // handles (e.g. "myprotocol").
    let name = unsafe { std::ffi::CStr::from_ptr(protocol_name) };
    name == c"myprotocol"
}

// ------------------------------------------------------------------ //
//  Lifecycle                                                           //
// ------------------------------------------------------------------ //

/// Allocate and initialise plugin-private state.
///
/// Store it in `na_class.plugin_class` (use `Box::into_raw()` for Rust
/// state).  Parse connection details from `na_info`.  If `listen` is
/// `true`, set up to accept incoming connections.
///
/// **REQUIRED** -- must not be `None`.
pub(crate) unsafe extern "C" fn na_abc_initialize(
    _na_class: *mut na_class_t,
    _na_info: *const na_info,
    _listen: bool,
) -> na_return_t {
    // TODO: Allocate plugin-private class state and store in
    // na_class.plugin_class.  Parse connection info from na_info
    // (protocol_name, host_name, na_init_info).  If listen is true,
    // start accepting incoming connections.
    //
    // Example (storing Rust state):
    //   let state = Box::new(MyPluginState::new());
    //   (*na_class).plugin_class = Box::into_raw(state) as *mut c_void;
    NA_PROTOCOL_ERROR // not yet implemented
}

/// Tear down plugin-private state that was set up in `initialize()`.
///
/// Free `na_class.plugin_class` (use `Box::from_raw()` to reclaim
/// Rust state).
///
/// **REQUIRED** -- must not be `None`.
pub(crate) unsafe extern "C" fn na_abc_finalize(
    _na_class: *mut na_class_t,
) -> na_return_t {
    // TODO: Shut down transport, release resources, free
    // na_class.plugin_class.
    //
    // Example (reclaiming Rust state):
    //   let _state = Box::from_raw((*na_class).plugin_class as *mut MyPluginState);
    NA_SUCCESS
}

/// Clean up any global/static resources (temp files, shared memory, etc.)
/// that would otherwise survive process termination.
///
/// Called from `NA_Cleanup()`.  Optional -- may be `None`.
pub(crate) unsafe extern "C" fn na_abc_cleanup() {
    // TODO: Remove any global resources (temp files, shared memory segments)
    // that would persist after the process exits.
}

// ------------------------------------------------------------------ //
//  Contexts                                                            //
// ------------------------------------------------------------------ //

/// Allocate per-context state.  Store it in `*plugin_context_p`.
///
/// `id` is a caller-chosen context identifier (e.g. for multi-context).
pub(crate) unsafe extern "C" fn na_abc_context_create(
    _na_class: *mut na_class_t,
    _na_context: *mut na_context_t,
    plugin_context_p: *mut *mut c_void,
    _id: u8,
) -> na_return_t {
    // TODO: Allocate per-context state (e.g. completion queues, poll sets)
    // and return it via *plugin_context_p.  The context id can be used to
    // bind to specific hardware resources.
    unsafe { *plugin_context_p = std::ptr::null_mut() };
    NA_SUCCESS
}

/// Free per-context state allocated in `context_create()`.
pub(crate) unsafe extern "C" fn na_abc_context_destroy(
    _na_class: *mut na_class_t,
    _plugin_context: *mut c_void,
) -> na_return_t {
    // TODO: Free per-context state allocated in context_create().
    NA_SUCCESS
}

// ------------------------------------------------------------------ //
//  Operation IDs                                                       //
// ------------------------------------------------------------------ //

/// Allocate an operation ID.
///
/// `flags` is reserved for future use.  The returned op ID is passed
/// into send/recv/put/get calls and is used to track and cancel
/// in-flight operations.
///
/// **REQUIRED**.
pub(crate) unsafe extern "C" fn na_abc_op_create(
    _na_class: *mut na_class_t,
    _flags: std::ffi::c_ulong,
) -> *mut na_op_id_t {
    // TODO: Allocate and return an operation ID structure.  This is the
    // handle that tracks an in-flight send/recv/put/get/cancel.
    std::ptr::null_mut()
}

/// Free an operation ID allocated by `op_create()`.
///
/// **REQUIRED**.
pub(crate) unsafe extern "C" fn na_abc_op_destroy(
    _na_class: *mut na_class_t,
    _op_id: *mut na_op_id_t,
) {
    // TODO: Free the operation ID allocated by op_create().
}

// ------------------------------------------------------------------ //
//  Addressing                                                          //
// ------------------------------------------------------------------ //

/// Resolve a peer address from a string name (e.g. `"host:port"`).
///
/// Allocate an `na_addr_t` and return it via `*addr_p`.
///
/// **REQUIRED**.
pub(crate) unsafe extern "C" fn na_abc_addr_lookup(
    _na_class: *mut na_class_t,
    _name: *const std::ffi::c_char,
    _addr_p: *mut *mut na_addr_t,
) -> na_return_t {
    // TODO: Resolve a string address (e.g. "host:port") to an internal
    // address structure.  Allocate and return via *addr_p.
    NA_PROTOCOL_ERROR
}

/// Free an address returned by `addr_lookup`, `addr_self`, `addr_dup`, or
/// `addr_deserialize`.
///
/// **REQUIRED**.
pub(crate) unsafe extern "C" fn na_abc_addr_free(
    _na_class: *mut na_class_t,
    _addr: *mut na_addr_t,
) {
    // TODO: Free an address allocated by addr_lookup/addr_self/addr_dup/
    // addr_deserialize.
}

/// Return the address of this process/endpoint via `*addr_p`.
///
/// **REQUIRED**.
pub(crate) unsafe extern "C" fn na_abc_addr_self(
    _na_class: *mut na_class_t,
    _addr_p: *mut *mut na_addr_t,
) -> na_return_t {
    // TODO: Allocate and return the local endpoint address.
    NA_PROTOCOL_ERROR
}

/// Duplicate an address (deep copy).
///
/// **REQUIRED**.
pub(crate) unsafe extern "C" fn na_abc_addr_dup(
    _na_class: *mut na_class_t,
    _addr: *mut na_addr_t,
    _new_addr_p: *mut *mut na_addr_t,
) -> na_return_t {
    // TODO: Deep-copy addr into a newly allocated address.
    NA_PROTOCOL_ERROR
}

/// Return `true` if `addr1` and `addr2` refer to the same endpoint.
///
/// **REQUIRED**.
pub(crate) unsafe extern "C" fn na_abc_addr_cmp(
    _na_class: *mut na_class_t,
    _addr1: *mut na_addr_t,
    _addr2: *mut na_addr_t,
) -> bool {
    // TODO: Return true if addr1 and addr2 point to the same endpoint.
    false
}

/// Return `true` if `addr` is the local (self) address.
///
/// **REQUIRED**.
pub(crate) unsafe extern "C" fn na_abc_addr_is_self(
    _na_class: *mut na_class_t,
    _addr: *mut na_addr_t,
) -> bool {
    // TODO: Return true if addr is the local address.
    false
}

/// Convert an address to a human-readable string.
///
/// Write at most `*buf_size` bytes into `buf` and update `*buf_size`
/// to the total size needed (including the terminating NUL).  If `buf`
/// is NULL, just return the required size.
///
/// **REQUIRED**.
pub(crate) unsafe extern "C" fn na_abc_addr_to_string(
    _na_class: *mut na_class_t,
    _buf: *mut std::ffi::c_char,
    _buf_size: *mut usize,
    _addr: *mut na_addr_t,
) -> na_return_t {
    // TODO: Write a human-readable representation of addr into buf.
    // Update *buf_size to the total bytes needed (including NUL).
    // If buf is NULL, just report the required size.
    NA_PROTOCOL_ERROR
}

/// Return the number of bytes needed to serialise `addr`.
pub(crate) unsafe extern "C" fn na_abc_addr_get_serialize_size(
    _na_class: *mut na_class_t,
    _addr: *mut na_addr_t,
) -> usize {
    // TODO: Return the number of bytes needed to serialize addr.
    0
}

/// Serialise `addr` into `buf` (`buf_size` bytes available).
pub(crate) unsafe extern "C" fn na_abc_addr_serialize(
    _na_class: *mut na_class_t,
    _buf: *mut c_void,
    _buf_size: usize,
    _addr: *mut na_addr_t,
) -> na_return_t {
    // TODO: Serialize addr into buf (buf_size bytes available).
    NA_PROTOCOL_ERROR
}

/// Deserialise an address from `buf` into `*addr_p`.
pub(crate) unsafe extern "C" fn na_abc_addr_deserialize(
    _na_class: *mut na_class_t,
    _addr_p: *mut *mut na_addr_t,
    _buf: *const c_void,
    _buf_size: usize,
    _flags: u64,
) -> na_return_t {
    // TODO: Reconstruct an address from buf and return via *addr_p.
    NA_PROTOCOL_ERROR
}

// ------------------------------------------------------------------ //
//  Message sizes & tags                                                //
// ------------------------------------------------------------------ //

/// Return the maximum payload size for unexpected (unmatched) messages.
pub(crate) unsafe extern "C" fn na_abc_msg_get_max_unexpected_size(
    _na_class: *const na_class_t,
) -> usize {
    // TODO: Return the maximum payload size for an unexpected message.
    // This should reflect transport limits.
    0
}

/// Return the maximum payload size for expected (matched) messages.
pub(crate) unsafe extern "C" fn na_abc_msg_get_max_expected_size(
    _na_class: *const na_class_t,
) -> usize {
    // TODO: Return the maximum payload size for an expected message.
    0
}

/// Return the maximum tag value this plugin supports.
pub(crate) unsafe extern "C" fn na_abc_msg_get_max_tag(
    _na_class: *const na_class_t,
) -> na_tag_t {
    // TODO: Return the largest tag value that can be used for matching.
    // Tags are unsigned integers; typical transports support at least 2^30.
    0
}

// ------------------------------------------------------------------ //
//  Unexpected messages                                                 //
// ------------------------------------------------------------------ //

/// Post a non-blocking unexpected (unmatched) send.
///
/// When the operation completes, fill in an `na_cb_completion_data` and
/// call `na_cb_completion_add(context, &completion_data)` so that
/// `NA_Trigger()` can invoke `callback(arg)`.
///
/// **REQUIRED**.
pub(crate) unsafe extern "C" fn na_abc_msg_send_unexpected(
    _na_class: *mut na_class_t,
    _context: *mut na_context_t,
    _callback: na_cb_t,
    _arg: *mut c_void,
    _buf: *const c_void,
    _buf_size: usize,
    _plugin_data: *mut c_void,
    _dest_addr: *mut na_addr_t,
    _dest_id: u8,
    _tag: na_tag_t,
    _op_id: *mut na_op_id_t,
) -> na_return_t {
    // TODO: Initiate a non-blocking unexpected send to dest_addr.
    // When complete, fill in an na_cb_completion_data and call
    // na_cb_completion_add(context, &completion_data).
    NA_PROTOCOL_ERROR
}

/// Post a non-blocking unexpected receive.
///
/// When a matching message arrives, complete the operation via
/// `na_cb_completion_add()`.
///
/// **REQUIRED**.
pub(crate) unsafe extern "C" fn na_abc_msg_recv_unexpected(
    _na_class: *mut na_class_t,
    _context: *mut na_context_t,
    _callback: na_cb_t,
    _arg: *mut c_void,
    _buf: *mut c_void,
    _buf_size: usize,
    _plugin_data: *mut c_void,
    _op_id: *mut na_op_id_t,
) -> na_return_t {
    // TODO: Post a non-blocking unexpected receive.  When a message
    // arrives, complete the operation via na_cb_completion_add().
    NA_PROTOCOL_ERROR
}

// ------------------------------------------------------------------ //
//  Expected messages                                                   //
// ------------------------------------------------------------------ //

/// Post a non-blocking expected (matched) send.
///
/// **REQUIRED**.
pub(crate) unsafe extern "C" fn na_abc_msg_send_expected(
    _na_class: *mut na_class_t,
    _context: *mut na_context_t,
    _callback: na_cb_t,
    _arg: *mut c_void,
    _buf: *const c_void,
    _buf_size: usize,
    _plugin_data: *mut c_void,
    _dest_addr: *mut na_addr_t,
    _dest_id: u8,
    _tag: na_tag_t,
    _op_id: *mut na_op_id_t,
) -> na_return_t {
    // TODO: Initiate a non-blocking expected send.
    NA_PROTOCOL_ERROR
}

/// Post a non-blocking expected receive.
///
/// **REQUIRED**.
pub(crate) unsafe extern "C" fn na_abc_msg_recv_expected(
    _na_class: *mut na_class_t,
    _context: *mut na_context_t,
    _callback: na_cb_t,
    _arg: *mut c_void,
    _buf: *mut c_void,
    _buf_size: usize,
    _plugin_data: *mut c_void,
    _source_addr: *mut na_addr_t,
    _source_id: u8,
    _tag: na_tag_t,
    _op_id: *mut na_op_id_t,
) -> na_return_t {
    // TODO: Post a non-blocking expected receive.
    NA_PROTOCOL_ERROR
}

// ------------------------------------------------------------------ //
//  Memory handles                                                      //
// ------------------------------------------------------------------ //

/// Create a memory handle that describes a contiguous region for RMA.
///
/// `flags` is a bitmask of `NA_MEM_READ_ONLY` / `NA_MEM_WRITE_ONLY` /
/// `NA_MEM_READWRITE`.
///
/// **REQUIRED**.
pub(crate) unsafe extern "C" fn na_abc_mem_handle_create(
    _na_class: *mut na_class_t,
    _buf: *mut c_void,
    _buf_size: usize,
    _flags: std::ffi::c_ulong,
    _mem_handle_p: *mut *mut na_mem_handle_t,
) -> na_return_t {
    // TODO: Create a memory handle describing a contiguous buffer for RMA.
    // flags indicates access mode (NA_MEM_READ_ONLY, etc.).
    NA_PROTOCOL_ERROR
}

/// Free a memory handle.
pub(crate) unsafe extern "C" fn na_abc_mem_handle_free(
    _na_class: *mut na_class_t,
    _mem_handle: *mut na_mem_handle_t,
) {
    // TODO: Free a memory handle created by mem_handle_create.
}

/// Return the serialised size of a memory handle.
pub(crate) unsafe extern "C" fn na_abc_mem_handle_get_serialize_size(
    _na_class: *mut na_class_t,
    _mem_handle: *mut na_mem_handle_t,
) -> usize {
    // TODO: Return the number of bytes needed to serialize the handle.
    0
}

/// Serialise a memory handle into `buf`.
pub(crate) unsafe extern "C" fn na_abc_mem_handle_serialize(
    _na_class: *mut na_class_t,
    _buf: *mut c_void,
    _buf_size: usize,
    _mem_handle: *mut na_mem_handle_t,
) -> na_return_t {
    // TODO: Serialize the memory handle into buf so it can be sent to
    // a remote peer for RMA.
    NA_PROTOCOL_ERROR
}

/// Deserialise a memory handle from `buf`.
pub(crate) unsafe extern "C" fn na_abc_mem_handle_deserialize(
    _na_class: *mut na_class_t,
    _mem_handle_p: *mut *mut na_mem_handle_t,
    _buf: *const c_void,
    _buf_size: usize,
) -> na_return_t {
    // TODO: Reconstruct a remote memory handle from buf.
    NA_PROTOCOL_ERROR
}

// ------------------------------------------------------------------ //
//  RMA (put / get)                                                     //
// ------------------------------------------------------------------ //

/// Initiate a non-blocking RMA put (write to remote memory).
///
/// **REQUIRED** for RMA support.
pub(crate) unsafe extern "C" fn na_abc_put(
    _na_class: *mut na_class_t,
    _context: *mut na_context_t,
    _callback: na_cb_t,
    _arg: *mut c_void,
    _local_mem_handle: *mut na_mem_handle_t,
    _local_offset: na_offset_t,
    _remote_mem_handle: *mut na_mem_handle_t,
    _remote_offset: na_offset_t,
    _length: usize,
    _remote_addr: *mut na_addr_t,
    _remote_id: u8,
    _op_id: *mut na_op_id_t,
) -> na_return_t {
    // TODO: Write length bytes from (local_mem_handle + local_offset)
    // to (remote_mem_handle + remote_offset) on remote_addr.
    // Complete via na_cb_completion_add() when done.
    NA_PROTOCOL_ERROR
}

/// Initiate a non-blocking RMA get (read from remote memory).
///
/// **REQUIRED** for RMA support.
pub(crate) unsafe extern "C" fn na_abc_get(
    _na_class: *mut na_class_t,
    _context: *mut na_context_t,
    _callback: na_cb_t,
    _arg: *mut c_void,
    _local_mem_handle: *mut na_mem_handle_t,
    _local_offset: na_offset_t,
    _remote_mem_handle: *mut na_mem_handle_t,
    _remote_offset: na_offset_t,
    _length: usize,
    _remote_addr: *mut na_addr_t,
    _remote_id: u8,
    _op_id: *mut na_op_id_t,
) -> na_return_t {
    // TODO: Read length bytes from (remote_mem_handle + remote_offset)
    // on remote_addr into (local_mem_handle + local_offset).
    // Complete via na_cb_completion_add() when done.
    NA_PROTOCOL_ERROR
}

// ------------------------------------------------------------------ //
//  Progress / polling                                                  //
// ------------------------------------------------------------------ //

/// Return a file descriptor that becomes readable when progress can be
/// made.  Return `-1` if not supported (caller will fall back to
/// busy-polling).
///
/// Optional -- may be `None`.
pub(crate) unsafe extern "C" fn na_abc_poll_get_fd(
    _na_class: *mut na_class_t,
    _context: *mut na_context_t,
) -> std::ffi::c_int {
    // TODO: Return a file descriptor that can be polled (epoll/select) for
    // readability to know when progress can be made.  Return -1 if fd-based
    // notification is not supported.
    -1
}

/// Return `true` if the caller should enter an OS-level wait
/// (epoll/poll/select) on the fd returned by `poll_get_fd()`.
///
/// Return `false` if there is already work pending.
///
/// Optional -- may be `None`.
pub(crate) unsafe extern "C" fn na_abc_poll_try_wait(
    _na_class: *mut na_class_t,
    _context: *mut na_context_t,
) -> bool {
    // TODO: Return true if it is safe to block on poll_get_fd(); return
    // false if there is already pending work (so the caller should call
    // poll() immediately instead of waiting).
    false
}

/// Poll for completed operations.
///
/// Store the number of completions in `*count_p`.  Return `NA_SUCCESS`
/// if completions were found, `NA_TIMEOUT` if none.
///
/// **REQUIRED**.
pub(crate) unsafe extern "C" fn na_abc_poll(
    _na_class: *mut na_class_t,
    _context: *mut na_context_t,
    count_p: *mut std::ffi::c_uint,
) -> na_return_t {
    // TODO: Check for completed operations and process them.  For each
    // completion, call na_cb_completion_add().  Store the number of
    // completions processed in *count_p.
    if !count_p.is_null() {
        unsafe { *count_p = 0 };
    }
    NA_TIMEOUT
}

/// Cancel an in-flight operation.
///
/// On successful cancellation the operation's callback should still be
/// invoked with `NA_CANCELED` as the return value.
///
/// **REQUIRED**.
pub(crate) unsafe extern "C" fn na_abc_cancel(
    _na_class: *mut na_class_t,
    _context: *mut na_context_t,
    _op_id: *mut na_op_id_t,
) -> na_return_t {
    // TODO: Cancel the in-flight operation identified by op_id.  On
    // successful cancellation the operation's callback should still be
    // invoked with NA_CANCELED as the return value.
    NA_PROTOCOL_ERROR
}
