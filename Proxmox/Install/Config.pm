package Proxmox::Install::Config;

use strict;
use warnings;

use Carp;
use JSON qw(from_json to_json);

use Proxmox::Log;

use Proxmox::Install::ISOEnv;
use Proxmox::Sys::Net;

sub parse_kernel_cmdline {
    my ($cfg) = @_;

    my $cmdline = Proxmox::Install::RunEnv::get('kernel_cmdline');

    if ($cmdline =~ s/\b(ext4|xfs)\s?//i) {
        $cfg->{filesys} = $1;
    }

    if ($cmdline =~ s/\bhdsize=(\d+(\.\d+)?)\s?//i) {
        $cfg->{hdsize} = $1;
    }

    if ($cmdline =~ s/\bswapsize=(\d+(\.\d+)?)\s?//i) {
        $cfg->{swapsize} = $1;
    }

    if ($cmdline =~ s/\bmaxroot=(\d+(\.\d+)?)\s?//i) {
        $cfg->{maxroot} = $1;
    }

    if ($cmdline =~ s/\bminfree=(\d+(\.\d+)?)\s?//i) {
        $cfg->{minfree} = $1;
    }

    my $iso_env = Proxmox::Install::ISOEnv::get();
    if ($iso_env->{product} eq 'pve') {
        if ($cmdline =~ s/\bmaxvz=(\d+(\.\d+)?)\s?//i) {
            $cfg->{maxvz} = $1;
        }
    }

    my @filtered = grep {
        $_ !~ m/^(BOOT_IMAGE|root|ramdisk_size|splash|vga)=\S+$/
            && $_ !~ m/^(ro|rw|quiet)$/
            && $_ !~ m/^(prox(debug|tui|auto)|proxmox-\S+)$/
    } split(/\s+/, $cmdline);

    $cfg->{target_cmdline} = join(' ', @filtered);

    return $cfg;
}

my sub init_cfg {
    my $iso_env = Proxmox::Install::ISOEnv::get();

    my $country = Proxmox::Install::RunEnv::get('country');
    if (defined($country) && !defined($iso_env->{locales}->{country}->{$country})) {
        log_warn("ignoring detected country '$country', invalid or unknown\n");
        $country = undef;
    }

    my $initial = {
        # installer behavior related
        autoreboot => 1,

        # disk and filesystem related
        filesys => 'ext4',
        hdsize => undef,
        swapsize => undef,
        maxroot => undef,
        minfree => undef,
        maxvz => undef,
        zfs_opts => {
            ashift => 12,
            compress => 'on',
            checksum => 'on',
            copies => 1,
            arc_max => Proxmox::Install::RunEnv::default_zfs_arc_max(), # in MiB
        },
        btrfs_opts => {
            compress => 'off',
        },
        # TODO: single disk selection config
        target_hd => undef,
        disk_selection => {},
        existing_storage_auto_rename => 0,

        # locale
        country => $country,
        timezone => 'Europe/Vienna',
        keymap => 'en-us',

        # root credentials & details
        root_password => undef,
        mailto => 'mail@example.invalid',
        root_ssh_keys => [],

        # network related
        mngmt_nic => undef,
        hostname => undef,
        domain => undef,
        cidr => undef,
        gateway => undef,
        dns => undef,
        target_cmdline => undef,

        # proxmox-first-boot setup
        first_boot => {
            enabled => 0,
            # Must be kept in sync with proxmox_auto_installer::answer::FirstBootHookServiceOrdering
            # and the service files in the proxmox-first-boot package
            ordering_target => 'multi-user', # one of `network-pre`, `network-online` or `multi-user`
        },
    };

    $initial = parse_kernel_cmdline($initial);

    return $initial;
}

# merge a $new hash into the current config, with $new taking precedence
sub merge {
    my ($new) = @_;

    my $current = get();

    for my $k (sort keys $new->%*) { # first check all
        croak "unknown key '$k'" if !exists($current->{$k});
    }
    $current->{$_} = $new->{$_} for sort keys $new->%*; # then merge

    return $current;
}

my $_cfg = undef; # NOTE: global singleton

sub get {
    my ($k) = @_;
    $_cfg = init_cfg() if !defined($_cfg);
    return defined($k) ? $_cfg->{$k} : $_cfg;
}

sub set_key {
    my ($k, $v) = @_;
    my $cfg = get();
    croak "unknown key '$k'" if !exists($cfg->{$k});
    $cfg->{$k} = $v;
}

sub set_autoreboot { set_key('autoreboot', $_[0]); }
sub get_autoreboot { return get('autoreboot'); }

sub set_filesys { set_key('filesys', $_[0]); }
sub get_filesys { return get('filesys'); }

