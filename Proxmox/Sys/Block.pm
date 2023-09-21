package Proxmox::Sys::Block;

use strict;
use warnings;

use File::Basename;
use IO::File;
use List::Util qw(first);

use Proxmox::Install::ISOEnv;
use Proxmox::Sys::Command qw(syscmd);
use Proxmox::Sys::File qw(file_read_firstline);
use Proxmox::UI;

use base qw(Exporter);
our @EXPORT_OK = qw(get_cached_disks wipe_disk partition_bootable_disk);

my sub is_same_file {
    my ($file_a, $file_b) = @_;

    my ($dev_a ,$ino_a) = stat($file_a);
    my ($dev_b, $ino_b) = stat($file_b);

    return 0 if !($dev_a && $dev_b && $ino_a && $ino_b);

    return $ino_a == $ino_b && $dev_a == $dev_b;
}

my sub find_stable_path {
    my ($stabledir, $bdev) = @_;

    foreach my $path (<$stabledir/*>) {
	if (is_same_file($path, $bdev)) {
	    return $path;
	}
    }
    return;
}

sub get_disk_by_id_path {
    my ($dev) = @_;
    return find_stable_path('/dev/disk/by-id', $dev);
}

sub get_dev_uuid {
    my $bdev = shift;

    my $by_uuid_path = find_stable_path("/dev/disk/by-uuid", $bdev);

    return basename($by_uuid_path);
}

# [
#     [ <index>, "/dev/path", size_in_blocks, "model", logical_blocksize, <name as found in /sys/block> ]
# ]
my sub hd_list {
    if (is_test_mode()) {
	my $disks = Proxmox::Install::ISOEnv::get_test_images();

	my $i = 0;
	return [
	    map { [ $i++, $_, int((-s $_)/512), "TESTDISK", 512, "/sys/block/$_"] } $disks->@*
	];
    }

    my $res = [];
    my $count = 0;
    foreach my $bd (</sys/block/*>) {
	next if $bd =~ m|^/sys/block/ram\d+$|;
	next if $bd =~ m|^/sys/block/loop\d+$|;
	next if $bd =~ m|^/sys/block/md\d+$|;
	next if $bd =~ m|^/sys/block/dm-.*$|;
	next if $bd =~ m|^/sys/block/fd\d+$|;
	next if $bd =~ m|^/sys/block/sr\d+$|;

	my $info = `udevadm info --path $bd --query all`;
	next if !$info;
	next if $info !~ m/^E: DEVTYPE=disk$/m;
	next if $info =~ m/^E: ID_CDROM/m;
	next if $info =~ m/^E: ID_FS_TYPE=iso9660/m;

	my ($name) = $info =~ m/^N: (\S+)$/m;
	next if !$name;

	my $dev_path;
	if ($info =~ m/^E: DEVNAME=(\S+)$/m) {
	    $dev_path = $1;
	} else {
	    $dev_path = "/dev/$name";
	}

	my $size = file_read_firstline("$bd/size");
	next if !$size;
	chomp $size;
	next if $size !~ m/^\d+$/;
	$size = int($size);

	my $model = file_read_firstline("$bd/device/model") || '';
	$model =~ s/^\s+//;
	$model =~ s/\s+$//;
	if (length ($model) > 30) {
	    $model = substr ($model, 0, 30);
	}

	my $logical_bsize = file_read_firstline("$bd/queue/logical_block_size") // '';
	chomp $logical_bsize;
	if ($logical_bsize && $logical_bsize =~ m/^\d+$/) {
	    $logical_bsize = int($logical_bsize);
	} else {
	    $logical_bsize = undef;
	}

	push @$res, [$count++, $dev_path, $size, $model, $logical_bsize, "/sys/block/$name"];
    }

    return $res;
}

my $cached_disks;
sub cache_disks {
    $cached_disks = hd_list();
}
sub get_cached_disks {
    cache_disks() if !defined($cached_disks);
    return $cached_disks;
}

sub find_cached_disk_by_devname {
    my ($dev, $noerr) = @_;

    my $disks = get_cached_disks();
    my $record = first { $_->[1] eq $dev } $disks->@*; # ($disk, $devname, $size, $model, $lbsize)
    die "no such disk device '$dev'\n" if !defined($record) && !$noerr;

    return $record;
}

sub hd_size {
    my ($dev) = @_;

    my ($_disk, $_name, $size) = find_cached_disk_by_devname($dev)->@*;

    return int($size / 2);
}

sub logical_blocksize {
    my ($dev) = @_;

    my ($_disk, $_dev, $_size, $_model, $logical_bsize) = find_cached_disk_by_devname($dev)->@*;

    return $logical_bsize;
}

sub get_partition_dev {
    my ($dev, $partnum) = @_;

    if ($dev =~ m|^/dev/sd([a-h]?[a-z]\|i[a-v])$|) {
	return "${dev}$partnum";
    } elsif ($dev =~ m|^/dev/xvd[a-z]$|) {
	# Citrix Hypervisor blockdev
	return "${dev}$partnum";
    } elsif ($dev =~ m|^/dev/[hxev]d[a-z]$|) {
	return "${dev}$partnum";
    } elsif ($dev =~ m|^/dev/[^/]+/c\d+d\d+$|) {
	return "${dev}p$partnum";
    } elsif ($dev =~ m|^/dev/[^/]+/d\d+$|) {
	return "${dev}p$partnum";
    } elsif ($dev =~ m|^/dev/[^/]+/hd[a-z]$|) {
	return "${dev}$partnum";
    } elsif ($dev =~ m|^/dev/nvme\d+n\d+$|) {
	return "${dev}p$partnum";
    } else {
	die "unable to get device for partition $partnum on device $dev\n";
    }
}

sub udevadm_trigger_block {
    my ($nowait) = @_;

    sleep(1) if !$nowait; # give kernel time to reread part table

    # trigger udev to create /dev/disk/by-uuid
    syscmd("udevadm trigger --subsystem-match block");
    syscmd("udevadm settle --timeout 10");
};

sub wipe_disk {
    my ($disk) = @_;

    # sort longest first as we need to cleanup depth-first
    my @partitions = sort { length($b) <=> length($a) }
	split("\n", `lsblk --output kname --noheadings --path --list $disk`);

    for my $part (@partitions) {
	next if $part eq $disk;
	next if $part !~ /^\Q$disk\E/;
	eval { syscmd("pvremove -ff -y $part"); };
	eval { syscmd("zpool labelclear -f $part"); };
	eval { syscmd("dd if=/dev/zero of=$part bs=1M count=16"); };
    }
    eval { syscmd(['wipefs', '-a', @partitions]) };
    warn "$@" if $@;
};

sub partition_bootable_disk {
    my ($target_dev, $maxhdsizegb, $ptype) = @_;

    die "too dangerous" if is_test_mode();

    die "unknown partition type '$ptype'"
	if !($ptype eq '8E00' || $ptype eq '8300' || $ptype eq 'BF01');

    my $hdsize = hd_size($target_dev); # size in KB (1024 bytes)

    # For bigger disks default to generous ESP size to allow users having multiple kernels/UKI's
    my $esp_size = $hdsize > 100 * 1024 * 1024 ? 1024 : 512; # MB
    my $esp_end = $esp_size + 1;

    my $restricted_hdsize_mb = 0; # 0 ==> end of partition
    if ($maxhdsizegb) {
	my $maxhdsize = $maxhdsizegb * 1024 * 1024;
	if ($maxhdsize < $hdsize) {
	    $hdsize = $maxhdsize;
	    $restricted_hdsize_mb = int($hdsize/1024) . 'M';
	}
    }

    my $hdgb = int($hdsize/(1024*1024));

    my ($hard_limit, $soft_limit) = (2, 8);

    die "root disk '$target_dev' too small (${hdgb} GB < $hard_limit GB)\n" if $hdgb < $hard_limit;
    if ($hdgb < $soft_limit) {
	my $response_ok = Proxmox::UI::prompt(
	    "Root disk space ${hdgb} GB is below recommended minimum space of $soft_limit GB,"
	    ." installation might not be successful! Continue?"
	);
	die "root disk '$target_dev' too small (${hdgb} GB < $soft_limit GB), and warning not accepted.\n"
	    if !$response_ok;
    }

    syscmd("sgdisk -Z ${target_dev}");

    # 1 - BIOS boot partition (Grub Stage2): first free 1 MB
    # 2 - EFI ESP: next free 512 or 1024 MB
    # 3 - OS/Data partition: rest, up to $maxhdsize in MB

    my $grubbootdev = get_partition_dev($target_dev, 1);
    my $efibootdev = get_partition_dev($target_dev, 2);
    my $osdev = get_partition_dev ($target_dev, 3);

    my $pcmd = ['sgdisk'];

    my $pnum = 2;
    push @$pcmd, "-n${pnum}:1M:+${esp_size}M", "-t$pnum:EF00";

    $pnum = 3;
    push @$pcmd, "-n${pnum}:${esp_end}M:${restricted_hdsize_mb}", "-t$pnum:$ptype";

    push @$pcmd, $target_dev;

    my $os_size = $hdsize - $esp_end * 1024; # efi + 1M bios_boot + 1M alignment

    syscmd($pcmd) == 0 ||
	die "unable to partition harddisk '${target_dev}'\n";

    my $blocksize = logical_blocksize($target_dev);

    if ($blocksize != 4096) {
	$pnum = 1;
	$pcmd = ['sgdisk', '-a1', "-n$pnum:34:2047", "-t$pnum:EF02" , $target_dev];

	syscmd($pcmd) == 0 ||
	    die "unable to create bios_boot partition '${target_dev}'\n";
    }

    udevadm_trigger_block();

    foreach my $part ($efibootdev, $osdev) {
	syscmd("dd if=/dev/zero of=$part bs=1M count=256") if -b $part;
    }

    return ($os_size, $osdev, $efibootdev);
}


1;
