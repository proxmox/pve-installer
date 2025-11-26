package Proxmox::Sys::Net;

use strict;
use warnings;

use Proxmox::Sys::Command;
use Proxmox::Sys::Udev;
use JSON qw();

use base qw(Exporter);
our @EXPORT_OK = qw(
    parse_ip_address
    parse_ip_mask
    parse_fqdn
    validate_link_pin_map
    MIN_IFNAME_LEN
    MAX_IFNAME_LEN
    DEFAULT_PIN_PREFIX
);

use constant {
    # As dictated by the `pve-iface` schema.
    MIN_IFNAME_LEN => 2,
    # Maximum length of the (primary) name of a network interface.
    # IFNAMSIZ - 1 to account for NUL byte
    MAX_IFNAME_LEN => 15,
    DEFAULT_PIN_PREFIX => 'nic',
};

our $HOSTNAME_RE = "(?:[a-zA-Z0-9](?:[a-zA-Z0-9\-]{,61}?[a-zA-Z0-9])?)";
our $FQDN_RE = "(?:${HOSTNAME_RE}\\.)*${HOSTNAME_RE}";

my $IPV4OCTET = "(?:25[0-5]|(?:2[0-4]|1[0-9]|[1-9])?[0-9])";
my $IPV4RE = "(?:(?:$IPV4OCTET\\.){3}$IPV4OCTET)";
my $IPV6H16 = "(?:[0-9a-fA-F]{1,4})";
my $IPV6LS32 = "(?:(?:$IPV4RE|$IPV6H16:$IPV6H16))";

#<<< make perltidy ignore this.
my $IPV6RE = "(?:" .
    "(?:(?:" .                             "(?:$IPV6H16:){6})$IPV6LS32)|" .
    "(?:(?:" .                           "::(?:$IPV6H16:){5})$IPV6LS32)|" .
    "(?:(?:(?:" .              "$IPV6H16)?::(?:$IPV6H16:){4})$IPV6LS32)|" .
    "(?:(?:(?:(?:$IPV6H16:){0,1}$IPV6H16)?::(?:$IPV6H16:){3})$IPV6LS32)|" .
    "(?:(?:(?:(?:$IPV6H16:){0,2}$IPV6H16)?::(?:$IPV6H16:){2})$IPV6LS32)|" .
    "(?:(?:(?:(?:$IPV6H16:){0,3}$IPV6H16)?::(?:$IPV6H16:){1})$IPV6LS32)|" .
    "(?:(?:(?:(?:$IPV6H16:){0,4}$IPV6H16)?::" .           ")$IPV6LS32)|" .
    "(?:(?:(?:(?:$IPV6H16:){0,5}$IPV6H16)?::" .            ")$IPV6H16)|" .
    "(?:(?:(?:(?:$IPV6H16:){0,6}$IPV6H16)?::" .                    ")))";
#>>>

my $IPRE = "(?:$IPV4RE|$IPV6RE)";

my $ipv4_mask_hash = {
    '128.0.0.0' => 1,
    '192.0.0.0' => 2,
    '224.0.0.0' => 3,
    '240.0.0.0' => 4,
    '248.0.0.0' => 5,
    '252.0.0.0' => 6,
    '254.0.0.0' => 7,
    '255.0.0.0' => 8,
    '255.128.0.0' => 9,
    '255.192.0.0' => 10,
    '255.224.0.0' => 11,
    '255.240.0.0' => 12,
    '255.248.0.0' => 13,
    '255.252.0.0' => 14,
    '255.254.0.0' => 15,
    '255.255.0.0' => 16,
    '255.255.128.0' => 17,
    '255.255.192.0' => 18,
    '255.255.224.0' => 19,
    '255.255.240.0' => 20,
    '255.255.248.0' => 21,
    '255.255.252.0' => 22,
    '255.255.254.0' => 23,
    '255.255.255.0' => 24,
    '255.255.255.128' => 25,
    '255.255.255.192' => 26,
    '255.255.255.224' => 27,
    '255.255.255.240' => 28,
    '255.255.255.248' => 29,
    '255.255.255.252' => 30,
    '255.255.255.254' => 31,
    '255.255.255.255' => 32,
};

