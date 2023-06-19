package Proxmox::Log;

use strict;
use warnings;

use Carp;
use POSIX qw(strftime);
use Time::HiRes qw(gettimeofday);

use base qw(Exporter);
our @EXPORT = qw(log_debug log_error log_info log_notice log_warn);

my $log_fd;
sub init {
    my ($log_file) = @_;
    croak "log fd is already defined, refuse to reinitialize!" if defined($log_fd);
    $log_file = "/tmp/install.log" if !defined($log_file);
    $log_fd = IO::File->new(">${log_file}") or croak "could not open log file - $!\n";
}

my sub iso_like_date_time {
    my ($now_seconds, $now_microseconds) = gettimeofday;
    my $now_millis = int($now_microseconds / 1000);
    return strftime("%F %H:%M:%S", localtime($now_seconds)) . sprintf(".%03d", $now_millis);
}

my sub _log {
    my ($level, $message) = @_;

    my $fd = $log_fd;
    if (!defined($log_fd)) {
	carp "log FD not initialized, falling back to stderr";
	$fd = *STDERR;
    }

    my $date_time = iso_like_date_time();
    $level = uc($level);
    chomp $message;

    print $log_fd "$date_time $level: $message\n";
}

sub log_cmd { _log('cmd', @_); }
sub log_debug { _log('debug', @_); }
sub log_error { _log('error', @_); }
sub log_info { _log('info', @_); }
sub log_warn { _log('warn', @_); }

1;
