use std::collections::HashMap;
use std::os::unix::io::RawFd;
use std::sync::Arc;

use futures::prelude::*;
use libp2p::swarm::SwarmEvent;
use libp2p::{Multiaddr, PeerId, StreamProtocol};
use libp2p_stream as stream;
use parking_lot::Mutex;
use tokio::sync::mpsc;
use tracing::{debug, error, warn};

use crate::protocol::{self, MessageType, WireHeader};
use crate::state::{
    MemHandleEntry, NaLibp2pAddr, OperationQueues, StashedMessage,
};
use crate::bindings::*;
use crate::{NA_PROTOCOL_ERROR, NA_SUCCESS};

/// Protocol identifier for Mercury NA over libp2p.
pub const MERCURY_PROTOCOL: &str = "/mercury-na/1.0.0";

/// Shared map of known peer addresses, for use during dial.
type PeerAddrMap = Arc<Mutex<HashMap<PeerId, Multiaddr>>>;

/// A message to be sent through a per-peer sender channel.
struct OutboundMsg {
    header: WireHeader,
    payload: Vec<u8>,
    /// Where to deliver the send result. None = fire-and-forget (incoming handler).
    result_tx: Option<tokio::sync::oneshot::Sender<Result<(), String>>>,
}

// ---------------------------------------------------------------------------
//  Commands (NA thread → async)
// ---------------------------------------------------------------------------

pub enum Command {
    SendMessage {
        dest_peer_id: PeerId,
        msg_type: MessageType,
        tag: u32,
        payload: Vec<u8>,
        op_ptr: usize,
        source_peer_id: PeerId,
    },
    RmaPut {
        remote_peer_id: PeerId,
        data: Vec<u8>,
        remote_handle_id: u64,
        remote_offset: u64,
        op_ptr: usize,
        source_peer_id: PeerId,
    },
    RmaGetRequest {
        remote_peer_id: PeerId,
        remote_handle_id: u64,
        remote_offset: u64,
        length: usize,
        local_handle_id: u64,
        local_offset: u64,
        op_ptr: usize,
        source_peer_id: PeerId,
    },
    AddKnownAddr {
        peer_id: PeerId,
        multiaddr: Multiaddr,
    },
    Shutdown,
}

// ---------------------------------------------------------------------------
//  Completions (async → NA thread)
// ---------------------------------------------------------------------------

pub struct CompletionItem {
    pub op_ptr: usize,
    pub result: na_return_t,
    pub recv_info: Option<RecvInfo>,
}

pub struct RecvInfo {
    pub actual_size: usize,
    pub source_addr: *mut NaLibp2pAddr,
    pub tag: u32,
}

unsafe impl Send for RecvInfo {}
unsafe impl Send for CompletionItem {}

// ---------------------------------------------------------------------------
//  Eventfd helpers
// ---------------------------------------------------------------------------

pub fn create_eventfd() -> RawFd {
    let fd = unsafe {
        libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC)
    };
    assert!(fd >= 0, "eventfd creation failed");
    fd
}

pub fn signal_eventfd(fd: RawFd) {
    let val: u64 = 1;
    unsafe {
        libc::write(fd, val.to_ne_bytes().as_ptr() as *const _, 8);
    }
}

pub fn drain_eventfd(fd: RawFd) {
    let mut buf = [0u8; 8];
    unsafe {
        libc::read(fd, buf.as_mut_ptr() as *mut _, 8);
    }
}

pub fn close_eventfd(fd: RawFd) {
    unsafe {
        libc::close(fd);
    }
}

// ---------------------------------------------------------------------------
//  Per-peer sender pool
// ---------------------------------------------------------------------------

/// Manages a per-peer sender channel and writer task.
/// All outbound messages to a peer are serialized through a single channel,
/// and the writer task sends them one at a time on a single yamux stream.
struct PeerSenderPool {
    senders: HashMap<PeerId, mpsc::UnboundedSender<OutboundMsg>>,
}

impl PeerSenderPool {
    fn new() -> Self {
        Self {
            senders: HashMap::new(),
        }
    }

    /// Get or create a sender channel for the given peer.
    /// If the peer doesn't have a sender yet, spawns a writer task.
    fn get_or_create(
        &mut self,
        peer: PeerId,
        control: stream::Control,
    ) -> mpsc::UnboundedSender<OutboundMsg> {
        let entry = self.senders.entry(peer);
        match entry {
            std::collections::hash_map::Entry::Occupied(ref e) => {
                if !e.get().is_closed() {
                    return e.get().clone();
                }
                // Channel closed — writer task died, create a new one
            }
            std::collections::hash_map::Entry::Vacant(_) => {}
        }

        let (tx, rx) = mpsc::unbounded_channel::<OutboundMsg>();
        tokio::spawn(peer_writer_task(peer, control, rx));
        self.senders.insert(peer, tx.clone());
        tx
    }
}

