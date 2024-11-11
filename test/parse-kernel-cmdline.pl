#!/usr/bin/env perl

use strict;
use warnings;

use Test::More;
use Test::MockModule qw(strict);

use Proxmox::Install::RunEnv;
use Proxmox::Install::Config qw(parse_kernel_cmdline);

my $proxmox_install_runenv = Test::MockModule->new('Proxmox::Install::RunEnv');
my $proxmox_install_isoenv = Test::MockModule->new('Proxmox::Install::ISOEnv');

$proxmox_install_isoenv->redefine(
    get => sub {
	my ($k) = @_;
	return { product => 'pve' } if !defined($k);
	die "iso environment key $k not mocked!\n";
    },
);

my sub mock_kernel_cmdline {
    my ($cmdline) = @_;

    $proxmox_install_runenv->redefine(
	get => sub {
	    my ($k) = @_;
	    return $cmdline if $k eq 'kernel_cmdline';
	    die "iso environment key $k not mocked!\n";
	},
    );
}

sub is_parsed {
    my ($cmdline, $expected) = @_;

    mock_kernel_cmdline($cmdline);
    my $cfg = Proxmox::Install::Config::parse_kernel_cmdline({});

    is($cfg->{target_cmdline}, $expected, "filtered kernel commandline matched expected: ${expected}");
}

is_parsed(
    'BOOT_IMAGE=/vmlinuz-6.8.12-1-pve root=/dev/mapper/pve-root ro quiet',
    ''
);

is_parsed(
    'BOOT_IMAGE=/vmlinuz-6.8.12-1-pve root=/dev/mapper/pve-root ro= quiet',
    'ro='
);

is_parsed(
    'a BOOT_IMAGE=/vmlinuz-6.8.12-1-pve b root=/dev/mapper/pve-root c ro d quiet e',
    'a b c d e'
);

is_parsed('proxmox-foo foo=bar proxtui', 'foo=bar');
is_parsed('proxmox-foo nomodeset quiet', 'nomodeset');

done_testing();
