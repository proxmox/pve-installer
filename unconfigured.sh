#!/bin/bash

reboot_action="reboot"

trap "err_reboot" ERR

# NOTE: we nowadays get exec'd by the initrd's PID 1, so we're the new PID 1

parse_cmdline() {
    start_auto_installer=0
    proxdebug=0
    proxtui=0
    serial=0
    # shellcheck disable=SC2013 # per word splitting is wanted here
    for par in $(cat /proc/cmdline); do
        case $par in
            proxdebug|proxmox-debug)
                proxdebug=1
            ;;
            proxtui|proxmox-tui-mode)
                proxtui=1
            ;;
            proxauto|proxmox-start-auto-installer)
                start_auto_installer=1
            ;;
            console=ttyS*)
                serial=1
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
        echo "unmounting ISO"
        if umount -v -a --types iso9660; then
            break
        fi
        if test -n $try; then
            echo "unmount failed - trying again in 5 seconds"
            sleep 5
        fi
    done

    if [ -n "$iso_dev" ]; then
        eject "$iso_dev" || true # cannot really work currently, don't care
    fi

    umount -l -n /dev

    # at this stage, all disks are sync'd & unmounted, so `-n/--no-sync` is safe to use here
    if [ "$reboot_action" = "poweroff" ]; then
	echo "powering off - please remove the ISO boot media"
	sleep 3
	poweroff -nf
    else
	echo "rebooting - please remove the ISO boot media"
	sleep 3
	reboot -nf
    fi

    sleep 5
    echo "trigger reset system request"
    # we do not expect the reboot above to fail, so rather to avoid kpanic when pid 1 exits
    echo b > /proc/sysrq-trigger
    sleep 100
}

real_reboot() {
    trap - ERR

    if [[ -x /etc/init.d/networking ]]; then
        /etc/init.d/networking stop
    fi

    # stop udev (release file handles)
    udevd_pid="$(pgrep systemd-udevd)"
    if [ -n "$udevd_pid" ]; then
        if kill -s TERM "$udevd_pid"; then
            if ! waitpid --exited --timeout 5 "$udevd_pid"; then
                echo "failed to wait for udevd exit - $?"
                kill -s KILL "$udevd_pid" || :
            fi
        fi
    else
        # cannot hurt I guess..
        pkill systemd-udevd || :
    fi

    swap=$(awk '/^\/dev\// { print $1 }' /proc/swaps);
    if [ -n "$swap" ]; then
        echo -n "Deactivating swap..."
        swapoff "$swap"
        echo "done."
    fi

    # just to be sure
    sync

    umount -l -n /target >/dev/null 2>&1
    umount -l -n /dev/pts
    umount -l -n /dev/shm
    umount -l -n /run
    [ -d /sys/firmware/efi/efivars ] && umount -l -n /sys/firmware/efi/efivars

    # do not unmount proc and sys for now, at least /proc is still required to trigger the actual
    # reboot, and both are virtual FS only anyway

    echo "Terminate all remaining processes"
    kill -s TERM -1 # TERMinate all but current init (our self) PID 1
    sleep 2
    echo "Kill any remaining processes"
    kill -s KILL -1 # KILL all but current init (our self) PID 1
    sleep 0.5

    eject_and_reboot

    exit 0 # shouldn't be reached, kernel will panic in that case
}

err_reboot() {
    printf "\nInstallation aborted - unable to continue (type exit or CTRL-D to reboot)\n"

    # in case of error, always default to rebooting
    reboot_action="reboot"

    debugsh || true
    real_reboot
}

make_static_nodes() {
    [ -e "/lib/modules/$(uname -r)/modules.devname" ] || return 0
    [ -x /bin/kmod ] || return 0

    /bin/kmod static-nodes --format=tmpfiles --output=/proc/self/fd/1 | \
        while read -r type name mode uid gid age arg; do
            [ -e "$name" ] && continue
            case "$type" in
                c|b|c!|b!)
                    # shellcheck disable=SC2086
                    mknod -m "$mode" "$name" "$type" ${arg//:/ } ;;
                d|d!) mkdir "$name" ;;
                *) echo "unparseable line ($type $name $mode $uid $gid $age $arg)" >&2 ;;
            esac
        done
}

