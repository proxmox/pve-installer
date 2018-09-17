#!/usr/bin/perl

use strict;
use warnings;

use PVE::Tools;
use JSON;

# country codes from:
my $country_codes_file = "/usr/share/iso-codes/json/iso_3166-1.json";

my $iso_3166_codes = from_json(PVE::Tools::file_get_contents($country_codes_file, 64 * 1024));

my $country = { map { lc($_->{'alpha_2'}) => $_->{'common_name'} // $_->{'name'} } @{$iso_3166_codes->{'3166-1'}} };

# we need mappings for X11, console, and kvm vnc

# LC(-LC)? => [DESC, kvm, console, X11, X11variant]
my $keymaps = PVE::Tools::kvmkeymaps();

foreach my $km (sort keys %$keymaps) {
    my ($desc, $kvm, $console, $x11, $x11var) = @{$keymaps->{$km}};

    if ($km =~m/^([a-z][a-z])-([a-z][a-z])$/i) {
	defined ($country->{$2}) || die "undefined country code '$2'";
    } else {
	defined ($country->{$km}) || die "undefined country code '$km'";
    }

    $x11var = '' if !defined ($x11var);
    print "map:$km:$desc:$kvm:$console:$x11:$x11var:\n";
}

my $defmap = {
   'us' => 'en-us',
   'be' => 'fr-be',
   'br' => 'pt-br',
   'ca' => 'en-us',
   'dk' => 'dk',
   'nl' => 'en-us', # most Dutch people us US layout
   'fi' => 'fi',
   'fr' => 'fr',
   'de' => 'de',
   'at' => 'de',
   'hu' => 'hu',
   'is' => 'is',
   'it' => 'it',
   'va' => 'it',
   'jp' => 'jp',
   'lt' => 'lt',
   'mk' => 'mk',
   'no' => 'no',
   'pl' => 'pl',
   'pt' => 'pt',
   'si' => 'si',
   'es' => 'es',
   'gi' => 'es',
   'ch' => 'de-ch',
   'gb' => 'en-gb',
   'lu' => 'fr-ch',
   'li' => 'de-ch',
};


my $mirrors = PVE::Tools::debmirrors();
foreach my $cc (keys %$mirrors) {
    die "undefined country code '$cc'" if !defined ($country->{$cc});
}

foreach my $cc (sort keys %$country) {
    my $map = $defmap->{$cc} || '';
    my $mir = $mirrors->{$cc} || '';
    print "$cc:$country->{$cc}:$map:$mir:\n";
}
