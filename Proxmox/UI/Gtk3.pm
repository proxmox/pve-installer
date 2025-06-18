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

sub finished {
    my ($self, $success, $msg) = @_;
    # handled manually in proxinstall at the moment
}

sub prompt {
    my ($self, $query) = @_;

    my $window = $self->{state}->{window};
    my $dialog = Gtk3::MessageDialog->new($window, 'modal', 'question', 'ok-cancel', $query);
    my $response = $dialog->run();
    $dialog->destroy();

    return ($response // '') eq 'ok';
}

sub display_html {
    my ($self, $raw_html, $html_dir) = @_;

    my $html_view = $self->{state}->{html_view};

    # always set base-path to common path, all resources are accesible from there.
    $html_view->load_html($raw_html, "file://$html_dir/");
}

sub progress {
    my ($self, $ratio, $text) = @_;

    my ($bar, $status) = $self->{state}->@{ 'progress_bar', 'progress_status' };

    $bar->set_fraction($ratio);
    $bar->set_text(sprintf("%d%%", int($ratio * 100)));
    $status->set_text($text) if defined($text);
}

sub process_events {
    my ($self) = @_;
    Gtk3::main_iteration() while Gtk3::events_pending();
}

1;
