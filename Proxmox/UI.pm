package Proxmox::UI;

# a few simple abstractions to be a bit more general over what UI is in use, this is basically an
# UI², a User-Interface Interface

use strict;
use warnings;

use Carp;

use Proxmox::UI::Gtk3;
use Proxmox::UI::StdIO;

my $ui = undef;

sub init_gtk {
    my ($state) = @_;

    croak "overriding existing UI!" if defined($ui);

    $ui = Proxmox::UI::Gtk3->new($state);

    return $ui;
}
sub init_stdio {
    my ($state) = @_;

    croak "overriding existing UI!" if defined($ui);

    $ui = Proxmox::UI::StdIO->new($state);

    return $ui;
}

sub get_ui {
    return $ui // croak "no UI initialized!";
}

sub message {
    my ($msg) = @_;
    get_ui()->message($msg);
}

sub error {
    my ($msg) = @_;
    get_ui()->error($msg);
}

sub prompt {
    my ($query) = @_;
    return get_ui()->prompt($query);
}

}

1;
