use std::collections::HashMap;
use std::ffi::{c_void, CStr};
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use libp2p::{Multiaddr, PeerId};
use parking_lot::Mutex;
use tracing::error;

use crate::bindings::*;
use crate::protocol::MessageType;
use crate::runtime::{self, Command};
use crate::state::*;
use crate::{NA_CANCELED, NA_INVALID_ARG, NA_NOMEM, NA_OVERFLOW, NA_PROTOCOL_ERROR, NA_SUCCESS};

// ---------------------------------------------------------------------------
//  Helpers
// ---------------------------------------------------------------------------

unsafe fn get_class(na_class: *mut na_class_t) -> &'static NaLibp2pClass {
    &*((*na_class).plugin_class as *const NaLibp2pClass)
}

// ---------------------------------------------------------------------------
//  Protocol discovery
// ---------------------------------------------------------------------------

pub(crate) unsafe extern "C" fn na_libp2p_get_protocol_info(
    na_info_p: *const na_info,
    na_protocol_info_p: *mut *mut na_protocol_info,
) -> na_return_t {
    // Read protocol_name from na_info if available, otherwise default to "tcp"
    let proto = if !na_info_p.is_null() && !unsafe { (*na_info_p).protocol_name }.is_null() {
        unsafe { CStr::from_ptr((*na_info_p).protocol_name) }
    } else {
        c"tcp"
    };
    let info = unsafe {
        na_protocol_info_alloc(
            c"libp2p".as_ptr(),
            proto.as_ptr(),
            c"tcp0".as_ptr(),
        )
    };
    if info.is_null() {
        return NA_NOMEM;
    }
    unsafe { *na_protocol_info_p = info };
    NA_SUCCESS
}

pub(crate) unsafe extern "C" fn na_libp2p_check_protocol(
    protocol_name: *const std::ffi::c_char,
) -> bool {
    let name = unsafe { CStr::from_ptr(protocol_name) }.to_string_lossy();
    // Accept "libp2p" for backward compat, plus "tcp", "quic", "tcp,quic", "quic,tcp"
    if name == "libp2p" {
        return true;
    }
    name.split(',').all(|part| matches!(part.trim(), "tcp" | "quic"))
}

// ---------------------------------------------------------------------------
//  Lifecycle
// ---------------------------------------------------------------------------

pub(crate) unsafe extern "C" fn na_libp2p_initialize(
    na_class: *mut na_class_t,
    na_info_p: *const na_info,
    listen: bool,
) -> na_return_t {
    // Init tracing (ignore errors if already initialized)
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    // Parse protocol_name to determine transport configuration
    let protocol_name = if !na_info_p.is_null() && !unsafe { (*na_info_p).protocol_name }.is_null()
    {
        unsafe { CStr::from_ptr((*na_info_p).protocol_name) }
            .to_string_lossy()
            .into_owned()
    } else {
        String::new()
    };

    let transport_config = parse_transport_config(&protocol_name);

    tracing::debug!(
        "initialize: protocol_name='{}', transport_config={:?}",
        protocol_name,
        transport_config
    );

    // Build tokio runtime
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            error!("Failed to build tokio runtime: {e}");
            return NA_PROTOCOL_ERROR;
        }
    };

    let event_fd = runtime::create_eventfd();

    // Determine listen addresses based on transport config
    let bind_ip = if listen { "0.0.0.0" } else { "127.0.0.1" };
    let mut listen_addrs: Vec<Multiaddr> = Vec::new();
    if transport_config.tcp {
        listen_addrs.push(format!("/ip4/{bind_ip}/tcp/0").parse().unwrap());
    }
    if transport_config.quic {
        listen_addrs.push(format!("/ip4/{bind_ip}/udp/0/quic-v1").parse().unwrap());
    }

    let queues = Arc::new(Mutex::new(OperationQueues::new()));
    let mem_handles: Arc<Mutex<HashMap<u64, MemHandleEntry>>> =
        Arc::new(Mutex::new(HashMap::new()));

    let (cmd_tx, completion_rx, peer_id, resolved_addrs, join_handle) =
        match runtime::spawn_swarm_task(
            &rt,
            listen_addrs,
            &transport_config,
            queues.clone(),
            mem_handles.clone(),
            event_fd,
        ) {
            Ok(v) => v,
            Err(e) => {
                error!("Failed to spawn swarm: {e}");
                runtime::close_eventfd(event_fd);
                return NA_PROTOCOL_ERROR;
            }
        };

    // Pick the priority transport's address as the self address.
    let self_ma = resolve_self_multiaddr(&resolved_addrs, transport_config.priority);

    // Resolve all listen addrs (replace 0.0.0.0 with concrete IP)
    let all_listen_addrs: Vec<Multiaddr> = resolved_addrs
        .iter()
        .map(|ma| resolve_multiaddr(ma))
        .collect();

    let self_addr = Arc::new(NaLibp2pAddr {
        peer_id,
        multiaddr: Some(self_ma),
        is_self: true,
    });

    let state = Box::new(NaLibp2pClass {
        self_addr,
        queues,
        cmd_tx,
        completion_rx: Mutex::new(completion_rx),
        event_fd,
        runtime: Some(rt),
        swarm_join: Some(join_handle),
        mem_handles,
        next_mem_handle_id: AtomicU64::new(1),
        transport_config,
        listen_addrs: all_listen_addrs,
    });

    unsafe { (*na_class).plugin_class = Box::into_raw(state) as *mut c_void };
    NA_SUCCESS
}