sub set_hdsize { set_key('hdsize', $_[0]); }
sub get_hdsize { return get('hdsize'); }

sub set_swapsize { set_key('swapsize', $_[0]); }
sub get_swapsize { return get('swapsize'); }

sub set_maxroot { set_key('maxroot', $_[0]); }
sub get_maxroot { return get('maxroot'); }

sub set_minfree { set_key('minfree', $_[0]); }
sub get_minfree { return get('minfree'); }

sub set_maxvz { set_key('maxvz', $_[0]); }
sub get_maxvz { return get('maxvz'); }

sub set_zfs_opt {
    my ($k, $v) = @_;
    my $zfs_opts = get('zfs_opts');
    croak "unknown zfs opts key '$k'" if !exists($zfs_opts->{$k});
    $zfs_opts->{$k} = $v;
}

sub get_zfs_opt {
    my ($k) = @_;
    my $zfs_opts = get('zfs_opts');
    return defined($k) ? $zfs_opts->{$k} : $zfs_opts;
}

sub set_btrfs_opt {
    my ($k, $v) = @_;
    my $opts = get('btrfs_opts');
    croak "unknown btrfs opts key '$k'" if !exists($opts->{$k});
    $opts->{$k} = $v;
}

sub get_btrfs_opt {
    my ($k) = @_;
    my $opts = get('btrfs_opts');
    return defined($k) ? $opts->{$k} : $opts;
}

sub set_target_hd { set_key('target_hd', $_[0]); }
sub get_target_hd { return get('target_hd'); }

sub set_disk_selection {
    my ($id, $v) = @_;
    my $disk_selection = get('disk_selection');
    $disk_selection->{$id} = $v;
}

sub get_disk_selection {
    my ($id) = @_;
    my $disk_selection = get('disk_selection');
    return defined($id) ? $disk_selection->{$id} : $disk_selection;
}

sub set_country { set_key('country', $_[0]); }
sub get_country { return get('country'); }

sub set_timezone { set_key('timezone', $_[0]); }
sub get_timezone { return get('timezone'); }

sub set_keymap { set_key('keymap', $_[0]); }
sub get_keymap { return get('keymap'); }

sub set_root_password {
    my ($key) = @_;
    croak "unknown root password option '$key'"
        if $key ne 'plain' && $key ne 'hashed';

    set_key('root_password', { $_[0] => $_[1] });
}

sub get_root_password {
    my ($key) = @_;
    croak "unknown root password option '$key'"
        if $key ne 'plain' && $key ne 'hashed';

    my $password = get('root_password');
    return defined($password->{$key}) ? $password->{$key} : undef;
}

sub set_mailto { set_key('mailto', $_[0]); }
sub get_mailto { return get('mailto'); }

sub set_root_ssh_keys { set_key('root_ssh_keys', $_[0]); }
sub get_root_ssh_keys { return get('root_ssh_keys'); }

sub set_mngmt_nic { set_key('mngmt_nic', $_[0]); }
sub get_mngmt_nic { return get('mngmt_nic'); }

sub set_hostname { set_key('hostname', $_[0]); }
sub get_hostname { return get('hostname'); }

sub set_domain { set_key('domain', $_[0]); }
sub get_domain { return get('domain'); }

sub get_fqdn { # virtual config
    my ($hostname, $domain) = (get('hostname'), get('domain'));
    return defined($hostname) && defined($domain) ? "${hostname}.${domain}" : undef;
}

sub set_cidr { set_key('cidr', $_[0]); }
sub get_cidr { return get('cidr'); }

sub get_ip_addr { #'virtual config
    my $cidr = get('cidr') // return;
    my ($ip, $mask) = split('/', $cidr);
    return $ip;
}

sub get_ip_version { # virtual config
    my $ip = get_ip_addr() // return;
    return Proxmox::Sys::Net::parse_ip_address($ip);
}

sub set_gateway { set_key('gateway', $_[0]); }
sub get_gateway { return get('gateway'); }

sub set_dns { set_key('dns', $_[0]); }
sub get_dns { return get('dns'); }

sub set_target_cmdline { set_key('target_cmdline', $_[0]); }
sub get_target_cmdline { return get('target_cmdline'); }

sub set_existing_storage_auto_rename { set_key('existing_storage_auto_rename', $_[0]); }
sub get_existing_storage_auto_rename { return get('existing_storage_auto_rename'); }

sub set_first_boot_opt {
    my ($k, $v) = @_;
    my $opts = get('first_boot');
    croak "unknown first boot override key '$k'" if !exists($opts->{$k});
    $opts->{$k} = $v;
}

sub get_first_boot_opt {
    my ($k) = @_;
    my $opts = get('first_boot');
    return defined($k) ? $opts->{$k} : $opts;
}

1;
