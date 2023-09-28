package Proxmox::Install;

use strict;
use warnings;

use Cwd 'abs_path';
use Encode;
use POSIX ":sys_wait_h";

use Proxmox::Install::ISOEnv;
use Proxmox::Install::RunEnv;

use Proxmox::Install::Config;
use Proxmox::Install::StorageConfig;

use Proxmox::Sys::Block qw(get_cached_disks wipe_disk partition_bootable_disk);
use Proxmox::Sys::Command qw(run_command syscmd);
use Proxmox::Sys::File qw(file_read_firstline file_write_all);
use Proxmox::UI;

# TODO: move somewhere better?
my $postfix_main_cf = <<_EOD;
# See /usr/share/postfix/main.cf.dist for a commented, more complete version

myhostname=__FQDN__

smtpd_banner = \$myhostname ESMTP \$mail_name (Debian/GNU)
biff = no

# appending .domain is the MUA's job.
append_dot_mydomain = no

# Uncomment the next line to generate "delayed mail" warnings
#delay_warning_time = 4h

alias_maps = hash:/etc/aliases
alias_database = hash:/etc/aliases
mydestination = \$myhostname, localhost.\$mydomain, localhost
relayhost =
mynetworks = 127.0.0.0/8
inet_interfaces = loopback-only
recipient_delimiter = +

compatibility_level = 2

_EOD

my ($last_display_change, $display_info_counter) = (0, 0);
my $display_info_items = [
    "extract1-license.htm",
    "extract2-rulesystem.htm",
    "extract3-spam.htm",
    "extract4-virus.htm",
];

sub reset_last_display_change {
    $last_display_change = 0;
}

sub display_info {
    my $min_display_time = 15;

    my $ctime = time();
    return if ($ctime - $last_display_change) < $min_display_time;

    my $page = $display_info_items->[$display_info_counter % scalar(@$display_info_items)];
    $display_info_counter++;

    Proxmox::UI::display_html($page);
    $last_display_change = time();
}


sub update_progress {
    my ($frac, $start, $end, $text) = @_;

    my $res = Proxmox::UI::progress($frac, $start, $end, $text);

    display_info() if $res < 0.9;

    Proxmox::UI::process_events();
}

my $fssetup = {
    ext4 => {
	mkfs => 'mkfs.ext4 -F',
	mkfs_root_opt => '',
	mkfs_data_opt => '-m 0',
	root_mountopt => 'errors=remount-ro',
    },
    xfs => {
	mkfs => 'mkfs.xfs -f',
	mkfs_root_opt => '',
	mkfs_data_opt => '',
	root_mountopt => '',
    },
};

sub create_filesystem {
    my ($dev, $name, $type, $start, $end, $fs, $fe) = @_;

    my $range = $end - $start;
    my $rs = $start + $range*$fs;
    my $re = $start + $range*$fe;
    my $max = 0;

    my $fsdata = $fssetup->{$type} || die "internal error - unknown file system '$type'";
    my $opts = $name eq 'root' ? $fsdata->{mkfs_root_opt} : $fsdata->{mkfs_data_opt};

    update_progress(0, $rs, $re, "creating $name filesystem");

    run_command("$fsdata->{mkfs} $opts $dev", sub {
	my $line = shift;

	if ($line =~ m/Writing inode tables:\s+(\d+)\/(\d+)/) {
	    $max = $2;
	} elsif ($max && $line =~ m/(\d+)\/$max/) {
	    update_progress(($1/$max)*0.9, $rs, $re);
	} elsif ($line =~ m/Creating journal.*done/) {
	    update_progress(0.95, $rs, $re);
	} elsif ($line =~ m/Writing superblocks and filesystem.*done/) {
	    update_progress(1, $rs, $re);
	}
    });
}

sub debconfig_set {
    my ($targetdir, $dcdata) = @_;

    my $cfgfile = "/tmp/debconf.txt";
    file_write_all("$targetdir/$cfgfile", $dcdata);
    syscmd("chroot $targetdir debconf-set-selections $cfgfile");
    unlink "$targetdir/$cfgfile";
}

sub diversion_add {
    my ($targetdir, $cmd, $new_cmd) = @_;

    syscmd("chroot $targetdir dpkg-divert --package proxmox --add --rename $cmd") == 0
        || die "unable to exec dpkg-divert\n";

    syscmd("ln -sf ${new_cmd} $targetdir/$cmd") == 0
        || die "unable to link diversion to ${new_cmd}\n";
}

sub diversion_remove {
    my  ($targetdir, $cmd) = @_;

    syscmd("mv $targetdir/${cmd}.distrib $targetdir/${cmd};") == 0
        || die "unable to remove $cmd diversion\n";

    syscmd("chroot $targetdir dpkg-divert --remove $cmd") == 0
        || die "unable to remove $cmd diversion\n";
}

sub btrfs_create {
    my ($partitions, $mode) = @_;

    die "unknown btrfs mode '$mode'"
	if !($mode eq 'single' || $mode eq 'raid0' || $mode eq 'raid1' || $mode eq 'raid10');

    my $cmd = ['mkfs.btrfs', '-f'];

    push @$cmd, '-d', $mode, '-m', $mode;

    push @$cmd, @$partitions;

    syscmd($cmd);
}

sub zfs_create_rpool {
    my ($vdev, $pool_name, $root_volume_name) = @_;

    my $iso_env = Proxmox::Install::ISOEnv::get();

    my $zfs_opts = Proxmox::Install::Config::get_zfs_opt();

    my $cmd = "zpool create -f -o cachefile=none";
    $cmd .= " -o ashift=$zfs_opts->{ashift}" if defined($zfs_opts->{ashift});

    syscmd("$cmd $pool_name $vdev") == 0 || die "unable to create zfs root pool\n";

    syscmd("zfs create $pool_name/ROOT")  == 0 || die "unable to create zfs $pool_name/ROOT volume\n";

    if ($iso_env->{product} eq 'pve') {
	syscmd("zfs create $pool_name/data")  == 0 || die "unable to create zfs $pool_name/data volume\n";
    }

    syscmd("zfs create $pool_name/ROOT/$root_volume_name")  == 0 ||
	die "unable to create zfs $pool_name/ROOT/$root_volume_name volume\n";

    # default to `relatime` on, fast enough for the installer and production
    syscmd("zfs set atime=on relatime=on $pool_name") == 0 || die "unable to set zfs properties\n";

    my $value = $zfs_opts->{compress} // 'on';
    syscmd("zfs set compression=$value $pool_name") if defined($value) && $value ne 'off';

    $value = $zfs_opts->{checksum} // 'on';
    syscmd("zfs set checksum=$value $pool_name") if defined($value) && $value ne 'on';

    $value = $zfs_opts->{copies} // 1;
    syscmd("zfs set copies=$value $pool_name") if defined($value) && $value != 1;
}

