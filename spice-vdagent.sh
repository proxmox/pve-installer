#!/bin/sh

[ -e '/dev/virtio-ports/com.redhat.spice.0' ] || exit 0

mkdir -p /var/run/spice-vdagentd
/sbin/spice-vdagentd -X
/bin/spice-vdagent

# TODO: make installer more responsive, then we can enable auto resize
#primary=$(xrandr | awk '/connected primary/ {print $1}')
#while true; do
#    xrandr --output "$primary" --auto
#    xsetroot -solid grey
#    sleep 0.5
#done
