package Proxmox::Install::RunEnv;

use strict;
use warnings;

use Carp;
use JSON qw(from_json to_json);

use Proxmox::Log;
use Proxmox::Sys::Command qw(run_command CMD_FINISHED);
use Proxmox::Sys::File qw(file_read_all file_read_firstline);
use Proxmox::Sys::Block;
use Proxmox::Sys::Net;

use Proxmox::Install::ISOEnv;

my sub fromjs : prototype($) {
    return from_json($_[0], { utf8 => 1 });
}

my $mem_total = undef;
# Returns the system memory size in MiB, and falls back to 512 MiB if it
# could not be determined.
sub query_total_memory : prototype() {
    return $mem_total if defined($mem_total);

    open(my $MEMINFO, '<', '/proc/meminfo');

    my $res = 512; # default to 512 if something goes wrong
    while (my $line = <$MEMINFO>) {
        if ($line =~ m/^MemTotal:\s+(\d+)\s*kB/i) {
            $res = int($1 / 1024);
        }
    }
    close($MEMINFO);

    $mem_total = $res;
    return $mem_total;
}

my $_cached_cpu_info = undef;

sub query_cpu_info : prototype() {
    return $_cached_cpu_info if defined($_cached_cpu_info);

    my $cpu_info = {
        vendor_id => 'unknown',
        hvm_supported => 0,
    };

    open(my $CPUINFO_FD, '<', '/proc/cpuinfo');
    while (my $line = <$CPUINFO_FD>) {
        if ($line =~ /^flags\s*:.*(vmx|svm)/m) {
            $cpu_info->{hvm_supported} = 1;
        } elsif ($line =~ /^vendor_id\s*:\s*(GenuineIntel|AuthenticAMD)/) {
            $cpu_info->{vendor_id} = $1;
        } elsif ($line eq "") {
            # processed a whole section, and for the info we currently need that's enough -> return
            last;
        }
    }
    close($CPUINFO_FD);

    $_cached_cpu_info = $cpu_info;
    return $cpu_info;
}

# Returns a hash.
#
# {
#     <ifname> => {
#         mac => <mac address>,
#         index => <index>,
#         name => <ifname>,
#         state => <UP|DOWN|LOWERLAYERDOWN|DORMANT|TESTING|NOTPRESENT|UNKNOWN>,
#         pinned_id => <sequential numerical ID>,
#         driver => <driver name>,
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

    # FIXME: not the same as the battle proven way we used in the installer for years?
    my $interfaces = fromjs(qx/ip --json address show/);

    my $pinned_counter = 0;
    for my $if (@$interfaces) {
        my ($index, $name, $state, $mac, $addresses) =
            $if->@{qw(ifindex ifname operstate address addr_info)};

        next if !$name || $name eq 'lo'; # could also check flags for LOOPBACK..
        if (!$mac) {
            log_info("skipped interface $name, no mac address detected");
            next;
        }

        my @valid_addrs;
        if (uc($state) eq 'UP') {
            for my $addr (@$addresses) {
                next if $addr->{scope} eq 'link';

                my ($family, $addr, $prefix) = $addr->@{qw(family local prefixlen)};

                push @valid_addrs,
                    {
                        family => $family,
                        address => $addr,
                        prefix => $prefix,
                    };
            }
        }

        my $driver = readlink "/sys/class/net/$name/device/driver" || 'unknown';
        $driver =~ s!^.*/!!;

        $ifs->{$name} = {
            index => $index,
            name => $name,
            pinned_id => "${pinned_counter}",
            mac => $mac,
            state => uc($state),
            driver => $driver,
        };
        $ifs->{$name}->{addresses} = \@valid_addrs if @valid_addrs;

        $pinned_counter++;
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

    log_info("query routes");
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
    log_info("query DNS from resolv.conf (managed by DHCP client)");
    open my $fh, '<', '/etc/resolv.conf' or return;

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
}

