package ProxmoxInstallerSetup;

use strict;
use warnings;

sub setup {
    return {
	product => 'pve',
	enable_btrfs => 0,
	bridged_network => 1,
    };
}

1;