/// A per-peer writer task. Opens a stream to the peer and sends messages
/// one at a time. Reopens the stream if it breaks.
async fn peer_writer_task(
    peer: PeerId,
    mut control: stream::Control,
    mut rx: mpsc::UnboundedReceiver<OutboundMsg>,
) {
    let mut stream_opt: Option<libp2p::Stream> = None;

    while let Some(msg) = rx.recv().await {
        let result = send_one_message(&mut control, peer, &mut stream_opt, &msg).await;
        if let Some(tx) = msg.result_tx {
            let _ = tx.send(result.map_err(|e| e.to_string()));
        }
    }
}

/// Send a single message on the per-peer stream. Reopens the stream if needed.
async fn send_one_message(
    control: &mut stream::Control,
    peer: PeerId,
    stream_opt: &mut Option<libp2p::Stream>,
    msg: &OutboundMsg,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Try to write on the existing stream, or open a new one.
    for attempt in 0..3u32 {
        // Ensure we have a stream
        if stream_opt.is_none() {
            match control
                .open_stream(peer, StreamProtocol::new(MERCURY_PROTOCOL))
                .await
            {
                Ok(s) => *stream_opt = Some(s),
                Err(e) => {
                    if attempt < 2 {
                        warn!("open_stream to {peer} failed (attempt {attempt}): {e}");
                        tokio::time::sleep(std::time::Duration::from_millis(20 << attempt)).await;
                        continue;
                    }
                    return Err(Box::new(e));
                }
            }
        }

        let stream = stream_opt.as_mut().unwrap();
        match protocol::write_message(stream, &msg.header, &msg.payload).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                warn!("write to {peer} failed (attempt {attempt}): {e}");
                // Stream is broken — drop it and try with a new one
                *stream_opt = None;
                if attempt >= 2 {
                    return Err(Box::new(e));
                }
                tokio::time::sleep(std::time::Duration::from_millis(20 << attempt)).await;
            }
        }
    }
    Err("send failed after retries".into())
}

// ---------------------------------------------------------------------------
//  Shared async state
// ---------------------------------------------------------------------------

struct SharedAsyncState {
    queues: Arc<Mutex<OperationQueues>>,
    mem_handles: Arc<Mutex<HashMap<u64, MemHandleEntry>>>,
    completion_tx: mpsc::UnboundedSender<CompletionItem>,
    event_fd: RawFd,
    self_peer_id: PeerId,
    peer_addrs: PeerAddrMap,
}

// ---------------------------------------------------------------------------
//  Swarm task
// ---------------------------------------------------------------------------

/// Build and spawn the swarm on the tokio runtime.
/// Returns (cmd_tx, completion_rx, self_peer_id, listen_ip, listen_port, join_handle).
pub fn spawn_swarm_task(
    rt: &tokio::runtime::Runtime,
    listen_addr: Multiaddr,
    queues: Arc<Mutex<OperationQueues>>,
    mem_handles: Arc<Mutex<HashMap<u64, MemHandleEntry>>>,
    event_fd: RawFd,
) -> Result<
    (
        mpsc::UnboundedSender<Command>,
        mpsc::UnboundedReceiver<CompletionItem>,
        PeerId,
        std::net::IpAddr,
        u16,
        tokio::task::JoinHandle<()>,
    ),
    Box<dyn std::error::Error + Send + Sync>,
