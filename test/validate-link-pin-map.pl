#!/usr/bin/env perl

use strict;
use warnings;

use Test::More;

use Proxmox::Sys::Net qw(validate_link_pin_map);

eval { validate_link_pin_map({ 'ab:cd:ef:12:34:56' => '' }) };
is(
    $@,
    "interface name for 'ab:cd:ef:12:34:56' must be at least 2 characters long\n",
    "empty name is rejected",
);

eval { validate_link_pin_map({ 'ab:cd:ef:12:34:56' => 'a' }) };
is(
    $@,
    "interface name for 'ab:cd:ef:12:34:56' must be at least 2 characters long\n",
    "1 character name is rejected",
);

eval { validate_link_pin_map({ 'ab:cd:ef:12:34:56' => 'waytoolonginterfacename' }) };
is(
    $@,
    "interface name 'waytoolonginterfacename' for 'ab:cd:ef:12:34:56' cannot be longer than 15 characters\n",
    "too long name is rejected",
);

eval { validate_link_pin_map({ 'ab:cd:ef:12:34:56' => 'nic0', '12:34:56:ab:cd:ef' => 'nic0' }) };
like(
    $@, qr/^duplicate interface name mapping 'nic0' for: /, "duplicate assignment is rejected",
);
like($@, qr/ab:cd:ef:12:34:56/, "duplicate error message contains expected mac address");
like($@, qr/12:34:56:ab:cd:ef/, "duplicate error message contains expected mac address");

eval { validate_link_pin_map({ 'ab:cd:ef:12:34:56' => 'nic-' }) };
is(
    $@,
    "interface name 'nic-' for 'ab:cd:ef:12:34:56' is invalid: name must start with a letter and "
        . "contain only ascii characters, digits and underscores\n",
    "name with dash is rejected",
);

eval { validate_link_pin_map({ 'ab:cd:ef:12:34:56' => '0nic' }) };
is(
    $@,
    "interface name '0nic' for 'ab:cd:ef:12:34:56' is invalid: name must start with a letter and "
        . "contain only ascii characters, digits and underscores\n",
    "name starting with number is rejected",
);

eval { validate_link_pin_map({ 'ab:cd:ef:12:34:56' => '_a' }) };
is(
    $@,
    "interface name '_a' for 'ab:cd:ef:12:34:56' is invalid: name must start with a letter and "
        . "contain only ascii characters, digits and underscores\n",
    "name starting with underscore is rejected",
);

eval { validate_link_pin_map({ 'ab:cd:ef:12:34:56' => '12345' }) };
is(
    $@,
    "interface name '12345' for 'ab:cd:ef:12:34:56' is invalid: name must start with a letter and "
        . "contain only ascii characters, digits and underscores\n",
    "fully numeric name is rejected",
);

done_testing();
