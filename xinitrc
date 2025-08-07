#!/bin/sh

/usr/bin/xrdb -load /.Xdefaults
/usr/bin/X11/xsetroot -solid grey
openbox &
/.spice-vdagent.sh &
/usr/bin/proxinstall
