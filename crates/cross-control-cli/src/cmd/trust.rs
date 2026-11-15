//! `trust list | add | remove`.

use cross_control_daemon::trust::{add_known_peer, remove_known_peer, EntrySource};
use serde_json::json;

use crate::exit;
use crate::{Ctx, TrustAction};

pub fn run(ctx: &Ctx, action: TrustAction) -> anyhow::Result<u8> {
    let config = ctx.config()?;
    let store = super::trust_store(&config);
    match action {
        TrustAction::List => {
            let (name, fingerprint) = super::local_identity(&config)?;
            let entries = store.entries()?;
            let source = |s: EntrySource| match s {
                EntrySource::Config => "config",
                EntrySource::KnownPeers => "known_peers",
            };
            if ctx.json {
                Ctx::print_json(&json!({
                    "self": { "name": name, "fingerprint": fingerprint },
                    "trusted": entries.iter().map(|e| json!({
                        "name": e.name,
                        "fingerprint": e.fingerprint,
                        "source": source(e.source),
                    })).collect::<Vec<_>>(),
                    "trust_on_first_use": config.security.trust_on_first_use,
                    "known_peers": store.known_peers_path(),
                }));
                return Ok(exit::OK);
            }
            println!("This machine: {name}");
            println!("  {fingerprint}");
            println!();
            if entries.is_empty() {
                println!("No trusted machines. Use `cross-control pair <address>` or");
                println!("`cross-control trust add <name> <fingerprint>`.");
            } else {
                println!("Trusted machines:");
                for entry in entries {
                    println!(
                        "  {:<20} {}  ({})",
                        entry.name,
                        entry.fingerprint,
                        source(entry.source)
                    );
                }
            }
            if config.security.trust_on_first_use {
                println!();
                println!(
                    "Warning: security.trust_on_first_use is on; unknown machines are trusted"
                );
                println!("automatically on first contact.");
            }
        }
        TrustAction::Add { name, fingerprint } => {
            let changed = add_known_peer(store.known_peers_path(), &name, &fingerprint)?;
            if ctx.json {
                Ctx::print_json(&json!({ "changed": changed, "name": name }));
            } else if changed {
                println!("Trusted \"{name}\" ({fingerprint}).");
                println!("A running daemon picks this up on the peer's next connection.");
            } else {
                println!("\"{name}\" is already trusted with that fingerprint.");
            }
        }
        TrustAction::Remove { name } => {
            let changed = remove_known_peer(store.known_peers_path(), &name)?;
            let pinned = config
                .screens
                .iter()
                .any(|s| s.name == name && s.fingerprint.is_some());
            if ctx.json {
                Ctx::print_json(&json!({
                    "changed": changed,
                    "name": name,
                    "pinned_in_config": pinned,
                }));
            } else if changed {
                println!("No longer trusting \"{name}\".");
                println!("Restart the daemon to drop an existing connection.");
            } else {
                println!(
                    "\"{name}\" is not in {}.",
                    store.known_peers_path().display()
                );
                if pinned {
                    println!("It is pinned in config.toml; remove its `fingerprint` there.");
                }
            }
        }
    }
    Ok(exit::OK)
}
