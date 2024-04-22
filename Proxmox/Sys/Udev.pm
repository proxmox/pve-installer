package Proxmox::Sys::Udev;

use strict;
use warnings;

my $UDEV_REGEX = qr/^E: ([^=]+)=(.*)$/;

sub query_udevadm_info {
    my ($sys_path) = @_;

    my $info = `udevadm info --path $sys_path --query all`;
    warn "no details found for device '${sys_path}'\n" if !$info;
    return $info;
}

# return hash of E: properties returned by udevadm
sub parse_udevadm_info {
    my ($udev_raw) = @_;

    my $details = {};
    for my $line (split('\n', $udev_raw)) {
	if ($line =~ $UDEV_REGEX) {
	    $details->{$1} = $2;
	}
    }
    return $details;
}

sub get_udev_properties {
    my ($sys_path) = @_;
    my $info = query_udevadm_info($sys_path) or return;
    return parse_udevadm_info($info);
}

1;
