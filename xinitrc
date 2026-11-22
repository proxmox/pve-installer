#!/bin/sh

/usr/bin/xrdb -load /.Xdefaults
/usr/bin/X11/xsetroot -solid grey
openbox --config-file /var/lib/proxmox-installer/openbox-rc.xml &
/.spice-vdagent.sh &
/usr/bin/proxinstall