my $ipv4_reverse_mask = [
    '0.0.0.0',
    '128.0.0.0',
    '192.0.0.0',
    '224.0.0.0',
    '240.0.0.0',
    '248.0.0.0',
    '252.0.0.0',
    '254.0.0.0',
    '255.0.0.0',
    '255.128.0.0',
    '255.192.0.0',
    '255.224.0.0',
    '255.240.0.0',
    '255.248.0.0',
    '255.252.0.0',
    '255.254.0.0',
    '255.255.0.0',
    '255.255.128.0',
    '255.255.192.0',
    '255.255.224.0',
    '255.255.240.0',
    '255.255.248.0',
    '255.255.252.0',
    '255.255.254.0',
    '255.255.255.0',
    '255.255.255.128',
    '255.255.255.192',
    '255.255.255.224',
    '255.255.255.240',
    '255.255.255.248',
    '255.255.255.252',
    '255.255.255.254',
    '255.255.255.255',
];

# returns (addr, version) tuple
sub parse_ip_address {
    my ($text) = @_;

    if ($text =~ m!^\s*($IPV4RE)\s*$!) {
        return ($1, 4);
    } elsif ($text =~ m!^\s*($IPV6RE)\s*$!) {
        return ($1, 6);
    }
    return (undef, undef);
}

sub parse_ip_mask {
    my ($text, $ip_version) = @_;
    $text =~ s/^\s+//;
    $text =~ s/\s+$//;
    if ($ip_version == 6 && ($text =~ m/^(\d+)$/) && $1 >= 8 && $1 <= 126) {
        return $text;
    } elsif ($ip_version == 4 && ($text =~ m/^(\d+)$/) && $1 >= 8 && $1 <= 32) {
        return $text;
    } elsif ($ip_version == 4 && defined($ipv4_mask_hash->{$text})) {
        # costs nothing to handle 255.x.y.z style masks, so continue to allow it
        return $ipv4_mask_hash->{$text};
    }
    return;
}

sub iface_is_vf {
    my ($iface_name) = @_;
    return -l "/sys/class/net/$iface_name/device/physfn";
}

# Duplicated from pve-common/src/PVE/Network.pm from now
sub ip_link_is_physical {
    my ($ip_link) = @_;

    # ether alone isn't enough, as virtual interfaces can also have link_type
    # ether
    return $ip_link->{link_type} eq 'ether'
        && (!defined($ip_link->{linkinfo}) || !defined($ip_link->{linkinfo}->{info_kind}));
}

my $LINK_FILE_TEMPLATE = <<EOF;
# setup by the Proxmox installer.
[Match]
MACAddress=%s
Type=ether

[Link]
Name=%s
EOF

sub get_pin_link_file_content {
    my ($mac, $pin_name) = @_;

    return sprintf($LINK_FILE_TEMPLATE, $mac, $pin_name);
}

sub get_ip_config {
    my $ifaces = {};
    my $default;
    my $pinned_counter = 0;

    my $links = `ip -o l`;
    foreach my $l (split /\n/, $links) {
        my ($index, $name, $flags, $state, $mac) =
            $l =~ m/^(\d+):\s+(\S+):\s+<(\S+)>.*\s+state\s+(\S+)\s+.*\s+link\/ether\s+(\S+)\s+/;
        next if !$name || $name eq 'lo';

        my $driver = readlink "/sys/class/net/$name/device/driver" || 'unknown';
        $driver =~ s!^.*/!!;

        $ifaces->{"$index"} = {
            name => $name,
            pinned_id => "${pinned_counter}",
            driver => $driver,
            flags => $flags,
            state => $state,
            mac => $mac,
        };
        $pinned_counter++;

        my $addresses = `ip -o a s $name`;
        for my $addr_line (split /\n/, $addresses) {
            my ($family, $ip, $prefix) =
                $addr_line =~ m/^\Q$index\E:\s+\Q$name\E\s+(inet|inet6)\s+($IPRE)\/(\d+)\s+/;
            next if !$ip;
            next if $addr_line =~ /scope\s+link/; # ignore link local

            my $mask = $prefix;

            if ($family eq 'inet') {
                next if !$ip =~ /$IPV4RE/;
                next if $prefix < 8 || $prefix > 32;
                $mask = @$ipv4_reverse_mask[$prefix];
            } else {
                next if !$ip =~ /$IPV6RE/;
            }

            $default = $index if !$default;

            $ifaces->{"$index"}->{"$family"} = {
                prefix => $prefix,
                mask => $mask,
                addr => $ip,
            };
        }
    }

    my $route = `ip route`;
    my ($gateway) = $route =~ m/^default\s+via\s+(\S+)\s+/m;

    my $resolvconf = `cat /etc/resolv.conf`;
    my ($dnsserver) = $resolvconf =~ m/^nameserver\s+(\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3})$/m;
    my ($domain) = $resolvconf =~ m/^domain\s+(\S+)$/m;

    return {
        default => $default,
        ifaces => $ifaces,
        gateway => $gateway,
        dnsserver => $dnsserver,
        domain => $domain,
    };
}

