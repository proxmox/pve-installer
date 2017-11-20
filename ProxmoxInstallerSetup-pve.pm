package ProxmoxInstallerSetup;

use strict;
use warnings;

sub setup {
    return {
	product => 'pve',
	release => '5.1',
	enable_btrfs => 0,
    };
}

1;


