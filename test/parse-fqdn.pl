#!/usr/bin/env perl

use strict;
use warnings;

use Test::More;

use Proxmox::Sys::Net qw(parse_fqdn);

# some constants just to avoid repeating ourselves
use constant ERR_INVALID => "Hostname does not look like a fully qualified domain name\n";
use constant ERR_ALPHANUM => "FQDN must only consist of alphanumeric characters and dashes\n";
use constant ERR_NUMERIC => "Purely numeric hostnames are not allowed\n";
use constant ERR_TOOLONG => "FQDN too long\n";
use constant ERR_EMPTY => "FQDN cannot be empty\n";

sub is_parsed {
    my ($fqdn, $expected) = @_;

    my @parsed = parse_fqdn($fqdn);
    is_deeply(\@parsed, $expected, "FQDN is valid and compared successfully: $fqdn");
}

sub is_invalid {
    my ($fqdn, $expected_err) = @_;

    my $parsed = eval { parse_fqdn($fqdn) };
    is($parsed, undef, "invalid FQDN did fail parsing: $fqdn");
    is($@, $expected_err, "invalid FQDN threw correct error: $fqdn");
}

is_invalid(undef, ERR_EMPTY);
is_invalid('', ERR_EMPTY);

is_parsed('foo.example.com', ['foo', 'example.com']);
is_parsed('foo-bar.com', ['foo-bar', 'com']);
is_parsed('a-b.com', ['a-b', 'com']);

is_invalid('foo', ERR_INVALID);
is_invalid('-foo.com', ERR_ALPHANUM);
is_invalid('foo-.com', ERR_ALPHANUM);
is_invalid('foo.com-', ERR_ALPHANUM);
is_invalid('-o-.com', ERR_ALPHANUM);

# https://bugzilla.proxmox.com/show_bug.cgi?id=1054
is_invalid('123.com', ERR_NUMERIC);
is_parsed('foo123.com', ['foo123', 'com']);
is_parsed('123foo.com', ['123foo', 'com']);

is_parsed('a' x 63 . '.com', ['a' x 63, 'com']);
is_invalid('a' x 250 . '.com', ERR_TOOLONG);
is_invalid('a' x 64 . '.com', ERR_ALPHANUM);

# https://bugzilla.proxmox.com/show_bug.cgi?id=5230
is_invalid('123@foo.com', ERR_ALPHANUM);

done_testing();