my $get_raid_devlist = sub {

    my $dev_name_hash = {};

    my $cached_disks = get_cached_disks();
    my $devlist = [];
    for (my $i = 0; $i < @$cached_disks; $i++) {
	my $disk_id = Proxmox::Install::Config::get_disk_selection($i) // next;

	my $hd = $cached_disks->[$disk_id];
	my ($disk, $devname, $size, $model, $logical_bsize) = @$hd;
	die "device '$devname' is used more than once\n" if $dev_name_hash->{$devname};
	$dev_name_hash->{$devname} = $hd;
	push @$devlist, $hd;
    }

    return $devlist;
};

sub zfs_mirror_size_check {
    my ($expected, $actual) = @_;

    die "mirrored disks must have same size\n"
	if abs($expected - $actual) > $expected / 10;
}

sub legacy_bios_4k_check {
    my ($lbs) = @_;
    my $run_env = Proxmox::Install::RunEnv::get();
    die "Booting from 4kn drive in legacy BIOS mode is not supported.\n"
	if $run_env->{boot_type} ne 'efi' && $lbs == 4096;
}

sub get_zfs_raid_setup {
    my $filesys = Proxmox::Install::Config::get_filesys();

    my $devlist = &$get_raid_devlist();

    my $diskcount = scalar(@$devlist);
    die "$filesys needs at least one device\n" if $diskcount < 1;

    my $cmd= '';
    if ($filesys eq 'zfs (RAID0)') {
	foreach my $hd (@$devlist) {
	    legacy_bios_4k_check(@$hd[4]);
	    $cmd .= " @$hd[1]";
	}
    } elsif ($filesys eq 'zfs (RAID1)') {
	die "zfs (RAID1) needs at least 2 device\n" if $diskcount < 2;
	$cmd .= ' mirror ';
	my $hd = @$devlist[0];
	my $expected_size = @$hd[2]; # all disks need approximately same size
	foreach my $hd (@$devlist) {
	    zfs_mirror_size_check($expected_size, @$hd[2]);
	    legacy_bios_4k_check(@$hd[4]);
	    $cmd .= " @$hd[1]";
	}
    } elsif ($filesys eq 'zfs (RAID10)') {
	die "zfs (RAID10) needs at least 4 device\n" if $diskcount < 4;
	die "zfs (RAID10) needs an even number of devices\n" if $diskcount & 1;

	for (my $i = 0; $i < $diskcount; $i+=2) {
	    my $hd1 = @$devlist[$i];
	    my $hd2 = @$devlist[$i+1];
	    zfs_mirror_size_check(@$hd1[2], @$hd2[2]); # pairs need approximately same size
	    legacy_bios_4k_check(@$hd1[4]);
	    legacy_bios_4k_check(@$hd2[4]);
	    $cmd .= ' mirror ' . @$hd1[1] . ' ' . @$hd2[1];
	}

    } elsif ($filesys =~ m/^zfs \(RAIDZ-([123])\)$/) {
	my $level = $1;
	my $mindisks = 2 + $level;
	die "zfs (RAIDZ-$level) needs at least $mindisks devices\n" if scalar(@$devlist) < $mindisks;
	my $hd = @$devlist[0];
	my $expected_size = @$hd[2]; # all disks need approximately same size
	$cmd .= " raidz$level";
	foreach my $hd (@$devlist) {
	    zfs_mirror_size_check($expected_size, @$hd[2]);
	    legacy_bios_4k_check(@$hd[4]);
	    $cmd .= " @$hd[1]";
	}
    } else {
	die "unknown zfs mode '$filesys'\n";
    }

    return ($devlist, $cmd);
}

# If the maximum ARC size for ZFS was explicitly changed by the user, applies
# it to the new system by setting the `zfs_arc_max` module parameter in /etc/modprobe.d/zfs.conf
my sub zfs_setup_module_conf {
    my ($targetdir) = @_;

    my $arc_max = Proxmox::Install::Config::get_zfs_opt('arc_max');
    my $arc_max_mib = Proxmox::Install::RunEnv::clamp_zfs_arc_max($arc_max) * 1024 * 1024;

    if ($arc_max > 0) {
	file_write_all("$targetdir/etc/modprobe.d/zfs.conf", "options zfs zfs_arc_max=$arc_max\n")
    }
}

sub get_btrfs_raid_setup {
    my $filesys = Proxmox::Install::Config::get_filesys();

    my $devlist = &$get_raid_devlist();

    my $diskcount = scalar(@$devlist);
    die "$filesys needs at least one device\n" if $diskcount < 1;

    foreach my $hd (@$devlist) {
	legacy_bios_4k_check(@$hd[4]);
    }

    my $mode;

    if ($diskcount == 1) {
	$mode = 'single';
    } else {
	if ($filesys eq 'btrfs (RAID0)') {
	    $mode = 'raid0';
	} elsif ($filesys eq 'btrfs (RAID1)') {
	    die "btrfs (RAID1) needs at least 2 device\n" if $diskcount < 2;
	    $mode = 'raid1';
	} elsif ($filesys eq 'btrfs (RAID10)') {
	    die "btrfs (RAID10) needs at least 4 device\n" if $diskcount < 4;
	    $mode = 'raid10';
	} else {
	    die "unknown btrfs mode '$filesys'\n";
	}
    }

    return ($devlist, $mode);
}

sub get_pv_list_from_vgname {
    my ($vgname) = @_;

    my $res;

    my $parser = sub {
	my $line = shift;
	$line =~ s/^\s+//;
	$line =~ s/\s+$//;
	return if !$line;
	my ($pv, $vg_uuid) = split(/\s+/, $line);

	if (!defined($res->{$vg_uuid}->{pvs})) {
	    $res->{$vg_uuid}->{pvs} = "$pv";
	} else {
	    $res->{$vg_uuid}->{pvs} .= ", $pv";
	}
    };
    run_command("pvs --noheadings -o pv_name,vg_uuid -S vg_name='$vgname'", $parser, undef, 1);

    return $res;
}

