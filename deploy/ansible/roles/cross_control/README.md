# Ansible role: `cross_control`

Deploys [cross-control](https://github.com/Adjoint-uk/cross-control) to a
group of machines and makes them trust each other — with no interactive
pairing. Each host creates its own identity; the role collects every host's
certificate fingerprint and gives each host the others'.

## What it does

1. Optionally copies the `cross-control` binary to the host.
2. Runs `cross-control identity` (creating the certificate, key and machine
   id on first run only) and publishes the fingerprint as a fact.
3. Renders `config.toml` from variables, validated with
   `cross-control config check` before it replaces the old file.
4. Renders `known_peers` from the other hosts' fingerprints. It is
   authoritative: a host removed from the play stops being trusted on the
   next run. A running daemon re-reads it on each connection.
5. Installs and starts a systemd user service.

Re-running changes nothing unless the inputs changed.

## Example

```yaml
# inventory.yml
all:
  children:
    kvm:
      vars:
        cross_control_binary_src: ../../target/release/cross-control
      hosts:
        workstation:
          cross_control_screens:
            - { name: laptop, position: Right, address: "192.0.2.20:24800" }
        laptop:
          cross_control_screens:
            - { name: workstation, position: Left, address: "192.0.2.10:24800" }
```

```bash
ansible-playbook -i inventory.yml site.yml
```

## Variables

See [`defaults/main.yml`](defaults/main.yml) for all of them, with comments.
The ones you will usually set:

| Variable | Default | Purpose |
|---|---|---|
| `cross_control_screens` | `[]` | Machines next to this one (`name`, `position`, optional `address`) |
| `cross_control_screen_adjacency` | `[]` | Links between other machines (multi-hop) |
| `cross_control_monitors` | `[]` | This machine's monitors |
| `cross_control_binary_src` | `""` | Binary on the controller to install (empty: already installed) |
| `cross_control_name` | inventory hostname | The name peers use for this machine |
| `cross_control_trust_hosts` | all hosts in the play | Who this machine trusts |
| `cross_control_extra_trusted` | `[]` | Machines outside the play (`name`, `fingerprint`) |
| `cross_control_cert` / `cross_control_key` | `""` | Certificates from your own PKI |
| `cross_control_service` | `true` | Manage the systemd user service |
| `cross_control_input_backend` | `auto` | `none` runs headless (CI, containers) |

## Requirements

- Linux targets with systemd (for the service) and, for real input, the
  user in the `input` group with access to `/dev/uinput`.
- For the service to run without an active login, enable lingering:
  `loginctl enable-linger <user>`.
- All hosts that should trust each other in the same play (the role reads
  their facts).

## Testing

`tests/run.sh` deploys two machines on one host, checks the second run is a
no-op, starts both daemons and checks they connect, then removes one host and
checks it is no longer trusted. CI runs it on every pull request.

```bash
pip install ansible-core
cargo build -p cross-control-cli
deploy/ansible/tests/run.sh
```
