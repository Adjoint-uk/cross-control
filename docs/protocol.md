# Protocol Specification

## Transport

cross-control uses QUIC (RFC 9000) via the quinn library. All connections use TLS 1.3 with self-signed certificates. Certificate trust is established via SHA-256 fingerprint pinning (trust-on-first-use).

Default port: **24800** (TCP/UDP).

## Wire Format

Each message is framed as:

```
[4 bytes: payload length (big-endian u32)][payload: bincode v2 encoded]
```

Maximum message size: 1 MiB (1,048,576 bytes).

## QUIC Streams

| Stream | Direction | Purpose |
|--------|-----------|---------|
| Control (stream 0) | Bidirectional | Handshake, device registration, screen geometry, Enter/Leave, keepalive |
| Input | Unidirectional (controller -> controlled) | Real-time EventBatch messages |
| Clipboard | Bidirectional (on demand) | Clipboard Offer/Request/Data on machine switch |

## Connection Lifecycle

1. **Discovery**: Peer found via mDNS or static configuration
2. **Connect**: QUIC connection with TLS 1.3
3. **Handshake**: `Hello` / `Welcome` exchange on stream 0
4. **Device registration**: `DeviceAnnounce` for each input device
5. **Active session**: Barrier crossings trigger `Enter`/`EnterAck`/`Leave`
6. **Input forwarding**: `EventBatch` messages on unidirectional streams
7. **Clipboard sync**: `Offer`/`Request`/`Data` on each machine switch
8. **Disconnect**: `Bye` message for graceful shutdown

## Message Types

### Control Messages

- `Hello { version, machine_id, name, screen }` - Initial handshake
- `Welcome { version, machine_id, name, screen }` - Handshake response
- `DeviceAnnounce(DeviceInfo)` - New input device available
- `DeviceGone { device_id }` - Device removed
- `ScreenUpdate(ScreenGeometry)` - Display geometry changed
- `Enter { edge, position }` - Cursor crossing to remote
- `EnterAck` - Remote ready to receive input
- `Leave { edge, position }` - Cursor returning to local
- `Ping { seq }` / `Pong { seq }` - Keepalive
- `Bye` - Graceful disconnect

### Input Messages

- `InputMessage { device_id, timestamp_us, events: Vec<InputEvent> }` - Batched input events

### Clipboard Messages

- `Offer { formats, size_hint }` - Clipboard content available
- `Request { format }` - Request content in specific format
- `Data(ClipboardContent)` - Clipboard payload

## Version Negotiation

The `Hello`/`Welcome` exchange includes a `ProtocolVersion { major, minor }`. Peers must have matching major versions. Minor version differences are tolerated (newer features are silently ignored by older peers).

Current version: **0.1**
