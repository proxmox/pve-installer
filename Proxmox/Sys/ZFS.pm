package Proxmox::Sys::ZFS;

use strict;
use warnings;

use Proxmox::Sys::Command qw(run_command);

use base qw(Exporter);
our @EXPORT_OK = qw(get_exported_pools);

# Parses the output of running `zpool import`, which shows all importable
# ZFS pools.
# Unfortunately, `zpool` does not support JSON or any other machine-readable
# format for output, so we have to do it the hard way.
#
# Example output of `zpool import` looks like this:
#
#   root@proxmox:/# zpool import
#      pool: testpool
#        id: 4958685680270539150
#     state: ONLINE
#    action: The pool can be imported using its name or numeric identifier.
#    config:
#
#           testpool    ONLINE
#             vdc       ONLINE
#             vdd       ONLINE
#
#      pool: rpool
#        id: 9412322616744093413
#     state: ONLINE
#   status: The pool was last accessed by another system.
#    action: The pool can be imported using its name or numeric identifier and
#           the '-f' flag.
#      see: https://openzfs.github.io/openzfs-docs/msg/ZFS-8000-EY
#    config:
#
#           rpool       ONLINE
#             mirror-0  ONLINE
#               vda3    ONLINE
#               vdb3    ONLINE
#
sub zpool_import_parse_output {
    my ($fh) = @_;

    my $pools = []; # all found pools
    my $pool = {}; # last found pool in output

    while (my $line = <$fh>) {
	# first, if we find the start of a new pool, add it to the list with
	# its name
	if ($line =~ /^\s+pool: (.+)$/) {
	    # push the previous parsed pool to the result list
	    push @$pools, $pool if %$pool;
	    $pool = { name => $1 };
	    next;
	}

	# ignore any (garbage) output before the actual list, just in case
	next if !%$pool;

	# add any possibly-useful attribute to the last (aka. current) pool
	if ($line =~ /^\s*(id|state|status|action): (.+)$/) {
	    chomp($pool->{$1} = $2);
	    next;
	}
    }

    # add the final parsed pool to the list
    push @$pools, $pool if %$pool;
    return $pools;
}

# Returns an array of all importable ZFS pools on the system.
# Each entry is a hash of the format:
#
# {
#     name => '<pool name>',
#     id => <numeric pool-id>,
#     /* see also zpool_state_to_name() in lib/libzfs/libzfs_pool.c /*
#     state => 'OFFLINE' | 'REMOVED' | 'FAULTED' | 'SPLIT' | 'UNAVAIL' \
#	| 'FAULTED' | 'DEGRADED' | 'ONLINE',
#     status => '<human-readable status string>', optional
#     action => '<human-readable action string>', optional
# }
sub get_exported_pools {
    my $raw = run_command(['zpool', 'import']);
    open (my $fh, '<', \$raw) or die 'failed to open in-memory stream';

    return zpool_import_parse_output($fh);
}

1;