/// Parse protocol_name string into a TransportConfig.
fn parse_transport_config(protocol_name: &str) -> TransportConfig {
    use crate::state::{TransportConfig, TransportKind};

    if protocol_name.is_empty() || protocol_name == "libp2p" {
        // Default / backward compat: TCP only
        return TransportConfig {
            tcp: true,
            quic: false,
            priority: TransportKind::Tcp,
        };
    }

    let parts: Vec<&str> = protocol_name.split(',').map(|s| s.trim()).collect();
    let mut tcp = false;
    let mut quic = false;

    for part in &parts {
        match *part {
            "tcp" => tcp = true,
            "quic" => quic = true,
            _ => {}
        }
    }

    // If nothing valid was parsed, default to TCP
    if !tcp && !quic {
        return TransportConfig {
            tcp: true,
            quic: false,
            priority: TransportKind::Tcp,
        };
    }

    // Priority is the first listed transport
    let priority = match parts[0] {
        "quic" => TransportKind::Quic,
        _ => TransportKind::Tcp,
    };

    TransportConfig {
        tcp,
        quic,
        priority,
    }
}

/// Pick the self-address from the resolved listen addresses based on the
/// priority transport. If the IP is 0.0.0.0, resolves to a concrete address.
fn resolve_self_multiaddr(
    resolved_addrs: &[Multiaddr],
    priority: crate::state::TransportKind,
) -> Multiaddr {
    use crate::state::TransportKind;

    let addr = match priority {
        TransportKind::Tcp => resolved_addrs
            .iter()
            .find(|a| {
                a.iter()
                    .any(|p| matches!(p, libp2p::multiaddr::Protocol::Tcp(_)))
            })
            .unwrap_or(&resolved_addrs[0]),
        TransportKind::Quic => resolved_addrs
            .iter()
            .find(|a| {
                a.iter()
                    .any(|p| matches!(p, libp2p::multiaddr::Protocol::QuicV1))
            })
            .unwrap_or(&resolved_addrs[0]),
    };

    resolve_multiaddr(addr)
}

/// Resolve a multiaddr by replacing 0.0.0.0 with a concrete routable address.
fn resolve_multiaddr(addr: &Multiaddr) -> Multiaddr {
    let mut result = Multiaddr::empty();
    for proto in addr.iter() {
        match proto {
            libp2p::multiaddr::Protocol::Ip4(ip) if ip.is_unspecified() => {
                let resolved = hostname_ip().unwrap_or(std::net::IpAddr::V4(ip));
                match resolved {
                    std::net::IpAddr::V4(v4) => result.push(libp2p::multiaddr::Protocol::Ip4(v4)),
                    std::net::IpAddr::V6(v6) => result.push(libp2p::multiaddr::Protocol::Ip6(v6)),
                }
            }
            other => result.push(other),
        }
    }
    result
}

fn hostname_ip() -> Option<std::net::IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    socket.local_addr().ok().map(|a| a.ip())
}

pub(crate) unsafe extern "C" fn na_libp2p_finalize(
    na_class: *mut na_class_t,
) -> na_return_t {
    if (*na_class).plugin_class.is_null() {
        return NA_SUCCESS;
    }
    let mut state = unsafe { Box::from_raw((*na_class).plugin_class as *mut NaLibp2pClass) };

    // Send shutdown
    let _ = state.cmd_tx.send(Command::Shutdown);

    // Wait for swarm task
    if let Some(rt) = state.runtime.take() {
        if let Some(join) = state.swarm_join.take() {
            let _ = rt.block_on(join);
        }
        // Drop runtime
        drop(rt);
    }

    runtime::close_eventfd(state.event_fd);
    drop(state);

    unsafe { (*na_class).plugin_class = std::ptr::null_mut() };
    NA_SUCCESS
}

pub(crate) unsafe extern "C" fn na_libp2p_cleanup() {}

// ---------------------------------------------------------------------------
//  Contexts
// ---------------------------------------------------------------------------

pub(crate) unsafe extern "C" fn na_libp2p_context_create(
    _na_class: *mut na_class_t,
    _na_context: *mut na_context_t,
    plugin_context_p: *mut *mut c_void,
    _id: u8,
) -> na_return_t {
    unsafe { *plugin_context_p = std::ptr::null_mut() };
    NA_SUCCESS
}

pub(crate) unsafe extern "C" fn na_libp2p_context_destroy(
    _na_class: *mut na_class_t,
    _plugin_context: *mut c_void,
) -> na_return_t {
    NA_SUCCESS
}

// ---------------------------------------------------------------------------
//  Operation IDs
// ---------------------------------------------------------------------------

pub(crate) unsafe extern "C" fn na_libp2p_op_create(
    _na_class: *mut na_class_t,
    _flags: std::ffi::c_ulong,
) -> *mut na_op_id_t {
    let op = Box::new(NaLibp2pOpId {
        completion_data: unsafe { std::mem::zeroed() },
        context: std::ptr::null_mut(),
        addr: std::ptr::null_mut(),
        tag: 0,
        buf: std::ptr::null_mut(),
        buf_size: 0,
        local_handle_id: 0,
        local_offset: 0,
        rma_length: 0,
    });
    let op_ptr = Box::into_raw(op);

    // Set plugin release callback
    unsafe {
        (*op_ptr).completion_data.plugin_callback = Some(na_libp2p_release);
        (*op_ptr).completion_data.plugin_callback_args = op_ptr as *mut c_void;
    }

    op_ptr as *mut na_op_id_t
}

pub(crate) unsafe extern "C" fn na_libp2p_op_destroy(
    _na_class: *mut na_class_t,
    op_id: *mut na_op_id_t,
) {
    if !op_id.is_null() {
        let _ = unsafe { Box::from_raw(op_id as *mut NaLibp2pOpId) };
    }
}

