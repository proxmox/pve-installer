package Proxmox::Install::Config;

use strict;
use warnings;

use Carp;
use JSON qw(from_json to_json);

use Proxmox::Install::ISOEnv;

my sub init_cfg {
    return {
	# installer behavior related
	autoreboot => 1,

	# disk and filesystem related
	filesys => 'ext4',
	hdsize => undef,
	swapsize => undef,
	maxroot => undef,
	minfree => undef,
	maxvz => undef,
    };
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

sub parse_kernel_cmdline {
    my $cfg = get();

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

1;
