#!/usr/bin/env bash
# End-to-end test of `cross-control service` on macOS: install the launchd
# agent, check launchd starts the daemon, check an unchanged reinstall is a
# no-op, stop it cleanly (launchd must not restart it), and uninstall.
#
# Uses private config/runtime directories and the headless input backend, so
# it touches nothing else but the agent's plist. CI runs it on the macOS runner:
#
#   deploy/macos/tests/launch-agent.sh target/debug
set -euo pipefail

bin=${1:-target/debug}
cc="$(cd "$bin" && pwd)/cross-control"
# Short paths: macOS limits Unix socket paths to 104 bytes.
root=$(mktemp -d /tmp/cc-la.XXXX)
export CROSS_CONTROL_CONFIG_DIR="$root/config"
export CROSS_CONTROL_RUNTIME_DIR="$root/run"
mkdir -p "$CROSS_CONTROL_CONFIG_DIR" "$CROSS_CONTROL_RUNTIME_DIR"
cat >"$CROSS_CONTROL_CONFIG_DIR/config.toml" <<'TOML'
[identity]
name = "launch-agent-test"
[daemon]
bind = "127.0.0.1"
port = 0
discovery = false
[input]
backend = "none"
[clipboard]
enabled = false
TOML

label=uk.adjoint.cross-control
service="gui/$(id -u)/$label"
plist="$HOME/Library/LaunchAgents/$label.plist"

fail() {
	echo "FAIL: $*" >&2
	exit 1
}

# json <field> <document>: print one top-level field of a JSON document.
json() { python3 -c 'import json,sys; print(json.loads(sys.argv[2]).get(sys.argv[1]))' "$1" "$2"; }

wait_until() {
	local what=$1 seconds=$2
	shift 2
	for _ in $(seq $((seconds * 4))); do
		if "$@"; then return 0; fi
		sleep 0.25
	done
	fail "timed out waiting for: $what"
}

running() { "$cc" status >/dev/null 2>&1; }
stopped() { ! running; }

cleanup() {
	local code=$?
	if [ "$code" -ne 0 ]; then
		echo "--- launchctl print ---"
		launchctl print "$service" 2>&1 || true
		echo "--- daemon log ---"
		cat "$CROSS_CONTROL_RUNTIME_DIR/cross-control.log" 2>/dev/null || true
	fi
	"$cc" service uninstall >/dev/null 2>&1 || true
	rm -rf "$root"
	exit "$code"
}
trap cleanup EXIT

out=$("$cc" service install --enable --json)
[ "$(json changed "$out")" = True ] || fail "first install reported no change: $out"
[ -f "$plist" ] || fail "no plist at $plist"
plutil -lint "$plist" >/dev/null || fail "invalid plist"

wait_until "the daemon to start" 30 running
pid=$(json pid "$("$cc" status --json)")
echo "daemon running as PID $pid"
launchctl print "$service" | grep -q "pid = $pid" || fail "launchd does not own PID $pid"

out=$("$cc" service install --enable --json)
[ "$(json changed "$out")" = False ] || fail "reinstalling an unchanged agent reported a change: $out"
running || fail "an unchanged reinstall stopped the daemon"
[ "$(json pid "$("$cc" status --json)")" = "$pid" ] || fail "an unchanged reinstall restarted the daemon"

out=$("$cc" stop --json)
[ "$(json graceful "$out")" = True ] || fail "stop was not graceful: $out"
wait_until "the daemon to stop" 15 stopped
# KeepAlive restarts only on failure: a clean stop must stay stopped.
sleep 7
stopped || fail "launchd restarted the daemon after a clean stop"

out=$("$cc" service uninstall --json)
[ "$(json changed "$out")" = True ] || fail "uninstall reported no change: $out"
[ ! -e "$plist" ] || fail "the plist is still there"
if launchctl print "$service" >/dev/null 2>&1; then fail "the agent is still loaded"; fi

echo "launch agent end-to-end: ok"
