#!/bin/bash

trap "err_reboot" ERR

# NOTE: we nowadays get exec'd by the initrd's PID 1, so we're the new PID 1

parse_cmdline() {
    proxdebug=0
    # shellcheck disable=SC2013 # per word splitting is wanted here
    for par in $(cat /proc/cmdline); do
        case $par in
            proxdebug)
            proxdebug=1
            ;;
        esac
    done;
}

debugsh() {
    /bin/bash
}

eject_and_reboot() {
    iso_dev=$(awk '/ iso9660 / {print $1}' /proc/mounts)

    for try in 5 4 3 2 1; do
        echo "unmounting all"
        if umount -a; then
            break
        fi
        if test -n $try; then
            echo "unmount failed -trying again in 5 seconds"
            sleep 5
        fi
    done

    if [ -n "$iso_dev" ]; then
        eject "$iso_dev"
    fi

    echo "rebooting - please remove the ISO boot media"
    sleep 3
    echo b > /proc/sysrq-trigger
    sleep 100
}

real_reboot() {
    trap - ERR

    /etc/init.d/networking stop

    # stop udev (release file handles)
    /etc/init.d/udev stop

    echo -n "Deactivating swap..."
    swap=$(awk '/^\/dev\// { print $1 }' /proc/swaps);
    if [ -n "$swap" ]; then
       swapoff "$swap"
    fi
    echo "done."

    umount -l -n /target >/dev/null 2>&1
    umount -l -n /dev/pts
    umount -l -n /dev/shm
    umount -l -n /dev
    umount -l -n /run
    [ -d /sys/firmware/efi/efivars ] && umount -l -n /sys/firmware/efi/efivars

    # do not unmount proc and sys for now, at least /proc is still required to trigger the actual
    # reboot, and both are virtual FS only anyway

    kill -s KILL -1 # kill all but current init (our self) PID 1
    sleep 1

    eject_and_reboot

    exit 0 # shouldn't be reached, kernel will panic in that case
}

err_reboot() {
    printf "\nInstallation aborted - unable to continue (type exit or CTRL-D to reboot)\n"
    debugsh || true
    real_reboot
}

# NOTE: dbus must be launched before this, else iwd cannot work
# FIXME: very crude, still needs to actually copy over any iwd config to target
handle_wireless() {
    wireless_found=
    for iface in /sys/class/net/*; do
        if [ -d "$iface/wireless" ]; then
            wireless_found=1
        fi
    done
    if [ -z $wireless_found ]; then
        return;
    fi

    if [ -x /usr/libexec/iwd ]; then
        echo "wireless device(s) found, starting iwd; use 'iwctl' to manage connections (experimental)"
        /usr/libexec/iwd &
    else
        echo "wireless device found but iwd not available, ignoring"
    fi
}

PATH=/sbin:/bin:/usr/sbin:/usr/bin:/usr/X11R6/bin

echo "Starting Proxmox installation"

# ensure udev doesn't ignores our request; FIXME: not required anymore, as we use switch_root now
export SYSTEMD_IGNORE_CHROOT=1

mount -n -t proc proc /proc
mount -n -t sysfs sysfs /sys
if [ -d /sys/firmware/efi ]; then
    echo "EFI boot mode detected, mounting efivars filesystem"
    mount -n -t efivarfs efivarfs /sys/firmware/efi/efivars
fi
mount -n -t tmpfs tmpfs /run

parse_cmdline

# always load most common input drivers
modprobe -q psmouse || true
modprobe -q sermouse ||  true
modprobe -q usbhid ||  true

# load device mapper - used by lilo
modprobe -q dm_mod || true

echo "Installing additional hardware drivers"
export RUNLEVEL=S
export PREVLEVEL=N
/etc/init.d/udev start

mkdir -p /dev/shm
mount -t tmpfs tmpfs /dev/shm

# allow pseudo terminals for debuggin in X
mkdir -p /dev/pts
mount -vt devpts devpts /dev/pts -o gid=5,mode=620

# set the hostname
hostname proxmox

if command -v dbus-daemon; then
    echo "starting D-Bus daemon"
    mkdir /run/dbus
    dbus-daemon --system --syslog-only

    if [ $proxdebug -ne 0 ]; then # FIXME: better intergration, e.g., use iwgtk?
        handle_wireless # no-op if not wireless dev is found
    fi
fi

if [ $proxdebug -ne 0 ]; then
    /sbin/agetty -o '-p -- \\u' --noclear tty9 &
    printf "\nDropping in debug shell before starting installation\n"
    echo "type exit or CTRL-D to continue and start the installation wizard"
    debugsh || true
fi

# try to get ip config with dhcp
echo -n "Attempting to get DHCP leases... "
dhclient -v
echo "done"

echo -n "Starting chrony for opportunistic time-sync... "
chronyd || echo "starting chrony failed ($?)"

echo "Starting a root shell on tty3."
setsid /sbin/agetty -a root --noclear tty3 &

xinit -- -dpi 96 >/dev/tty2 2>&1

# just to be sure everything is on disk
sync

if [ $proxdebug -ne 0 ]; then 
    printf "\nDebug shell after installation exited (type exit or CTRL-D to reboot)\n"
    debugsh || true
fi

echo "Installation done, rebooting... "

killall5 -15

real_reboot

# never reached
exit 0