> {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<Command>();
    let (completion_tx, completion_rx) = mpsc::unbounded_channel::<CompletionItem>();

    let (peer_id, listen_ip, listen_port, join_handle) = rt.block_on(async {
        let mut swarm = libp2p::SwarmBuilder::with_new_identity()
            .with_tokio()
            .with_tcp(
                libp2p::tcp::Config::default().nodelay(true),
                libp2p::noise::Config::new,
                libp2p::yamux::Config::default,
            )?
            .with_behaviour(|_| stream::Behaviour::new())?
            .with_swarm_config(|cfg| {
                cfg.with_idle_connection_timeout(std::time::Duration::from_secs(3600))
            })
            .build();

        let peer_id = *swarm.local_peer_id();
        let mut control = swarm.behaviour().new_control();

        // Listen
        swarm.listen_on(listen_addr.clone())?;

        // Wait for the actual listen address
        let (listen_ip, listen_port) = loop {
            match swarm.next().await {
                Some(SwarmEvent::NewListenAddr { address, .. }) => {
                    debug!("Listening on {address}");
                    let mut ip: Option<std::net::IpAddr> = None;
                    let mut port: Option<u16> = None;
                    for proto in address.iter() {
                        match proto {
                            libp2p::multiaddr::Protocol::Ip4(a) => ip = Some(a.into()),
                            libp2p::multiaddr::Protocol::Ip6(a) => ip = Some(a.into()),
                            libp2p::multiaddr::Protocol::Tcp(p) => port = Some(p),
                            _ => {}
                        }
                    }
                    if let (Some(i), Some(p)) = (ip, port) {
                        break (i, p);
                    }
                }
                Some(_) => {}
                None => {
                    return Err("Swarm stream ended during init".into());
                }
            }
        };

        // Accept incoming streams
        let mut incoming = control
            .accept(StreamProtocol::new(MERCURY_PROTOCOL))
            .expect("Failed to register protocol");

        let peer_addrs: PeerAddrMap = Arc::new(Mutex::new(HashMap::new()));

        let shared = Arc::new(SharedAsyncState {
            queues,
            mem_handles,
            completion_tx,
            event_fd,
            self_peer_id: peer_id,
            peer_addrs,
        });

        // Spawn incoming stream handler
        let shared_incoming = shared.clone();
        let incoming_control = swarm.behaviour().new_control();
        tokio::spawn(async move {
            // Per-peer sender pool for responses sent from incoming handlers
            let response_pool = Arc::new(tokio::sync::Mutex::new(PeerSenderPool::new()));
            while let Some((peer, stream)) = incoming.next().await {
                let shared = shared_incoming.clone();
                let ctrl = incoming_control.clone();
                let pool = response_pool.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_incoming_stream(peer, stream, shared, ctrl, pool).await {
                        warn!("Error handling incoming stream from {peer}: {e}");
                    }
                });
            }
        });

        // Main command loop
        let shared_cmd = shared.clone();
        let cmd_control = swarm.behaviour().new_control();
        let join_handle = tokio::spawn(async move {
            let shared = shared_cmd;
            let mut cmd_rx = cmd_rx;
            let mut sender_pool = PeerSenderPool::new();

            loop {
                tokio::select! {
                    event = swarm.next() => {
                        match event {
                            Some(SwarmEvent::ConnectionEstablished { peer_id, .. }) => {
                                debug!("Connection established with {peer_id}");
                            }
                            Some(SwarmEvent::ConnectionClosed { peer_id, cause, num_established, .. }) => {
                                warn!("Connection closed with {peer_id} (remaining={num_established}), cause: {cause:?}");
                                // Auto-reconnect
                                if num_established == 0 {
                                    if let Some(addr) = shared.peer_addrs.lock().get(&peer_id).cloned() {
                                        swarm.add_peer_address(peer_id, addr.clone());
                                        let dial_opts = libp2p::swarm::dial_opts::DialOpts::peer_id(peer_id)
                                            .addresses(vec![addr])
                                            .condition(libp2p::swarm::dial_opts::PeerCondition::DisconnectedAndNotDialing)
                                            .build();
                                        let _ = swarm.dial(dial_opts);
                                    }
                                    // Remove the old sender — writer task will be recreated on next send
                                    sender_pool.senders.remove(&peer_id);
                                }
                            }
                            Some(SwarmEvent::OutgoingConnectionError { peer_id, error, .. }) => {
                                warn!("Outgoing connection error to {peer_id:?}: {error}");
                            }
                            Some(SwarmEvent::IncomingConnectionError { error, .. }) => {
                                warn!("Incoming connection error: {error}");
                            }
                            Some(_) => {}
                            None => break,
                        }
                    }
                    cmd = cmd_rx.recv() => {
                        match cmd {
                            Some(Command::Shutdown) | None => break,
                            Some(Command::AddKnownAddr { peer_id, multiaddr }) => {
                                debug!("Adding known address for {peer_id}: {multiaddr}");
                                shared.peer_addrs.lock().insert(peer_id, multiaddr.clone());
                                swarm.add_peer_address(peer_id, multiaddr.clone());
                                let dial_opts = libp2p::swarm::dial_opts::DialOpts::peer_id(peer_id)
                                    .addresses(vec![multiaddr])
                                    .condition(libp2p::swarm::dial_opts::PeerCondition::DisconnectedAndNotDialing)
                                    .build();
                                if let Err(e) = swarm.dial(dial_opts) {
                                    warn!("Failed to dial {peer_id}: {e}");
                                }
                            }
                            Some(Command::SendMessage { dest_peer_id, msg_type, tag, payload, op_ptr, source_peer_id }) => {
                                if !swarm.is_connected(&dest_peer_id) {
                                    if let Some(addr) = shared.peer_addrs.lock().get(&dest_peer_id).cloned() {
                                        let dial_opts = libp2p::swarm::dial_opts::DialOpts::peer_id(dest_peer_id)
                                            .addresses(vec![addr])
                                            .condition(libp2p::swarm::dial_opts::PeerCondition::DisconnectedAndNotDialing)
                                            .build();
                                        let _ = swarm.dial(dial_opts);
                                    }
                                }
                                let shared = shared.clone();
                                let tx = sender_pool.get_or_create(dest_peer_id, cmd_control.clone());
                                let (result_tx, result_rx) = tokio::sync::oneshot::channel();
                                let msg = OutboundMsg {
                                    header: WireHeader {
                                        msg_type,
                                        tag,
                                        payload_size: payload.len() as u32,
                                        source_peer_id,
                                        handle_id: 0,
                                        offset: 0,
                                        rma_length: 0,
                                        local_handle_id: 0,
                                        local_offset: 0,
                                    },
                                    payload,
                                    result_tx: Some(result_tx),
                                };
                                let _ = tx.send(msg);
                                tokio::spawn(async move {
                                    let na_ret = match result_rx.await {
                                        Ok(Ok(())) => NA_SUCCESS,
                                        Ok(Err(e)) => {
                                            error!("Send to {dest_peer_id} failed: {e}");
                                            NA_PROTOCOL_ERROR
                                        }
                                        Err(_) => {
                                            error!("Send to {dest_peer_id}: sender dropped");
                                            NA_PROTOCOL_ERROR
                                        }
                                    };
                                    let _ = shared.completion_tx.send(CompletionItem {
                                        op_ptr,
                                        result: na_ret,
                                        recv_info: None,
                                    });
                                    signal_eventfd(shared.event_fd);
                                });
                            }
                            Some(Command::RmaPut { remote_peer_id, data, remote_handle_id, remote_offset, op_ptr, source_peer_id }) => {
                                if !swarm.is_connected(&remote_peer_id) {
                                    if let Some(addr) = shared.peer_addrs.lock().get(&remote_peer_id).cloned() {
                                        let dial_opts = libp2p::swarm::dial_opts::DialOpts::peer_id(remote_peer_id)
                                            .addresses(vec![addr])
                                            .condition(libp2p::swarm::dial_opts::PeerCondition::DisconnectedAndNotDialing)
                                            .build();
                                        let _ = swarm.dial(dial_opts);
                                    }
                                }
                                let shared = shared.clone();
                                let tx = sender_pool.get_or_create(remote_peer_id, cmd_control.clone());
                                let (result_tx, result_rx) = tokio::sync::oneshot::channel();
                                let msg = OutboundMsg {
                                    header: WireHeader {
                                        msg_type: MessageType::RmaPut,
                                        tag: 0,
                                        payload_size: data.len() as u32,
                                        source_peer_id,
                                        handle_id: remote_handle_id,
                                        offset: remote_offset,
                                        rma_length: data.len() as u64,
                                        local_handle_id: 0,
                                        local_offset: 0,
                                    },
                                    payload: data,
                                    result_tx: Some(result_tx),
                                };
                                let _ = tx.send(msg);
                                tokio::spawn(async move {
                                    let na_ret = match result_rx.await {
                                        Ok(Ok(())) => NA_SUCCESS,
                                        Ok(Err(e)) => {
                                            error!("RMA put to {remote_peer_id} failed: {e}");
                                            NA_PROTOCOL_ERROR
                                        }
                                        Err(_) => NA_PROTOCOL_ERROR,
                                    };
                                    let _ = shared.completion_tx.send(CompletionItem {
                                        op_ptr,
                                        result: na_ret,
                                        recv_info: None,
                                    });
                                    signal_eventfd(shared.event_fd);
                                });
                            }
                            Some(Command::RmaGetRequest { remote_peer_id, remote_handle_id, remote_offset, length, local_handle_id, local_offset, op_ptr, source_peer_id }) => {
                                if !swarm.is_connected(&remote_peer_id) {
                                    if let Some(addr) = shared.peer_addrs.lock().get(&remote_peer_id).cloned() {
                                        let dial_opts = libp2p::swarm::dial_opts::DialOpts::peer_id(remote_peer_id)
                                            .addresses(vec![addr])
                                            .condition(libp2p::swarm::dial_opts::PeerCondition::DisconnectedAndNotDialing)
                                            .build();
                                        let _ = swarm.dial(dial_opts);
                                    }
                                }
                                let shared = shared.clone();
                                let tx = sender_pool.get_or_create(remote_peer_id, cmd_control.clone());
                                let (result_tx, result_rx) = tokio::sync::oneshot::channel();
                                let msg = OutboundMsg {
                                    header: WireHeader {
                                        msg_type: MessageType::RmaGetReq,
                                        tag: 0,
                                        payload_size: 0,
                                        source_peer_id,
                                        handle_id: remote_handle_id,
                                        offset: remote_offset,
                                        rma_length: length as u64,
                                        local_handle_id,
                                        local_offset,
                                    },
                                    payload: vec![],
                                    result_tx: Some(result_tx),
                                };
                                let _ = tx.send(msg);
                                tokio::spawn(async move {
                                    let na_ret = match result_rx.await {
                                        Ok(Ok(())) => NA_SUCCESS,
                                        Ok(Err(e)) => {
                                            error!("RMA get request to {remote_peer_id} failed: {e}");
                                            NA_PROTOCOL_ERROR
                                        }
                                        Err(_) => NA_PROTOCOL_ERROR,
                                    };
                                    // For RMA get, we only complete the op on error here.
                                    // On success, the completion happens when we receive the RmaGetResp.
                                    if na_ret != NA_SUCCESS {
                                        let _ = shared.completion_tx.send(CompletionItem {
                                            op_ptr,
                                            result: na_ret,
                                            recv_info: None,
                                        });
                                        signal_eventfd(shared.event_fd);
                                    }
                                });
                            }
                        }
                    }
                }
            }
        });

        Ok::<_, Box<dyn std::error::Error + Send + Sync>>((
            peer_id,
            listen_ip,
            listen_port,
            join_handle,
        ))
    })?;

    Ok((cmd_tx, completion_rx, peer_id, listen_ip, listen_port, join_handle))
}

