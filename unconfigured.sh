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
    /bin/bash
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

    umount -l -n /target >/dev/null 2>&1
    umount -l -n /dev
    umount -l -n /sys
    umount -l -n /proc

    exit 0
}

err_reboot() {

    echo "\nInstallation aborted - unable to continue (type exit or CTRL-D to reboot)"
    debugsh
    real_reboot
}

echo "Starting Proxmox installation"

PATH=/sbin:/bin:/usr/sbin:/usr/bin:/usr/X11R6/bin

# ensure udev isn't snippy and ignores our request
export SYSTEMD_IGNORE_CHROOT=1

mount -n -t proc proc /proc
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

mkdir -p /dev/shm
mount -t tmpfs tmpfs /dev/shm

# set the hostname 
hostname proxmox

# try to get ip config with dhcp
echo -n "Attempting to get DHCP leases... "
dhclient -v
echo "done"

xinit -- -dpi 96 >/dev/tty2 2>&1

# just to be sure everything is on disk
sync

if [ $proxdebug -ne 0 ]; then 
    echo "Debugging mode (type exit or CTRL-D to reboot)"
    debugsh 
fi

echo "Installation done, rebooting... "
#mdadm -S /dev/md0 >/dev/tty2 2>&1
kill $(pidof dhclient) 2>&1 > /dev/null
real_reboot

# never reached
exit 0
