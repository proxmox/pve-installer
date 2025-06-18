package Proxmox::Sys::File;

use strict;
use warnings;

use IO::File;

use base qw(Exporter);
our @EXPORT_OK = qw(file_read_firstline file_read_all file_write_all);

sub file_read_firstline {
    my ($filename) = @_;

    my $fh = IO::File->new($filename, "r");
    return if !$fh;

    my $res = <$fh>;
    chomp $res if $res;
    $fh->close;

    return $res;
}

sub file_read_all {
    my ($filename) = @_;

    my $fh = IO::File->new($filename, "r") || die "can't open '$filename' - $!\n";

    local $/; # slurp mode
    my $content = <$fh>;
    close $fh;

    return $content;
}

sub file_write_all {
    my ($filename, $content) = @_;

    my $fd = IO::File->new(">$filename") || die "unable to open file '$filename' - $!\n";
    print $fd $content;
    $fd->close();
}

1;
