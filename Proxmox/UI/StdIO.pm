package Proxmox::UI::StdIO;

use strict;
use warnings;

use JSON qw(from_json to_json);

use base qw(Proxmox::UI::Base);

use Proxmox::Log;

my sub send_msg : prototype($$) {
    my ($type, %values) = @_;

    my $json = to_json({ type => $type, %values }, { utf8 => 1, canonical => 1 });
    print STDOUT "$json\n";
}

my sub recv_msg : prototype() {
    my $response = <STDIN> // ''; # FIXME: error handling?
    chomp($response);

    return eval { from_json($response, { utf8 => 1 }) };
}

sub init {
    my ($self) = @_;

    STDOUT->autoflush(1);
}

sub message {
    my ($self, $msg) = @_;

    &send_msg('message', message => $msg);
}

sub error {
    my ($self, $msg) = @_;

    log_error("error: $msg");
    &send_msg('error', message => $msg);
}

sub finished {
    my ($self, $success, $msg) = @_;

    my $state = $success ? 'ok' : 'err';
    log_info("finished: $state, $msg");
    &send_msg('finished', state => $state, message => $msg);
}

sub prompt {
    my ($self, $query) = @_;

    &send_msg('prompt', query => $query);
    my $response = &recv_msg();

    if (defined($response) && $response->{type} eq 'prompt-answer') {
	return lc($response->{answer}) eq 'ok';
    }
}

sub display_html {
    my ($raw_html, $html_dir) = @_;

    log_error("display_html() not available for stdio backend!");
}

sub progress {
    my ($self, $ratio, $text) = @_;

    $text = '' if !defined($text);

    &send_msg('progress', ratio => $ratio, text => $text);
}

sub process_events {
    my ($self) = @_;

    # nothing to do for now?
}

1;