// ---------------------------------------------------------------------------
//  Addressing
// ---------------------------------------------------------------------------

pub(crate) unsafe extern "C" fn na_libp2p_addr_lookup(
    na_class: *mut na_class_t,
    name: *const std::ffi::c_char,
    addr_p: *mut *mut na_addr_t,
) -> na_return_t {
    let cls = unsafe { get_class(na_class) };
    let name_str = unsafe { CStr::from_ptr(name) }.to_string_lossy();

    tracing::debug!("addr_lookup: raw input='{}'", name_str);

    // Strip protocol prefix. Mercury strips the "libp2p+" prefix before
    // calling us, so we receive "tcp:/ip4/..." or "quic:/ip4/...".
    let input = name_str
        .strip_prefix("tcp:")
        .or_else(|| name_str.strip_prefix("quic:"))
        .unwrap_or(&name_str);

    // Split off the /p2p/<peer_id> component
    let p2p_pos = match input.rfind("/p2p/") {
        Some(p) => p,
        None => {
            error!("addr_lookup: no /p2p/ component in '{input}'");
            return NA_PROTOCOL_ERROR;
        }
    };
    let transport_str = &input[..p2p_pos];
    let peer_id_str = &input[p2p_pos + 5..];

    let peer_id = match peer_id_str.parse::<PeerId>() {
        Ok(p) => p,
        Err(e) => {
            error!("addr_lookup: bad peer_id '{peer_id_str}': {e}");
            return NA_PROTOCOL_ERROR;
        }
    };

    // Parse transport multiaddr (everything before /p2p/)
    let transport_ma: Option<Multiaddr> = if transport_str.is_empty() {
        None
    } else {
        match transport_str.parse::<Multiaddr>() {
            Ok(ma) => Some(ma),
            Err(e) => {
                error!("addr_lookup: bad transport multiaddr '{transport_str}': {e}");
                return NA_PROTOCOL_ERROR;
            }
        }
    };

    tracing::debug!("addr_lookup: peer_id={peer_id}, transport={transport_ma:?}");

    // Tell the swarm about this peer's address
    if let Some(ref ma) = transport_ma {
        let _ = cls.cmd_tx.send(Command::AddKnownAddr {
            peer_id,
            multiaddr: ma.clone(),
        });
    }

    let addr = NaLibp2pAddr::alloc_boxed(peer_id, transport_ma, false);
    unsafe { *addr_p = addr as *mut na_addr_t };
    NA_SUCCESS
}

pub(crate) unsafe extern "C" fn na_libp2p_addr_free(
    _na_class: *mut na_class_t,
    addr: *mut na_addr_t,
) {
    if !addr.is_null() {
        let _ = unsafe { Box::from_raw(addr as *mut NaLibp2pAddr) };
    }
}

pub(crate) unsafe extern "C" fn na_libp2p_addr_self(
    na_class: *mut na_class_t,
    addr_p: *mut *mut na_addr_t,
) -> na_return_t {
    let cls = unsafe { get_class(na_class) };
    let dup = NaLibp2pAddr::dup(&cls.self_addr);
    unsafe { *addr_p = dup as *mut na_addr_t };
    NA_SUCCESS
}

pub(crate) unsafe extern "C" fn na_libp2p_addr_dup(
    _na_class: *mut na_class_t,
    addr: *mut na_addr_t,
    new_addr_p: *mut *mut na_addr_t,
) -> na_return_t {
    if addr.is_null() {
        return NA_INVALID_ARG;
    }
    let src = unsafe { &*(addr as *const NaLibp2pAddr) };
    let dup = NaLibp2pAddr::dup(src);
    unsafe { *new_addr_p = dup as *mut na_addr_t };
    NA_SUCCESS
}

pub(crate) unsafe extern "C" fn na_libp2p_addr_cmp(
    _na_class: *mut na_class_t,
    addr1: *mut na_addr_t,
    addr2: *mut na_addr_t,
) -> bool {
    if addr1 == addr2 {
        return true;
    }
    if addr1.is_null() || addr2.is_null() {
        return false;
    }
    let a1 = unsafe { &*(addr1 as *const NaLibp2pAddr) };
    let a2 = unsafe { &*(addr2 as *const NaLibp2pAddr) };
    a1.peer_id == a2.peer_id
}

pub(crate) unsafe extern "C" fn na_libp2p_addr_is_self(
    na_class: *mut na_class_t,
    addr: *mut na_addr_t,
) -> bool {
    if addr.is_null() {
        return false;
    }
    let cls = unsafe { get_class(na_class) };
    let a = unsafe { &*(addr as *const NaLibp2pAddr) };
    a.peer_id == cls.self_addr.peer_id
}

pub(crate) unsafe extern "C" fn na_libp2p_addr_to_string(
    na_class: *mut na_class_t,
    buf: *mut std::ffi::c_char,
    buf_size: *mut usize,
    addr: *mut na_addr_t,
) -> na_return_t {
    use crate::state::transport_prefix_for_multiaddr;

    if addr.is_null() || buf_size.is_null() {
        return NA_INVALID_ARG;
    }
    let a = unsafe { &*(addr as *const NaLibp2pAddr) };

    // Check for transport hint in the buffer (Mercury overwrites the "libp2p+"
    // prefix, so we receive "tcp..." or "quic..." at the start of buf).
    // This only applies to self-address queries.
    let s = if a.is_self && !buf.is_null() && !na_class.is_null() {
        let cls = unsafe { get_class(na_class) };
        let hint = read_transport_hint(buf);
        if let Some(transport_hint) = hint {
            // Look up the requested transport's address from listen_addrs
            if let Some(ma) = find_listen_addr_for_hint(&cls.listen_addrs, &transport_hint) {
                let prefix = transport_prefix_for_multiaddr(&ma);
                format!("{prefix}:{}/p2p/{}", ma, a.peer_id)
            } else {
                // Requested transport not available, fall back to default
                a.to_addr_string()
            }
        } else {
            a.to_addr_string()
        }
    } else {
        a.to_addr_string()
    };

    let needed = s.len() + 1;

    if buf.is_null() {
        unsafe { *buf_size = needed };
        return NA_SUCCESS;
    }

    if unsafe { *buf_size } < needed {
        unsafe { *buf_size = needed };
        return NA_OVERFLOW;
    }

    unsafe {
        std::ptr::copy_nonoverlapping(s.as_ptr(), buf as *mut u8, s.len());
        *(buf.add(s.len())) = 0;
        *buf_size = needed;
    }
    NA_SUCCESS
}

