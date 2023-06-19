package Proxmox::UI::Base;

use strict;
use warnings;

use Carp;

# state belongs fully to the UI
# env is a reference to the return value of Proxmox::Install::ISOEnv
sub new {
    my ($this, $state, $env) = @_;

    my $class = ref($this) || $this;

    my $self = bless {
	state => $state,
	env => $env,
    }, $class;

    return $self;
}

sub message {
    my ($self, $msg) = @_;

    croak "implement me in sub-class";
}

sub error {
    my ($self, $msg) = @_;

    croak "implement me in sub-class";
}

sub prompt {
    my ($self, $query) = @_;

    croak "implement me in sub-class";
}

sub display_html {
    my ($self, $raw_html, $html_dir) = @_;

    croak "implement me in sub-class";
}

1;
