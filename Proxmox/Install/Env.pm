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

my sub read_cmap {
    my ($lib_dir) = @_;
    my $countryfn = "${lib_dir}/country.dat";
    open (my $TMP, "<:encoding(utf8)", "$countryfn") || die "unable to open '$countryfn' - $!\n";
    my ($country, $countryhash, $kmap, $kmaphash) = ({}, {}, {}, {});
    while (defined (my $line = <$TMP>)) {
	if ($line =~ m|^map:([^\s:]+):([^:]+):([^:]+):([^:]+):([^:]+):([^:]*):$|) {
	    $kmap->{$1} = {
		name => $2,
		kvm => $3,
		console => $4,
		x11 => $5,
		x11var => $6,
	    };
	    $kmaphash->{$2} = $1;
	} elsif ($line =~ m|^([a-z]{2}):([^:]+):([^:]*):([^:]*):$|) {
	    $country->{$1} = {
		name => $2,
		kmap => $3,
		mirror => $4,
	    };
	    $countryhash->{lc($2)} = $1;
	} else {
	    warn "unable to parse 'country.dat' line: $line";
	}
    }
    close ($TMP);
    $TMP = undef;

    my $zones = {};
    my $cczones = {};
    my $zonefn = "/usr/share/zoneinfo/zone.tab";
    open ($TMP, '<', "$zonefn") || die "unable to open '$zonefn' - $!\n";
    while (defined (my $line = <$TMP>)) {
	next if $line =~ m/^\#/;
	next if $line =~ m/^\s*$/;
	if ($line =~ m|^([A-Z][A-Z])\s+\S+\s+(([^/]+)/\S+)\s|) {
	    my $cc = lc($1);
	    $cczones->{$cc}->{$2} = 1;
	    $country->{$cc}->{zone} = $2 if !defined ($country->{$cc}->{zone});
	    $zones->{$2} = 1;

	}
    }
    close ($TMP);

    return {
	zones => $zones,
	cczones => $cczones,
	country => $country,
	countryhash => $countryhash,
	kmap => $kmap,
	kmaphash => $kmaphash,
    }
}

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

my sub get_locations {
    my $is_test = is_test_mode();

    my $base_lib_dir = '/var/lib/proxmox-installer';
    my $iso_dir = $is_test ? $ENV{'CD_BUILDER_DIR'} || "../pve-cd-builder/tmp/data-gz/" : "/cdrom";

    return {
	iso => $iso_dir,
	lib => $is_test ? Cwd::cwd() . "/testdir/${base_lib_dir}" : $base_lib_dir,
	pkg => "${iso_dir}/proxmox/packages/",
    };
}

sub setup {
    my $cd_info = get_cd_info();
    my $product = $cd_info->{product};

    my $cfg = $product_cfg->{$product} or die "unknown product '$product'\n";
    $cfg->{product} = $product;

    my $locations = get_locations();

    my $env = {
	product => $product,
	cfg => $cfg,
	iso => $cd_info,
	locations => $locations,
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