/// Read a transport hint from the buffer. Mercury overwrites the "libp2p+"
/// prefix (7 bytes), so what we see starts at the transport-specific part.
/// Returns Some("tcp") or Some("quic") if recognized, None otherwise.
fn read_transport_hint(buf: *const std::ffi::c_char) -> Option<String> {
    // Read up to 5 bytes to check for "tcp" or "quic" prefix
    let mut hint_bytes = [0u8; 5];
    for i in 0..5 {
        let b = unsafe { *(buf.add(i) as *const u8) };
        if b == 0 {
            break;
        }
        hint_bytes[i] = b;
    }
    let hint_str = std::str::from_utf8(&hint_bytes).unwrap_or("");
    if hint_str.starts_with("quic") {
        Some("quic".to_string())
    } else if hint_str.starts_with("tcp") {
        Some("tcp".to_string())
    } else {
        None
    }
}

/// Find a listen address matching the requested transport hint.
fn find_listen_addr_for_hint(listen_addrs: &[Multiaddr], hint: &str) -> Option<Multiaddr> {
    match hint {
        "tcp" => listen_addrs.iter().find(|a| {
            a.iter()
                .any(|p| matches!(p, libp2p::multiaddr::Protocol::Tcp(_)))
        }),
        "quic" => listen_addrs.iter().find(|a| {
            a.iter()
                .any(|p| matches!(p, libp2p::multiaddr::Protocol::QuicV1))
        }),
        _ => None,
    }
    .cloned()
}

pub(crate) unsafe extern "C" fn na_libp2p_addr_get_serialize_size(
    _na_class: *mut na_class_t,
    addr: *mut na_addr_t,
) -> usize {
    if addr.is_null() {
        return 0;
    }
    let a = unsafe { &*(addr as *const NaLibp2pAddr) };
    let peer_bytes = a.peer_id.to_bytes();
    let ma_str_len = a.multiaddr.as_ref().map(|m| m.to_string().len()).unwrap_or(0);
    // [2B peer_id_len][peer_id_bytes][2B ma_str_len][ma_str_bytes]
    2 + peer_bytes.len() + 2 + ma_str_len
}

pub(crate) unsafe extern "C" fn na_libp2p_addr_serialize(
    _na_class: *mut na_class_t,
    buf: *mut c_void,
    buf_size: usize,
    addr: *mut na_addr_t,
) -> na_return_t {
    if addr.is_null() || buf.is_null() {
        return NA_INVALID_ARG;
    }
    let a = unsafe { &*(addr as *const NaLibp2pAddr) };
    let peer_bytes = a.peer_id.to_bytes();
    let ma_str = a.multiaddr.as_ref().map(|m| m.to_string()).unwrap_or_default();
    let ma_bytes = ma_str.as_bytes();

    let out = buf as *mut u8;
    let mut pos = 0usize;

    macro_rules! write_bytes {
        ($data:expr) => {
            let d: &[u8] = $data;
            if pos + d.len() > buf_size {
                return NA_OVERFLOW;
            }
            unsafe { std::ptr::copy_nonoverlapping(d.as_ptr(), out.add(pos), d.len()) };
            pos += d.len();
        };
    }

    write_bytes!(&(peer_bytes.len() as u16).to_be_bytes());
    write_bytes!(&peer_bytes);
    write_bytes!(&(ma_bytes.len() as u16).to_be_bytes());
    write_bytes!(ma_bytes);

    NA_SUCCESS
}

pub(crate) unsafe extern "C" fn na_libp2p_addr_deserialize(
    _na_class: *mut na_class_t,
    addr_p: *mut *mut na_addr_t,
    buf: *const c_void,
    buf_size: usize,
    _flags: u64,
) -> na_return_t {
    if buf.is_null() || addr_p.is_null() || buf_size < 4 {
        return NA_INVALID_ARG;
    }

    let data = unsafe { std::slice::from_raw_parts(buf as *const u8, buf_size) };
    let mut pos = 0;

    // peer_id
    if pos + 2 > data.len() {
        return NA_PROTOCOL_ERROR;
    }
    let peer_id_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
    pos += 2;
    if pos + peer_id_len > data.len() {
        return NA_PROTOCOL_ERROR;
    }
    let peer_id = match PeerId::from_bytes(&data[pos..pos + peer_id_len]) {
        Ok(p) => p,
        Err(_) => return NA_PROTOCOL_ERROR,
    };
    pos += peer_id_len;

    // multiaddr (string-encoded)
    if pos + 2 > data.len() {
        return NA_PROTOCOL_ERROR;
    }
    let ma_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
    pos += 2;
    let multiaddr = if ma_len > 0 {
        if pos + ma_len > data.len() {
            return NA_PROTOCOL_ERROR;
        }
        let ma_str = match std::str::from_utf8(&data[pos..pos + ma_len]) {
            Ok(s) => s,
            Err(_) => return NA_PROTOCOL_ERROR,
        };
        match ma_str.parse::<Multiaddr>() {
            Ok(ma) => Some(ma),
            Err(_) => return NA_PROTOCOL_ERROR,
        }
    } else {
        None
    };

    let addr = NaLibp2pAddr::alloc_boxed(peer_id, multiaddr, false);
    unsafe { *addr_p = addr as *mut na_addr_t };
    NA_SUCCESS
}

