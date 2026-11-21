#!/bin/sh
# cross-control package: make /dev/uinput usable now, without a reboot.
modprobe uinput 2>/dev/null || true
if command -v udevadm >/dev/null 2>&1; then
    udevadm control --reload-rules 2>/dev/null || true
    udevadm trigger --sysname-match=uinput 2>/dev/null || true
fi
cat <<'MSG'
cross-control is installed. To share or receive input, your user needs to be
in the input group (then log out and back in):

    sudo usermod -aG input "$USER"

Then run `cross-control doctor`, and `cross-control service install --enable`
to start it at login.
MSG
exit 0
