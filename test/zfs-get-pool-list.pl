#!/usr/bin/perl

use strict;
use warnings;

use File::Temp;
use Test::More tests => 8;

use Proxmox::Sys::ZFS;
use Proxmox::UI;

my $log_file = File::Temp->new();
Proxmox::Log::init($log_file->filename);

Proxmox::UI::init_stdio();

our $ZPOOL_IMPORT_TEST_OUTPUT = <<EOT;
   pool: testpool
     id: 4958685680270539150
  state: ONLINE
 action: The pool can be imported using its name or numeric identifier.
 config:

        testpool    ONLINE
          vdc       ONLINE
          vdd       ONLINE

   pool: rpool
     id: 9412322616744093413
  state: FAULTED
status: The pool was last accessed by another system.
 action: The pool can be imported using its name or numeric identifier and
        the '-f' flag.
   see: https://openzfs.github.io/openzfs-docs/msg/ZFS-8000-EY
 config:

        rpool       ONLINE
          mirror-0  ONLINE
            vda3    ONLINE
            vdb3    ONLINE
EOT

my $pools = {
    testpool => { id => '4958685680270539150', state => 'ONLINE' },
    rpool => { id => '9412322616744093413', state => 'FAULTED' },
};

open(my $fh, '<', \$ZPOOL_IMPORT_TEST_OUTPUT);
my $result = Proxmox::Sys::ZFS::zpool_import_parse_output($fh);
while (my ($name, $info) = each %$pools) {
    my ($p) = grep { $_->{name} eq $name } @$result;
    ok(defined($p), "pool $name was found");
    is($p->{id}, $info->{id}, "pool $name has correct id");
    is($p->{state}, $info->{state}, "pool $name has correct state");
    like(
        $p->{action},
        qr/^The pool can be imported using its name or numeric identifier/,
        "pool $name can be imported",
    );
}