// ---------------------------------------------------------------------------
//  Message sizes & tags
// ---------------------------------------------------------------------------

pub(crate) unsafe extern "C" fn na_libp2p_msg_get_max_unexpected_size(
    _na_class: *const na_class_t,
) -> usize {
    MAX_MSG_SIZE
}

pub(crate) unsafe extern "C" fn na_libp2p_msg_get_max_expected_size(
    _na_class: *const na_class_t,
) -> usize {
    MAX_MSG_SIZE
}

pub(crate) unsafe extern "C" fn na_libp2p_msg_get_max_tag(
    _na_class: *const na_class_t,
) -> na_tag_t {
    u32::MAX
}

// ---------------------------------------------------------------------------
//  Unexpected messages
// ---------------------------------------------------------------------------

pub(crate) unsafe extern "C" fn na_libp2p_msg_send_unexpected(
    na_class: *mut na_class_t,
    context: *mut na_context_t,
    callback: na_cb_t,
    arg: *mut c_void,
    buf: *const c_void,
    buf_size: usize,
    _plugin_data: *mut c_void,
    dest_addr: *mut na_addr_t,
    _dest_id: u8,
    tag: na_tag_t,
    op_id: *mut na_op_id_t,
) -> na_return_t {
    let cls = unsafe { get_class(na_class) };
    let dest = unsafe { &*(dest_addr as *const NaLibp2pAddr) };
    let op = unsafe { &mut *(op_id as *mut NaLibp2pOpId) };

    op.completion_data.callback = callback;
    op.completion_data.callback_info.arg = arg;
    op.completion_data.callback_info.type_ = na_cb_type_NA_CB_SEND_UNEXPECTED;
    op.context = context;
    op.tag = tag;

    let payload = if buf_size > 0 && !buf.is_null() {
        unsafe { std::slice::from_raw_parts(buf as *const u8, buf_size).to_vec() }
    } else {
        Vec::new()
    };

    let _ = cls.cmd_tx.send(Command::SendMessage {
        dest_peer_id: dest.peer_id,
        msg_type: MessageType::Unexpected,
        tag,
        payload,
        op_ptr: op_id as usize,
        source_peer_id: cls.self_addr.peer_id,
    });

    NA_SUCCESS
}

pub(crate) unsafe extern "C" fn na_libp2p_msg_recv_unexpected(
    na_class: *mut na_class_t,
    context: *mut na_context_t,
    callback: na_cb_t,
    arg: *mut c_void,
    buf: *mut c_void,
    buf_size: usize,
    _plugin_data: *mut c_void,
    op_id: *mut na_op_id_t,
) -> na_return_t {
    let cls = unsafe { get_class(na_class) };
    let op = unsafe { &mut *(op_id as *mut NaLibp2pOpId) };

    op.completion_data.callback = callback;
    op.completion_data.callback_info.arg = arg;
    op.completion_data.callback_info.type_ = na_cb_type_NA_CB_RECV_UNEXPECTED;
    op.context = context;
    op.buf = buf;
    op.buf_size = buf_size;
    op.addr = std::ptr::null_mut();

    let mut queues = cls.queues.lock();

    // Check stash first
    if let Some(stashed) = queues.unexpected_msg_stash.pop_front() {
        let copy_size = stashed.payload.len().min(buf_size);
        if copy_size > 0 && !buf.is_null() {
            unsafe {
                std::ptr::copy_nonoverlapping(stashed.payload.as_ptr(), buf as *mut u8, copy_size);
            }
        }

        op.completion_data.callback_info.info.recv_unexpected =
            na_cb_info_recv_unexpected {
                actual_buf_size: stashed.payload.len(),
                source: stashed.source_addr as *mut na_addr_t,
                tag: stashed.tag,
            };
        op.completion_data.callback_info.ret = NA_SUCCESS;

        unsafe { na_cb_completion_add(context, &mut op.completion_data) };
        return NA_SUCCESS;
    }

    // No stashed message — enqueue the op
    queues
        .unexpected_recv_ops
        .push_back(op_id as *mut NaLibp2pOpId);
    NA_SUCCESS
}

// ---------------------------------------------------------------------------
//  Expected messages
// ---------------------------------------------------------------------------

pub(crate) unsafe extern "C" fn na_libp2p_msg_send_expected(
    na_class: *mut na_class_t,
    context: *mut na_context_t,
    callback: na_cb_t,
    arg: *mut c_void,
    buf: *const c_void,
    buf_size: usize,
    _plugin_data: *mut c_void,
    dest_addr: *mut na_addr_t,
    _dest_id: u8,
    tag: na_tag_t,
    op_id: *mut na_op_id_t,
) -> na_return_t {
    let cls = unsafe { get_class(na_class) };
    let dest = unsafe { &*(dest_addr as *const NaLibp2pAddr) };
    let op = unsafe { &mut *(op_id as *mut NaLibp2pOpId) };

    op.completion_data.callback = callback;
    op.completion_data.callback_info.arg = arg;
    op.completion_data.callback_info.type_ = na_cb_type_NA_CB_SEND_EXPECTED;
    op.context = context;
    op.tag = tag;

    let payload = if buf_size > 0 && !buf.is_null() {
        unsafe { std::slice::from_raw_parts(buf as *const u8, buf_size).to_vec() }
    } else {
        Vec::new()
    };

    let _ = cls.cmd_tx.send(Command::SendMessage {
        dest_peer_id: dest.peer_id,
        msg_type: MessageType::Expected,
        tag,
        payload,
        op_ptr: op_id as usize,
        source_peer_id: cls.self_addr.peer_id,
    });

    NA_SUCCESS
}

