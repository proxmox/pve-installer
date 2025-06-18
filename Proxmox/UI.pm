package Proxmox::UI;

# a few simple abstractions to be a bit more general over what UI is in use, this is basically an
# UI², a User-Interface Interface

use strict;
use warnings;

use Carp;

use Proxmox::Sys::File qw(file_read_all);

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

sub finished {
    my ($success, $msg) = @_;
    get_ui()->finished(!!$success, $msg);
}

sub prompt {
    my ($query) = @_;
    return get_ui()->prompt($query);
}

sub display_html {
    my ($filename, $transform) = @_;

    my $env = get_env();
    my $html_dir = "$env->{locations}->{lib}/html";

    my $path;
    if (-f "$html_dir/$env->{product}/$filename") {
        $path = "$html_dir/$env->{product}/$filename";
    } else {
        $path = "$html_dir/$filename";
    }

    my $raw_html = file_read_all($path);

    $raw_html = $transform->($raw_html, $env) if $transform;

    $raw_html =~ s/__FULL_PRODUCT_NAME__/$env->{cfg}->{fullname}/g;

    return get_ui()->display_html($raw_html, $html_dir);
}

sub progress {
    my ($frac, $start, $end, $text) = @_;

    my $part = $end - $start;
    my $ratio = $start + $frac * $part;

    get_ui()->progress($ratio, $text);

    return $ratio;
}

sub process_events {
    get_ui()->process_events();
}

1;
