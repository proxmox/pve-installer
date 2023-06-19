package Proxmox::UI::StdIO;

use strict;
use warnings;

use base qw(Proxmox::UI::Base);

sub message {
    my ($self, $msg) = @_;

    print STDOUT "message: $msg\n";
}

sub error {
    my ($self, $msg) = @_;

    print STDOUT "error: $msg\n";
}

sub prompt {
    my ($self, $query) = @_;

    print STDOUT "prompt: $query\n";

    my $response = <STDIN> // ''; # FIXME: error handling?

    chomp($response);

    return lc($response) eq 'ok';
}

1;
