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

- **Transport**: All connections use QUIC with TLS 1.3 (self-signed certificates with fingerprint pinning)
- **Authentication**: Certificate fingerprint pinning (trust-on-first-use, similar to SSH)
- **Input**: Events are only forwarded between explicitly paired machines
- **Clipboard**: Size limits prevent memory exhaustion; clipboard sync is optional
- **No unsafe code**: The entire workspace denies `unsafe_code`