start_udevd() {
    echo > /sys/kernel/uevent_helper # unregister mdev from early init

    # mount a devtmpfs over /dev, if somebody did not already do it
    if grep -E -q "^[^[:space:]]+ /dev devtmpfs" /proc/mounts; then
        mount -n -o remount,nosuid,size=100M,mode=0755 -t devtmpfs devtmpfs /dev
    elif ! mount -n -o nosuid,size=100M,mode=0755 -t devtmpfs devtmpfs /dev; then
        echo "udev requires devtmpfs support, not started"
        return 0
    fi

    make_static_nodes

    # clean up parts of the database created by the initramfs udev
    udevadm info --cleanup-db

    # actually start daemon
    /usr/lib/systemd/systemd-udevd --daemon

    echo "Synthesizing the initial hotplug events (subsystems)"
    if ! udevadm trigger --type=subsystems --action=add; then
        echo "udevadm failed - $?"
    fi
    echo "Synthesizing the initial hotplug events (devices)"
    if ! udevadm trigger --type=devices --action=add; then
        echo "udevadm failed - $?"
    fi

    if [ -e /sbin/MAKEDEV ]; then
        ln -sf /sbin/MAKEDEV /dev/MAKEDEV
    else
        ln -sf /bin/true /dev/MAKEDEV
    fi

    echo "Waiting for /dev to be fully populated"
    if ! udevadm settle; then
        echo "udevadm settle timed out - $?"
    fi

    return 0
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
mkdir -p /run/proxmox-installer

parse_cmdline

# always load most common input drivers
modprobe -q psmouse || true
modprobe -q sermouse ||  true
modprobe -q usbhid ||  true

# load device mapper - used by lilo
modprobe -q dm_mod || true

echo "Installing additional hardware drivers"
start_udevd

mkdir -p /dev/shm
mount -t tmpfs tmpfs /dev/shm

# allow pseudo terminals for debugging in X
mkdir -p /dev/pts
mount -vt devpts devpts /dev/pts -o gid=5,mode=620

# shellcheck disable=SC2207
console_dim=($(IFS=' ' stty size)) # [height, width]
DPI=96
if (("${console_dim[0]}" > 100)) && (("${console_dim[1]}" > 400)); then
    # heuristic only, high resolution can still mean normal/low DPI if it's a really big screen
    # FIXME: use `edid-decode` as it can contain physical dimensions to calculate actual dpi?
    echo "detected huge console, setting bigger font/dpi"
    DPI=192
    export GDK_SCALE=2
    setfont /usr/share/consolefonts/Uni2-Terminus32x16.psf.gz
fi

# set the hostname
hostname proxmox

if command -v dbus-daemon; then
    echo "starting D-Bus daemon"
    mkdir /run/dbus
    dbus-daemon --system --syslog-only

    if [ $proxdebug -ne 0 ]; then # FIXME: better integration, e.g., use iwgtk?
        handle_wireless # no-op if not wireless dev is found
    fi
fi

# we use a trimmed down debootstrap so make busybox tools available to compensate that
busybox --install -s || true

setupcon || echo "setupcon failed, TUI rendering might be garbled - $?"

if [ "$serial" -ne 0 ]; then
    echo "Setting terminal size to 80x24 for serial install"
    stty columns 80 rows 24
fi

if [ $proxdebug -ne 0 ]; then
    /sbin/agetty -o '-p -- \\u' --noclear tty9 &
    printf "\nDropping in debug shell before starting installation\n"
    echo "type 'exit' or press CTRL + D to continue and start the installation wizard"
    debugsh || true
fi

# add custom DHCP options for auto installer
if [ $start_auto_installer -ne 0 ]; then
    echo "Preparing DHCP as potential source to get location of automatic-installation answer file"
    cat >> /etc/dhcp/dhclient.conf <<EOF
option proxmox-auto-installer-manifest-url code 250 = text;
option proxmox-auto-installer-cert-fingerprint code 251 = text;
also request proxmox-auto-installer-manifest-url, proxmox-auto-installer-cert-fingerprint;
EOF
fi

# try to get ip config with dhcp
echo -n "Attempting to get DHCP leases... "
dhclient -v
echo "done"

echo "Starting chrony for opportunistic time-sync... "
chronyd || echo "starting chrony failed ($?)"

echo "Starting a root shell on tty3."
setsid /sbin/agetty -a root --noclear tty3 &

echo "Setting console loglevel to warn."
sysctl -w kernel.printk='4 4 1 7'

/usr/bin/proxmox-low-level-installer dump-env

if [ $proxtui -ne 0 ]; then
    echo "Starting the TUI installer"
    /usr/bin/proxmox-tui-installer 2>/dev/tty2
elif [ $start_auto_installer -ne 0 ]; then
    echo "Caching device info from udev"
    /usr/bin/proxmox-low-level-installer dump-udev

    if [ -f /cdrom/auto-installer-mode.toml ]; then
        echo "Fetching answers for automatic installation"
        /usr/bin/proxmox-fetch-answer >/run/automatic-installer-answers
    else
        printf "\nAutomatic installation selected but no config for fetching the answer file found!\n"
        echo "Starting debug shell, to fetch the answer file manually use:"
        echo "  proxmox-fetch-answer MODE >/run/automatic-installer-answers"
        echo "and enter 'exit' or press 'CTRL' + 'D' when finished."
        debugsh || true
    fi
    echo "Starting automatic installation"

    # the auto-installer creates "/run/proxmox-reboot-on-error" if `global.reboot_on_error = true`
    if /usr/bin/proxmox-auto-installer </run/automatic-installer-answers; then
        if ! /usr/bin/proxmox-post-hook </run/automatic-installer-answers; then
            echo "Post-installation hook failed (exit-code $?) - see above for errors."
            # drop into debug shell if we shouldn't reboot on error
            if [ ! -f /run/proxmox-reboot-on-error ]; then
                err_reboot
            else
                echo "Waiting 30s to allow gathering the error before reboot."
                sleep 30
            fi
        fi
    else
        echo "Auto-installation failed (exit-code $?) - see above for errors."
        if [ ! -f /run/proxmox-reboot-on-error ]; then
            err_reboot
        fi
    fi

    if [ -f /run/proxmox-poweroff-after-install ]; then
	reboot_action="poweroff"
    fi
else
    echo "Starting the installer GUI - see tty2 (CTRL+ALT+F2) for any errors..."
    xinit -- -dpi "$DPI" -s 0 >/dev/tty2 2>&1
fi

# just to be sure everything is on disk
sync

if [ $proxdebug -ne 0 ]; then 
    printf "\nDebug shell after installation exited (type exit or CTRL-D to reboot)\n"
    debugsh || true
fi

if [ "$reboot_action" = "poweroff" ]; then
    echo 'Installation done, powering off...'
else
    echo 'Installation done, rebooting...'
fi

killall5 -15

real_reboot

# never reached
# shellcheck disable=SC2317
exit 0
