package ProxmoxInstallerSetup;

use strict;
use warnings;

sub setup {
    return {
	product => 'pmg',
	release => '5.0',
	enable_btrfs => 0,
    };
}

1;


