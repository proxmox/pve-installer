#!/usr/bin/perl

use strict;
use warnings;

use File::Temp;
use Test::More;

use Proxmox::Sys::Command qw(run_command CMD_FINISHED);
use Proxmox::Sys::File qw(file_read_all);
use Proxmox::UI;

my $log_file = File::Temp->new();
Proxmox::Log::init($log_file->filename);

Proxmox::UI::init_stdio();

is(run_command('echo test'), "test\n", 'basic usage');

is(run_command('echo test', undef, undef, 1), 0, 'system()-mode');

my $ret = run_command('bash -c "echo test; sleep 1000; echo test"', sub {
    my $line = shift;
    is($line, 'test', 'using CMD_FINISHED - produced correct log line');

    return CMD_FINISHED;
});
is($ret, '', 'using CMD_FINISHED');

# https://bugzilla.proxmox.com/show_bug.cgi?id=4872
my $prev;
eval {
    local $SIG{ALRM} = sub { die "timed out!\n" };
    $prev = alarm(1);
    $ret = run_command('sleep 5');
};
alarm($prev);

is($@, "timed out!\n", 'SIGALRM interaction');

# Check the log for errors/warnings
my $log = file_read_all($log_file->filename);
ok($log !~ m/(WARN|ERROR): /, 'no warnings or errors logged');
print $log if $log =~ m/(WARN|ERROR): /;

done_testing();
