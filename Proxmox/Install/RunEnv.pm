package Proxmox::Install::RunEnv;

use strict;
use warnings;

use Carp;
use JSON qw(from_json to_json);
use Proxmox::Log;

use base qw(Exporter);
our @EXPORT = qw(is_test_mode);

my sub fromjs : prototype($) {
    return from_json($_[0], { utf8 => 1 });
}

# Returns a hash.
# {
#     name => {
#         size => <bytes>,
#     }
# }
my sub query_blockdevs : prototype() {
    my $disks = {};

    my $lsblk = fromjs(qx/lsblk --bytes --json/);
    for my $disk ($lsblk->{blockdevices}->@*) {
	my ($name, $ro, $size, $type, $mountpoints) = $disk->@{qw(name ro size type mountpoints)};

	next if $type ne 'disk' || $ro;
	next if grep { defined($_) } @$mountpoints;

	$disks->{$name} = { size => $size };
    }

    return $disks;
}

# Returns a hash.
#
# {
#     <ifname> => {
#         mac => <mac address>,
#         index => <index>,
#         name => <ifname>,
#         addresses => [
#             family => <inet|inet6>,
#             address => <mac address>,
#             prefix => <length>,
#         ],
#     },
# }
my sub query_netdevs : prototype() {
    my $ifs = {};
    my $default;

    my $interfaces = fromjs(qx/ip --json address show/);

    for my $if (@$interfaces) {
	my ($index, $name, $state, $mac, $addresses) =
	    $if->@{qw(ifindex ifname operstate address addr_info)};

	next if $state ne 'UP';

	my @valid_addrs;
	for my $addr (@$addresses) {
	    next if $addr->{scope} eq 'link';

	    my ($family, $addr, $prefix) = $addr->@{qw(family local prefixlen)};

	    push @valid_addrs, {
		family => $family,
		address => $addr,
		prefix => $prefix,
	    };
	}

	if (@valid_addrs) {
	    $ifs->{$name} = {
		index => $index,
		name => $name,
		mac => $mac,
		addresses => \@valid_addrs,
	    };
	}
    }

    return $ifs;
}

# Returns a hash.
#
# {
#     gateway4 => {
#         dst => "default",
#         gateway => <ipv4>,
#         dev => <ifname>,
#     },
#     gateway6 => {
#         dst => "default",
#         gateway => <ipv6>,
#         dev => <ifname>,
#     },
# }
my sub query_routes : prototype() {
    my ($gateway4, $gateway6);

    my $route4 = fromjs(qx/ip -4 --json route show/);
    for my $route (@$route4) {
	if ($route->{dst} eq 'default') {
	    $gateway4 = {
		dev => $route->{dev},
		gateway => $route->{gateway},
	    };
	    last;
	}
    }

    my $route6 = fromjs(qx/ip -6 --json route show/);
    for my $route (@$route6) {
	if ($route->{dst} eq 'default') {
	    $gateway6 = {
		dev => $route->{dev},
		gateway => $route->{gateway},
	    };
	    last;
	}
    }

    my $routes;
    $routes->{gateway4} = $gateway4 if $gateway4;
    $routes->{gateway6} = $gateway6 if $gateway6;

    return $routes;
}

# If `/etc/resolv.conf` fails to open this returns nothing.
# Otherwise it returns a hash:
# {
#     dns => <first dns entry>,
#
my sub query_dns : prototype() {
    open my $fh , '<', '/etc/resolv.conf' or return;

    my @dns;
    my $domain;
    while (defined(my $line = <$fh>)) {
	if ($line =~ /^nameserver\s+(\S+)/) {
	    push @dns, $1;
	} elsif (!defined($domain) && $line =~ /^domain\s+(\S+)/) {
	    $domain = $1;
	}
    }

    my $output = {
	domain => $domain,
	@dns ? (dns => \@dns) : (),
    };
};

# Uses `traceroute` and `geoiplookup`/`geoiplookup6` to figure out the current country.
# Has a 10s timeout and uses the stops at the first entry found in the geoip database.
my sub detect_country_tracing_to : prototype($$) {
    my ($ipver, $destination) = @_;

    print "trying to detect country...\n";
    open(my $TRACEROUTE_FH, '-|',
	'traceroute', "-$ipver", '-N', '1', '-q', '1', '-n', $destination)
	or return undef;

    my $geoip_bin = ($ipver == 6) ? 'geoiplookup6' : 'geoiplookup';

    my $country;

    my $previous_alarm = alarm (10);
    eval {
	local $SIG{ALRM} = sub { die "timed out!\n" };
	my $line;
	while (defined ($line = <$TRACEROUTE_FH>)) {
	    log_debug("DC TRACEROUTE: $line");
	    if ($line =~ m/^\s*\d+\s+(\S+)\s/) {
		my $geoip = qx/$geoip_bin $1/;
		log_debug("DC GEOIP: $geoip");
		if ($geoip =~ m/GeoIP Country Edition:\s*([A-Z]+),/) {
		    $country = lc ($1);
		    log_info("DC FOUND: $country\n");
		    last;
		}
	    }
	}
    };
    my $err = $@;
    alarm ($previous_alarm);

    close($TRACEROUTE_FH);

    if ($err) {
	die "unable to detect country - $err\n";
    } elsif ($country) {
	print "detected country: " . uc($country) . "\n";
    }

    return $country;
}

# Returns the entire environment as a hash.
# {
#     country => <short country>,
#     disks => <see query_blockdevs()>,
#     network => {
#         interfaces => <see query_netdevs()>,
#         routes => <see query_routes()>,
#     },
#     dns => <see query_dns()>,
# }
sub query_installation_environment : prototype() {
    my $output = {};

    my $routes = query_routes();

    $output->{disks} = query_blockdevs();
    $output->{network} = {
	interfaces => query_netdevs(),
	routes => $routes,
    };
    $output->{dns} = query_dns();

    my $err;
    my $country;
    if ($routes->{gateway4}) {
	$country = eval { detect_country_tracing_to(4 => '8.8.8.8') };
	$err = $@ if !$country;
    }

    if (!$country && $routes->{gateway6}) {
	$country = eval { detect_country_tracing_to(6 => '2001:4860:4860::8888') };
	$err = $@ if !$country;
    }

    if (defined($country)) {
	$output->{country} = $country;
    } else {
	warn ($err // "unable to detect country\n");
    }

    return $output;
}

1;
