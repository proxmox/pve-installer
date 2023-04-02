package Proxmox::Install::Setup;

use strict;
use warnings;

my $product_cfg = {
    pve => {
	fullname => 'Proxmox VE',
	port => '8006',
	enable_btrfs => 1,
	bridged_network => 1,
    },
    pmg => {
	fullname => 'Proxmox Mail Gateway',
	port => '8006',
	enable_btrfs => 0,
	bridged_network => 0,
    },
    pbs => {
	fullname => 'Proxmox Backup Server',
	port => '8007',
	enable_btrfs => 0,
	bridged_network => 0,
    },
};

sub setup {
    my $cd_info = get_cd_info();
    my $product = $cd_info->{product};

    my $setup_info = $product_cfg->{$product};
    die "unknown product '$product'\n" if !$setup_info;

    $setup_info->{product} = $product;

    return ($setup_info, $cd_info);
}

sub get_cd_info {
    my $info_fn = '/.cd-info'; # default place in the ISO environment
    if (!-f $info_fn && -f "cd-info.test") {
	$info_fn = "cd-info.test"; # use from CWD for test mode
    }

    open(my $fh, '<', $info_fn) or die "Could not open CD info file '$info_fn' $!";

    my $cd_info = {};
    while (my $line = <$fh>) {
	chomp $line;
	if ($line =~ /^(\S+)=['"]?(.+?)['"]?$/) { # we control cd-info content, so good enough.
	    $cd_info->{lc($1)} = $2;
	}
    }
    close ($fh);

    die "CD-info is missing required key 'product'!\n" if !defined $cd_info->{product};

    return $cd_info;
}

1;