pub(crate) unsafe extern "C" fn na_libp2p_msg_recv_expected(
    na_class: *mut na_class_t,
    context: *mut na_context_t,
    callback: na_cb_t,
    arg: *mut c_void,
    buf: *mut c_void,
    buf_size: usize,
    _plugin_data: *mut c_void,
    source_addr: *mut na_addr_t,
    _source_id: u8,
    tag: na_tag_t,
    op_id: *mut na_op_id_t,
) -> na_return_t {
    let cls = unsafe { get_class(na_class) };
    let op = unsafe { &mut *(op_id as *mut NaLibp2pOpId) };

    op.completion_data.callback = callback;
    op.completion_data.callback_info.arg = arg;
    op.completion_data.callback_info.type_ = na_cb_type_NA_CB_RECV_EXPECTED;
    op.context = context;
    op.buf = buf;
    op.buf_size = buf_size;
    op.tag = tag;
    op.addr = if source_addr.is_null() {
        std::ptr::null_mut()
    } else {
        source_addr as *mut NaLibp2pAddr
    };

    let mut queues = cls.queues.lock();

    let source_peer_id = if !source_addr.is_null() {
        Some(unsafe { (*(source_addr as *const NaLibp2pAddr)).peer_id })
    } else {
        None
    };

    let matched_idx = queues.expected_msg_stash.iter().position(|msg| {
        let tag_match = msg.tag == tag;
        let addr_match = match source_peer_id {
            Some(pid) => {
                if msg.source_addr.is_null() {
                    true
                } else {
                    unsafe { (*msg.source_addr).peer_id == pid }
                }
            }
            None => true,
        };
        tag_match && addr_match
    });

    if let Some(idx) = matched_idx {
        let stashed = queues.expected_msg_stash.remove(idx).unwrap();
        let copy_size = stashed.payload.len().min(buf_size);
        if copy_size > 0 && !buf.is_null() {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    stashed.payload.as_ptr(),
                    buf as *mut u8,
                    copy_size,
                );
            }
        }

        // Free the stashed source addr
        if !stashed.source_addr.is_null() {
            let _ = unsafe { Box::from_raw(stashed.source_addr) };
        }

        op.completion_data.callback_info.info.recv_expected =
            na_cb_info_recv_expected {
                actual_buf_size: stashed.payload.len(),
            };
        op.completion_data.callback_info.ret = NA_SUCCESS;

        unsafe { na_cb_completion_add(context, &mut op.completion_data) };
        return NA_SUCCESS;
    }

    queues
        .expected_recv_ops
        .push_back(op_id as *mut NaLibp2pOpId);
    NA_SUCCESS
}

// ---------------------------------------------------------------------------
//  Memory handles
// ---------------------------------------------------------------------------

pub(crate) unsafe extern "C" fn na_libp2p_mem_handle_create(
    na_class: *mut na_class_t,
    buf: *mut c_void,
    buf_size: usize,
    flags: std::ffi::c_ulong,
    mem_handle_p: *mut *mut na_mem_handle_t,
) -> na_return_t {
    let cls = unsafe { get_class(na_class) };
    let handle_id = cls.next_handle_id();

    let handle = Box::new(NaLibp2pMemHandle {
        buf,
        buf_size,
        handle_id,
        flags: flags as u64,
        owner_peer_id: None,
    });

    // Register in shared registry
    cls.mem_handles.lock().insert(
        handle_id,
        MemHandleEntry {
            buf,
            buf_size,
            flags: flags as u64,
        },
    );

    unsafe { *mem_handle_p = Box::into_raw(handle) as *mut na_mem_handle_t };
    NA_SUCCESS
}

pub(crate) unsafe extern "C" fn na_libp2p_mem_handle_free(
    na_class: *mut na_class_t,
    mem_handle: *mut na_mem_handle_t,
) {
    if mem_handle.is_null() {
        return;
    }
    let handle = unsafe { Box::from_raw(mem_handle as *mut NaLibp2pMemHandle) };
    if handle.owner_peer_id.is_none() {
        let cls = unsafe { get_class(na_class) };
        cls.mem_handles.lock().remove(&handle.handle_id);
    }
}

pub(crate) unsafe extern "C" fn na_libp2p_mem_handle_get_serialize_size(
    na_class: *mut na_class_t,
    mem_handle: *mut na_mem_handle_t,
) -> usize {
    if mem_handle.is_null() {
        return 0;
    }
    let handle = unsafe { &*(mem_handle as *const NaLibp2pMemHandle) };
    let cls = unsafe { get_class(na_class) };
    let peer_id = handle
        .owner_peer_id
        .unwrap_or(cls.self_addr.peer_id);
    let peer_bytes_len = peer_id.to_bytes().len();
    // [8B handle_id][8B buf_size][8B flags][2B peer_id_len][peer_id_bytes]
    8 + 8 + 8 + 2 + peer_bytes_len
}

