#!/bin/sh

/usr/bin/xrdb -load /.Xdefaults
/usr/bin/X11/xsetroot -solid grey
# no iconify button, as there is no taskbar to restore a minimized window
if sed '/<titleLayout>/s/I//' /etc/xdg/openbox/rc.xml >/run/proxmox-installer/openbox-rc.xml; then
    openbox --config-file /run/proxmox-installer/openbox-rc.xml &
else
    openbox &
fi
/.spice-vdagent.sh &
/usr/bin/proxinstall
