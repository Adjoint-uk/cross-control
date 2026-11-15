//! `pair`: interactive first contact with another machine.

use std::net::SocketAddr;

use cross_control_daemon::session::PeerSession;
use cross_control_daemon::setup;
use cross_control_daemon::trust::add_known_peer;
use serde_json::json;

use crate::exit;
use crate::Ctx;

/// Ask the user to confirm a fingerprint, ssh-style. Without a terminal the
/// answer is no: pass `--yes` to pair non-interactively.
fn confirm(prompt: &str) -> anyhow::Result<bool> {
    use std::io::{BufRead, IsTerminal, Write};
    if !std::io::stdin().is_terminal() {
        anyhow::bail!("not a terminal; re-run with --yes to trust the fingerprint shown above");
    }
    print!("{prompt} [y/N] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes" | "YES" | "Yes"))
}

/// Connect to a peer, learn its name and TLS fingerprint, confirm it with
/// the user, record it in `known_peers`, and print what to run on the other
/// machine so it trusts us back.
pub async fn run(ctx: &Ctx, address: &str, name: Option<String>, yes: bool) -> anyhow::Result<u8> {
    let config = ctx.config()?;
    let config_dir = setup::config_dir();
    let (cert_pem, key_pem) = setup::load_identity_certs(&config, &config_dir)?;
    let machine_id = setup::load_or_create_machine_id(&config_dir)?;
    let (our_name, our_fingerprint) = super::local_identity(&config)?;
    let our_layout = config.display_layout();
    // With --json, stdout carries only the final document.
    let say = |line: &str| {
        if !ctx.json {
            println!("{line}");
        }
    };

    // Accept "host" or "host:port"; default to the standard port.
    let addr: SocketAddr = address
        .parse()
        .or_else(|_| format!("{address}:24800").parse())
        .map_err(|e| anyhow::anyhow!("invalid address '{address}': {e}"))?;

    // Bind an ephemeral endpoint just for this pairing exchange.
    let bind: SocketAddr = "0.0.0.0:0".parse()?;
    let transport = cross_control_protocol::QuicTransport::bind(bind, &cert_pem, &key_pem)?;

    say(&format!("Connecting to {addr} …"));
    // No pin yet: accept whatever certificate is presented (the peer still
    // has to prove it holds the key) and show its fingerprint for the user
    // to verify out of band.
    let conn = transport
        .connect_pinned(addr, "cross-control", None)
        .await?;
    let fingerprint = conn
        .peer_fingerprint()
        .ok_or_else(|| anyhow::anyhow!("peer presented no TLS certificate"))?;

    // Handshake to learn the peer's name. A peer that does not trust us yet
    // refuses before saying who it is; then the user must supply the name.
    let (control_tx, control_rx) = conn.open_control_stream().await?;
    let mut session = PeerSession::new(conn, control_tx, control_rx);
    let announced = match session
        .handshake_initiator(machine_id, &our_name, &our_layout, |_| Ok(()))
        .await
    {
        Ok(()) => Some(session.name.clone()),
        Err(e) => {
            tracing::debug!(error = %e, "peer refused the handshake");
            None
        }
    };
    let _ = session.disconnect();

    let trust_us = format!("cross-control trust add \"{our_name}\" {our_fingerprint}");
    let peer_name = match (name, announced) {
        (Some(given), Some(announced)) if given != announced => {
            anyhow::bail!("the machine at {addr} calls itself \"{announced}\", not \"{given}\"")
        }
        (Some(given), _) => given,
        (None, Some(announced)) => announced,
        (None, None) => anyhow::bail!(
            "the machine at {addr} ({fingerprint}) does not trust this one yet, so it did not \
             say its name.\nEither run this on it first:\n\n  {trust_us}\n\nor re-run with \
             --name <its identity.name>."
        ),
    };

    say("");
    say(&format!("Machine \"{peer_name}\" at {addr}"));
    say(&format!("  Fingerprint: {fingerprint}"));
    say("");
    say(&format!(
        "Check that this matches `cross-control identity` on \"{peer_name}\"."
    ));
    if !yes && !confirm("Trust this machine?")? {
        anyhow::bail!("not trusted");
    }

    let store = super::trust_store(&config);
    let changed = add_known_peer(store.known_peers_path(), &peer_name, &fingerprint)?;
    if ctx.json {
        Ctx::print_json(&json!({
            "changed": changed,
            "name": peer_name,
            "fingerprint": fingerprint,
            "address": addr.to_string(),
            "trust_us": trust_us,
        }));
        return Ok(exit::OK);
    }
    println!(
        "Trusted \"{peer_name}\" (saved to {}).",
        store.known_peers_path().display()
    );
    println!();
    println!("On \"{peer_name}\", trust this machine back:");
    println!();
    println!("  {trust_us}");
    println!();
    println!(
        "Then place it in your layout in {}:",
        ctx.config_file().display()
    );
    println!();
    println!("  [[screens]]");
    println!("  name = \"{peer_name}\"");
    println!("  address = \"{addr}\"");
    println!("  position = \"Right\"   # Left / Right / Above / Below");
    Ok(exit::OK)
}