sub ask_existing_vg_rename_or_abort {
    my ($vgname) = @_;

    # this normally only happens if one put a disk with a PVE installation in
    # this server and that disk is not the installation target.
    my $duplicate_vgs = get_pv_list_from_vgname($vgname);
    return if !$duplicate_vgs;

    my $message = "Detected existing '$vgname' Volume Group(s)! Do you want to:\n";

    for my $vg_uuid (keys %$duplicate_vgs) {
	my $vg = $duplicate_vgs->{$vg_uuid};

	# no high randomnes properties, but this is only for the cases where
	# we either have multiple "$vgname" vgs from multiple old PVE disks, or
	# we have a disk with both a "$vgname" and "$vgname-old"...
	my $short_uid = sprintf "%08X", rand(0xffffffff);
	$vg->{new_vgname} = "$vgname-OLD-$short_uid";

	$message .= "rename VG backed by PV '$vg->{pvs}' to '$vg->{new_vgname}'\n";
    }
    $message .= "or cancel the installation?";

    my $response_ok = Proxmox::UI::prompt($message);

    if ($response_ok) {
	for my $vg_uuid (keys %$duplicate_vgs) {
	    my $vg = $duplicate_vgs->{$vg_uuid};
	    my $new_vgname = $vg->{new_vgname};

	    syscmd("vgrename $vg_uuid $new_vgname") == 0 ||
		die "could not rename VG from '$vg->{pvs}' ($vg_uuid) to '$new_vgname'!\n";
	}
    } else {
	warn "Canceled installation by user, due to already existing volume group '$vgname'\n";
	die "\n"; # causes abort without re-showing an error dialogue
    }
}

sub create_lvm_volumes {
    my ($lvmdev, $os_size, $swap_size) = @_;

    my $iso_env = Proxmox::Install::ISOEnv::get();
    my $vgname = $iso_env->{product};

    ask_existing_vg_rename_or_abort($vgname);

    my $rootdev = "/dev/$vgname/root";
    my $datadev = "/dev/$vgname/data";
    my $swapfile;

    # we use --metadatasize 250k, which results in "pe_start = 512"
    # so pe_start is aligned on a 128k boundary (advantage for SSDs)
    syscmd("/sbin/pvcreate --metadatasize 250k -y -ff $lvmdev") == 0 ||
	die "unable to initialize physical volume $lvmdev\n";
    syscmd("/sbin/vgcreate $vgname $lvmdev") == 0 ||
	die "unable to create volume group '$vgname'\n";

    my $hdgb = int($os_size / (1024 * 1024));

    # always leave some space at the end to avoid roudning issues with LVM's physical extent (PE)
    # size of 4 MB.
    my $space = $hdgb <= 32 ? 4 * 1024 : (($hdgb > 128 ? 16 : $hdgb / 8) * 1024 * 1024);

    my $rootsize;
    my $datasize = 0;

    if ($iso_env->{product} eq 'pve') {

	my $maxroot_mb;
	if (my $maxroot = Proxmox::Install::Config::get_maxroot()) {
	    $maxroot_mb = $maxroot * 1024;
	} else {
	    $maxroot_mb = 96 * 1024;
	}

	my $rest = $os_size - $swap_size;
	my $rest_mb = int($rest / 1024);

	my $rootsize_mb;
	if ($rest_mb < 12 * 1024) {
	    # no point in wasting space, try to get us actually installed and align down to 4 MB
	    $rootsize_mb = ($rest_mb - 4) & ~3;
	} elsif ($rest_mb < 48 * 1024) {
	    my $masked = int($rest_mb / 2) & ~3; # align down to 4 MB
	    $rootsize_mb = $masked;
	} else {
	    $rootsize_mb = $rest_mb / 4 + 12 * 1024;
	}

	$rootsize_mb = $maxroot_mb if $rootsize_mb > $maxroot_mb;
	$rootsize = int($rootsize_mb * 1024);
	$rootsize &= ~0xFFF; # align down to 4 MB boundaries

	$rest -= $rootsize; # in KB

	my $minfree = $space;
	if (defined(my $cfg_minfree = Proxmox::Install::Config::get_minfree())) {
	    $minfree = $cfg_minfree * 1024 * 1024 >= $rest ? $space : $cfg_minfree * 1024 * 1024;
	}

	$rest = int($rest - $minfree) & ~0xFFF; # align down to 4 MB boundaries

	if (defined(my $maxvz = Proxmox::Install::Config::get_maxvz())) {
	    $rest = $maxvz * 1024 * 1024 <= $rest ? $maxvz * 1024 * 1024 : $rest;
	}

	$datasize = $rest;

    } else {
	my $cfg_minfree = Proxmox::Install::Config::get_minfree();
	my $minfree = defined($cfg_minfree) ? $cfg_minfree * 1024 * 1024 : $space;
	$rootsize = int($os_size - $minfree - $swap_size); # in KB
	$rootsize &= ~0xFFF; # align down to 4 MB boundaries
    }

    if ($swap_size) {
	syscmd("/sbin/lvcreate -Wy --yes -L${swap_size}K -nswap $vgname") == 0 ||
	    die "unable to create swap volume\n";

	$swapfile = "/dev/$vgname/swap";
    }

    syscmd("/sbin/lvcreate -Wy --yes -L${rootsize}K -nroot $vgname") == 0 ||
	die "unable to create root volume\n";

    if ($datasize > 4 * 1024 * 1024) {
	my $metadatasize = int($datasize/100); # default 1% of data
	$metadatasize = 1024*1024 if $metadatasize < 1024*1024; # but at least 1G
	$metadatasize = 16*1024*1024 if $metadatasize > 16*1024*1024; # but at most 16G
	$metadatasize &= ~0xFFF; # align down to 4 MB boundaries

	# otherwise the metadata is taken out of $minfree
	$datasize -= 2 * $metadatasize;

	# 1 4MB PE to allow for rounding
	$datasize -= 4 * 1024;

	syscmd("/sbin/lvcreate -Wy --yes -L${datasize}K -ndata $vgname") == 0 ||
	    die "unable to create data volume\n";

	syscmd("/sbin/lvconvert --yes --type thin-pool --poolmetadatasize ${metadatasize}K $vgname/data") == 0 ||
	    die "unable to create data thin-pool\n";
    } else {
	my $maxvz = Proxmox::Install::Config::get_maxvz();
	if ($iso_env->{product} eq 'pve' && !defined($maxvz)) {
	    Proxmox::UI::message("Skipping auto-creation of LVM thinpool for guest data due to low space.");
	}
	$datadev = undef;
    }

    syscmd("/sbin/vgchange -a y $vgname") == 0 ||
	die "unable to activate volume group\n";

    return ($rootdev, $swapfile, $datadev);
}

