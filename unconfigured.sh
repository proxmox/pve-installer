#!/bin/bash

trap "err_reboot" ERR

parse_cmdline() {
    root=
    proxdebug=0
    for par in $(cat /proc/cmdline); do 
	case $par in
	    root=*)
		root=${par#root=}
		;;
	    proxdebug)
		proxdebug=1
		;;
	esac
    done;
}

debugsh() {
    getty 38400 tty1 -i -n -l /bin/bash
}

real_reboot() {

    trap - ERR 

    /etc/init.d/networking stop 

    # stop udev (release file handles)
    /etc/init.d/udev stop

    echo -n "Deactivating swap..."
    swap=$(grep /dev /proc/swaps);
    if [ -n "$swap" ]; then
       set $swap
       swapoff $1
    fi
    echo "done."

    umount -l -n /etc/network/run

    umount -l -n /dev/shm
    umount -l -n /dev/.static/dev
    umount -l -n /dev

    umount -l -n /target >/dev/null 2>&1
    umount -l -n /tmp
    umount -l -n /var/tmp
    umount -l -n /var/log
    umount -l -n /var/run
    umount -l -n /var/lib/xkb

    umount -l -n /proc
    umount -l -n /sys

    exit 0
}

err_reboot() {

    echo "\nInstallation aborted - unable to continue (type exit or CTRL-D to reboot)"
    debugsh
    real_reboot
}

echo "Starting Proxmox installation"

PATH=/sbin:/bin:/usr/sbin:/usr/bin:/usr/X11R6/bin

mount -n -t proc proc /proc
mount -n -t tmpfs tmpfs /tmp
mount -n -t tmpfs tmpfs /var/tmp
mount -n -t tmpfs tmpfs /var/log
mount -n -t tmpfs tmpfs /var/run
mount -n -t tmpfs tmpfs /var/lib/xkb
mount -n -t sysfs sysfs /sys

parse_cmdline

# always load most common input drivers
modprobe -q psmouse || /bin/true
modprobe -q sermouse || /bin/true
modprobe -q usbhid || /bin/true

# load device mapper - used by lilo
modprobe -q dm_mod || /bin/true

echo "Installing additional hardware drivers"
export RUNLEVEL=S 
export PREVLEVEL=N
/etc/init.d/udev start

# used by if up down scripts
mount -t tmpfs none /dev/shm
mkdir /dev/shm/network
mount -t tmpfs tmpfs /etc/network/run

# set the hostname 
hostname proxmox

#check system time
/usr/bin/checktime

# try to get ip config with dhcp
echo -n "Detecting network settings... "
/etc/init.d/networking start >/dev/tty2 2>&1
echo "done"

xinit -- -dpi 96 >/dev/tty2 2>&1

if [ $proxdebug -ne 0 ]; then 
    echo "Debugging mode (type exit or CTRL-D to reboot)"
    debugsh 
fi

echo "Installation done, rebooting... "
#mdadm -S /dev/md0 >/dev/tty2 2>&1
real_reboot

# never reached
exit 0
