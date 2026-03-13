# Mercury NA Plugin: libp2p — Design Document

This document describes the internal architecture and design decisions of the
Mercury NA (Network Abstraction) plugin that uses
[libp2p](https://libp2p.io/) as the transport layer. It is intended for
developers who need to understand, maintain, or extend this codebase.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Source Layout](#2-source-layout)
3. [Transport Stack](#3-transport-stack)
4. [Threading Model](#4-threading-model)
5. [State Structures](#5-state-structures)
6. [FFI Entry Point and Ops Table](#6-ffi-entry-point-and-ops-table)
7. [Build System](#7-build-system)
8. [Lifecycle: Initialize and Finalize](#8-lifecycle-initialize-and-finalize)
9. [Addressing](#9-addressing)
10. [Messaging: Unexpected and Expected](#10-messaging-unexpected-and-expected)
11. [RMA: Put and Get](#11-rma-put-and-get)
12. [Progress Engine: Poll and Trigger](#12-progress-engine-poll-and-trigger)
13. [Per-Peer Stream Multiplexing](#13-per-peer-stream-multiplexing)
14. [Wire Protocol](#14-wire-protocol)
15. [Connection Management](#15-connection-management)
16. [Synchronization Strategy](#16-synchronization-strategy)
17. [Operation Lifecycle](#17-operation-lifecycle)
18. [Cancellation](#18-cancellation)
19. [Memory Ownership and Safety](#19-memory-ownership-and-safety)
20. [Configuration and Tuning](#20-configuration-and-tuning)
21. [Design Decisions and Trade-offs](#21-design-decisions-and-trade-offs)

---

## 1. Overview

Mercury is an RPC framework for HPC that delegates network I/O to swappable
"NA plugins." Each plugin implements ~34 callback functions collected in a
`na_class_ops` struct. Mercury loads the plugin's `.so` at runtime, finds the
exported `na_<name>_class_ops_g` symbol, and drives all communication through
those callbacks.

This plugin (`libna_plugin_libp2p.so`) implements the NA interface using
**Rust** and the **rust-libp2p** networking library. It produces a
C-compatible shared library via Rust's `cdylib` crate type.

**Key properties:**

- Every peer gets a cryptographic Ed25519 identity automatically.
- Connections are encrypted (Noise protocol) and multiplexed (Yamux).
- The plugin uses a **two-thread model**: a synchronous NA thread for FFI
  callbacks and an asynchronous Tokio thread for network I/O.
- All outbound messages to a given peer are serialized through a **single
  persistent Yamux stream** per peer direction (see §13).

---

## 2. Source Layout

```
src/
  lib.rs          # Crate root: FFI ops table, bindings module, return-code aliases
  plugin.rs       # All 34 NA callback implementations (1192 lines)
  state.rs        # Data structures: NaLibp2pClass, NaLibp2pAddr, NaLibp2pOpId,
                  #   NaLibp2pMemHandle, OperationQueues, StashedMessage, MemHandleEntry
  runtime.rs      # Async runtime bridge: Command/Completion channels, swarm task,
                  #   per-peer sender pool, incoming stream handler
  protocol.rs     # Wire message format: WireHeader, MessageType, read/write functions
build.rs          # Locates Mercury headers via env vars or pkg-config, runs bindgen
wrapper.h         # C header wrapper for bindgen (includes na.h, na_types.h, etc.)
Cargo.toml        # Crate dependencies and build configuration
CMakeLists.txt    # CMake wrapper: invokes cargo, builds C test suite
```

---

## 3. Transport Stack

```
Application  (Mercury NA API)
    │
libp2p-stream  (/mercury-na/1.0.0 protocol)
    │
Yamux          (stream multiplexing, up to 8192 concurrent streams)
    │
Noise          (authenticated encryption, XX handshake)
    │
TCP            (OS-assigned port, nodelay enabled)
```

- **TCP** with `nodelay(true)` — disables Nagle's algorithm for lower latency.
- **Noise** (XX handshake) — provides authenticated, encrypted channels. Each
  peer generates an Ed25519 keypair on initialization via
  `SwarmBuilder::with_new_identity()`.
- **Yamux** — multiplexes multiple logical streams over a single TCP connection.
  Configured for up to 8192 concurrent streams with a proportionally scaled
  receive window (see §20).
- **libp2p-stream** — application-level stream protocol registered under
  `/mercury-na/1.0.0`. Provides `Control::open_stream()` and
  `Control::accept()` for opening and receiving streams.

---

## 4. Threading Model

```
Mercury C application
  │  extern "C" callbacks (NA thread)
  ▼
plugin.rs  (synchronous)
  │  Command channel (tokio mpsc::UnboundedSender)
  ▼
runtime.rs  (asynchronous, Tokio thread)
  ├── libp2p Swarm event loop
  ├── Per-peer writer tasks (PeerSenderPool)
  │     └── One persistent yamux stream per peer direction
  ├── Incoming stream handler (reads messages in a loop)
  └── TCP + Noise + Yamux → Network
  │
  │  Completion channel (tokio mpsc::UnboundedReceiver) + eventfd signal
  ▼
plugin.rs  NA_Poll() → NA_Trigger() → user callbacks
```

### NA thread (synchronous)

All `extern "C"` callback functions run on whatever thread Mercury calls them
from (typically a single "NA thread" or a progress thread). These functions:

- Are non-blocking — they enqueue commands and return immediately.
- Access plugin state through `na_class.plugin_class`, which is a raw pointer
  to a heap-allocated `NaLibp2pClass`.
- Drain completions in `na_libp2p_poll()`.

### Tokio thread (asynchronous)

A multi-threaded Tokio runtime is created during `initialize()`. The main async
task runs the libp2p Swarm event loop using `tokio::select!` to multiplex:

1. **Swarm events** — connection established, connection closed, errors.
2. **Command channel** — messages from the NA thread requesting sends, RMA
   operations, address registration, or shutdown.

Additional async tasks run for:
- **Per-peer writer tasks** — one per remote peer, sending messages on a
  persistent stream (see §13).
- **Incoming stream handlers** — one per accepted incoming stream, reading
  messages in a loop.

### Inter-thread communication

- **Commands (NA → async)**: `tokio::sync::mpsc::UnboundedSender<Command>` —
  unbounded so that `send()` never blocks the NA thread.
- **Completions (async → NA)**: `tokio::sync::mpsc::UnboundedReceiver<CompletionItem>` —
  drained by `na_libp2p_poll()`.
- **eventfd**: A Linux `eventfd(EFD_NONBLOCK | EFD_CLOEXEC)` bridges the two
  threads. The async side calls `signal_eventfd()` after pushing a completion.
  Mercury's progress loop can `epoll`/`select` on this fd (returned by
  `na_libp2p_poll_get_fd()`) instead of busy-polling.

---

## 5. State Structures

### `NaLibp2pClass` — plugin-level state

Stored as an opaque pointer in `na_class.plugin_class`. Allocated in
`initialize()`, freed in `finalize()`.

```rust
pub struct NaLibp2pClass {
    pub self_addr: Arc<NaLibp2pAddr>,                         // this peer's address
    pub queues: Arc<Mutex<OperationQueues>>,                   // shared with async side
    pub cmd_tx: mpsc::UnboundedSender<Command>,                // NA → async commands
    pub completion_rx: Mutex<mpsc::UnboundedReceiver<CompletionItem>>, // async → NA completions
    pub event_fd: RawFd,                                       // poll notification fd
    pub runtime: Option<tokio::runtime::Runtime>,               // Tokio runtime handle
    pub swarm_join: Option<tokio::task::JoinHandle<()>>,        // swarm task join handle
    pub mem_handles: Arc<Mutex<HashMap<u64, MemHandleEntry>>>,  // RMA handle registry
    pub next_mem_handle_id: AtomicU64,                          // monotonic handle ID counter
}
```

The `completion_rx` is wrapped in `parking_lot::Mutex` (not `tokio::sync::Mutex`)
because it is accessed from the NA thread which is synchronous. The `queues`
and `mem_handles` use `Arc<parking_lot::Mutex<...>>` because they are shared
between the NA thread and the Tokio runtime.

### `OperationQueues` — pending ops and stashed messages

```rust
pub struct OperationQueues {
    pub unexpected_recv_ops: VecDeque<*mut NaLibp2pOpId>,  // pending unexpected recv ops
    pub expected_recv_ops: VecDeque<*mut NaLibp2pOpId>,    // pending expected recv ops
    pub unexpected_msg_stash: VecDeque<StashedMessage>,    // early unexpected messages
    pub expected_msg_stash: VecDeque<StashedMessage>,      // early expected messages
    pub pending_rma_ops: VecDeque<*mut NaLibp2pOpId>,      // pending RMA get ops
}
```

This struct is shared between the NA thread and the async runtime via
`Arc<Mutex<...>>`. The NA thread enqueues recv operations; the async incoming
handler dequeues and matches them against arriving messages (or stashes
messages if no matching recv is posted).

### `NaLibp2pAddr` — peer address

```rust
pub struct NaLibp2pAddr {
    pub peer_id: PeerId,                // libp2p cryptographic peer identity
    pub ip: Option<std::net::IpAddr>,   // None for incoming-only peers
    pub port: Option<u16>,
    pub is_self: bool,
}
```

Addresses are heap-allocated via `Box` and cast to/from `*mut na_addr_t` for
FFI. The canonical string format is `libp2p://<ip>:<port>/<base58-peer-id>`.

Two peers are considered equal if their `PeerId` values match — the IP/port is
only used for initial connection establishment. This is because libp2p
connections are identified by cryptographic peer identity, not by network
address.

### `NaLibp2pOpId` — operation handle

```rust
#[repr(C)]
pub struct NaLibp2pOpId {
    pub completion_data: na_cb_completion_data,  // MUST be first field
    pub context: *mut na_context_t,
    pub addr: *mut NaLibp2pAddr,                 // for expected recv: source filter
    pub tag: na_tag_t,
    pub buf: *mut c_void,                        // recv buffer
    pub buf_size: usize,
    pub local_handle_id: u64,                    // for RMA get
    pub local_offset: u64,
    pub rma_length: usize,
}
```

The struct is `#[repr(C)]` and `completion_data` **must be the first field**
because Mercury casts between `na_op_id_t*` and `na_cb_completion_data*`. The
plugin release callback (`na_libp2p_release`) is set during `op_create()` via
`completion_data.plugin_callback`.

### `NaLibp2pMemHandle` — memory handle

```rust
pub struct NaLibp2pMemHandle {
    pub buf: *mut c_void,              // local buffer pointer (null for remote handles)
    pub buf_size: usize,
    pub handle_id: u64,                // unique ID for this handle
    pub flags: u64,
    pub owner_peer_id: Option<PeerId>, // None = local, Some = deserialized remote
}
```

Local handles have `owner_peer_id = None` and a valid `buf` pointer. Remote
handles (created via `mem_handle_deserialize`) have `owner_peer_id = Some(...)`
and `buf = null`.

### `StashedMessage` — early-arriving message

```rust
pub struct StashedMessage {
    pub payload: Vec<u8>,
    pub source_addr: *mut NaLibp2pAddr,
    pub tag: u32,
}
```

When a message arrives before a matching `recv` is posted, it is stashed in the
appropriate queue. The `source_addr` is a heap-allocated `NaLibp2pAddr` that
will be transferred to the matching operation when it completes.

### `MemHandleEntry` — handle registry entry

```rust
pub struct MemHandleEntry {
    pub buf: *mut c_void,
    pub buf_size: usize,
    pub flags: u64,
}
```

Registered in `NaLibp2pClass.mem_handles` so the async side can perform RMA
operations (read from / write to local memory) without round-tripping through
the NA thread.

---

## 6. FFI Entry Point and Ops Table

`lib.rs` exports a single public symbol:

```rust
#[no_mangle]
pub static na_libp2p_class_ops_g: na_class_ops = na_class_ops {
    class_name: c"libp2p",
    initialize: Some(plugin::na_libp2p_initialize),
    finalize: Some(plugin::na_libp2p_finalize),
    // ... 34 total callback slots
};
```

Mercury's plugin loader `dlopen`s the `.so`, `dlsym`s `na_libp2p_class_ops_g`,
and calls the callbacks through function pointers.

Callbacks that are not implemented are set to `None` (null pointer). Mercury
provides defaults or errors for unimplemented optional callbacks.

FFI bindings are generated at build time by `bindgen` from Mercury's C headers.
The generated code lives in `$OUT_DIR/bindings.rs` and is included via
`include!(concat!(env!("OUT_DIR"), "/bindings.rs"))`.

---

## 7. Build System

### `build.rs` (Cargo build script)

1. **Locates Mercury** via `MERCURY_INCLUDE_DIR` / `MERCURY_LIB_DIR` environment
   variables (set by CMake), or falls back to `pkg-config` for the `na` library.
   `MERCURY_INCLUDE_DIR` supports colon-separated paths.
2. **Runs bindgen** on `wrapper.h` to produce Rust FFI types. Only types and
   functions needed by the plugin are allowlisted.
3. **Emits linker flags**: `rustc-link-search` for the Mercury lib directory,
   `rustc-link-lib=dylib=na` to link against `libna.so`.

### CMakeLists.txt

The CMake wrapper:
1. Finds the Mercury package (`find_package(mercury)`)
2. Invokes `cargo build --release` with the correct `MERCURY_INCLUDE_DIR` and
   `MERCURY_LIB_DIR` set
3. Copies the `.so` from `target/release/` to `build/cargo-build/release/`
4. Builds the C test suite from `test/`

---

## 8. Lifecycle: Initialize and Finalize

### `initialize()` — `plugin.rs:57`

Called once per `NA_Initialize()`. Steps:

1. Initialize `tracing-subscriber` for logging (controlled by `RUST_LOG`).
2. Create a multi-threaded Tokio runtime (`tokio::runtime::Builder::new_multi_thread()`).
3. Create an `eventfd` for poll notification.
4. Determine listen address:
   - `listen=true` (server): `/ip4/0.0.0.0/tcp/0` — all interfaces, OS-assigned port.
   - `listen=false` (client): `/ip4/127.0.0.1/tcp/0` — localhost only.
5. Create shared `OperationQueues` and `mem_handles` (both `Arc<Mutex<...>>`).
6. Call `runtime::spawn_swarm_task()` which, on the Tokio runtime:
   a. Builds a `Swarm` with TCP+Noise+Yamux and `libp2p_stream::Behaviour`.
   b. Sets idle connection timeout to 3600 seconds.
   c. Calls `swarm.listen_on(addr)` and waits for `SwarmEvent::NewListenAddr`.
   d. Registers the `/mercury-na/1.0.0` protocol via `control.accept()`.
   e. Spawns the incoming stream handler task.
   f. Enters the main command loop (`tokio::select!` on swarm events + commands).
7. If listening on `0.0.0.0`, resolve to a concrete IP (by probing a UDP
   socket to 8.8.8.8:80 — never sends data, just gets the local route).
8. Construct `NaLibp2pClass` with all state, store as `na_class.plugin_class`.

### `finalize()` — `plugin.rs:146`

Called on `NA_Finalize()`. Steps:

1. Send `Command::Shutdown` to the async side.
2. `block_on(join_handle)` — wait for the swarm task to exit.
3. Drop the Tokio runtime.
4. Close the `eventfd`.
5. Drop the `NaLibp2pClass` via `Box::from_raw`.
6. Set `na_class.plugin_class = null`.

---

## 9. Addressing

### Address format

```
libp2p://<ip>:<port>/<base58-peer-id>
```

Also accepted during lookup:
- `libp2p+libp2p://<ip>:<port>/<peer-id>` (Mercury canonical form)
- `<ip>:<port>/<peer-id>` (bare form)

### `addr_lookup()` — `plugin.rs:239`

Parses the address string, extracts the PeerId and IP:port, constructs a
multiaddr (`/ip4/<ip>/tcp/<port>`), and sends `Command::AddKnownAddr` to the
swarm. The swarm calls `swarm.add_peer_address()` so libp2p knows where to
dial the peer, then immediately attempts a dial with
`PeerCondition::DisconnectedAndNotDialing` to establish the connection eagerly.

### Address comparison

Two addresses are equal if and only if their `PeerId` values match. The
IP/port is not considered because libp2p identifies peers by cryptographic
identity, not network address.

### Serialization format

```
[2 bytes]  peer_id_len (big-endian u16)
[N bytes]  peer_id (protobuf-encoded public key)
[1 byte]   has_addr (0 or 1)
-- if has_addr == 1:
[1 byte]   ip_version (4 or 6)
[4 or 16]  ip_bytes
[2 bytes]  port (big-endian u16)
```

---

## 10. Messaging: Unexpected and Expected

Mercury defines two messaging patterns:

- **Unexpected**: Fire-and-forget messages matched FIFO on the receiver. The
  receiver posts `recv_unexpected()` without specifying a source or tag.
- **Expected**: Solicited messages matched by `(source_peer_id, tag)`. The
  receiver posts `recv_expected()` specifying which peer and tag to expect.

### Send flow (both types)

1. `msg_send_unexpected()` / `msg_send_expected()` on the NA thread:
   a. Fill in `op.completion_data` with the user's callback, arg, and type.
   b. Copy the payload into a `Vec<u8>`.
   c. Send `Command::SendMessage { dest_peer_id, msg_type, tag, payload, op_ptr, source_peer_id }`
      to the async side.
   d. Return `NA_SUCCESS` immediately (non-blocking).

2. On the async side (command loop in `runtime.rs`):
   a. If not connected to the destination, attempt a dial.
   b. Get or create a per-peer sender via `PeerSenderPool::get_or_create()`.
   c. Create an `OutboundMsg` with a `oneshot::channel` for the result.
   d. Send the `OutboundMsg` through the per-peer channel.
   e. Spawn a task waiting on the oneshot result → push `CompletionItem` + signal eventfd.

3. The per-peer writer task (`peer_writer_task`):
   a. Receives `OutboundMsg` from the mpsc channel.
   b. Writes the message on the persistent stream via `protocol::write_message()`.
   c. On success, sends `Ok(())` through the oneshot.
   d. On failure, drops the broken stream and retries (up to 3 attempts with
      exponential backoff starting at 20ms).

### Receive flow (both types)

#### When `recv` is posted before the message arrives:

1. `msg_recv_unexpected()` / `msg_recv_expected()`:
   a. Check the appropriate stash (unexpected or expected).
   b. If the stash has a matching message → complete immediately
      by copying payload, setting `recv_info` in `completion_data`,
      and calling `na_cb_completion_add(context, &completion_data)`.
   c. If no match → enqueue the op pointer in the appropriate
      `recv_ops` queue.

2. When a message later arrives on the async side:
   a. `handle_incoming_msg()` takes the `queues` lock.
   b. For unexpected: matches the first pending recv (FIFO).
   c. For expected: searches for a pending recv matching `(peer_id, tag)`.
   d. If match: copies payload into `op.buf`, sends `CompletionItem` via
      completion channel, signals eventfd.

#### When the message arrives before `recv` is posted:

1. `handle_incoming_msg()` finds no matching pending recv.
2. The message is stashed in `unexpected_msg_stash` or `expected_msg_stash`.
3. When `recv` is later posted, the stash is checked first (see above).

### Matching rules

- **Unexpected**: FIFO — any pending recv matches any incoming message.
- **Expected**: The incoming message's `(source_peer_id, tag)` must match the
  recv op's `(addr.peer_id, tag)`. If the recv's `addr` is null, any source
  matches. If the stashed message's `source_addr` is null, any peer matches.

---

## 11. RMA: Put and Get

RMA (Remote Memory Access) allows one-sided data transfer to/from memory
regions registered with `mem_handle_create()`.

### Memory handle lifecycle

1. **Create**: `mem_handle_create()` allocates a `NaLibp2pMemHandle`, assigns
   a unique `handle_id`, and registers a `MemHandleEntry` in the shared
   `mem_handles` map so the async side can access the buffer.
2. **Serialize**: Encodes `(handle_id, buf_size, flags, owner_peer_id)` in a
   binary format for transmission to the remote peer.
3. **Deserialize**: On the remote side, creates a `NaLibp2pMemHandle` with
   `owner_peer_id = Some(remote_peer_id)` and `buf = null` (no local memory).
4. **Free**: Removes the entry from `mem_handles` (only for locally-owned
   handles).

### `put()` — `plugin.rs:962`

One-sided write to remote memory:

1. Read data from the local buffer at `(local_handle.buf + local_offset)`.
2. Send `Command::RmaPut` with the data, remote handle ID, and remote offset.
3. Async side opens/reuses a stream and writes an `RmaPut` wire message.
4. On the remote side, `handle_rma_put()` looks up the handle ID in the
   shared `mem_handles` map and copies the payload directly into the
   registered buffer at the specified offset.
5. The put completes on the sender side when the write succeeds.

### `get()` — `plugin.rs:1011`

One-sided read from remote memory (two-phase):

1. Enqueue the op in `pending_rma_ops` (needed to match the response later).
2. Send `Command::RmaGetRequest` with remote handle ID, remote offset, length,
   and the local handle ID + offset where the data should land.
3. Async side sends an `RmaGetReq` wire message to the remote.
4. Remote's `handle_rma_get_request()` reads from its local buffer and sends
   back an `RmaGetResp` via the per-peer response pool.
5. The requester's `handle_rma_get_response()` receives the response, matches
   it against `pending_rma_ops` by `(local_handle_id, local_offset)`, copies
   the data into the local buffer, and pushes a completion.

---

## 12. Progress Engine: Poll and Trigger

Mercury drives operation completion through a two-step progress pattern:

```c
NA_Poll(na_class, context, &count);    // collect completed ops
NA_Trigger(context, count, &actual);   // fire user callbacks
```

### `poll_get_fd()` — `plugin.rs:1067`

Returns the `eventfd` file descriptor. Mercury can `epoll_wait` or `select` on
this fd to avoid busy-polling. The async side writes to this fd whenever it
pushes a completion.

### `poll_try_wait()` — `plugin.rs:1075`

Returns `true` if the completion channel is empty (nothing to drain). Mercury
uses this as a quick check before calling `poll()`.

### `poll()` — `plugin.rs:1084`

Always returns `NA_SUCCESS` (critical — Mercury HG core requires this).

Steps:
1. Drain the eventfd (consume the notification).
2. Lock `completion_rx` and drain all pending `CompletionItem`s via `try_recv()`.
3. For each completion:
   a. Set `completion_data.callback_info.ret` to the result code.
   b. If the completion carries `RecvInfo` (for recv operations):
      - Set `recv_unexpected` or `recv_expected` info in the callback info union.
      - For expected recv, free the temporary source address.
   c. Call `na_cb_completion_add(context, &completion_data)` to queue the user
      callback for the subsequent `NA_Trigger()`.
4. Set `*count_p` to the number of completions drained.

---

## 13. Per-Peer Stream Multiplexing

### The Problem

The initial design opened a **new yamux stream for every message**. Under high
concurrency (Mercury's RPC test with 16 progress threads), this created 200+
short-lived streams in rapid succession. Yamux connections entered a broken
state — streams failed with "connection is closed" errors from `write_zero_err()`,
yet `SwarmEvent::ConnectionClosed` was never emitted by the swarm. Retries and
reconnection attempts could not recover reliably because:

- The swarm still considered the connection alive (no ConnectionClosed event).
- `open_stream` would succeed but writes would fail.
- Multiple concurrent tasks attempting reconnection competed with each other.

### The Solution: `PeerSenderPool`

All outbound messages to a given peer are serialized through a **single
persistent yamux stream**, managed by a dedicated per-peer writer task.

```rust
struct PeerSenderPool {
    senders: HashMap<PeerId, mpsc::UnboundedSender<OutboundMsg>>,
}
```

`get_or_create(peer, control)`:
- If a sender exists and is not closed, return it.
- Otherwise, spawn a new `peer_writer_task` and return the sender.

```
            ┌─ OutboundMsg ─→ peer_writer_task(Peer A) ─→ yamux stream A
Command ─→ PeerSenderPool
            └─ OutboundMsg ─→ peer_writer_task(Peer B) ─→ yamux stream B
```

### `peer_writer_task`

```rust
async fn peer_writer_task(peer, control, rx) {
    let mut stream_opt: Option<libp2p::Stream> = None;
    while let Some(msg) = rx.recv().await {
        let result = send_one_message(&control, peer, &mut stream_opt, &msg).await;
        if let Some(tx) = msg.result_tx {
            let _ = tx.send(result);
        }
    }
}
```

Key behaviors:
- Opens a stream lazily on the first message.
- Keeps the stream open across messages (stream reuse).
- On write failure, drops the broken stream and retries with a fresh one
  (up to 3 attempts, exponential backoff: 20ms, 40ms, 80ms).
- The reader on the other side calls `handle_incoming_stream()` which loops
  with `read_message()` until `UnexpectedEof`, handling multiple messages per
  stream.

### Two sender pools

There are **two** `PeerSenderPool` instances:

1. **Command-side pool** (`sender_pool` in the command loop): Handles all
   outbound messages initiated by the NA thread — `SendMessage`, `RmaPut`,
   `RmaGetRequest`.
2. **Response-side pool** (`response_pool` in the incoming handler): Handles
   `RmaGetResp` messages that need to be sent back from the incoming handler.
   This is wrapped in `Arc<tokio::sync::Mutex<PeerSenderPool>>` because it is
   shared across multiple incoming-stream handler tasks.

The response pool uses `tokio::sync::Mutex` (not `parking_lot::Mutex`) because
it is held across `.await` points.

---

## 14. Wire Protocol

Protocol identifier: `/mercury-na/1.0.0`

Messages are sent back-to-back on the same stream (no delimiter needed — the
header encodes the payload length).

### Wire format

```
 1 byte   msg_type         (1=Unexpected, 2=Expected, 3=RmaPut, 4=RmaGetReq, 5=RmaGetResp)
 4 bytes  tag              (big-endian u32)
 4 bytes  payload_size     (big-endian u32)
 2 bytes  peer_id_len      (big-endian u16)
 N bytes  source_peer_id   (protobuf-encoded public key)
 8 bytes  handle_id        (big-endian u64, RMA operations)
 8 bytes  offset           (big-endian u64, RMA operations)
 8 bytes  rma_length       (big-endian u64, RMA operations)
 8 bytes  local_handle_id  (big-endian u64, RMA get request/response)
 8 bytes  local_offset     (big-endian u64, RMA get request/response)
 P bytes  payload
```

All fields are always present in every message, even when unused (e.g., RMA
fields are zero for Unexpected/Expected messages). This simplifies parsing —
`read_message()` always reads the same sequence of fields.

### `write_message()` — `protocol.rs:42`

Serializes the header and payload into a single `Vec<u8>` buffer, then calls
`write_all` followed by `flush`. Building a single buffer avoids multiple
small writes and partial header transmission.

### `read_message()` — `protocol.rs:74`

Reads fields sequentially with `read_exact`. Returns `UnexpectedEof` when the
peer closes the stream (normal termination of the message loop).

---

## 15. Connection Management

### Connection establishment

Connections are established in two ways:

1. **During `addr_lookup()`**: The command loop calls `swarm.add_peer_address()`
   and `swarm.dial()` with `PeerCondition::DisconnectedAndNotDialing`.
2. **On-demand during send**: Before dispatching a `SendMessage` or RMA command,
   the command loop checks `swarm.is_connected()` and dials if not connected.

The `PeerCondition::DisconnectedAndNotDialing` condition prevents duplicate
dials when multiple sends target the same unconnected peer simultaneously.

### Connection persistence

Idle connection timeout is set to 3600 seconds (1 hour). Connections are
expected to last the lifetime of the application.

### Auto-reconnect

On `SwarmEvent::ConnectionClosed` (when `num_established == 0`):
1. Look up the peer's address from `peer_addrs`.
2. Re-add the address and re-dial.
3. Remove the old sender from `sender_pool` — the writer task will be
   recreated on the next send.

### Peer address tracking

A `PeerAddrMap` (`Arc<Mutex<HashMap<PeerId, Multiaddr>>>`) tracks known peer
addresses. Updated during `AddKnownAddr` commands. Used for auto-reconnect.

---

## 16. Synchronization Strategy

| Data structure | Lock type | Reason |
|----------------|-----------|--------|
| `OperationQueues` | `parking_lot::Mutex` | Shared between sync NA thread and async incoming handler. `parking_lot` is chosen for performance (no poisoning, smaller, faster). Lock is held briefly. |
| `mem_handles` | `parking_lot::Mutex` | Same as above — shared between sync and async. |
| `completion_rx` | `parking_lot::Mutex` | Accessed only from the NA thread (in `poll()`), but needs interior mutability because `get_class()` returns `&NaLibp2pClass`. |
| `response_pool` | `tokio::sync::Mutex` | Shared across async tasks and held across `.await` points (specifically, when awaiting `result_rx` in `handle_rma_get_request`). Tokio Mutex is required here. |

### Lock ordering

There is no circular dependency between locks:
- `queues` and `mem_handles` are never held simultaneously.
- `completion_rx` is only accessed from `poll()` on the NA thread.
- `response_pool` is only accessed from incoming stream handlers.

---

## 17. Operation Lifecycle

```
op_create()          → Box::new(NaLibp2pOpId), set plugin_callback
    │
msg_send/recv()      → fill completion_data, enqueue or send command
    │
[async processing]   → command handling, network I/O, incoming handler
    │
CompletionItem       → pushed to completion channel, eventfd signaled
    │
poll()               → drain completions, call na_cb_completion_add()
    │
NA_Trigger()         → Mercury fires user callback, then plugin_callback
    │
na_libp2p_release()  → no-op (op lifetime managed by op_create/destroy)
    │
op_destroy()         → Box::from_raw (frees the op)
```

The `plugin_callback` (`na_libp2p_release`) is a no-op in this plugin because
operation memory is managed by `op_create` / `op_destroy`, not by completion.
Mercury calls `plugin_callback` after each user callback returns, providing a
hook for per-completion cleanup if needed.

---

## 18. Cancellation

`cancel()` searches all three operation queues in order:

1. `unexpected_recv_ops`
2. `expected_recv_ops`
3. `pending_rma_ops`

If the op is found, it is removed from the queue, its result is set to
`NA_CANCELED`, and `na_cb_completion_add()` is called to schedule the
cancellation callback.

Send operations that are already in-flight (dispatched to the async side)
cannot be cancelled — they will complete normally.

---

## 19. Memory Ownership and Safety

### Raw pointers across threads

Several types contain raw pointers (`*mut c_void`, `*mut NaLibp2pAddr`, etc.)
and are sent across threads via channels. This requires `unsafe impl Send`.
Safety relies on Mercury's guarantees:

- **Operation buffers** (`op.buf`): Mercury guarantees the buffer is valid
  until the operation completes (user callback returns).
- **Memory handles** (`MemHandleEntry.buf`): Valid from `mem_handle_create()`
  until `mem_handle_free()`.
- **Op pointers** (`op_ptr: usize`): Cast to `usize` to cross channel
  boundaries. Valid because ops are not freed until after completion.

### Box-based FFI allocation

All FFI-visible heap objects (`NaLibp2pAddr`, `NaLibp2pOpId`,
`NaLibp2pMemHandle`, `NaLibp2pClass`) use the pattern:

```rust
let ptr = Box::into_raw(Box::new(value));   // allocate, leak to FFI
// ...
let owned = Box::from_raw(ptr);             // reclaim, drop
```

This ensures proper alignment, size, and destructor behavior.

### `NaLibp2pOpId` layout

The `#[repr(C)]` attribute and `completion_data` being the first field are
**critical**. Mercury casts `na_op_id_t*` to `na_cb_completion_data*` internally.
If `completion_data` is not at offset 0, Mercury will corrupt memory.

---

## 20. Configuration and Tuning

### Yamux tuning (in rust-libp2p fork)

The default yamux v0.13 configuration in the forked `rust-libp2p` was modified
in `muxers/yamux/src/lib.rs`:

```rust
cfg.set_max_connection_receive_window(Some(8192 * 256 * 1024)); // ~2 GB
cfg.set_max_num_streams(8192);
```

This allows up to 8192 concurrent streams (default was 256) with a
proportionally scaled receive window to avoid deadlocks. The receive window
must be set **before** `max_num_streams` due to an assertion ordering
constraint in the yamux library.

### Message size limits

| Parameter | Value |
|-----------|-------|
| Max unexpected message size | 64 KB |
| Max expected message size   | 64 KB |
| Max tag value               | 2^32 - 1 |

### Connection timeout

Idle connection timeout: 3600 seconds (1 hour).

### TCP configuration

`nodelay(true)` is enabled on all TCP connections to avoid Nagle's algorithm
buffering delay.

---

## 21. Design Decisions and Trade-offs

### Why per-peer stream multiplexing instead of stream-per-message?

The original design opened a new yamux stream for every message. This was
conceptually clean but broke under high concurrency:

- 200+ rapid stream open/close cycles destabilized yamux connections.
- Streams would fail with "connection is closed" errors, but no
  `SwarmEvent::ConnectionClosed` was emitted.
- The swarm still considered the connection alive, so reconnection logic
  could not trigger.
- Multiple concurrent tasks attempting to reconnect competed with each other.

The per-peer multiplexing approach eliminates stream creation/teardown overhead
entirely. All messages are serialized through a single persistent stream per
peer direction. The writer task handles reconnection internally with
exponential backoff.

**Trade-off**: Messages to the same peer are now serialized (head-of-line
blocking). In practice, this is not a problem because:
1. Mercury messages are small (≤64 KB).
2. The underlying TCP+Yamux stack provides flow control.
3. Per-message latency is dominated by network, not serialization.

### Why two separate sender pools?

The command-side pool and the response-side pool are separate because:

1. **Deadlock avoidance**: The command loop holds the swarm (borrowed mutably
   in `select!`). If the incoming handler needed to send through the command
   loop, it would need to wait for the command loop to process the send, which
   could be blocked processing swarm events — a potential deadlock.
2. **Async context**: The incoming handler runs in spawned async tasks. The
   response pool uses `tokio::sync::Mutex` because the lock is held across
   `.await` points. The command-side pool uses `parking_lot::Mutex` internally
   (via `HashMap`) and is only accessed from the command loop (single task, no
   `.await` while holding).

### Why `parking_lot::Mutex` instead of `std::sync::Mutex`?

- No poisoning (if a thread panics while holding the lock, other threads can
  still acquire it).
- Smaller in memory.
- Faster uncontended acquisition.
- The plugin never needs poisoning semantics.

### Why `mpsc::UnboundedSender` for commands?

The command channel is unbounded so that `send()` never blocks the NA thread.
Mercury expects callbacks like `msg_send_unexpected()` to return immediately.
Backpressure is provided by the application layer (Mercury's credit system and
operation limits) rather than the channel.

### Why eventfd?

Linux `eventfd` is the lightest-weight notification mechanism:
- Single fd, no pipe buffering overhead.
- `EFD_NONBLOCK` so reads never block.
- Integrates with `epoll` (Mercury's preferred progress mechanism).
- Counter semantics: multiple writes coalesce into a single readable event,
  avoiding thundering herd.

### Why raw pointer casts to `usize` in commands?

`Command` and `CompletionItem` are sent across `tokio::sync::mpsc` channels,
which require `Send`. Raw pointers are not `Send`. Casting to `usize` is a
standard pattern in Rust FFI. Safety is guaranteed by Mercury's lifetime
contracts (buffers are valid until completion, ops are valid until destroyed).

### Why does `poll()` always return `NA_SUCCESS`?

Mercury's HG (higher-level RPC layer) treats any non-success return from
`NA_Poll()` as a fatal error and aborts. Even when there are no completions to
drain, returning `NA_SUCCESS` with `*count = 0` is correct — it simply means
"no progress this time." This is consistent with how other NA plugins behave.
