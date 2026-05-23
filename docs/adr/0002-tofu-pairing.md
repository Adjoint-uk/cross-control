# ADR 0002 — TOFU certificate pinning (no CA)

**Status:** Accepted
**Date:** 2026-05-23

## Context

[ADR 0001](0001-quic-transport.md) commits cross-control to QUIC with mandatory TLS 1.3. That decision leaves one question open: **whose certificate do we trust?**

The two well-trodden answers do not fit:

- **A public CA (Let's Encrypt, ZeroSSL, etc.)** — designed for public hostnames reachable from the internet. cross-control peers are personal devices on private networks; most have no DNS name, no public IP, and no inbound reachability for ACME. Requiring a public CA would force users to either expose machines to the internet or run a private CA — both are larger configuration surfaces than the rest of the project combined.
- **A user-run private CA** — works, but pushes a credential-management problem onto every user. The CA key becomes a high-value secret. Rotation and revocation require infrastructure the user does not want to build.

A third answer — **trust on first use (TOFU)** — has 25+ years of field deployment in SSH, which solves an almost identical problem: pairing personal machines without a global trust anchor.

The competitive landscape supports the choice. Among software KVMs:
- **Synergy v1 / Barrier / Input Leap / Deskflow** — TLS is optional and cert handling is a recurring bug source.
- **RKVM** — pre-shared keys, no rotation story.
- **LAN Mouse** — no encryption at all.

None of them pin. None of them have a trust model that a security-conscious user would accept on a hostile network.

## Decision

**cross-control uses TOFU certificate pinning. There is no CA, no public PKI, and no way to disable verification.**

Concretely:

1. Each daemon generates a self-signed Ed25519 (or ECDSA P-256) cert on first run. The cert lives in the daemon config directory next to the machine ID.
2. On first contact with a new peer, the daemon records the peer's cert SHA-256 fingerprint. Subsequent connections **require** the fingerprint to match — a mismatch refuses the connection with an SSH-style `REMOTE HOST IDENTIFICATION HAS CHANGED` error.
3. First contact can be tightened two ways:
   - **Out-of-band fingerprint** — the user reads the fingerprint from one machine (CLI `generate-cert` or `status`) and pins it in the other's config (`fingerprint = "..."` under `[[screens]]`). Eliminates the first-contact MitM window.
   - **mDNS fingerprint TXT record** — the [discovery layer](../../crates/cross-control-discovery/src/mdns.rs) publishes the cert fingerprint in the `fp` TXT record. A peer that discovers us via mDNS sees the fingerprint *before* connecting and can refuse if it doesn't match an expected value.
4. Fingerprints are stored as lowercase hex in the daemon config directory. Rotation = delete the pinned entry and re-pair.
5. The handshake (`Hello` / `Welcome` in [`cross-control-types`](../../crates/cross-control-types/src/message.rs)) carries `MachineId` so the daemon binds `(machine_id, fingerprint)` together — a rebound IP cannot impersonate a different peer.

## What TOFU does and does not protect against

TOFU is honest about its limits. Document them so users can decide.

| Threat                                                | Protected? | Notes                                                          |
|-------------------------------------------------------|------------|----------------------------------------------------------------|
| Passive eavesdropping on input/clipboard              | Yes        | Always-on TLS 1.3 — ADR 0001                                   |
| Active MitM **after** first contact                   | Yes        | Pinned fingerprint mismatch refuses the connection             |
| Active MitM **on** first contact (no OOB fingerprint) | **No**     | This is the TOFU window; tighten with `fingerprint = "..."`    |
| Active MitM on first contact, mDNS fingerprint        | Mostly     | Only safe if the attacker cannot forge mDNS — i.e. trusted LAN |
| Stolen daemon config dir (cert + machine ID)          | No         | Same threat model as a stolen SSH key. Treat the dir as a secret. |
| Compromised host                                      | No         | Out of scope. Same as every KVM.                               |
| Network downgrade to plaintext                        | Yes        | No plaintext path exists (ADR 0001). Downgrade is unrepresentable. |

## Consequences

### Positive

- **No infrastructure for the user.** No CA, no ACME, no DNS challenge, no key escrow. Generate cert, write down fingerprint once if you care about first-contact security, done.
- **Familiar mental model.** SSH-style pinning is well-understood by the audience this project targets. The fingerprint-mismatch error message is the same loud red flag users already know.
- **Composable with mDNS.** Because the fingerprint travels in the mDNS TXT record, a peer discovered via discovery already has the fingerprint in hand before TLS — pinning happens at discovery time, not handshake time.
- **No global trust anchor to compromise.** There is no CA key whose theft compromises the network. Each peer pair is independent.

### Negative

- **First contact is the weak point.** A user on a hostile network who pairs for the first time *without* an out-of-band fingerprint can be MitM-ed. We accept this and document it. The CLI prompt on first contact should display the fingerprint clearly so the user can verify out-of-band.
- **No revocation list.** If a private key leaks, every machine that pinned the corresponding fingerprint must be told individually to drop it. We accept this — the population of peers is small (typically 2–6), and a CRL infrastructure would dwarf the protocol.
- **Cert rotation is manual.** SSH lives with this. So can we.
- **Clinical positioning needs more.** The radiotherapy use case (see [`docs/research-kvm-landscape.md`](../research-kvm-landscape.md)) may require formal cert management. That is a Phase 4+ concern; ADR 0002 covers the personal-machines case and is explicit that clinical deployments need a separate decision.

## Alternatives considered

### Run an internal CA per user

Mature, well-understood. Rejected because:
- Generating, rotating, and storing the CA key is more configuration than the rest of the daemon combined.
- The CA key becomes a single point of compromise.
- For 2–6 personal peers, pinning has the same security with less infrastructure.

### Pre-shared symmetric keys (the RKVM path)

Simple, no PKI. Rejected because:
- A single shared key means revoking one machine revokes all of them.
- TLS 1.3 with self-signed certs gives forward secrecy and per-session keys for free.
- Defeats the per-peer identity story (`MachineId`).

### Sigstore / public transparency log

Trendy, interesting. Rejected because:
- Requires every machine to have outbound internet to verify, defeating the offline-LAN use case.
- The threat model — paired personal devices — does not benefit from a public log.

### "TLS optional" with a `--insecure` flag

Standard practice in many projects. Rejected because:
- Insecure flags become the default in tutorials, then production. See "optional TLS" in Synergy/Barrier/Deskflow.
- ADR 0001 already forbids it. Repeating the prohibition here.

## File and config locations

| Path                                        | Contents                                          |
|---------------------------------------------|---------------------------------------------------|
| `$XDG_CONFIG_HOME/cross-control/cross-control.crt` | This machine's TLS cert (PEM)                |
| `$XDG_CONFIG_HOME/cross-control/cross-control.key` | This machine's TLS key (PEM, 0600)           |
| `$XDG_CONFIG_HOME/cross-control/machine-id`        | UUID v4 string                               |
| `$XDG_CONFIG_HOME/cross-control/known_peers`       | (future) line-per-peer pinned fingerprints   |
| `[[screens]] fingerprint = "..."` in `config.toml` | Pre-shared fingerprint for a specific peer   |

The `known_peers` file does not exist yet — the current implementation pins via the per-screen `fingerprint` field in the config. A `known_peers` file with auto-populated entries is a Phase 2 polish item.

## Revisit triggers

- A practical attack on TLS 1.3 with self-signed Ed25519 certs.
- A user-facing UX study showing the SSH-style prompt is materially unsafe in our context.
- The clinical positioning becomes a primary use case and requires CA-based identity for compliance.

None likely in the v1.x series.
