package Proxmox::Install::Config;

use strict;
use warnings;

use Carp;
use JSON qw(from_json to_json);

use Proxmox::Log;

use Proxmox::Install::ISOEnv;
use Proxmox::Sys::Net;

my sub parse_kernel_cmdline {
    my ($cfg) = @_;

    my $cmdline = Proxmox::Install::RunEnv::get('kernel_cmdline');

    if ($cmdline =~ m/\s(ext4|xfs)(\s.*)?$/) {
	$cfg->{filesys} = $1;
    }

    if ($cmdline =~ m/hdsize=(\d+(\.\d+)?)[\s\n]/i) {
	$cfg->{hdsize} = $1;
    }

    if ($cmdline =~ m/swapsize=(\d+(\.\d+)?)[\s\n]/i) {
	$cfg->{swapsize} = $1;
    }

    if ($cmdline =~ m/maxroot=(\d+(\.\d+)?)[\s\n]/i) {
	$cfg->{maxroot} = $1;
    }

    if ($cmdline =~ m/minfree=(\d+(\.\d+)?)[\s\n]/i) {
	$cfg->{minfree} = $1;
    }

    my $iso_env = Proxmox::Install::ISOEnv::get();
    if ($iso_env->{product} eq 'pve') {
	if ($cmdline =~ m/maxvz=(\d+(\.\d+)?)[\s\n]/i) {
	    $cfg->{maxvz} = $1;
	}
    }

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
	# TODO: single disk selection config
	target_hd => undef,
	disk_selection => {},

	# locale
	country => $country,
	timezone => 'Europe/Vienna',
	keymap => 'en-us',

	# root credentials & details
	password => undef,
	mailto => 'mail@example.invalid',

	# network related
	mngmt_nic => undef,
	# FIXME: fix call sites and remove below, it's just an ugly relict of GTK GUI and time
	# pressure on creating the single source of truth for installation config
	mngmt_nic_id => undef,
	hostname => undef,
	domain => undef,
	cidr => undef,
	gateway => undef,
	dns => undef,
    };

    $initial = parse_kernel_cmdline($initial);

    return $initial;
}

# merge a $new hash into the current config, with $new taking precedence
sub merge {
    my ($new) = @_;

    my $current = get();

    for my $k (sort keys $new->%*) { # first check all
	croak "unknown key '$k'" if !exists($current->{$k})
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

sub set_password { set_key('password', $_[0]); }
sub get_password { return get('password'); }

sub set_mailto { set_key('mailto', $_[0]); }
sub get_mailto { return get('mailto'); }

sub set_mngmt_nic { set_key('mngmt_nic', $_[0]); }
sub get_mngmt_nic { return get('mngmt_nic'); }

sub set_mngmt_nic_id { set_key('mngmt_nic_id', $_[0]); }
sub get_mngmt_nic_id { return get('mngmt_nic_id'); }

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

1;