sub udevadm_netdev_details {
    my $ip_config = get_ip_config();

    my $result = {};
    for my $dev (values $ip_config->{ifaces}->%*) {
        my $name = $dev->{name};
        $result->{$name} = Proxmox::Sys::Udev::get_udev_properties("/sys/class/net/$name");
    }
    return $result;
}

# Tries to detect the hostname for this system given via DHCP, if available.
# The hostname _might_ also include the local domain name, depending on the
# concrete DHCP server implementation.
#
# DHCP servers can set option 12 to inform the client about it's hostname [0].
# dhclient dumps all options set by the DHCP server it in lease file, so just
# read it from there.
#
# [0] RFC 2132, section 3.14
sub get_dhcp_hostname : prototype() {
    my $leasefile = '/var/lib/dhcp/dhclient.leases';
    return if !-f $leasefile;

    open(my $fh, '<', $leasefile) or return;

    my $name = undef;
    while (my $line = <$fh>) {
        # "The name may or may not be qualified with the local domain name"
        # Thus, only match the first part.
        if ($line =~ m/^\s+option host-name \"(${FQDN_RE})\";$/) {
            $name = $1;
            last;
        }
    }

    close($fh);
    return $name if defined($name) && $name =~ m/^([^\.]+)(?:\.(?:\S+))?$/;
}

# Follows the rules as laid out by proxmox_installer_common::utils::Fqdn
sub parse_fqdn : prototype($) {
    my ($text) = @_;

    die "FQDN cannot be empty\n"
        if !$text || length($text) == 0;

    die "FQDN too long\n"
        if length($text) > 253;

    die "Purely numeric hostnames are not allowed\n"
        if $text =~ /^[0-9]+(?:\.|$)/;

    die "FQDN must only consist of alphanumeric characters and dashes\n"
        if $text !~ m/^${FQDN_RE}$/;

    if ($text =~ m/^([^\.]+)\.(\S+)$/) {
        return ($1, $2);
    }

    die "Hostname does not look like a fully qualified domain name\n";
}

# Does some basic checks on a link name mapping.
# Takes a hashref mapping mac => name.
#
# Includes checks for:
# - empty interface names
# - overlong interface names
# - duplicate interface names
# - only contains ASCII alphanumeric characters and underscore, as enforced by
#   our `pve-iface` json schema.
sub validate_link_pin_map : prototype($) {
    my ($mapping) = @_;
    my $reverse_mapping = {};

    while (my ($mac, $name) = each %$mapping) {
        if (!defined($name) || length($name) < MIN_IFNAME_LEN) {
            die "interface name for '$mac' must be at least "
                . MIN_IFNAME_LEN
                . " characters long\n";
        }

        if (length($name) > MAX_IFNAME_LEN) {
            die "interface name '$name' for '$mac' cannot be longer than "
                . MAX_IFNAME_LEN
                . " characters\n";
        }

        if ($name !~ m/^[a-z][a-z0-9_]{1,20}([:\.]\d+)?$/i) {
            die "interface name '$name' for '$mac' is invalid: name must start with a letter and "
                . "contain only ascii characters, digits and underscores\n";
        }

        if ($reverse_mapping->{$name}) {
            die "duplicate interface name mapping '$name' for: $mac, $reverse_mapping->{$name}\n";
        }

        $reverse_mapping->{$name} = $mac;
    }
}

1;