sub compute_swapsize {
    my ($hdsize) = @_;

    my $hdgb = int($hdsize/(1024*1024));

    my $swapsize_kb;
    if (defined(my $swapsize = Proxmox::Install::Config::get_swapsize())) {
	$swapsize_kb = $swapsize * 1024 * 1024;
    } else {
	my $run_env = Proxmox::Install::RunEnv::get();
	my $ss = int($run_env->{total_memory});
	$ss = 4096 if $ss < 4096 && $hdgb >= 64;
	$ss = 2048 if $ss < 2048 && $hdgb >= 32;
	$ss = 1024 if $ss >= 2048 && $hdgb <= 16;
	$ss = 512 if $ss < 512;
	$ss = int($hdgb * 128) if $ss > $hdgb * 128;
	$ss = 8192 if $ss > 8192;
	$swapsize_kb = int($ss * 1024) & ~0xFFF; # align to 4 MB to avoid all to odd SWAP size
    }

    return $swapsize_kb;
}

my sub chroot_chown {
    my ($root, $path, %param) = @_;

    my $recursive = $param{recursive} ? ' -R' : '';
    my $user = $param{user};
    die "can not chown without user parameter\n" if !defined($user);
    my $group = $param{group} // $user;

    syscmd("chroot $root /bin/chown $user:$group $recursive $path") == 0 ||
	die "chroot: unable to change owner for '$path'\n";
}

my sub chroot_chmod {
    my ($root, $path, %param) = @_;

    my $recursive = $param{recursive} ? ' -R' : '';
    my $mode = $param{mode};
    die "can not chmod without mode parameter\n" if !defined($mode);

    syscmd("chroot $root /bin/chmod $mode $recursive $path") == 0 ||
	die "chroot: unable to change permission mode for '$path'\n";
}

sub prepare_proxmox_boot_esp {
    my ($espdev, $targetdir) = @_;

    syscmd("chroot $targetdir proxmox-boot-tool init $espdev") == 0 ||
	die "unable to init ESP and install proxmox-boot loader on '$espdev'\n";
}

sub prepare_grub_efi_boot_esp {
    my ($dev, $espdev, $targetdir) = @_;

    syscmd("mount -n $espdev -t vfat $targetdir/boot/efi") == 0 ||
	die "unable to mount $espdev\n";

    eval {
	my $rc = syscmd("chroot $targetdir /usr/sbin/grub-install --target x86_64-efi --no-floppy --bootloader-id='proxmox' $dev");
	if ($rc != 0) {
	    my $run_env = Proxmox::Install::RunEnv::get();
	    if ($run_env->{boot_type} eq 'efi') {
		die "unable to install the EFI boot loader on '$dev'\n";
	    } else {
		warn "unable to install the EFI boot loader on '$dev', ignoring (not booted using UEFI)\n";
	    }
	}
	# also install fallback boot file (OVMF does not boot without)
	mkdir("$targetdir/boot/efi/EFI/BOOT");
	syscmd("cp $targetdir/boot/efi/EFI/proxmox/grubx64.efi $targetdir/boot/efi/EFI/BOOT/BOOTx64.EFI") == 0 ||
	    die "unable to copy efi boot loader\n";
    };
    my $err = $@;

    eval {
	syscmd("umount $targetdir/boot/efi") == 0 ||
	    die "unable to umount $targetdir/boot/efi\n";
    };
    warn $@ if $@;

    die "failed to prepare EFI boot using Grub on '$espdev': $err" if $err;
}

