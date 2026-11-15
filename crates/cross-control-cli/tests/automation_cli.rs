//! The CLI as configuration management drives it: `--json` output, exit
//! codes, and idempotent commands. Each test runs the real binary against a
//! private config/runtime directory.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

struct Home(PathBuf);

impl Home {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "cc-auto-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("config")).unwrap();
        std::fs::create_dir_all(dir.join("run")).unwrap();
        Self(dir)
    }

    fn config_dir(&self) -> PathBuf {
        self.0.join("config/cross-control")
    }

    /// Run the binary; returns (exit code, stdout, stderr).
    fn run(&self, args: &[&str]) -> (i32, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_cross-control"))
            .args(args)
            .env("CROSS_CONTROL_CONFIG_DIR", self.config_dir())
            .env("CROSS_CONTROL_RUNTIME_DIR", self.0.join("run"))
            .env_remove("RUST_LOG")
            .output()
            .expect("run cross-control");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    /// Run with `--json`, expecting `code`; returns the parsed document.
    fn json(&self, args: &[&str], code: i32) -> Value {
        let mut full = args.to_vec();
        full.push("--json");
        let (got, stdout, stderr) = self.run(&full);
        assert_eq!(got, code, "{args:?}: stdout={stdout} stderr={stderr}");
        serde_json::from_str(&stdout)
            .unwrap_or_else(|e| panic!("{args:?}: stdout is not JSON ({e}): {stdout}"))
    }

    fn write(&self, rel: &str, text: &str) -> PathBuf {
        let path = self.0.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        path
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn s(path: &Path) -> &str {
    path.to_str().unwrap()
}

#[test]
fn identity_is_created_once_and_stable() {
    let home = Home::new("identity");
    let first = home.json(&["identity"], 0);
    assert_eq!(first["created"], true);
    let fp = first["fingerprint"].as_str().unwrap().to_string();
    assert!(fp.starts_with("SHA256:"), "{fp}");
    assert!(home.config_dir().join("cross-control.key").exists());

    let second = home.json(&["identity"], 0);
    assert_eq!(
        second["created"], false,
        "second run must not create anything"
    );
    assert_eq!(second["fingerprint"], fp.as_str());
    assert_eq!(second["machine_id"], first["machine_id"]);
}

#[test]
fn identity_uses_configured_certificate() {
    let home = Home::new("pki");
    let cert = cross_control_certgen::generate_certificate("issued").unwrap();
    let cert_path = home.write("pki/host.crt", &cert.cert_pem);
    let key_path = home.write("pki/host.key", &cert.key_pem);
    let config = home.write(
        "custom.toml",
        &format!(
            // Literal strings: Windows paths contain backslashes.
            "[identity]\nname = \"desk\"\ncert = '{}'\nkey = '{}'\n",
            s(&cert_path),
            s(&key_path)
        ),
    );

    let id = home.json(&["identity", "--config", s(&config)], 0);
    assert_eq!(id["name"], "desk");
    assert_eq!(id["fingerprint"], cert.fingerprint.as_str());
    assert!(
        !home.config_dir().join("cross-control.crt").exists(),
        "must not generate a certificate when one is configured"
    );
}

#[test]
fn config_check_gates_invalid_files() {
    let home = Home::new("check");
    let good = home.write(
        "good.toml",
        "[identity]\nname = \"a\"\n\n[[screens]]\nname = \"b\"\nposition = \"Right\"\n",
    );
    let doc = home.json(&["config", "check", s(&good)], 0);
    assert_eq!(doc["valid"], true);
    assert_eq!(doc["screens"], 1);

    // Two neighbours on the same edge: invalid layout.
    let bad = home.write(
        "bad.toml",
        "[identity]\nname = \"a\"\n\n[[screens]]\nname = \"b\"\nposition = \"Right\"\n\n\
         [[screens]]\nname = \"c\"\nposition = \"Right\"\n",
    );
    let (code, _, stderr) = home.run(&["config", "check", s(&bad)]);
    assert_eq!(code, 4, "{stderr}");
    assert!(stderr.contains("Right"), "{stderr}");

    let (code, _, _) = home.run(&["config", "check", "/no/such/file.toml"]);
    assert_eq!(code, 4, "a missing explicit config is a config error");

    // Half-configured certificate.
    let half = home.write("half.toml", "[identity]\nname = \"a\"\ncert = \"/x.crt\"\n");
    let (code, _, _) = home.run(&["config", "check", s(&half)]);
    assert_eq!(code, 4);
}

#[test]
fn explicit_missing_config_refuses_to_run() {
    let home = Home::new("missing");
    let (code, _, stderr) = home.run(&["identity", "--config", "/no/such/config.toml"]);
    assert_eq!(code, 4, "{stderr}");
    assert!(stderr.contains("not found"), "{stderr}");
}

#[test]
fn trust_add_and_remove_report_changes() {
    let home = Home::new("trust");
    let peer = cross_control_certgen::generate_certificate("peer").unwrap();
    let fp = peer.fingerprint.as_str();

    assert_eq!(
        home.json(&["trust", "add", "laptop", fp], 0)["changed"],
        true
    );
    assert_eq!(
        home.json(&["trust", "add", "laptop", fp], 0)["changed"],
        false,
        "re-adding the same entry must be a no-op"
    );

    let list = home.json(&["trust", "list"], 0);
    let trusted = list["trusted"].as_array().unwrap();
    assert_eq!(trusted.len(), 1);
    assert_eq!(trusted[0]["name"], "laptop");
    assert_eq!(trusted[0]["fingerprint"], fp);
    assert_eq!(trusted[0]["source"], "known_peers");
    assert!(list["self"]["fingerprint"]
        .as_str()
        .unwrap()
        .starts_with("SHA256:"));

    assert_eq!(
        home.json(&["trust", "remove", "laptop"], 0)["changed"],
        true
    );
    assert_eq!(
        home.json(&["trust", "remove", "laptop"], 0)["changed"],
        false
    );
    assert_eq!(
        home.json(&["trust", "list"], 0)["trusted"],
        Value::Array(vec![])
    );

    let (code, _, _) = home.run(&["trust", "add", "laptop", "SHA256:nope"]);
    assert_eq!(code, 1, "an invalid fingerprint is an error");
}

#[test]
fn status_and_stop_report_not_running() {
    let home = Home::new("status");
    let doc = home.json(&["status"], 3);
    assert_eq!(doc["running"], false);
    assert_eq!(doc["peers"], Value::Array(vec![]));

    let (code, _, _) = home.run(&["status"]);
    assert_eq!(code, 3, "plain status follows the same exit code");

    let (code, _, stderr) = home.run(&["stop"]);
    assert_eq!(code, 3, "{stderr}");
}

// systemd units are Linux-only.
#[cfg(unix)]
#[test]
fn service_unit_runs_this_binary_with_config() {
    let home = Home::new("service");
    let config = home.write("etc/cc.toml", "[identity]\nname = \"a\"\n");
    let (code, unit, stderr) = home.run(&["service", "unit", "--config", s(&config)]);
    assert_eq!(code, 0, "{stderr}");
    let exe = std::fs::canonicalize(env!("CARGO_BIN_EXE_cross-control")).unwrap();
    let exec_line = unit
        .lines()
        .find(|l| l.starts_with("ExecStart="))
        .expect("unit has ExecStart");
    assert!(exec_line.contains(s(&exe)), "{exec_line}");
    assert!(exec_line.contains("--config"), "{exec_line}");
    assert!(exec_line.contains(s(&std::fs::canonicalize(&config).unwrap())));
}

#[test]
fn json_output_is_not_polluted_by_logs() {
    // Logs go to stderr; stdout must be exactly one JSON document even with
    // verbose logging.
    let home = Home::new("logs");
    let out = Command::new(env!("CARGO_BIN_EXE_cross-control"))
        .args(["identity", "--json"])
        .env("CROSS_CONTROL_CONFIG_DIR", home.config_dir())
        .env("CROSS_CONTROL_RUNTIME_DIR", home.0.join("run"))
        .env("RUST_LOG", "debug")
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str::<Value>(&stdout).expect("stdout is pure JSON");
}

#[test]
fn doctor_reports_checks_and_fails_only_on_failures() {
    let home = Home::new("doctor");
    let peer = cross_control_certgen::generate_certificate("peer").unwrap();
    let config = home.write(
        "doctor.toml",
        "[identity]\nname = \"a\"\n[daemon]\nport = 0\n[input]\nbackend = \"none\"\n\
         [[screens]]\nname = \"b\"\nposition = \"Right\"\n",
    );

    // Neighbour "b" is not trusted yet: a warning, not a failure.
    let doc = home.json(&["doctor", "--config", s(&config)], 0);
    assert_eq!(doc["ok"], true);
    let checks = doc["checks"].as_array().unwrap();
    assert!(checks
        .iter()
        .any(|c| c["name"] == "trust" && c["status"] == "warn" && c["fix"].is_string()));

    home.run(&[
        "trust",
        "add",
        "b",
        &peer.fingerprint,
        "--config",
        s(&config),
    ]);
    let doc = home.json(&["doctor", "--config", s(&config)], 0);
    assert!(doc["checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["name"] == "trust" && c["status"] == "ok"));

    // An invalid config is a failure: exit 1, ok = false.
    let bad = home.write("bad.toml", "[identity\n");
    let doc = home.json(&["doctor", "--config", s(&bad)], 1);
    assert_eq!(doc["ok"], false);
}
