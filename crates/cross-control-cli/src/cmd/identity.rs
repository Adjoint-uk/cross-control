//! `identity` and `generate-cert`.

use cross_control_daemon::setup;
use serde_json::json;

use crate::exit;
use crate::Ctx;

/// Show this machine's identity, creating it on first use. Safe to run
/// repeatedly: `created` (in `--json`) is true only on the run that
/// generated something.
pub fn show(ctx: &Ctx) -> anyhow::Result<u8> {
    let config = ctx.config()?;
    let dir = setup::config_dir();
    let cert_path = setup::identity_cert_path(&config, &dir);
    let id_path = dir.join("machine-id");
    let created = !cert_path.exists() || !id_path.exists();

    let (cert_pem, _) = setup::load_identity_certs(&config, &dir)?;
    let machine_id = setup::load_or_create_machine_id(&dir)?;
    let fingerprint = cross_control_certgen::fingerprint_from_pem(&cert_pem)?;
    let known_peers = super::trust_store(&config).known_peers_path().to_path_buf();

    if ctx.json {
        Ctx::print_json(&json!({
            "name": config.identity.name,
            "fingerprint": fingerprint,
            "machine_id": machine_id.as_uuid().to_string(),
            "created": created,
            "cert": cert_path,
            "config": ctx.config_file(),
            "known_peers": known_peers,
        }));
    } else {
        println!("Name:        {}", config.identity.name);
        println!("Fingerprint: {fingerprint}");
        println!("Machine id:  {}", machine_id.as_uuid());
        println!("Certificate: {}", cert_path.display());
        if created {
            println!("(identity created)");
        }
    }
    Ok(exit::OK)
}

/// Write a new certificate and key to `output` (an explicit, standalone
/// alternative to the identity created automatically).
pub fn generate_cert(ctx: &Ctx, output: &str) -> anyhow::Result<u8> {
    let hostname = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "cross-control".to_string());
    let cert = cross_control_certgen::generate_certificate(&hostname)?;

    let cert_path = std::path::Path::new(output).join("cross-control.crt");
    let key_path = std::path::Path::new(output).join("cross-control.key");
    setup::write_private_file(&key_path, cert.key_pem.as_bytes())?;
    std::fs::write(&cert_path, &cert.cert_pem)?;

    if ctx.json {
        Ctx::print_json(&json!({
            "cert": cert_path,
            "key": key_path,
            "fingerprint": cert.fingerprint,
        }));
    } else {
        println!("Certificate: {}", cert_path.display());
        println!("Private key: {}", key_path.display());
        println!("Fingerprint: {}", cert.fingerprint);
    }
    Ok(exit::OK)
}
