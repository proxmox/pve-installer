#!/usr/bin/perl

use strict;
use warnings;

use JSON qw(from_json);

use Proxmox::Log;
use Proxmox::UI;

pipe(my $parent_reader, my $child_writer) || die "failed to create r/w pipe - $!\n";
pipe(my $child_reader, my $parent_writer) || die "failed to create w/r pipe - $!\n";

$parent_writer->autoflush(1);
$child_writer->autoflush(1);
$child_reader->autoflush(1);

my $child_pid = fork() // die "fork failed - $!\n";

if ($child_pid) {
    eval 'use Test::More tests => 3;';

    # parent, the hypothetical low-level installer
    close($parent_reader);
    close($parent_writer);

    # "mock" stdin and stdout for Proxmox::UI::StdIO
    *STDIN = $child_reader;
    *STDOUT = $child_writer;

    Proxmox::Log::init('&STDERR'); # log to stderr
    Proxmox::UI::init_stdio({}, {});

    Proxmox::UI::message('foo');
    Proxmox::UI::error('bar');
    is(Proxmox::UI::prompt('baz?'), 1, 'prompt should get positive answer');
    is(Proxmox::UI::prompt('not baz? :('), '', 'prompt should get negative answer');

    Proxmox::UI::finished(1, 'install successful');
    Proxmox::UI::finished(0, 'install failed');

    Proxmox::UI::progress(0.2, 0, 1, '20% done');
    Proxmox::UI::progress(0.2, 0, 1);
    Proxmox::UI::progress(0.99, 0, 1, '99% done');
    Proxmox::UI::progress(1, 0, 1, 'done');

    waitpid($child_pid, 0);
    is($?, 0); # check child exit status
    done_testing();
} else {
    eval 'use Test::More tests => 10;';

    # child, e.g. the TUI
    close($child_reader);
    close($child_writer);

    my $next_msg = sub {
	chomp(my $msg = <$parent_reader>);
	return from_json($msg, { utf8 => 1 });
    };

    is_deeply($next_msg->(), { type => 'message', message => 'foo' }, 'should receive message');
    is_deeply($next_msg->(), { type => 'error', message => 'bar' }, 'should receive error');

    is_deeply($next_msg->(), { type => 'prompt', query => 'baz?' }, 'prompt works');
    print $parent_writer "{\"type\":\"prompt-answer\",\"answer\":\"ok\"}\n";

    is_deeply($next_msg->(), { type => 'prompt', query => 'not baz? :(' }, 'prompt works');
    print $parent_writer "{\"type\":\"prompt-answer\",\"answer\":\"cancel\"}\n";

    is_deeply(
	$next_msg->(), { type => 'finished', state => 'ok', message => 'install successful'},
	'should receive successful finished message');

    is_deeply(
	$next_msg->(), { type => 'finished', state => 'err', message => 'install failed'},
	'should receive failed finished message');

    is_deeply(
	$next_msg->(), { type => 'progress', ratio => 0.2, text => '20% done' },
	'should get 20% done progress message');

    is_deeply(
	$next_msg->(), { type => 'progress', ratio => 0.2, text => '' },
	'should get progress continuation message');

    is_deeply(
	$next_msg->(), { type => 'progress', ratio => 0.99, text => '99% done' },
	'should get 99% done progress message');

    is_deeply(
	$next_msg->(), { type => 'progress', ratio => 1, text => 'done' },
	'should get 100% done progress message');

    done_testing();
}

