#!/bin/sh

/usr/bin/xrdb -load /.Xdefaults
/usr/bin/X11/xsetroot -solid grey
# Nothing that hides the installer: no iconify button, window menu or show desktop shortcut, as
# there is no taskbar to restore a minimized window. And one desktop, so that moving a dialog to the
# screen edge cannot switch to an empty one.
if sed -e '/<titleLayout>/s/[IN]//g' \
    -e '/<desktops>/,/<\/desktops>/s|<number>[0-9]*</number>|<number>1</number>|' \
    -e 's|<manageDesktops>yes</manageDesktops>|<manageDesktops>no</manageDesktops>|' \
    -e 's|<action name="ShowMenu"><menu>client-menu</menu></action>||' \
    -e 's|<action name="ToggleShowDesktop"/>||' \
    /etc/xdg/openbox/rc.xml >/run/proxmox-installer/openbox-rc.xml; then
    openbox --config-file /run/proxmox-installer/openbox-rc.xml &
else
    openbox &
fi
/.spice-vdagent.sh &
/usr/bin/proxinstall