// ---------------------------------------------------------------------------
//  Incoming stream handler
// ---------------------------------------------------------------------------

async fn handle_incoming_stream(
    _peer: PeerId,
    mut stream: libp2p::Stream,
    shared: Arc<SharedAsyncState>,
    control: stream::Control,
    response_pool: Arc<tokio::sync::Mutex<PeerSenderPool>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Read messages in a loop — the sender may reuse this stream for multiple messages.
    loop {
        let result = protocol::read_message(&mut stream).await;
        let (header, payload) = match result {
            Ok(hp) => hp,
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(Box::new(e)),
        };

        match header.msg_type {
            MessageType::Unexpected => {
                handle_incoming_msg(&shared, header, payload, false);
            }
            MessageType::Expected => {
                handle_incoming_msg(&shared, header, payload, true);
            }
            MessageType::RmaPut => {
                handle_rma_put(&shared, header, payload);
            }
            MessageType::RmaGetReq => {
                handle_rma_get_request(&shared, &response_pool, control.clone(), header).await?;
            }
            MessageType::RmaGetResp => {
                handle_rma_get_response(&shared, header, payload);
            }
        }
    }

    Ok(())
}

fn handle_incoming_msg(
    shared: &SharedAsyncState,
    header: WireHeader,
    payload: Vec<u8>,
    expected: bool,
) {
    let source_addr = NaLibp2pAddr::alloc_boxed(header.source_peer_id, None, None, false);

    let mut queues = shared.queues.lock();

    let ops_queue = if expected {
        &mut queues.expected_recv_ops
    } else {
        &mut queues.unexpected_recv_ops
    };

    // Try to find a matching pending recv operation
    let matched_idx = if expected {
        ops_queue.iter().position(|&op_ptr| {
            let op = unsafe { &*op_ptr };
            let tag_match = op.tag == header.tag;
            let addr_match = if op.addr.is_null() {
                true
            } else {
                unsafe { (*op.addr).peer_id == header.source_peer_id }
            };
            tag_match && addr_match
        })
    } else {
        if ops_queue.is_empty() {
            None
        } else {
            Some(0)
        }
    };

    if let Some(idx) = matched_idx {
        let op_ptr = ops_queue.remove(idx).unwrap();
        let op = unsafe { &mut *op_ptr };

        // Copy payload into op's buffer
        let copy_size = payload.len().min(op.buf_size);
        if copy_size > 0 && !op.buf.is_null() {
            unsafe {
                std::ptr::copy_nonoverlapping(payload.as_ptr(), op.buf as *mut u8, copy_size);
            }
        }

        // Complete via completion channel
        let recv_info = RecvInfo {
            actual_size: payload.len(),
            source_addr,
            tag: header.tag,
        };

        let _ = shared.completion_tx.send(CompletionItem {
            op_ptr: op_ptr as usize,
            result: NA_SUCCESS,
            recv_info: Some(recv_info),
        });
        signal_eventfd(shared.event_fd);
    } else {
        // No matching recv posted — stash the message
        let stash = if expected {
            &mut queues.expected_msg_stash
        } else {
            &mut queues.unexpected_msg_stash
        };
        stash.push_back(StashedMessage {
            payload,
            source_addr,
            tag: header.tag,
        });
    }
}

