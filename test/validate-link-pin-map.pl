#!/usr/bin/env perl

use strict;
use warnings;

use Test::More;

use Proxmox::Sys::Net qw(validate_link_pin_map);

eval { validate_link_pin_map({ 'ab:cd:ef:12:34:56' => '' }) };
is($@, "interface name for 'ab:cd:ef:12:34:56' cannot be empty\n");

eval { validate_link_pin_map({ 'ab:cd:ef:12:34:56' => 'waytoolonginterfacename' }) };
is(
    $@,
    "interface name 'waytoolonginterfacename' for 'ab:cd:ef:12:34:56' cannot be longer than 15 characters\n",
);

eval { validate_link_pin_map({ 'ab:cd:ef:12:34:56' => 'nic0', '12:34:56:ab:cd:ef' => 'nic0' }) };
like($@, qr/^duplicate interface name mapping 'nic0' for: /);
like($@, qr/ab:cd:ef:12:34:56/);
like($@, qr/12:34:56:ab:cd:ef/);

eval { validate_link_pin_map({ 'ab:cd:ef:12:34:56' => 'nic-' }) };
is(
    $@,
    "interface name 'nic-' for 'ab:cd:ef:12:34:56' is invalid: name must only consist of alphanumeric characters and underscores\n",
);

eval { validate_link_pin_map({ 'ab:cd:ef:12:34:56' => '0nic' }) };
is(
    $@,
    "interface name '0nic' for 'ab:cd:ef:12:34:56' is invalid: name must not start with a number\n",
);

eval { validate_link_pin_map({ 'ab:cd:ef:12:34:56' => '12345' }) };
is(
    $@,
    "interface name '12345' for 'ab:cd:ef:12:34:56' is invalid: name must not be fully numeric\n",
);

done_testing();
