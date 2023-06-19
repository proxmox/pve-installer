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

sub display_html {
    my ($raw_html, $html_dir) = @_;

    # ignore for now
}

sub progress {
    my ($self, $ratio, $text) = @_;

    print STDOUT "progress: $ratio $text\n";
}

sub process_events {
    my ($self) = @_;

    # nothing to do for now?
}

1;
