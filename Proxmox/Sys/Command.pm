package Proxmox::Sys::Command;

use strict;
use warnings;

use Carp;
use IO::File;
use IPC::Open3;
use IO::Select;
use String::ShellQuote;
use POSIX ":sys_wait_h";
use Time::HiRes qw(usleep);

use Proxmox::Install::ISOEnv;
use Proxmox::Log;
use Proxmox::UI;

use base qw(Exporter);
our @EXPORT_OK = qw(run_command syscmd CMD_FINISHED);

use constant {
    CMD_RESERVED => 1 << 0, # reserve 1 as it's often the default return value of closures
    CMD_FINISHED => 1 << 1,
};

my sub shellquote {
    my $str = shift;
    return String::ShellQuote::shell_quote($str);
}

my sub cmd2string {
    my ($cmd) = @_;

    die "no arguments" if !$cmd;
    return $cmd if !ref($cmd);

    my $quoted_args = [map { shellquote($_) } $cmd->@*];

    return join(' ', $quoted_args->@*);
}

# Safely for the (sub-)process specified by $pid to exit, using a timeout.
#
# When kill => 1 is set, at first a TERM-signal is sent to the process before
# checking if it exited.
# If that fails, KILL is sent to process and then up to timeout => $timeout
# seconds (default: 5) are waited for the process to exit.
#
# On sucess, the exitcode of the process is returned, otherwise `undef` (aka.
# the process was unkillable).
my sub wait_for_process {
    my ($pid, %params) = @_;

    kill('TERM', $pid) if $params{kill};

    my $timeout = ($params{timeout} // 5) * 20; # waiting 0.05 secs per loop
    for (0 .. $timeout) {
        my $terminated = waitpid($pid, WNOHANG);
        return $? if $terminated > 0;

        usleep(50_000) if $_ != $timeout; # sleep 0.05 sec, on all but last round
        kill('KILL', $pid) if $params{kill} && $_ == 10; # just once after 0.5s
    }

    log_warn("failed to kill child pid $pid, probably stuck in D-state?\n");

    # We tried our best, better let the child hang in the back then completely
    # blocking installer progress .. it's a rather short-lived environment anyway
}

sub syscmd {
    my ($cmd) = @_;

    return run_command($cmd, undef, undef, 1);
}

# Runs a command an a subprocess, properly handling IO via piping, cleaning up and passing back the
# exit code.
#
# If $cmd contains a pipe |, the command will be executed inside a bash shell.
# If $cmd contains 'chpasswd', the input will be specially quoted for that purpose.
#
# Arguments:
# * $cmd - The command to run, either a single string or array with individual arguments
# * $func - Logging subroutine to call, receives both stdout and stderr.
#           Can return CMD_FINISHED to exit early and ignore the rest of the process output.
#           Should use an explicit  `return;` to avoid misinterpretation of return value.
# * $input - Stdin contents for the spawned subprocess
# * $noout - Whether to append any process output to the return value
# * $noprint - Whether to print any process output to the parents stdout
sub run_command {
    my ($cmd, $func, $input, $noout, $noprint) = @_;

    my $cmdstr;
    if (!ref($cmd)) {
        $cmdstr = $cmd;
        if ($cmd =~ m/|/) {
            # see 'man bash' for option pipefail
            $cmd = ['/bin/bash', '-c', "set -o pipefail && $cmd"];
        } else {
            $cmd = [$cmd];
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
        POSIX::_exit(1);
        kill('KILL', $$);
    }
    die $err if $err;

    print $writer $input if defined $input;
    close $writer;

    my $select = IO::Select->new();
    $select->add($reader);
    $select->add($error);

    my ($ostream, $logout) = ('', '', '');
    my $caught_sig;

    while ($select->count) {
        my @handles = $select->can_read(0.2);

        # If we catch a signal, stop processing & clean up
        if ($!{EINTR}) {
            $caught_sig = 1;
            last;
        }

        Proxmox::UI::process_events();

        next if !scalar(@handles); # timeout

        foreach my $h (@handles) {
            my $buf = '';
            my $count = sysread($h, $buf, 4096);
            if (!defined($count)) {
                my $err = $!;
                wait_for_process($pid, kill => 1);
                die "command '$cmd' failed: $err";
            }
            $select->remove($h) if !$count;
            if ($h eq $reader) {
                $ostream .= $buf if !($noout || $func);
                $logout .= $buf;
                while ($logout =~ s/^([^\010\r\n]*)(\r|\n|(\010)+|\r\n)//s) {
                    my $line = $1;
                    if ($func) {
                        my $ret = $func->($line);
                        if (defined($ret) && $ret == CMD_FINISHED) {
                            wait_for_process($pid, kill => 1);
                            return $ostream;
                        }
                    }
                }

            } elsif ($h eq $error) {
                $ostream .= $buf if !($noout || $func);
            }
            print $buf if !$noprint;
            STDOUT->flush();
            log_info($buf);
        }
    }

    &$func($logout) if $func;

    my $ec = wait_for_process($pid, kill => $caught_sig);

    # behave like standard system(); returns -1 in case of errors too
    return ($ec // -1) if $noout;

    if (!defined($ec)) {
        # Don't fail completely here to let the install continue
        warn "command '$cmdstr' failed to exit properly\n";
    } elsif ($ec == -1) {
        croak "command '$cmdstr' failed to execute\n";
    } elsif (my $sig = ($ec & 127)) {
        croak "command '$cmdstr' failed - got signal $sig\n";
    } elsif (my $exitcode = ($ec >> 8)) {
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
