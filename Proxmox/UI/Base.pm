package Proxmox::UI::Base;

use strict;
use warnings;

use Carp;

sub new {
    my ($this, $state) = @_;

    my $class = ref($this) || $this;

    my $self = bless { state => $state }, $class;

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

1;
