# Automation and configuration management

Everything `cross-control` does can be scripted, so a fleet can be set up
with Ansible, Salt, Puppet, a shell script or anything else that can copy
files and run commands. No interactive pairing is needed: the deployment
tool collects each machine's fingerprint and distributes it to the others.

## The model

Each machine has three pieces of state:

| What | Where (default) | Managed by |
|---|---|---|
| Identity: certificate, key, machine id | `~/.config/cross-control/` | Created by `cross-control identity` on first run, or supplied via `[identity] cert`/`key` |
| Layout | `~/.config/cross-control/config.toml` | Templated by you |
| Trust | `~/.config/cross-control/known_peers` | `cross-control trust add`, or templated |

A deployment is therefore:

1. Install the binary and run `cross-control identity --json` on every host.
   Collect `name` and `fingerprint` from each.
2. On every host, render `config.toml` (layout) and add every *other*
   host's fingerprint with `cross-control trust add NAME FINGERPRINT`
   (or template `known_peers` directly).
3. Validate with `cross-control config check`, then install and start the
   service with `cross-control service install --enable`.

Every step is idempotent: re-running it changes nothing, and reports that.

### Directory overrides

`CROSS_CONTROL_CONFIG_DIR` and `CROSS_CONTROL_RUNTIME_DIR` replace the
configuration directory (identity, `config.toml`, `known_peers`) and the
runtime directory (PID and status files) on every platform. `service
install` copies them into the generated unit, so the service uses the same
identity and trust store that was configured.

## Command reference for automation

All commands accept `--config PATH` and `--json`. With `--json`, stdout is
exactly one JSON document; logs go to stderr.

### `identity`

Prints this machine's identity, creating it on first use.

```json
{
  "name": "workstation",
  "fingerprint": "SHA256:ab:cd:…",
  "machine_id": "0b0e…",
  "created": false,
  "cert": "/home/me/.config/cross-control/cross-control.crt",
  "config": "/home/me/.config/cross-control/config.toml",
  "known_peers": "/home/me/.config/cross-control/known_peers"
}
```

`created` is `true` only on the run that generated the certificate or
machine id; use it as Ansible's `changed_when`.

To use certificates from your own PKI instead, set both:

```toml
[identity]
name = "workstation"
cert = "/etc/pki/cross-control/workstation.crt"
key  = "/etc/pki/cross-control/workstation.key"
```

### `trust add NAME FINGERPRINT` / `trust remove NAME` / `trust list`

`add` and `remove` print `{"changed": true|false, …}`. Adding an identical
entry is a no-op. `list` prints this machine's identity and every trusted
peer with its source (`config` or `known_peers`). A running daemon reads
`known_peers` on each connection, and peers with a configured `address` are
redialed automatically, so no restart is needed.

### `config check [PATH]`

Validates a configuration file without starting anything. Exit code 4 if it
is invalid or missing; suitable for Ansible's `template: validate:`
(`cross-control config check %s`).

### `doctor`

Checks that the machine is ready (input devices and permissions, uinput,
config, identity, trust versus layout, UDP port, clipboard) and prints a fix
for each problem. `--json` gives `{"ok": bool, "checks": [...]}`; exit code
1 if any check failed (warnings pass). Useful as a post-deployment assertion.

### `status`

Exit code 0 when the daemon is running, 3 when it is not (the LSB
convention), so it doubles as a health check. `--json` adds peers, latency
and focus.

### `stop`

Asks the running daemon to shut down over its local control channel (a
Unix socket in the runtime directory, or a named pipe on Windows), then
waits until the process has exited. `--json` reports
`{"stopped": PID, "graceful": true}`. If the daemon does not answer, the
process is terminated instead (`"graceful": false`). Exit code 3 if the
daemon is not running.

The control channel is also the single-instance lock: a second `start` for
the same runtime directory fails with "already running".

### `service unit` / `service install [--enable]` / `service uninstall`

Starts the daemon at login, running *this* binary (with `--config`, if
given, and any directory overrides):

- **Linux:** a systemd user unit. `install` writes it to
  `~/.config/systemd/user/` and reloads systemd only if it changed
  (`{"changed": …}`); `--enable` also runs `systemctl --user enable --now`.
- **Windows:** a Task Scheduler logon task running `cross-controlw.exe` in
  the user's session; see [Windows](windows.md#start-at-logon). `install`
  registers it only if it changed; `--enable` also starts it.

`service unit` prints the definition (unit file, or task XML), so you can
template it yourself instead.

## Headless smoke tests

`[input] backend = "none"` runs the daemon without touching input devices:
it still discovers, authenticates and keeps sessions with its peers, so a
deployment can be verified in containers or CI where no keyboard, mouse or
`/dev/uinput` exists. `cross-control status --json` then lists the connected
peers.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | error |
| 2 | invalid command line |
| 3 | daemon not running (`status`, `stop`) |
| 4 | invalid or missing configuration |
| 5 | peer not authorized |

## Ansible role

A ready-made role lives in [`deploy/ansible`](../deploy/ansible): it
installs the binary, creates identities, renders the layout and an
authoritative `known_peers` from the other hosts' fingerprints, and manages
the service. It is tested end to end in CI (two machines deployed, re-run
idempotently, connected, one removed). See its
[README](../deploy/ansible/roles/cross_control/README.md).

## Ansible sketch

The role boils down to roughly this:

```yaml
- name: Collect identities
  hosts: kvm
  tasks:
    - name: Ensure identity
      ansible.builtin.command: cross-control identity --json
      register: cc_identity
      changed_when: (cc_identity.stdout | from_json).created

    - ansible.builtin.set_fact:
        cc: "{{ cc_identity.stdout | from_json }}"

- name: Configure layout and trust
  hosts: kvm
  tasks:
    - name: Layout
      ansible.builtin.template:
        src: config.toml.j2
        dest: ~/.config/cross-control/config.toml
        validate: cross-control config check %s
      notify: restart cross-control

    - name: Trust every other machine
      ansible.builtin.command: >-
        cross-control trust add --json
        {{ hostvars[item].cc.name | quote }} {{ hostvars[item].cc.fingerprint }}
      loop: "{{ groups['kvm'] | difference([inventory_hostname]) }}"
      register: cc_trust
      changed_when: (cc_trust.stdout | from_json).changed

    - name: Service
      ansible.builtin.command: cross-control service install --enable --json
      register: cc_service
      changed_when: (cc_service.stdout | from_json).changed

  handlers:
    - name: restart cross-control
      ansible.builtin.systemd:
        name: cross-control
        scope: user
        state: restarted
```

Removing a host from the inventory and re-running stops it being trusted
only if you also remove it (`cross-control trust remove`) or template
`known_peers` instead of adding entries.
