package Proxmox::UI;

# a few simple abstractions to be a bit more general over what UI is in use, this is basically an
# UI², a User-Interface Interface

use strict;
use warnings;

use Carp;

use Proxmox::UI::Gtk3;
use Proxmox::UI::StdIO;

my ($_ui, $_env) = (undef, undef);

# state belongs fully to the UI
# env is a reference to the return value of Proxmox::Install::ISOEnv
sub init_gtk {
    my ($state, $env) = @_;

    croak "overriding existing UI!" if defined($_ui);

    $_ui = Proxmox::UI::Gtk3->new($state, $env);
    $_env = $env;

    return $_ui;
}
sub init_stdio {
    my ($state, $env) = @_;

    croak "overriding existing UI!" if defined($_ui);

    $_ui = Proxmox::UI::StdIO->new($state, $env);
    $_env = $env;

    return $_ui;
}

sub get_ui {
    return $_ui // croak "no UI initialized!";
}

my sub get_env {
    return $_env // croak "env not initialized!";
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
