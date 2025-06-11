package Proxmox::Install::ISOEnv;

use strict;
use warnings;

use Carp;
use Cwd ();
use JSON qw(from_json);

use Proxmox::Sys::File qw(file_read_all);

use base qw(Exporter);
our @EXPORT = qw(is_test_mode);

my $product_cfg = {
    pve => {
	fullname => 'Proxmox VE',
	port => '8006',
	enable_btrfs => 1,
	bridged_network => 1,
    },
    itPVE => {
	fullname => 'itPVE',
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
    pdm => {
	fullname => 'Proxmox Datacenter Manager',
	port => '8443', # TODO: confirm
	enable_btrfs => 0,
	bridged_network => 0,
    },
};

my sub read_locale_info {
    my ($lib_dir) = @_;

    my $json = file_read_all("${lib_dir}/locale-info.json");
    return from_json($json, { utf8 => 1 });
}

my sub get_cd_info {
    my $info_fn = '/.cd-info'; # default place in the ISO environment
    if (!-f $info_fn && -f "cd-info.test") {
	$info_fn = "cd-info.test"; # use from CWD for test mode
    }

    open(my $fh, '<', $info_fn) or die "could not open CD info file '$info_fn' - $!";

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
	run => $is_test ? Cwd::cwd() . "/testdir/run/proxmox-installer" : '/run/proxmox-installer',
    };
}

# setup global environment already on module load
$ENV{DEBIAN_FRONTEND} = 'noninteractive';
$ENV{LC_ALL} = 'C';
$ENV{'LVM_SUPPRESS_FD_WARNINGS'} = '1';

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
	'run-env-cache-file' => "$locations->{run}/run-env-info.json",
	locales => read_locale_info($locations->{lib}),
    };

    return $env;
}

my $_env = undef;
sub get {
    my ($k) = @_;
    $_env = setup() if !defined($_env);
    return defined($k) ? $_env->{$k} : $_env;
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
