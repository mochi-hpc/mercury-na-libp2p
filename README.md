# Mercury NA Plugin: libp2p

A Mercury Network Abstraction (NA) plugin that uses
[libp2p](https://libp2p.io/) as the underlying transport. The plugin is
written in Rust and produces a C-compatible shared library
(`libna_plugin_libp2p.so`) that Mercury loads at runtime.

## Transport Stack

```
Application  (Mercury NA API)
    |
libp2p-stream  (/mercury-na/1.0.0 protocol)
    |
Yamux          (stream multiplexing)
    |
Noise          (authenticated encryption)
    |
TCP            (OS-assigned port)
```

Every peer gets a cryptographic identity (Ed25519 key pair) generated
automatically on initialization. Connections are encrypted and
multiplexed — multiple logical message streams share a single TCP
connection per peer pair.

## Prerequisites

| Dependency | Version | Notes |
|------------|---------|-------|
| Rust (rustc, cargo) | stable >= 1.77 | `c""` literals require 1.77+ |
| libclang-dev | >= 13 | Required by the `bindgen` crate |
| CMake      | >= 3.15 | Build system |
| Mercury    | >= 2.4  | NA headers and libraries |
| pkg-config |         | Locates Mercury if not passed via CMake |

Mercury must be installed or its install prefix must be discoverable by
CMake (`-Dmercury_DIR=...` or via `CMAKE_PREFIX_PATH`).

## Building

### CMake (plugin + tests)

```bash
cd mercury-na-plugin-libp2p
mkdir -p build && cd build
cmake .. -Dmercury_DIR=/path/to/mercury/lib/cmake/mercury
make -j$(nproc)
```

This runs `cargo build --release` under the hood. The shared library is
placed at:

```
build/cargo-build/release/libna_plugin_libp2p.so
```

### Cargo only (plugin .so)

If Mercury is discoverable via `pkg-config`:

```bash
PKG_CONFIG_PATH=/path/to/mercury/lib/pkgconfig cargo build --release
```

Or set paths explicitly:

```bash
MERCURY_INCLUDE_DIR=/path/to/mercury/include \
MERCURY_LIB_DIR=/path/to/mercury/lib \
cargo build --release
```

Multiple include directories can be colon-separated:

```bash
MERCURY_INCLUDE_DIR=/path/to/include:/path/to/extra/headers ...
```

### CMake Options

| Variable | Default | Description |
|----------|---------|-------------|
| `mercury_DIR` | — | Path to Mercury's CMake config |
| `NA_PLUGIN_INSTALL_DIR` | `<prefix>/lib` | Where `make install` places the `.so` |

## Running the Tests

```bash
cd build
NA_PLUGIN_PATH=./cargo-build/release ctest --output-on-failure
```

Seven test suites are included:

| Test | What it covers |
|------|----------------|
| `libp2p_init`      | Plugin load, initialize/finalize, protocol info |
| `libp2p_proc`      | Address serialization round-trip |
| `libp2p_msg`       | Unexpected and expected message send/recv |
| `libp2p_lookup`    | Address lookup and string conversion |
| `libp2p_rpc`       | Full Mercury RPC including 16-thread concurrent progress |
| `libp2p_bulk`      | RMA put/get bulk data transfer |
| `libp2p_relay_msg` | Message send/recv over a relayed connection (3-process) |

## Using the Plugin

### 1. Set the plugin path

Tell Mercury where to find `libna_plugin_libp2p.so`:

```bash
export NA_PLUGIN_PATH=/path/to/build/cargo-build/release
```

### 2. Initialize

```c
#include <na.h>

/* Server — listens on all interfaces, OS-assigned port */
na_class_t *na_class = NA_Initialize("libp2p+libp2p://", NA_TRUE);

/* Client — ephemeral port on localhost */
na_class_t *na_class = NA_Initialize("libp2p+libp2p://", NA_FALSE);
```

The info string format is `libp2p+libp2p://`. The plugin ignores
everything after the protocol prefix during init; the listen address and
port are determined automatically (TCP port 0 = OS-assigned).

### 3. Get the self address

After initialization the plugin listens on an OS-assigned TCP port.
Retrieve the self address to share with peers:

```c
na_addr_t self_addr;
NA_Addr_self(na_class, &self_addr);

char buf[256];
na_size_t buf_size = sizeof(buf);
NA_Addr_to_string(na_class, buf, &buf_size, self_addr);
/* buf now contains e.g. "libp2p:/ip4/192.168.1.10/tcp/43210/p2p/12D3KooW..." */
```

### 4. Look up a remote peer

```c
na_addr_t target_addr;
NA_Addr_lookup(na_class,
               "libp2p:/ip4/192.168.1.20/tcp/43211/p2p/12D3KooWABC...",
               &target_addr);
```

#### Address format

Addresses use the standard libp2p
[multiaddr](https://multiformats.io/multiaddr/) convention, prefixed
with the Mercury plugin name:

```
libp2p:<multiaddr>/p2p/<peer-id>
```

Examples:

```
libp2p:/ip4/192.168.1.20/tcp/43211/p2p/12D3KooWABC...   (TCP)
libp2p:/ip4/10.0.0.1/udp/9090/quic-v1/p2p/12D3KooW...  (QUIC)
libp2p:/ip6/::1/tcp/5555/p2p/12D3KooW...                (IPv6 TCP)
```

`<peer-id>` is the base58-encoded libp2p peer ID (typically starts with
`12D3KooW`). The following prefixed form is also accepted during lookup:

```
libp2p+libp2p:<multiaddr>/p2p/<peer-id>   (Mercury canonical prefix)
```

### 5. Send and receive messages

Standard Mercury NA messaging works as expected:

```c
/* Sender */
NA_Msg_send_unexpected(na_class, context, callback, cb_arg,
                       buf, buf_size, plugin_buf,
                       target_addr, 0, tag, op_id);

/* Receiver */
NA_Msg_recv_unexpected(na_class, context, callback, cb_arg,
                       buf, buf_size, plugin_buf, op_id);
```

Expected (solicited) messages use `NA_Msg_send_expected` /
`NA_Msg_recv_expected` with a matching `(peer, tag)` pair.

### 6. RMA (put / get)

Register local memory, serialize the handle, and transfer:

```c
na_mem_handle_t local_handle;
NA_Mem_handle_create(na_class, buf, buf_size, flags, &local_handle);

/* Serialize and send the handle to the remote peer out-of-band */
NA_Mem_handle_serialize(na_class, serial_buf, serial_size, local_handle);

/* Remote side deserializes and issues put/get */
NA_Put(na_class, context, callback, cb_arg,
       local_handle, 0, remote_handle, 0, size,
       remote_addr, 0, op_id);
```

### 7. Circuit relay

When peers cannot reach each other directly (e.g., behind NAT or
firewalls), traffic can be routed through an intermediary **relay
server**. The build produces a standalone relay binary,
`mercury-na-relay-server`, that is installed alongside the plugin.

#### Start the relay server

```bash
mercury-na-relay-server \
    --port 4001 \
    --secret-key-seed 1 \
    --addr-file relay_addr.txt
```

| Flag | Description |
|------|-------------|
| `--port <N>` | TCP listen port (`0` for OS-assigned, default `0`) |
| `--secret-key-seed <N>` | Byte seed for deterministic Ed25519 key (required) |
| `--addr-file <path>` | Write the relay's full multiaddr to this file once listening (required) |

The relay server logs its multiaddr(s) to stdout and writes the first
one (with `0.0.0.0` resolved to `127.0.0.1`) to the addr-file. Example
output:

```
Relay server PeerId: 12D3KooWPjce...
Relay listening on /ip4/127.0.0.1/tcp/4001/p2p/12D3KooWPjce...
```

The relay runs until interrupted with Ctrl-C (SIGINT).

#### Configure NA peers to use the relay

Set the `MERCURY_RELAY_ADDR` environment variable to the relay's
multiaddr (including its `/p2p/<peer_id>` suffix), and include `relay`
in the protocol name:

```bash
export MERCURY_RELAY_ADDR=/ip4/10.0.0.1/tcp/4001/p2p/12D3KooWPjce...
```

```c
/* Server — listens via the relay */
na_class_t *na = NA_Initialize("tcp,relay://", NA_TRUE);

/* Client — connects through the relay */
na_class_t *na = NA_Initialize("tcp,relay://", NA_FALSE);
```

The server makes a reservation with the relay and becomes reachable at
a circuit address. `NA_Addr_self()` returns this circuit address so the
client can dial through the relay. All subsequent NA operations
(messaging, RMA) work transparently over the relayed connection.

#### Relay limitations

The relay server uses default libp2p relay v2 limits:

| Parameter | Default |
|-----------|---------|
| Max data per circuit | 128 KiB |
| Max circuit duration | 120 s |
| Max concurrent reservations | 128 |
| Max concurrent circuits | 16 |

These limits are suitable for control-plane RPC with small messages.
For bulk data transfer, use direct connections.

### 8. Progress loop

Drive completion with the standard Mercury progress pattern:

```c
unsigned int count;
NA_Poll(na_class, context, &count);
NA_Trigger(context, count, &actual);
```

The plugin exposes an `eventfd` via `NA_Poll_get_fd()` so that the
application can integrate with `epoll` / `select` instead of
busy-polling.

## Message Limits

| Parameter | Value |
|-----------|-------|
| Max unexpected message size | 64 KB |
| Max expected message size   | 64 KB |
| Max tag value               | 2^32 - 1 |

## Environment Variables

| Variable | Description |
|----------|-------------|
| `NA_PLUGIN_PATH` | Directory containing `libna_plugin_libp2p.so` (required) |
| `MERCURY_RELAY_ADDR` | Relay server multiaddr for circuit relay (e.g., `/ip4/.../tcp/.../p2p/12D3KooW...`) |
| `RUST_LOG` | Controls log verbosity. Examples: `error`, `warn`, `debug`, `na_plugin_libp2p=debug` |

## Architecture

```
Mercury C application
  |  extern "C" callbacks
  v
plugin.rs  (sync, NA thread)
  |  Command channel (tokio mpsc)
  v
runtime.rs (async, Tokio thread)
  |-- libp2p Swarm event loop
  |-- Per-peer sender pool (PeerSenderPool)
  |     \-- One persistent yamux stream per peer direction
  |-- Incoming stream handler (reads messages in a loop)
  \-- TCP + Noise + Yamux -> Network
  |
  |  Completion channel + eventfd signal
  v
plugin.rs  NA_Poll() -> NA_Trigger() -> user callbacks
```

**Two-thread model:**

- **NA thread** (synchronous) — all `extern "C"` callbacks run here.
  Sends commands to the async side and drains completions in `NA_Poll`.
- **Tokio thread** (asynchronous) — runs the libp2p Swarm, handles
  connection management, and performs all network I/O.

An `eventfd` bridges the two: the async side writes to it when
completions are ready, and `NA_Poll_get_fd()` returns the fd so Mercury
can `epoll`/`select` on it.

**Per-peer stream multiplexing:** all outbound messages to a given peer
are serialized through a single persistent yamux stream (managed by
`PeerSenderPool`). This avoids the overhead and instability of opening a
new yamux stream per message under high concurrency.

## Source Layout

```
src/
  lib.rs        # FFI ops table (na_libp2p_class_ops_g)
  plugin.rs     # All 34 NA callback implementations
  state.rs      # NaLibp2pClass, NaLibp2pAddr, NaLibp2pOpId, NaLibp2pMemHandle
  runtime.rs    # Tokio runtime, swarm task, per-peer sender pool
  protocol.rs   # Wire message format (header + payload framing)
relay-server/
  Cargo.toml    # Standalone crate for the relay server binary
  src/main.rs   # Minimal libp2p relay (TCP + Noise + Yamux + relay behaviour)
```

## Wire Protocol

Each message on the stream consists of a header followed by a payload:

```
 1 byte   msg_type  (1=Unexpected, 2=Expected, 3=RmaPut, 4=RmaGetReq, 5=RmaGetResp)
 4 bytes  tag       (big-endian u32)
 4 bytes  payload_size (big-endian u32)
 2 bytes  peer_id_len  (big-endian u16)
 N bytes  source peer ID
 8 bytes  handle_id    (RMA, big-endian u64)
 8 bytes  offset       (RMA, big-endian u64)
 8 bytes  rma_length   (RMA, big-endian u64)
 8 bytes  local_handle_id  (RMA, big-endian u64)
 8 bytes  local_offset     (RMA, big-endian u64)
 P bytes  payload
```

Multiple messages are sent back-to-back on the same stream (no
delimiter needed — the header encodes the payload length).