# Uses `traceroute` and `geoiplookup`/`geoiplookup6` to figure out the current country.
# Has a 10s timeout and uses the stops at the first entry found in the geoip database.
my sub detect_country_tracing_to : prototype($$) {
    my ($ipver, $destination) = @_;

    print STDERR "trying to detect country...\n";

    my $traceroute_cmd = ['traceroute', "-$ipver", '-N', '1', '-q', '1', '-n', $destination];
    my $geoip_bin = ($ipver == 6) ? 'geoiplookup6' : 'geoiplookup';

    my $country;
    my $previous_alarm;
    eval {
        local $SIG{ALRM} = sub { die "timed out!\n" };
        $previous_alarm = alarm(10);

        run_command(
            $traceroute_cmd,
            sub {
                my $line = shift;

                log_debug("DC TRACEROUTE: $line");
                if ($line =~ m/^\s*\d+\s+(\S+)\s/) {
                    my $geoip = qx/$geoip_bin $1/;
                    log_debug("DC GEOIP: $geoip");

                    if ($geoip =~ m/GeoIP Country Edition:\s*([A-Z]+),/) {
                        $country = lc($1);
                        log_info("DC FOUND: $country\n");
                        return CMD_FINISHED;
                    }
                    return;
                }
            },
            undef,
            undef,
            1,
        );

    };
    my $err = $@;
    alarm($previous_alarm);

    if ($err) {
        die "unable to detect country - $err\n";
    } elsif ($country) {
        print STDERR "detected country: " . uc($country) . "\n";
    }

    return $country;
}

# Returns the entire environment as a hash.
# {
#     country => <short country>,
#     ipconf = <see Proxmox::Sys::Net::get_ip_config()>,
#     kernel_cmdline = <contents of /proc/cmdline>,
#     total_memory = <memory size in MiB>,
#     hvm_supported = <1 if the CPU supports hardware-accelerated virtualization>,
#     cpu_vendor_id = <GenuineIntel|AuthenticAMD>,
#     secure_boot = <1 if SecureBoot is enabled>,
#     boot_type = <either 'efi' or 'bios'>,
#     default_zfs_arc_max => <default upper limit for the ZFS ARC size in MiB>,
#     disks => <see Proxmox::Sys::Block::hd_list()>,
#     network => {
#         interfaces => <see query_netdevs()>,
#         routes => <see query_routes()>,
#         dns => <see query_dns()>,
#     },
# }
sub query_installation_environment : prototype() {
    # check first if somebody already cached this for us and re-use that
    my $run_env_file = Proxmox::Install::ISOEnv::get('run-env-cache-file');
    if (-f "$run_env_file" && !Proxmox::Install::ISOEnv::is_test_mode()) {
        log_info("re-using cached runtime env from $run_env_file");
        my $cached_env = eval {
            my $run_env_raw = Proxmox::Sys::File::file_read_all($run_env_file);
            return fromjs($run_env_raw); # returns from eval
        };
        log_error("failed to parse cached runtime env - $@") if $@;
        return $cached_env if defined($cached_env) && scalar keys $cached_env->%*;
        log_warn("cached runtime env seems empty, query everything (again)");
    }
    # else re-query everything
    my $output = {};

    my $routes = query_routes();

    log_info("query block devices");
    $output->{disks} = Proxmox::Sys::Block::get_cached_disks();
    $output->{network} = {
        interfaces => query_netdevs(),
        routes => $routes,
        dns => query_dns(),
    };

    # avoid serializing out null or an empty string, that can trip up the UIs
    if (my $fqdn = Proxmox::Sys::Net::get_dhcp_hostname()) {
        if (defined(my $domain = $output->{network}->{dns}->{domain})) {
            # strip domain name suffix, if possible
            $fqdn =~ s/\.${domain}$//;
        }

        $output->{network}->{hostname} = $fqdn;
    }

    # FIXME: move whatever makes sense over to Proxmox::Sys::Net:: and keep that as single source,
    # it can then use some different structure just fine (after adapting the GTK GUI to that) but
    # **never** to (slightly different!) things for the same stuff...
    $output->{ipconf} = Proxmox::Sys::Net::get_ip_config();

    $output->{kernel_cmdline} = file_read_firstline("/proc/cmdline");
    $output->{total_memory} = query_total_memory();

    my $cpu_info = query_cpu_info();
    $output->{hvm_supported} = $cpu_info->{hvm_supported};
    $output->{cpu_vendor_id} = $cpu_info->{vendor_id};

    $output->{boot_type} = -d '/sys/firmware/efi' ? 'efi' : 'bios';
    $output->{default_zfs_arc_max} = default_zfs_arc_max();

    if ($output->{boot_type} eq 'efi') {
        my $content = eval {
            file_read_all(
                "/sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c");
        };
        if ($@) {
            log_warn("Failed to read secure boot state: $@\n");
        } else {
            my @secureboot = unpack("CCCCC", $content);
            $output->{secure_boot} = $secureboot[4] == 1 ? 1 : 0;
        }
    }

    my $err;
    my $country;
    if ($routes->{gateway4}) {
        log_info("trace country via IPv4");
        $country = eval { detect_country_tracing_to(4 => '8.8.8.8') };
        $err = $@ if !$country;
    }

    if (!$country && $routes->{gateway6}) {
        log_info("trace country via IPv6");
        $country = eval { detect_country_tracing_to(6 => '2001:4860:4860::8888') };
        $err = $@ if !$country;
    }

    if (defined($country)) {
        $output->{country} = $country;
    } else {
        warn($err || "unable to detect country\n");
    }

    return $output;
}

