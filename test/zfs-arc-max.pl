#!/usr/bin/perl

use strict;
use warnings;

use Test::More;
use Test::MockModule qw(strict);

my $proxmox_install_runenv = Test::MockModule->new('Proxmox::Install::RunEnv');
my $proxmox_install_isoenv = Test::MockModule->new('Proxmox::Install::ISOEnv');

sub mock_product {
    my ($product) = @_;

    $proxmox_install_isoenv->redefine(
	get => sub {
	    my ($k) = @_;
	    return $product if $k eq 'product';
	    die "iso environment key $k not mocked!\n";
	},
    );
}

my %default_tests = (
    16 => 64, # at least 64 MiB
    1024 => 102,
    4 * 1024 => 410,
    8 * 1024 => 819,
    150 * 1024 => 15360,
    160 * 1024 => 16384,
    1024 * 1024 => 16384, # maximum of 16 GiB
);

while (my ($total_mem, $expected) = each %default_tests) {
    $proxmox_install_runenv->redefine(
	get => sub {
	    my ($k) = @_;
	    return $total_mem if $k eq 'total_memory';
	    die "runtime environment key $k not mocked!\n";
	},
    );

    mock_product('pve');
    is(Proxmox::Install::RunEnv::default_zfs_arc_max(), $expected,
	"$expected MiB should be zfs_arc_max for PVE with $total_mem MiB system memory");

    mock_product('pbs');
    is(Proxmox::Install::RunEnv::default_zfs_arc_max(), $total_mem < 2048 ? $expected : 0,
	"zfs_arc_max should default to `0` for PBS with $total_mem MiB system memory");

    mock_product('pmg');
    is(Proxmox::Install::RunEnv::default_zfs_arc_max(), $total_mem < 4096 ? $expected : 0,
	"zfs_arc_max should default to `0` for PMG with $total_mem MiB system memory");

    mock_product('pdm');
    is(Proxmox::Install::RunEnv::default_zfs_arc_max(), $total_mem < 2048 ? $expected : 0,
	"zfs_arc_max should default to `0` for PDM with $total_mem MiB system memory");
}

my @clamp_tests = (
    # input, total system memory, expected
    [ 0, 4 * 1024, 0 ],
    [ 16, 4 * 1024, 64 ],
    [ 4 * 1024, 4 * 1024, 3 * 1024 ],
    [ 4 * 1024, 6 * 1024, 4 * 1024 ],
    [ 8 * 1024, 4 * 1024, 3 * 1024 ],
);

mock_product('pve');
foreach (@clamp_tests) {
    my ($input, $total_mem, $expected) = @$_;

    $proxmox_install_runenv->redefine(
	get => sub {
	    my ($k) = @_;
	    return $total_mem if $k eq 'total_memory';
	    die "runtime environment key $k not mocked!\n";
	},
    );

    is(Proxmox::Install::RunEnv::clamp_zfs_arc_max($input), $expected,
	"$input MiB zfs_arc_max should be clamped to $expected MiB with $total_mem MiB system memory");
}

done_testing();
