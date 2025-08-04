package Proxmox::Install::StorageConfig;

use strict;
use warnings;

use Proxmox::Install::ISOEnv;

sub get_zfs_pool_name {
    return is_test_mode() ? "test_rpool" : 'rpool';
}

sub get_zfs_root_volume_name {
    my $iso_env = Proxmox::Install::ISOEnv::get();
    return "$iso_env->{product}-1";
}

sub get_zfs_config {
    my ($iso_env) = @_;

    my $zfsrootvolname = get_zfs_root_volume_name();
    my $zfspoolname = get_zfs_pool_name();

    my $storage_cfg_zfs = <<__EOD__;
dir: local
	path /var/lib/vz
	content iso,vztmpl,backup,import

zfspool: local-zfs
	pool $zfspoolname/data
	sparse
	content images,rootdir
__EOD__
    return $storage_cfg_zfs;
}

sub get_btrfs_config {
    my $storage_cfg_btrfs = <<__EOD__;
dir: local
	path /var/lib/vz
	content iso,vztmpl,backup
	disable

btrfs: local-btrfs
	path /var/lib/pve/local-btrfs
	content iso,vztmpl,backup,images,rootdir,import
__EOD__
    return $storage_cfg_btrfs;
}

sub get_lvm_thin_config {
    my $storage_cfg_lvmthin = <<__EOD__;
dir: local
	path /var/lib/vz
	content iso,vztmpl,backup,import

lvmthin: local-lvm
	thinpool data
	vgname pve
	content rootdir,images
__EOD__
    return $storage_cfg_lvmthin;
}

sub get_local_config {
    my $storage_cfg_local = <<__EOD__;
dir: local
	path /var/lib/vz
	content iso,vztmpl,backup,rootdir,images,import
__EOD__
    return $storage_cfg_local;
}

1;
