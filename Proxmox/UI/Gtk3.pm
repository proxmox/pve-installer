package Proxmox::UI::Gtk3;

use strict;
use warnings;

use Gtk3;

use base qw(Proxmox::UI::Base);

sub message {
    my ($self, $msg) = @_;

    my $window = $self->{state}->{window};
    my $dialog = Gtk3::MessageDialog->new($window, 'modal', 'info', 'ok', $msg);
    $dialog->run();
    $dialog->destroy();
}

sub error {
    my ($self, $msg) = @_;

    my $window = $self->{state}->{window};
    my $dialog = Gtk3::MessageDialog->new($window, 'modal', 'error', 'ok', $msg);
    $dialog->run();
    $dialog->destroy();
}

sub prompt {
    my ($self, $query) = @_;

    my $window = $self->{state}->{window};
    my $dialog = Gtk3::MessageDialog->new($window, 'modal', 'question', 'ok-cancel', $query);
    my $response = $dialog->run();
    $dialog->destroy();

    return ($response // '') eq 'ok';
}

1;
