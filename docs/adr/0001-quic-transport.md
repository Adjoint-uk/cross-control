# ADR 0001 — QUIC as the only transport

**Status:** Accepted
**Date:** 2026-04-09

## Context

cross-control needs a network transport between machines. The incumbents in the software KVM space all use one of:

- **Synergy v1 protocol over TCP, optional TLS** — Barrier, Input Leap, Deskflow. TLS is bolt-on, certificate handling is fragile (Deskflow has multiple open issues around TLS cert generation and Windows↔Linux connection failures), and head-of-line blocking on a single TCP stream means a clipboard image transfer can stall mouse motion.
- **Proprietary TCP+TLS** — Synergy (commercial).
- **Custom UDP** — RKVM, LAN Mouse. Each rolls their own framing and encryption, with the security and complexity bill that implies.

None of them have:
- Encryption that is impossible to turn off
- Stream multiplexing (so input never blocks behind clipboard payloads)
- Fast resume after a network blip
- A standard, audited cryptographic stack

## Decision

**QUIC is the only transport. There is no fallback to TCP, no Synergy v1 compatibility mode, no "encryption optional" knob.**

Concretely:

1. All daemon-to-daemon communication uses QUIC (via `quinn`).
2. TLS 1.3 is mandatory and inherent to the connection — there is no code path that produces an unencrypted session.
3. Input events, clipboard data, and control messages travel on **separate QUIC streams** so a large clipboard payload cannot delay a mouse event.
4. Reconnects use QUIC 0-RTT where the peer is already known, for sub-second resume after a network blip.
5. Trust is established via TOFU certificate pinning (see ADR 0002 when written), not a CA.

## Consequences

### Positive

- **Encryption is architectural, not behavioural.** A user cannot misconfigure cross-control into running unencrypted. The closest analogue in our other projects is the "no `get` command" rule in `llm-secrets` — make the unsafe state unrepresentable.
- **No head-of-line blocking.** Independent QUIC streams mean clipboard transfers and input events do not interfere with each other. This is the single biggest UX improvement over Synergy v1.
- **0-RTT resume.** Roaming between Wi-Fi networks, suspending a laptop, or a brief NAT rebinding does not require a full handshake to recover.
- **Standard, audited crypto.** TLS 1.3 via rustls. We do not invent framing or key exchange.
- **Differentiation.** No other software KVM uses QUIC. This is a load-bearing part of the project's reason to exist (see `docs/research-kvm-landscape.md`).

### Negative

- **No interop with the Synergy v1 ecosystem.** Users of Barrier/Input Leap/Deskflow cannot connect a cross-control daemon to their existing peers. We accept this — interop with a 2009-era protocol would compromise every property above. Migration is one-way: cross-control to cross-control.
- **QUIC is UDP.** Some restrictive corporate firewalls block or rate-limit UDP. Users on such networks cannot use cross-control. We will not add a TCP fallback to work around this — that fallback would become the path of least resistance and erode the guarantees above. Document the limitation; do not engineer around it.
- **Larger dependency surface.** `quinn` + `rustls` + `ring` is more code than a hand-rolled TCP framer. We accept this in exchange for not maintaining our own TLS or framing.
- **Slightly higher minimum CPU.** TLS 1.3 + QUIC framing costs a few percent more CPU than plaintext TCP on tiny payloads. Negligible on any machine made after 2015.

## Alternatives considered

### TCP + TLS (the Deskflow path)

Familiar, works through every firewall, mature library support. Rejected because:
- Head-of-line blocking on a single stream is a real UX problem for clipboard + input on the same connection.
- "Optional TLS" tends to mean "TLS off in practice" — see the Deskflow issue tracker.
- Multiplexing requires either multiple TCP connections (hard to coordinate) or an application-layer framing protocol (reinventing QUIC, badly).

### Custom UDP (the RKVM / LAN Mouse path)

Maximum control, minimum dependencies. Rejected because:
- Rolling our own encryption is the wrong call for a 2026 project. The threat model includes hostile networks.
- Reinventing congestion control, retransmission, and stream multiplexing is the entire reason QUIC exists.

### WireGuard tunnel underneath plain TCP

Strong crypto, mature. Rejected because:
- Pushes the burden of tunnel setup onto the user, defeating the zero-config goal.
- Doesn't solve head-of-line blocking — you still have one TCP stream inside the tunnel.

## Revisit triggers

This decision should be revisited if any of the following happens:

1. A major OS or firewall vendor blocks UDP by default in a way that affects more than a small minority of users.
2. `quinn` becomes unmaintained and no comparable Rust QUIC implementation exists.
3. A QUIC-level vulnerability emerges that cannot be patched at the library level.

None of these are likely. The decision is intended to be permanent for the v1.x series.
