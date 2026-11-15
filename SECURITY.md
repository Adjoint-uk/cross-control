# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.x.x   | Yes (development) |

## Reporting a Vulnerability

If you discover a security vulnerability in cross-control, please report it responsibly.

**Do not open a public issue.**

Instead, email **security@adjoint.uk** with:

- A description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested fix (if any)

We will acknowledge receipt within 48 hours and aim to provide a fix within 7 days for critical issues.

## Security Considerations

cross-control handles input events and clipboard data across machines on a network. Security is a core design concern:

- **Transport**: All connections use QUIC with TLS 1.3. There is no plaintext mode.
- **Authentication**: Mutual. Each machine has a self-signed certificate; both sides must prove possession of their private key in the TLS handshake. A peer is identified by its certificate's SHA-256 fingerprint, bound to the name it announces.
- **Authorization**: A peer may connect only if its `(name, fingerprint)` is trusted — pinned in `config.toml` or listed in `known_peers` (`cross-control pair` / `cross-control trust add`). A known name presenting a different certificate is always refused. Trust-on-first-use is available (`security.trust_on_first_use`) but off by default.
- **Discovery**: mDNS adverts (including the fingerprint TXT record) are unauthenticated and treated only as hints; they never establish trust.
- **Input**: Events are only exchanged with authorized peers
- **Wire decoding**: Message sizes and decode allocations are bounded, so a peer cannot exhaust memory with a hostile length prefix
- **Clipboard**: Size limits prevent memory exhaustion; clipboard sync is optional
- **No unsafe code**: The entire workspace denies `unsafe_code`

See [ADR 0002](docs/adr/0002-tofu-pairing.md) for the trust model and what it does and does not protect against.
