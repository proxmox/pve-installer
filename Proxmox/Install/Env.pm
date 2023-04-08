package Proxmox::Install::Env;

use strict;
use warnings;

use Carp;

use base qw(Exporter);
our @EXPORT = qw(is_test_mode);

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

my sub get_cd_info {
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

sub setup {
    my $cd_info = get_cd_info();
    my $product = $cd_info->{product};

    my $cfg = $product_cfg->{$product} or die "unknown product '$product'\n";
    $cfg->{product} = $product;

    my $env = {
	product => $product,
	cfg => $cfg,
	iso => $cd_info,
    };

    return $env;
}

my $test_images;
# sets a test image to use as disk and enables the testmode
sub set_test_image {
    my ($new_test_image) = @_;
    croak "cannot disable test mode again after enabled" if defined($test_images) && !defined($new_test_image);
    $test_images = $new_test_image;
}
sub is_test_mode {
    return !!$test_images;
}
sub get_test_images {
    return [ split(/,/, $test_images) ];
}

1;
