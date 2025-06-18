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

use constant ZFS_ARC_MIN_MIB => 64;

my %default_tests_pve = (
    16 => ZFS_ARC_MIN_MIB, # at least 64 MiB
    1024 => 102,
    4 * 1024 => 410,
    8 * 1024 => 819,
    150 * 1024 => 15360,
    160 * 1024 => 16384,
    1024 * 1024 => 16384, # maximum of 16 GiB
);

my %default_tests_others = (
    16 => ZFS_ARC_MIN_MIB, # at least 64 MiB
    1024 => 102,
    4 * 1024 => 2048,
    8 * 1024 => 4096,
    150 * 1024 => 76800,
    160 * 1024 => 81920,
    1024 * 1024 => 524288,
);

mock_product('pve');
while (my ($total_mem, $expected) = each %default_tests_pve) {
    $proxmox_install_runenv->redefine(
        query_total_memory => sub { return $total_mem; },
    );

    is(
        Proxmox::Install::RunEnv::default_zfs_arc_max(),
        $expected,
        "zfs_arc_max should default to $expected for pve with $total_mem MiB system memory",
    );
}

while (my ($total_mem, $expected) = each %default_tests_others) {
    $proxmox_install_runenv->redefine(
        query_total_memory => sub { return $total_mem; },
    );

    foreach my $product ('pbs', 'pmg', 'pdm') {
        mock_product($product);
        is(
            Proxmox::Install::RunEnv::default_zfs_arc_max(),
            $expected,
            "zfs_arc_max should default to $expected for $product with $total_mem MiB system memory",
        );
    }
}

my @clamp_tests = (
    # input, total system memory, expected
    [0, 4 * 1024, 0],
    [16, 4 * 1024, 64],
    [4 * 1024, 4 * 1024, 3 * 1024],
    [4 * 1024, 6 * 1024, 4 * 1024],
    [8 * 1024, 4 * 1024, 3 * 1024],
);

mock_product('pve');
foreach (@clamp_tests) {
    my ($input, $total_mem, $expected) = @$_;

    $proxmox_install_runenv->redefine(
        query_total_memory => sub { return $total_mem; },
    );

    is(
        Proxmox::Install::RunEnv::clamp_zfs_arc_max($input),
        $expected,
        "$input MiB zfs_arc_max should be clamped to $expected MiB with $total_mem MiB system memory",
    );
}

done_testing();