fn handle_rma_put(shared: &SharedAsyncState, header: WireHeader, payload: Vec<u8>) {
    let handles = shared.mem_handles.lock();
    if let Some(entry) = handles.get(&header.handle_id) {
        let offset = header.offset as usize;
        let copy_len = payload.len().min(entry.buf_size.saturating_sub(offset));
        if copy_len > 0 {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    payload.as_ptr(),
                    (entry.buf as *mut u8).add(offset),
                    copy_len,
                );
            }
        }
    } else {
        warn!("RMA put: handle_id {} not found", header.handle_id);
    }
}

async fn handle_rma_get_request(
    shared: &SharedAsyncState,
    response_pool: &tokio::sync::Mutex<PeerSenderPool>,
    control: stream::Control,
    header: WireHeader,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let data = {
        let handles = shared.mem_handles.lock();
        if let Some(entry) = handles.get(&header.handle_id) {
            let offset = header.offset as usize;
            let length = (header.rma_length as usize).min(entry.buf_size.saturating_sub(offset));
            let mut buf = vec![0u8; length];
            if length > 0 {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        (entry.buf as *const u8).add(offset),
                        buf.as_mut_ptr(),
                        length,
                    );
                }
            }
            buf
        } else {
            warn!("RMA get request: handle_id {} not found", header.handle_id);
            return Ok(());
        }
    };

    // Send RmaGetResp back through the per-peer sender pool
    let resp_header = WireHeader {
        msg_type: MessageType::RmaGetResp,
        tag: 0,
        payload_size: data.len() as u32,
        source_peer_id: shared.self_peer_id,
        handle_id: header.handle_id,
        offset: header.offset,
        rma_length: data.len() as u64,
        local_handle_id: header.local_handle_id,
        local_offset: header.local_offset,
    };

    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
    {
        let mut pool = response_pool.lock().await;
        let tx = pool.get_or_create(header.source_peer_id, control);
        let _ = tx.send(OutboundMsg {
            header: resp_header,
            payload: data,
            result_tx: Some(result_tx),
        });
    }
    match result_rx.await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e.into()),
        Err(_) => Err("RMA get response sender dropped".into()),
    }
}

fn handle_rma_get_response(shared: &SharedAsyncState, header: WireHeader, payload: Vec<u8>) {
    let mut queues = shared.queues.lock();
    let matched_idx = queues.pending_rma_ops.iter().position(|&op_ptr| {
        let op = unsafe { &*op_ptr };
        op.local_handle_id == header.local_handle_id && op.local_offset == header.local_offset
    });

    if let Some(idx) = matched_idx {
        let op_ptr = queues.pending_rma_ops.remove(idx).unwrap();

        // Copy data into local mem handle
        {
            let handles = shared.mem_handles.lock();
            if let Some(entry) = handles.get(&header.local_handle_id) {
                let offset = header.local_offset as usize;
                let copy_len = payload.len().min(entry.buf_size.saturating_sub(offset));
                if copy_len > 0 {
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            payload.as_ptr(),
                            (entry.buf as *mut u8).add(offset),
                            copy_len,
                        );
                    }
                }
            }
        }

        let _ = shared.completion_tx.send(CompletionItem {
            op_ptr: op_ptr as usize,
            result: NA_SUCCESS,
            recv_info: None,
        });
        signal_eventfd(shared.event_fd);
    } else {
        warn!("RMA get response: no matching pending op");
    }
}