pub(crate) unsafe extern "C" fn na_libp2p_mem_handle_serialize(
    na_class: *mut na_class_t,
    buf: *mut c_void,
    buf_size: usize,
    mem_handle: *mut na_mem_handle_t,
) -> na_return_t {
    if buf.is_null() || mem_handle.is_null() {
        return NA_INVALID_ARG;
    }
    let handle = unsafe { &*(mem_handle as *const NaLibp2pMemHandle) };
    let cls = unsafe { get_class(na_class) };

    let out = buf as *mut u8;
    let mut pos = 0usize;

    macro_rules! write_bytes {
        ($data:expr) => {
            let d: &[u8] = $data;
            if pos + d.len() > buf_size {
                return NA_OVERFLOW;
            }
            unsafe { std::ptr::copy_nonoverlapping(d.as_ptr(), out.add(pos), d.len()) };
            pos += d.len();
        };
    }

    write_bytes!(&handle.handle_id.to_be_bytes());
    write_bytes!(&(handle.buf_size as u64).to_be_bytes());
    write_bytes!(&handle.flags.to_be_bytes());

    let peer_id = handle
        .owner_peer_id
        .unwrap_or(cls.self_addr.peer_id);
    let peer_bytes = peer_id.to_bytes();
    write_bytes!(&(peer_bytes.len() as u16).to_be_bytes());
    write_bytes!(&peer_bytes);

    NA_SUCCESS
}

pub(crate) unsafe extern "C" fn na_libp2p_mem_handle_deserialize(
    _na_class: *mut na_class_t,
    mem_handle_p: *mut *mut na_mem_handle_t,
    buf: *const c_void,
    buf_size: usize,
) -> na_return_t {
    if buf.is_null() || mem_handle_p.is_null() || buf_size < 26 {
        return NA_INVALID_ARG;
    }
    let data = unsafe { std::slice::from_raw_parts(buf as *const u8, buf_size) };
    let mut pos = 0;

    let handle_id = u64::from_be_bytes(data[pos..pos + 8].try_into().unwrap());
    pos += 8;
    let remote_buf_size = u64::from_be_bytes(data[pos..pos + 8].try_into().unwrap()) as usize;
    pos += 8;
    let flags = u64::from_be_bytes(data[pos..pos + 8].try_into().unwrap());
    pos += 8;

    if pos + 2 > data.len() {
        return NA_PROTOCOL_ERROR;
    }
    let peer_id_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
    pos += 2;
    if pos + peer_id_len > data.len() {
        return NA_PROTOCOL_ERROR;
    }
    let peer_id = match PeerId::from_bytes(&data[pos..pos + peer_id_len]) {
        Ok(p) => p,
        Err(_) => return NA_PROTOCOL_ERROR,
    };

    let handle = Box::new(NaLibp2pMemHandle {
        buf: std::ptr::null_mut(),
        buf_size: remote_buf_size,
        handle_id,
        flags,
        owner_peer_id: Some(peer_id),
    });

    unsafe { *mem_handle_p = Box::into_raw(handle) as *mut na_mem_handle_t };
    NA_SUCCESS
}

// ---------------------------------------------------------------------------
//  RMA
// ---------------------------------------------------------------------------

pub(crate) unsafe extern "C" fn na_libp2p_put(
    na_class: *mut na_class_t,
    context: *mut na_context_t,
    callback: na_cb_t,
    arg: *mut c_void,
    local_mem_handle: *mut na_mem_handle_t,
    local_offset: na_offset_t,
    remote_mem_handle: *mut na_mem_handle_t,
    remote_offset: na_offset_t,
    length: usize,
    remote_addr: *mut na_addr_t,
    _remote_id: u8,
    op_id: *mut na_op_id_t,
) -> na_return_t {
    let cls = unsafe { get_class(na_class) };
    let op = unsafe { &mut *(op_id as *mut NaLibp2pOpId) };
    let local_handle = unsafe { &*(local_mem_handle as *const NaLibp2pMemHandle) };
    let remote_handle = unsafe { &*(remote_mem_handle as *const NaLibp2pMemHandle) };
    let dest = unsafe { &*(remote_addr as *const NaLibp2pAddr) };

    op.completion_data.callback = callback;
    op.completion_data.callback_info.arg = arg;
    op.completion_data.callback_info.type_ = na_cb_type_NA_CB_PUT;
    op.context = context;

    // Read data from local buffer
    let data = if length > 0 && !local_handle.buf.is_null() {
        let src = unsafe { (local_handle.buf as *const u8).add(local_offset as usize) };
        unsafe { std::slice::from_raw_parts(src, length).to_vec() }
    } else {
        Vec::new()
    };

    let remote_peer_id = remote_handle
        .owner_peer_id
        .unwrap_or(dest.peer_id);

    let _ = cls.cmd_tx.send(Command::RmaPut {
        remote_peer_id,
        data,
        remote_handle_id: remote_handle.handle_id,
        remote_offset: remote_offset as u64,
        op_ptr: op_id as usize,
        source_peer_id: cls.self_addr.peer_id,
    });

    NA_SUCCESS
}

