package Proxmox::UI::StdIO;

use strict;
use warnings;

use base qw(Proxmox::UI::Base);

use Proxmox::Log;

sub init {
    my ($self) = @_;

    STDOUT->autoflush(1);
}

sub message {
    my ($self, $msg) = @_;

    print STDOUT "message: $msg\n";
}

sub error {
    my ($self, $msg) = @_;
    log_err("error: $msg\n");
    print STDOUT "error: $msg\n";
}

sub finished {
    my ($self, $success, $msg) = @_;

    my $state = $success ? 'ok' : 'err';
    log_info("finished: $state, $msg\n");
    print STDOUT "finished: $state, $msg\n";
}

sub prompt {
    my ($self, $query) = @_;

    $query =~ s/\n/ /g; # FIXME: use a better serialisation (e.g., JSON)
    print STDOUT "prompt: $query\n";

    my $response = <STDIN> // ''; # FIXME: error handling?

    chomp($response);

    return lc($response) eq 'ok';
}

sub display_html {
    my ($raw_html, $html_dir) = @_;

    log_error("display_html() not available for stdio backend!");
}

sub progress {
    my ($self, $ratio, $text) = @_;

    $text = '' if !defined($text);

    print STDOUT "progress: $ratio $text\n";
}

sub process_events {
    my ($self) = @_;

    # nothing to do for now?
}

1;