# OpenZFS specifies 64 MiB as the absolute minimum:
# https://openzfs.github.io/openzfs-docs/Performance%20and%20Tuning/Module%20Parameters.html#zfs-arc-max
our $ZFS_ARC_MIN_SIZE_MIB = 64; # MiB

# See https://bugzilla.proxmox.com/show_bug.cgi?id=4829
our $ZFS_ARC_MAX_SIZE_MIB = 16 * 1024; # 16384 MiB = 16 GiB
our $ZFS_ARC_SYSMEM_PERCENTAGE = 0.1; # use 10% of available system memory by default

# Calculates the default upper limit for the ZFS ARC size.
# Returns the default ZFS maximum ARC size in MiB.
# See also <https://bugzilla.proxmox.com/show_bug.cgi?id=4829> and
# https://openzfs.github.io/openzfs-docs/Performance%20and%20Tuning/Module%20Parameters.html#zfs-arc-max
sub default_zfs_arc_max {
    my $product = Proxmox::Install::ISOEnv::get('product');
    my $total_memory = query_total_memory();

    # By default limit PVE and low-memory systems, for all just use the 50% of system memory
    if ($product ne 'pve') {
        my $zfs_default_mib = int(sprintf('%.0f', $total_memory / 2));
        return $zfs_default_mib if $total_memory >= 2048 && $product ne 'pmg';
        return $zfs_default_mib if $total_memory >= 4096; # PMG's base memory requirement is much higher
    }

    my $default_mib = $total_memory * $ZFS_ARC_SYSMEM_PERCENTAGE;
    # int(sprintf()) rounds up instead of truncating
    my $rounded_mib = int(sprintf('%.0f', $default_mib));

    if ($rounded_mib > $ZFS_ARC_MAX_SIZE_MIB) {
        return $ZFS_ARC_MAX_SIZE_MIB;
    } elsif ($rounded_mib < $ZFS_ARC_MIN_SIZE_MIB) {
        return $ZFS_ARC_MIN_SIZE_MIB;
    }

    return $rounded_mib;
}

# Clamps the provided ZFS arc_max value (in MiB) to the accepted bounds. The
# lower is specified by `$ZFS_ARC_MIN_SIZE_MIB`, the upper by the available system memory.
# Returns the clamped value in MiB.
sub clamp_zfs_arc_max {
    my ($mib) = @_;

    return $mib if $mib == 0;

    # upper limit is total system memory with a GiB headroom for the base system
    my $total_mem_with_headroom_mib = query_total_memory() - 1024;
    if ($mib > $total_mem_with_headroom_mib) {
        $mib = $total_mem_with_headroom_mib; # do not return directly here, to catch < min ARC size
    }
    # lower limit is the ARC min size, else ZFS ignores the setting and uses 50% of total memory again
    if ($mib < $ZFS_ARC_MIN_SIZE_MIB) {
        return $ZFS_ARC_MIN_SIZE_MIB;
    }

    return $mib;
}

my $_env = undef;

sub get {
    my ($k) = @_;
    $_env = query_installation_environment() if !defined($_env);
    return defined($k) ? $_env->{$k} : $_env;
}

1;