pub(crate) unsafe extern "C" fn na_libp2p_get(
    na_class: *mut na_class_t,
    context: *mut na_context_t,
    callback: na_cb_t,
    arg: *mut c_void,
    local_mem_handle: *mut na_mem_handle_t,
    local_offset: na_offset_t,
    remote_mem_handle: *mut na_mem_handle_t,
    remote_offset: na_offset_t,
    length: usize,
    remote_addr: *mut na_addr_t,
    _remote_id: u8,
    op_id: *mut na_op_id_t,
) -> na_return_t {
    let cls = unsafe { get_class(na_class) };
    let op = unsafe { &mut *(op_id as *mut NaLibp2pOpId) };
    let local_handle = unsafe { &*(local_mem_handle as *const NaLibp2pMemHandle) };
    let remote_handle = unsafe { &*(remote_mem_handle as *const NaLibp2pMemHandle) };
    let dest = unsafe { &*(remote_addr as *const NaLibp2pAddr) };

    op.completion_data.callback = callback;
    op.completion_data.callback_info.arg = arg;
    op.completion_data.callback_info.type_ = na_cb_type_NA_CB_GET;
    op.context = context;
    op.local_handle_id = local_handle.handle_id;
    op.local_offset = local_offset as u64;
    op.rma_length = length;

    // Enqueue as pending RMA op
    cls.queues
        .lock()
        .pending_rma_ops
        .push_back(op_id as *mut NaLibp2pOpId);

    let remote_peer_id = remote_handle
        .owner_peer_id
        .unwrap_or(dest.peer_id);

    let _ = cls.cmd_tx.send(Command::RmaGetRequest {
        remote_peer_id,
        remote_handle_id: remote_handle.handle_id,
        remote_offset: remote_offset as u64,
        length,
        local_handle_id: local_handle.handle_id,
        local_offset: local_offset as u64,
        op_ptr: op_id as usize,
        source_peer_id: cls.self_addr.peer_id,
    });

    NA_SUCCESS
}

// ---------------------------------------------------------------------------
//  Progress / polling
// ---------------------------------------------------------------------------

pub(crate) unsafe extern "C" fn na_libp2p_poll_get_fd(
    na_class: *mut na_class_t,
    _context: *mut na_context_t,
) -> std::ffi::c_int {
    let cls = unsafe { get_class(na_class) };
    cls.event_fd as std::ffi::c_int
}

pub(crate) unsafe extern "C" fn na_libp2p_poll_try_wait(
    na_class: *mut na_class_t,
    _context: *mut na_context_t,
) -> bool {
    let cls = unsafe { get_class(na_class) };
    let rx = cls.completion_rx.lock();
    rx.is_empty()
}

pub(crate) unsafe extern "C" fn na_libp2p_poll(
    na_class: *mut na_class_t,
    _context: *mut na_context_t,
    count_p: *mut std::ffi::c_uint,
) -> na_return_t {
    let cls = unsafe { get_class(na_class) };
    let mut count: u32 = 0;

    runtime::drain_eventfd(cls.event_fd);

    // Drain completion channel
    {
        let mut rx = cls.completion_rx.lock();
        while let Ok(completion) = rx.try_recv() {
            let op = completion.op_ptr as *mut NaLibp2pOpId;
            let op_ref = unsafe { &mut *op };

            op_ref.completion_data.callback_info.ret = completion.result;

            if let Some(recv_info) = completion.recv_info {
                let cb_type = op_ref.completion_data.callback_info.type_;
                if cb_type == na_cb_type_NA_CB_RECV_UNEXPECTED {
                    // Copy payload into op's buffer was already done by the
                    // incoming handler; just set the recv info.
                    op_ref.completion_data.callback_info.info.recv_unexpected =
                        na_cb_info_recv_unexpected {
                            actual_buf_size: recv_info.actual_size,
                            source: recv_info.source_addr as *mut na_addr_t,
                            tag: recv_info.tag,
                        };
                } else if cb_type == na_cb_type_NA_CB_RECV_EXPECTED {
                    op_ref.completion_data.callback_info.info.recv_expected =
                        na_cb_info_recv_expected {
                            actual_buf_size: recv_info.actual_size,
                        };
                    if !recv_info.source_addr.is_null() {
                        let _ = Box::from_raw(recv_info.source_addr);
                    }
                } else {
                    if !recv_info.source_addr.is_null() {
                        let _ = Box::from_raw(recv_info.source_addr);
                    }
                }
            }

            unsafe { na_cb_completion_add(op_ref.context, &mut op_ref.completion_data) };
            count += 1;
        }
    }

    if !count_p.is_null() {
        unsafe { *count_p = count };
    }

    NA_SUCCESS
}

pub(crate) unsafe extern "C" fn na_libp2p_cancel(
    na_class: *mut na_class_t,
    context: *mut na_context_t,
    op_id: *mut na_op_id_t,
) -> na_return_t {
    let cls = unsafe { get_class(na_class) };
    let op_ptr = op_id as *mut NaLibp2pOpId;

    let mut queues = cls.queues.lock();

    // Try unexpected recv queue
    if let Some(pos) = queues
        .unexpected_recv_ops
        .iter()
        .position(|&p| p == op_ptr)
    {
        queues.unexpected_recv_ops.remove(pos);
        let op = unsafe { &mut *op_ptr };
        op.completion_data.callback_info.ret = NA_CANCELED;
        unsafe { na_cb_completion_add(context, &mut op.completion_data) };
        return NA_SUCCESS;
    }

    // Try expected recv queue
    if let Some(pos) = queues
        .expected_recv_ops
        .iter()
        .position(|&p| p == op_ptr)
    {
        queues.expected_recv_ops.remove(pos);
        let op = unsafe { &mut *op_ptr };
        op.completion_data.callback_info.ret = NA_CANCELED;
        unsafe { na_cb_completion_add(context, &mut op.completion_data) };
        return NA_SUCCESS;
    }

    // Try pending RMA ops
    if let Some(pos) = queues
        .pending_rma_ops
        .iter()
        .position(|&p| p == op_ptr)
    {
        queues.pending_rma_ops.remove(pos);
        let op = unsafe { &mut *op_ptr };
        op.completion_data.callback_info.ret = NA_CANCELED;
        unsafe { na_cb_completion_add(context, &mut op.completion_data) };
        return NA_SUCCESS;
    }

    NA_SUCCESS
}
