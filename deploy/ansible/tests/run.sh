#!/usr/bin/env bash
# End-to-end test of the cross_control role:
#
#   1. deploy two machines ("left", "right") on this host;
#   2. deploy again and require that nothing changes (idempotence);
#   3. start both daemons (headless input backend) and require that they
#      connect to each other — i.e. trust was distributed with no pairing;
#   4. drop "right" from the play and require that "left" stops trusting it.
#
# Needs ansible-playbook on PATH (pip install ansible-core) and a built
# binary: CC_BINARY=path/to/cross-control, default target/debug/cross-control.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
export CC_BINARY="${CC_BINARY:-$repo/target/debug/cross-control}"
export CC_TEST_ROOT="${CC_TEST_ROOT:-$(mktemp -d)}"
export ANSIBLE_CONFIG="$here/../ansible.cfg"

pids=()
cleanup() {
  for pid in "${pids[@]:-}"; do
    [ -n "$pid" ] && kill "$pid" 2>/dev/null || true
  done
  rm -rf "$CC_TEST_ROOT"
}
trap cleanup EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }
[ -x "$CC_BINARY" ] || fail "no binary at $CC_BINARY (cargo build -p cross-control-cli)"

playbook() {
  ansible-playbook -i "$here/inventory.yml" "$here/test.yml" "$@"
}

# cc HOST ARGS... — run the deployed binary as HOST.
cc() {
  local host="$1"; shift
  XDG_CONFIG_HOME="$CC_TEST_ROOT/$host/config" XDG_RUNTIME_DIR="$CC_TEST_ROOT/$host/run" \
    "$CC_TEST_ROOT/$host/bin/cross-control" "$@"
}

echo "== 1. first deployment"
playbook

echo "== 2. second deployment must change nothing"
recap="$(playbook | tee /dev/stderr | sed -n '/PLAY RECAP/,$p')"
echo "$recap" | grep -Eq 'left .*changed=0 ' || fail "left changed on re-run"
echo "$recap" | grep -Eq 'right .*changed=0 ' || fail "right changed on re-run"

echo "== 3. daemons connect using the distributed trust"
for host in left right; do
  fp="$(cc "$host" identity --json | python3 -c 'import json,sys; print(json.load(sys.stdin)["fingerprint"])')"
  other=$([ "$host" = left ] && echo right || echo left)
  grep -q "^$fp $host\$" "$CC_TEST_ROOT/$other/config/cross-control/known_peers" \
    || fail "$other does not trust $host's fingerprint"
  mkdir -p "$CC_TEST_ROOT/$host/run"
  cc "$host" start >"$CC_TEST_ROOT/$host.log" 2>&1 &
  pids+=("$!")
done

peers() {
  cc "$1" status --json 2>/dev/null \
    | python3 -c 'import json,sys; print(" ".join(p["name"] for p in json.load(sys.stdin)["peers"]))' \
    || true
}
connected=""
for _ in $(seq 1 60); do
  if [ "$(peers left)" = right ] && [ "$(peers right)" = left ]; then
    connected=yes
    break
  fi
  sleep 0.5
done
if [ -z "$connected" ]; then
  tail -n 20 "$CC_TEST_ROOT"/left.log "$CC_TEST_ROOT"/right.log >&2 || true
  fail "daemons did not connect (left sees: '$(peers left)', right sees: '$(peers right)')"
fi
echo "left <-> right connected"

echo "== 4. removing a host from the play revokes its trust"
playbook --limit left -e '{"cross_control_trust_hosts": ["left"]}' >/dev/null
if grep -q " right\$" "$CC_TEST_ROOT/left/config/cross-control/known_peers"; then
  fail "left still trusts right after it was removed"
fi

echo "PASS"