sub extract_data {
    my $iso_env = Proxmox::Install::ISOEnv::get();
    my $run_env = Proxmox::Install::RunEnv::get();

    my $proxmox_libdir = $iso_env->{locations}->{lib};
    my $proxmox_cddir = $iso_env->{locations}->{iso};
    my $proxmox_pkgdir = "${proxmox_cddir}/proxmox/packages/";

    my $targetdir = is_test_mode() ? "target" : "/target";
    mkdir $targetdir;
    my $basefile = "${proxmox_cddir}/$iso_env->{product}-base.squashfs";

    die "target '$targetdir' does not exist\n" if ! -d  $targetdir;

    my $starttime = [Time::HiRes::gettimeofday];

    my $bootdevinfo = [];

    my ($swapfile, $rootdev, $datadev);
    my ($use_zfs, $use_btrfs) = (0, 0);

    my $filesys = Proxmox::Install::Config::get_filesys();
    my $hdsize = Proxmox::Install::Config::get_hdsize();

    my $zfs_pool_name = Proxmox::Install::StorageConfig::get_zfs_pool_name();
    my $zfs_root_volume_name = Proxmox::Install::StorageConfig::get_zfs_root_volume_name();

    if ($filesys =~ m/zfs/) {
	Proxmox::Install::Config::set_target_hd(undef); # do not use this config
	$use_zfs = 1;
	$targetdir = "/$zfs_pool_name/ROOT/$zfs_root_volume_name";
    } elsif ($filesys =~ m/btrfs/) {
	Proxmox::Install::Config::set_target_hd(undef); # do not use this config
	$use_btrfs = 1;
    }

    if ($use_zfs) {
	my $i;
	for ($i = 5; $i > 0; $i--) {
	    syscmd("modprobe zfs");
	    last if -c "/dev/zfs";
	    sleep(1);
	}

	die "unable to load zfs kernel module\n" if !$i;
    }

    my $bootloader_err;

    eval {
	my $maxper = 0.25;

	update_progress(0, 0, $maxper, "cleanup root-disks");

	syscmd("vgchange -an") if !is_test_mode(); # deactivate all detected VGs

	if (is_test_mode()) {

	    my $test_images = Proxmox::Install::ISOEnv::get_test_images();
	    $rootdev = abs_path($test_images->[0]); # FIXME: use all selected for test too!
	    syscmd("umount $rootdev");

	    if ($use_btrfs) {

		die "unsupported btrfs mode (for testing environment)\n"
		    if $filesys ne 'btrfs (RAID0)';

		btrfs_create([$rootdev], 'single');

	    } elsif ($use_zfs) {

		die "unsupported zfs mode (for testing environment)\n"
		    if $filesys ne 'zfs (RAID0)';

		syscmd("zpool destroy $zfs_pool_name");

		zfs_create_rpool($rootdev, $zfs_pool_name, $zfs_root_volume_name);

	    } else {

		# nothing to do
	    }

	} elsif ($use_btrfs) {

	    my ($devlist, $btrfs_mode) = get_btrfs_raid_setup();

	    foreach my $hd (@$devlist) {
		wipe_disk(@$hd[1]);
	    }

	    update_progress(0, 0.02, $maxper, "create partitions");

	    my $btrfs_partitions = [];
	    foreach my $hd (@$devlist) {
		my $devname = @$hd[1];
		my $logical_bsize = @$hd[4];

		my ($size, $osdev, $efidev) = partition_bootable_disk($devname, $hdsize, '8300');
		$rootdev = $osdev if !defined($rootdev); # simply point to first disk
		my $by_id = Proxmox::Sys::Block::get_disk_by_id_path($devname);
		push @$bootdevinfo, {
		    esp => $efidev,
		    devname => $devname,
		    osdev => $osdev,
		    by_id => $by_id,
		    logical_bsize => $logical_bsize,
		};
		push @$btrfs_partitions, $osdev;
	    }

	    Proxmox::Sys::Block::udevadm_trigger_block();

	    update_progress(0, 0.03, $maxper, "create btrfs");

	    btrfs_create($btrfs_partitions, $btrfs_mode);

	} elsif ($use_zfs) {

	    my ($devlist, $vdev) = get_zfs_raid_setup();

	    foreach my $hd (@$devlist) {
		wipe_disk(@$hd[1]);
	    }

	    update_progress(0, 0.02, $maxper, "create partitions");

	    # install esp/boot part on all, we can only win!
	    for my $hd (@$devlist) {
		my $devname = @$hd[1];
		my $logical_bsize = @$hd[4];

		my ($size, $osdev, $efidev) = partition_bootable_disk($devname, $hdsize, 'BF01');

		push @$bootdevinfo, {
		    esp => $efidev,
		    devname => $devname,
		    osdev => $osdev,
		    logical_bsize => $logical_bsize,
		};
	    }

	    Proxmox::Sys::Block::udevadm_trigger_block();

	    foreach my $di (@$bootdevinfo) {
		my $devname = $di->{devname};
		$di->{by_id} = Proxmox::Sys::Block::get_disk_by_id_path($devname);

		my $osdev = Proxmox::Sys::Block::get_disk_by_id_path($di->{osdev}) || $di->{osdev};

		$vdev =~ s/ $devname/ $osdev/;
	    }

	    foreach my $hd (@$devlist) {
		my $devname = @$hd[1];
		my $by_id = Proxmox::Sys::Block::get_disk_by_id_path($devname);

		$vdev =~ s/ $devname/ $by_id/ if $by_id;
	    }

	    update_progress(0, 0.03, $maxper, "create rpool");

	    zfs_create_rpool($vdev, $zfs_pool_name, $zfs_root_volume_name);

	} else {
	    my $target_hd = Proxmox::Install::Config::get_target_hd();
	    die "target '$target_hd' is not a valid block device\n" if ! -b $target_hd;

	    wipe_disk($target_hd);

	    update_progress(0, 0.02, $maxper, "create partitions");

	    my $logical_bsize = Proxmox::Sys::Block::logical_blocksize($target_hd);

	    my ($os_size, $osdev, $efidev) = partition_bootable_disk($target_hd, $hdsize, '8E00');

	    Proxmox::Sys::Block::udevadm_trigger_block();

	    my $by_id = Proxmox::Sys::Block::get_disk_by_id_path($target_hd);
	    push @$bootdevinfo, {
		esp => $efidev,
		devname => $target_hd,
		osdev => $osdev,
		by_id => $by_id,
		logical_bsize => $logical_bsize,
	    };

	    update_progress(0, 0.03, $maxper, "create LVs");

	    my $swap_size = compute_swapsize($os_size);
	    ($rootdev, $swapfile, $datadev) =
		create_lvm_volumes($osdev, $os_size, $swap_size);

	    # trigger udev to create /dev/disk/by-uuid
	    Proxmox::Sys::Block::udevadm_trigger_block(1);
	}

	if ($use_zfs) {
	    # to be fast during installation
	    syscmd("zfs set sync=disabled $zfs_pool_name") == 0 ||
		die "unable to set zfs properties\n";
	}

	update_progress(0.04, 0, $maxper, "create swap space");
	if ($swapfile) {
	    syscmd("mkswap -f $swapfile") == 0 ||
		die "unable to create swap space\n";
	}

	update_progress(0.045, 0, $maxper, "creating root filesystems");

	foreach my $di (@$bootdevinfo) {
	    next if !$di->{esp};
	    # FIXME remove '-s1' once https://github.com/dosfstools/dosfstools/issues/111 is fixed
	    my $vfat_extra_opts = ($di->{logical_bsize} == 4096) ? '-s1' : '';
	    syscmd("mkfs.vfat $vfat_extra_opts -F32 $di->{esp}") == 0 ||
		die "unable to initialize EFI ESP on device $di->{esp}\n";
	}

	if ($use_zfs) {
	    # do nothing
	} elsif ($use_btrfs) {
	    # do nothing
	} else {
	    create_filesystem($rootdev, 'root', $filesys, 0.05, $maxper, 0, 1);
	}

	update_progress(1, 0.05, $maxper, "mounting target $rootdev");

	if ($use_zfs) {
	    # do nothing
	} else {
	    my $mount_opts = 'noatime';
	    $mount_opts .= ',nobarrier'
		if $use_btrfs || $filesys =~ /^ext\d$/;

	    syscmd("mount -n $rootdev -o $mount_opts $targetdir") == 0 ||
		die "unable to mount $rootdev\n";
	}

	mkdir "$targetdir/boot";
	mkdir "$targetdir/boot/efi";

	mkdir "$targetdir/var";
	mkdir "$targetdir/var/lib";

	if ($iso_env->{product} eq 'pve') {
	    mkdir "$targetdir/var/lib/vz";
	    mkdir "$targetdir/var/lib/pve";

	    if ($use_btrfs) {
		syscmd("btrfs subvolume create $targetdir/var/lib/pve/local-btrfs") == 0 ||
		    die "unable to create btrfs subvolume\n";
	    }
	}

	mkdir "$targetdir/mnt";
	mkdir "$targetdir/mnt/hostrun";
	syscmd("mount --bind /run $targetdir/mnt/hostrun") == 0 ||
	    die "unable to bindmount run on $targetdir/mnt/hostrun\n";

	update_progress(1, 0.05, $maxper, "extracting base system");

	my ($dev,$ino,$mode,$nlink,$uid,$gid,$rdev,$size) = stat ($basefile);
	$ino || die "unable to open file '$basefile' - $!\n";

	my $files = file_read_firstline("${proxmox_cddir}/proxmox/$iso_env->{product}-base.cnt") ||
	    die "unable to read base file count\n";

	my $per = 0;
	my $count = 0;

	run_command("unsquashfs -f -dest $targetdir -i $basefile", sub {
	    my $line = shift;
	    return if $line !~ m/^$targetdir/;
	    $count++;
	    my $nper = int (($count *100)/$files);
	    if ($nper != $per) {
		$per = $nper;
		my $frac = $per > 100 ? 1 : $per/100;
		update_progress($frac, $maxper, 0.5);
	    }
	});

	syscmd("mount -n -t tmpfs tmpfs $targetdir/tmp") == 0 || die "unable to mount tmpfs on $targetdir/tmp\n";

	mkdir "$targetdir/tmp/pkg";
	syscmd("mount -n --bind '$proxmox_pkgdir' '$targetdir/tmp/pkg'") == 0
	    || die "unable to bind-mount packages on $targetdir/tmp/pkg\n";
	syscmd("mount -n -t proc proc $targetdir/proc") == 0 || die "unable to mount proc on $targetdir/proc\n";
	syscmd("mount -n -t sysfs sysfs $targetdir/sys") == 0 || die "unable to mount sysfs on $targetdir/sys\n";
	if ($run_env->{boot_type} eq 'efi') {
	    syscmd("mount -n -t efivarfs efivarfs $targetdir/sys/firmware/efi/efivars") == 0 ||
		die "unable to mount efivarfs on $targetdir/sys/firmware/efi/efivars: $!\n";
	}
	syscmd("chroot $targetdir mount --bind /mnt/hostrun /run") == 0 ||
	    die "unable to re-bindmount hostrun on /run in chroot\n";

	update_progress(1, $maxper, 0.5, "configuring base system");

	# configure hosts
	my $hostname = Proxmox::Install::Config::get_hostname();
	my $domain = Proxmox::Install::Config::get_domain();
	my $ip_addr = Proxmox::Install::Config::get_ip_addr();

	my $hosts =
	    "127.0.0.1 localhost.localdomain localhost\n" .
	    "$ip_addr $hostname.$domain $hostname\n\n" .
	    "# The following lines are desirable for IPv6 capable hosts\n\n" .
	    "::1     ip6-localhost ip6-loopback\n" .
	    "fe00::0 ip6-localnet\n" .
	    "ff00::0 ip6-mcastprefix\n" .
	    "ff02::1 ip6-allnodes\n" .
	    "ff02::2 ip6-allrouters\n" .
	    "ff02::3 ip6-allhosts\n";

	file_write_all("$targetdir/etc/hosts", $hosts);

	file_write_all("$targetdir/etc/hostname", "$hostname\n");

	syscmd("/bin/hostname $hostname") if !is_test_mode();

	# configure interfaces

	my $ifaces = "auto lo\niface lo inet loopback\n\n";

	my $ip_version = Proxmox::Install::Config::get_ip_version();
	my $ntype = $ip_version == 4 ? 'inet' : 'inet6';

	my $ethdev = Proxmox::Install::Config::get_mngmt_nic();
	my $cidr = Proxmox::Install::Config::get_cidr();
	my $gateway = Proxmox::Install::Config::get_gateway();

	if ($iso_env->{cfg}->{bridged_network}) {
	    $ifaces .= "iface $ethdev $ntype manual\n";

	    $ifaces .=
		"\nauto vmbr0\niface vmbr0 $ntype static\n" .
		"\taddress $cidr\n" .
		"\tgateway $gateway\n" .
		"\tbridge-ports $ethdev\n" .
		"\tbridge-stp off\n" .
		"\tbridge-fd 0\n";
	} else {
	    $ifaces .= "auto $ethdev\n" .
		"iface $ethdev $ntype static\n" .
		"\taddress $cidr\n" .
		"\tgateway $gateway\n";
	}

	my $ipconf = $run_env->{ipconf};
	foreach my $iface (sort keys %{$ipconf->{ifaces}}) {
	    my $name = $ipconf->{ifaces}->{$iface}->{name};
	    next if $name eq $ethdev;

	    $ifaces .= "\niface $name $ntype manual\n";
	}

	file_write_all("$targetdir/etc/network/interfaces", $ifaces);

	# configure dns

	my $dnsserver = Proxmox::Install::Config::get_dns();
	my $resolvconf = "search $domain\nnameserver $dnsserver\n";
	file_write_all("$targetdir/etc/resolv.conf", $resolvconf);

	# configure fstab

	my $fstab = "# <file system> <mount point> <type> <options> <dump> <pass>\n";

	if ($use_zfs) {
	    # do nothing
	} elsif ($use_btrfs) {
	    my $fsuuid;
	    my $cmd = "blkid -u filesystem -t TYPE=btrfs -o export $rootdev";
	    run_command($cmd, sub {
		my $line = shift;

		if ($line =~ m/^UUID=([A-Fa-f0-9\-]+)$/) {
		    $fsuuid = $1;
		}
	    });

	    die "unable to detect FS UUID" if !defined($fsuuid);

	    $fstab .= "UUID=$fsuuid / btrfs defaults 0 1\n";
	} else {
	    my $root_mountopt = $fssetup->{$filesys}->{root_mountopt} || 'defaults';
	    $fstab .= "$rootdev / $filesys ${root_mountopt} 0 1\n";
	}

	# mount /boot/efi
	# Note: this is required by current grub, but really dangerous, because
	# vfat does not have journaling, so it triggers manual fsck after each crash
	# so we only mount /boot/efi if really required (efi systems).
	if ($run_env->{boot_type} eq 'efi' && !$use_zfs) {
	    if (scalar(@$bootdevinfo)) {
		my $di = @$bootdevinfo[0]; # simply use first disk

		if ($di->{esp}) {
		    my $efi_boot_uuid = $di->{esp};
		    if (my $uuid = Proxmox::Sys::Block::get_dev_uuid($di->{esp})) {
			$efi_boot_uuid = "UUID=$uuid";
		    }

		    $fstab .= "${efi_boot_uuid} /boot/efi vfat defaults 0 1\n";
		}
	    }
	}


	$fstab .= "$swapfile none swap sw 0 0\n" if $swapfile;

	$fstab .= "proc /proc proc defaults 0 0\n";

	file_write_all("$targetdir/etc/fstab", $fstab);
	file_write_all("$targetdir/etc/mtab", "");

	syscmd("cp ${proxmox_libdir}/policy-disable-rc.d $targetdir/usr/sbin/policy-rc.d") == 0 ||
		die "unable to copy policy-rc.d\n";
	syscmd("cp ${proxmox_libdir}/fake-start-stop-daemon $targetdir/sbin/") == 0 ||
		die "unable to copy start-stop-daemon\n";

	diversion_add($targetdir, "/sbin/start-stop-daemon", "/sbin/fake-start-stop-daemon");
	diversion_add($targetdir, "/usr/sbin/update-grub", "/bin/true");
	diversion_add($targetdir, "/usr/sbin/update-initramfs", "/bin/true");

	my $machine_id = run_command("systemd-id128 new");
	die "unable to create a new machine-id\n" if ! $machine_id;
	file_write_all("$targetdir/etc/machine-id", $machine_id);

	syscmd("cp /etc/hostid $targetdir/etc/") == 0 ||
	    die "unable to copy hostid\n";

	syscmd("touch  $targetdir/proxmox_install_mode");

	my $grub_install_devices_txt = '';
	foreach my $di (@$bootdevinfo) {
	    $grub_install_devices_txt .= ', ' if $grub_install_devices_txt;
	    $grub_install_devices_txt .= $di->{by_id} || $di->{devname};
	}

	my $keymap = Proxmox::Install::Config::get_keymap();
	# Note: keyboard-configuration/xbkb-keymap is used by console-setup
	my $xkmap = $iso_env->{locales}->{kmap}->{$keymap}->{x11} // 'us';

	debconfig_set ($targetdir, <<_EOD);
locales locales/default_environment_locale select en_US.UTF-8
locales locales/locales_to_be_generated select en_US.UTF-8 UTF-8
samba-common samba-common/dhcp boolean false
samba-common samba-common/workgroup string WORKGROUP
postfix postfix/main_mailer_type select No configuration
keyboard-configuration keyboard-configuration/xkb-keymap select $xkmap
d-i debian-installer/locale select en_US.UTF-8
grub-pc grub-pc/install_devices select $grub_install_devices_txt
_EOD

	my $pkg_count = 0;
	while (<${proxmox_pkgdir}/*.deb>) { $pkg_count++ };

	# btrfs/dpkg is extremely slow without --force-unsafe-io
	my $dpkg_opts = $use_btrfs ? "--force-unsafe-io" : "";

	$count = 0;
	while (<${proxmox_pkgdir}/*.deb>) {
	    chomp;
	    my $path = $_;
	    my ($deb) = $path =~ m/${proxmox_pkgdir}\/(.*\.deb)/;

	    # the grub-pc/grub-efi-amd64 packages (w/o -bin) are the ones actually updating grub
	    # upon upgrade - and conflict with each other - install the fitting one only
	    next if ($deb =~ /grub-pc_/ && $run_env->{boot_type} ne 'bios');
	    next if ($deb =~ /grub-efi-amd64_/ && $run_env->{boot_type} ne 'efi');

	    update_progress($count/$pkg_count, 0.5, 0.75, "extracting $deb");
	    print STDERR "extracting: $deb\n";
	    syscmd("chroot $targetdir dpkg $dpkg_opts --force-depends --no-triggers --unpack /tmp/pkg/$deb") == 0
		|| die "installation of package $deb failed\n";
	    update_progress((++$count)/$pkg_count, 0.5, 0.75);
	}

	# needed for postfix postinst in case no other NIC is active
	syscmd("chroot $targetdir ifup lo");

	my $cmd = "chroot $targetdir dpkg $dpkg_opts --force-confold --configure -a";
	$count = 0;
	run_command($cmd, sub {
	    my $line = shift;
	    if ($line =~ m/Setting up\s+(\S+)/) {
		update_progress((++$count)/$pkg_count, 0.75, 0.95, "configuring $1");
	    }
	});

	unlink "$targetdir/etc/mailname";
	$postfix_main_cf =~ s/__FQDN__/${hostname}.${domain}/;
	file_write_all("$targetdir/etc/postfix/main.cf", $postfix_main_cf);

	# make sure we have all postfix directories
	syscmd("chroot $targetdir /usr/sbin/postfix check");
	# cleanup mail queue
	syscmd("chroot $targetdir /usr/sbin/postsuper -d ALL");
	# create /etc/aliases.db (/etc/aliases is shipped in the base squashfs)
	syscmd("chroot $targetdir /usr/bin/newaliases");

	unlink  "$targetdir/proxmox_install_mode";

	my $country = Proxmox::Install::Config::get_country();
	my $timezone = Proxmox::Install::Config::get_timezone();

	# set timezone
	unlink ("$targetdir/etc/localtime");
	symlink ("/usr/share/zoneinfo/$timezone", "$targetdir/etc/localtime");
	file_write_all("$targetdir/etc/timezone", "$timezone\n");

	# set apt mirror
	if (my $mirror = $iso_env->{locales}->{country}->{$country}->{mirror}) {
	    my $fn = "$targetdir/etc/apt/sources.list";
	    syscmd("sed -i 's/ftp\\.debian\\.org/$mirror/' '$fn'");
	}

	# create extended_states for apt (avoid cron job warning if that
	# file does not exist)
	file_write_all("$targetdir/var/lib/apt/extended_states", '');

	# allow ssh root login
	syscmd(['sed', '-i', 's/^#\?PermitRootLogin.*/PermitRootLogin yes/', "$targetdir/etc/ssh/sshd_config"]);

	if ($iso_env->{product} eq 'pmg') {
	    # install initial clamav DB
	    my $srcdir = "${proxmox_cddir}/proxmox/clamav";
	    foreach my $fn ("main.cvd", "bytecode.cvd", "daily.cvd", "safebrowsing.cvd") {
		syscmd("cp \"$srcdir/$fn\" \"$targetdir/var/lib/clamav\"") == 0 ||
		    die "installation of clamav db file '$fn' failed\n";
	    }
	    syscmd("chroot $targetdir /bin/chown clamav:clamav -R /var/lib/clamav") == 0 ||
		die "unable to set owner for clamav database files\n";

	    # on-access scanner (blocks file access if it thinks file is bad) needs to be explicit
	    # configured by the user, otherwise it fails, and it doesn't make sense for most users.
	    unlink "$targetdir/etc/systemd/system/multi-user.target.wants/clamav-clamonacc.service"
		or warn "failed to disable clamav-clamonacc.service - $!";
	}

	if ($iso_env->{product} eq 'pve') {
	    # save installer settings
	    my $ucc = uc ($country);
	    debconfig_set($targetdir, "pve-manager pve-manager/country string $ucc\n");
	}

	update_progress(0.8, 0.95, 1, "make system bootable");

	if ($use_zfs) {
	    # add ZFS options while preserving existing kernel cmdline
	    my $zfs_snippet = "GRUB_CMDLINE_LINUX=\"\$GRUB_CMDLINE_LINUX root=ZFS=$zfs_pool_name/ROOT/$zfs_root_volume_name boot=zfs\"";
	    file_write_all("$targetdir/etc/default/grub.d/zfs.cfg", $zfs_snippet);

	    file_write_all("$targetdir/etc/kernel/cmdline", "root=ZFS=$zfs_pool_name/ROOT/$zfs_root_volume_name boot=zfs\n");

	    zfs_setup_module_conf($targetdir);
	}

	diversion_remove($targetdir, "/usr/sbin/update-grub");
	diversion_remove($targetdir, "/usr/sbin/update-initramfs");

	my $kapi;
	foreach my $fn (<$targetdir/lib/modules/*>) {
	    if ($fn =~ m!/(\d+\.\d+\.\d+-\d+-pve)$!) {
		die "found multiple kernels\n" if defined($kapi);
		$kapi = $1;
	    }
	}
	die "unable to detect kernel version\n" if !defined($kapi);

	if (!is_test_mode()) {

	    unlink ("$targetdir/etc/mtab");
	    symlink ("/proc/mounts", "$targetdir/etc/mtab");
	    syscmd("mount -n --bind /dev $targetdir/dev");

	    my $bootloader_err_list = [];
	    eval {
		syscmd("chroot $targetdir /usr/sbin/update-initramfs -c -k $kapi") == 0 ||
		    die "unable to install initramfs\n";

		my $native_4k_disk_bootable = 0;
		foreach my $di (@$bootdevinfo) {
		    $native_4k_disk_bootable |= ($di->{logical_bsize} == 4096);
		}

		foreach my $di (@$bootdevinfo) {
		    my $dev = $di->{devname};
		    if ($use_zfs) {
			prepare_proxmox_boot_esp($di->{esp}, $targetdir);
		    } else {
			if (!$native_4k_disk_bootable) {
			    eval {
				syscmd("chroot $targetdir /usr/sbin/grub-install --target i386-pc --no-floppy --bootloader-id='proxmox' $dev") == 0 ||
					die "unable to install the i386-pc boot loader on '$dev'\n";
			    };
			    push @$bootloader_err_list, $@ if $@;
			}

			if (my $esp = $di->{esp}) {
			    eval { prepare_grub_efi_boot_esp($dev, $esp, $targetdir) };
			    push @$bootloader_err_list, $@ if $@;
			}
		    }
		}

		syscmd("chroot $targetdir /usr/sbin/update-grub") == 0 ||
		    die "unable to update boot loader config\n";
	    };
	    push @$bootloader_err_list, $@ if $@;

	    if (scalar(@$bootloader_err_list) > 0) {
		$bootloader_err = "bootloader setup errors:\n";
		map { $bootloader_err .= "- $_" } @$bootloader_err_list;
		warn $bootloader_err;
	    }

	    syscmd("umount $targetdir/dev");
	}

	# cleanup

	unlink "$targetdir/usr/sbin/policy-rc.d";

	diversion_remove($targetdir, "/sbin/start-stop-daemon");

	# set root password
	my $octets = encode("utf-8", Proxmox::Install::Config::get_password());
	run_command("chroot $targetdir /usr/sbin/chpasswd", undef, "root:$octets\n");

	my $mailto = Proxmox::Install::Config::get_mailto();
	if ($iso_env->{product} eq 'pmg') {
	    # save admin email
	    file_write_all("$targetdir/etc/pmg/pmg.conf", "section: admin\n\temail ${mailto}\n");

	} elsif ($iso_env->{product} eq 'pve') {

	    # create pmxcfs DB

	    my $tmpdir = "$targetdir/tmp/pve";
	    mkdir $tmpdir;

	    # write vnc keymap to datacenter.cfg
	    my $vnckmap = $iso_env->{locales}->{kmap}->{$keymap}->{kvm} || 'en-us';
	    file_write_all("$tmpdir/datacenter.cfg", "keyboard: $vnckmap\n");

	    # save admin email
	    file_write_all("$tmpdir/user.cfg", "user:root\@pam:1:0:::${mailto}::\n");

	    # write storage.cfg
	    my $storage_cfg;
	    if ($use_zfs) {
		$storage_cfg = Proxmox::Install::StorageConfig::get_zfs_config($iso_env);
	    } elsif ($use_btrfs) {
		$storage_cfg = Proxmox::Install::StorageConfig::get_btrfs_config();
	    } elsif ($datadev) {
		$storage_cfg = Proxmox::Install::StorageConfig::get_lvm_thin_config();
	    } else {
		$storage_cfg = Proxmox::Install::StorageConfig::get_local_config();
	    }
	    file_write_all("$tmpdir/storage.cfg", $storage_cfg);

	    run_command("chroot $targetdir /usr/bin/create_pmxcfs_db /tmp/pve /var/lib/pve-cluster/config.db");

	    syscmd("rm -rf $tmpdir");
	} elsif ($iso_env->{product} eq 'pbs') {
	    my $base_cfg_path = "/etc/proxmox-backup";
	    mkdir "$targetdir/$base_cfg_path";

	    chroot_chown($targetdir, $base_cfg_path, user => 'backup', recursive => 1);
	    chroot_chmod($targetdir, $base_cfg_path, mode => '0700');

	    my $user_cfg_fn = "$base_cfg_path/user.cfg";
	    file_write_all("$targetdir/$user_cfg_fn", "user: root\@pam\n\temail ${mailto}\n");
	    chroot_chown($targetdir, $user_cfg_fn, user => 'root', group => 'backup');
	    chroot_chmod($targetdir, $user_cfg_fn, mode => '0640');
	}
    };

    my $err = $@;

    update_progress(1, 0, 1, "");

    print STDERR $err if $err && $err ne "\n";

    if (is_test_mode()) {
	my $elapsed = Time::HiRes::tv_interval($starttime);
	print STDERR "Elapsed extract time: $elapsed\n";

	syscmd("chroot $targetdir /usr/bin/dpkg-query -W --showformat='\${package}\n'> final.pkglist");
    }

    syscmd("umount $targetdir/run");
    syscmd("umount $targetdir/mnt/hostrun");
    syscmd("umount $targetdir/tmp/pkg");
    syscmd("umount $targetdir/tmp");
    syscmd("umount $targetdir/proc");
    syscmd("umount $targetdir/sys/firmware/efi/efivars");
    syscmd("umount $targetdir/sys");
    rmdir("$targetdir/mnt/hostrun");

    if ($use_zfs) {
	syscmd("zfs umount -a") == 0 ||
	    die "unable to unmount zfs\n";
    } else {
	syscmd("umount -d $targetdir");
    }

    if (!$err && $use_zfs) {
	syscmd("zfs set sync=standard $zfs_pool_name") == 0 ||
	    die "unable to set zfs properties\n";

	syscmd("zfs set mountpoint=/ $zfs_pool_name/ROOT/$zfs_root_volume_name") == 0 ||
	    die "zfs set mountpoint failed\n";

	syscmd("zpool set bootfs=$zfs_pool_name/ROOT/$zfs_root_volume_name $zfs_pool_name") == 0 ||
	    die "zpool set bootfs failed\n";
	syscmd("zpool export $zfs_pool_name");
    }

    if ($bootloader_err) {
	$err = $err && $err ne "\n" ? "$err\n$bootloader_err" : $bootloader_err;
    }

    die $err if $err;
}

1;
