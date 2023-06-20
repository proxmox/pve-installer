package Proxmox::Sys::Command;

use strict;
use warnings;

use Carp;
use IO::File;
use IPC::Open3;
use IO::Select;
use String::ShellQuote;
use POSIX ":sys_wait_h";

use Proxmox::Install::ISOEnv;
use Proxmox::Log;
use Proxmox::UI;

use base qw(Exporter);
our @EXPORT_OK = qw(run_command syscmd);

my sub shellquote {
    my $str = shift;
    return String::ShellQuote::shell_quote($str);
}

my sub cmd2string {
    my ($cmd) = @_;

    die "no arguments" if !$cmd;
    return $cmd if !ref($cmd);

    my $quoted_args = [ map { shellquote($_) } $cmd->@* ];

    return join (' ', $quoted_args->@*);
}

sub syscmd {
    my ($cmd) = @_;

    return run_command($cmd, undef, undef, 1);
}

sub run_command {
    my ($cmd, $func, $input, $noout) = @_;

    my $cmdstr;
    if (!ref($cmd)) {
	$cmdstr = $cmd;
	if ($cmd =~ m/|/) {
	    # see 'man bash' for option pipefail
	    $cmd = [ '/bin/bash', '-c', "set -o pipefail && $cmd" ];
	} else {
	    $cmd = [ $cmd ];
	}
    } else {
	$cmdstr = cmd2string($cmd);
    }

    my $cmdtxt;
    if ($input && ($cmdstr !~ m/chpasswd/)) {
	$cmdtxt = "# $cmdstr <<EOD\n$input";
	chomp $cmdtxt;
	$cmdtxt .= "\nEOD\n";
    } else {
	$cmdtxt = "# $cmdstr\n";
    }

    if (is_test_mode()) {
	print $cmdtxt;
	STDOUT->flush();
    }
    log_info($cmdtxt);

    my ($reader, $writer, $error) = (IO::File->new(), IO::File->new(), IO::File->new());

    my $orig_pid = $$;

    my $pid = eval { open3($writer, $reader, $error, @$cmd) || die $!; };
    my $err = $@;

    if ($orig_pid != $$) { # catch exec errors
	POSIX::_exit (1);
	kill ('KILL', $$);
    }
    die $err if $err;

    print $writer $input if defined $input;
    close $writer;

    my $select = IO::Select->new();
    $select->add($reader);
    $select->add($error);

    my ($ostream, $logout) = ('', '', '');

    while ($select->count) {
	my @handles = $select->can_read (0.2);

	Proxmox::UI::process_events();

	next if !scalar (@handles); # timeout

	foreach my $h (@handles) {
	    my $buf = '';
	    my $count = sysread ($h, $buf, 4096);
	    if (!defined ($count)) {
		my $err = $!;
		kill (9, $pid);
		waitpid ($pid, 0);
		die "command '$cmd' failed: $err";
	    }
	    $select->remove($h) if !$count;
	    if ($h eq $reader) {
		$ostream .= $buf if !($noout || $func);
		$logout .= $buf;
		while ($logout =~ s/^([^\010\r\n]*)(\r|\n|(\010)+|\r\n)//s) {
		    my $line = $1;
		    $func->($line) if $func;
		}

	    } elsif ($h eq $error) {
		$ostream .= $buf if !($noout || $func);
	    }
	    print $buf;
	    STDOUT->flush();
	    log_info($buf);
	}
    }

    &$func($logout) if $func;

    my $rv = waitpid ($pid, 0);

    return $? if $noout; # behave like standard system();

    if ($? == -1) {
	croak "command '$cmdstr' failed to execute\n";
    } elsif (my $sig = ($? & 127)) {
	croak "command '$cmdstr' failed - got signal $sig\n";
    } elsif (my $exitcode = ($? >> 8)) {
	croak "command '$cmdstr' failed with exit code $exitcode";
    }

    return $ostream;
}

# forks and runs the provided coderef in the child
# do not use syscmd or run_command as both confuse the GTK mainloop if
# run from a child process
sub run_in_background {
    my ($cmd) = @_;

    my $pid = fork() // die "fork failed: $!\n";
    if (!$pid) {
	eval { $cmd->(); };
	if (my $err = $@) {
	    warn "run_in_background error: $err\n";
	    POSIX::_exit(1);
	}
	POSIX::_exit(0);
    }
}

1;
