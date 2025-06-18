#!/usr/bin/perl

use strict;
use warnings;

use PVE::Tools;
use JSON qw(from_json to_json);

# Generates a
#
#   - country code => name/kmap/mirror
#   - name => country code
#
# mapping for each defined country
my sub generate_country_mappings {
    my ($country_codes, $defmap, $mirrors) = @_;

    my ($countries, $countryhash) = ({}, {});
    foreach my $cc (sort keys %$country_codes) {
        my $name = $country_codes->{$cc};
        my $kmap = $defmap->{$cc} || '';
        my $mirror = $mirrors->{$cc} || '';

        $countries->{$cc} = {
            name => $name,
            kmap => $kmap,
            mirror => $mirror,
        };
        $countryhash->{ lc($name) } = $cc;
    }

    return ($countries, $countryhash);
}

# we need mappings for X11, console, and kvm vnc
# LC(-LC)? => [DESC, kvm, console, X11, X11variant]
my sub generate_keymaps {
    my ($country_codes) = @_;

    my ($kmap, $kmaphash) = ({}, {});
    my $keymaps = PVE::Tools::kvmkeymaps();
    foreach my $km (sort keys %$keymaps) {
        my ($name, $kvm, $console, $x11, $x11var) = @{ $keymaps->{$km} };

        if ($km =~ m/^([a-z][a-z])-([a-z][a-z])$/i) {
            defined($country_codes->{$2}) || die "undefined country code '$2'";
        } else {
            defined($country_codes->{$km}) || die "undefined country code '$km'";
        }

        $x11var = '' if !defined($x11var);

        $kmap->{$km} = {
            name => $name,
            kvm => $kvm,
            console => $console,
            x11 => $x11,
            x11var => $x11var,
        };
        $kmaphash->{$name} = $km;
    }

    return ($kmap, $kmaphash);
}

my sub parse_zoneinfo {
    my ($countries) = @_;

    my $zonefn = "/usr/share/zoneinfo/zone.tab";
    open(my $ZONE_TAB_FH, '<', "$zonefn") || die "unable to open '$zonefn' - $!\n";

    my ($zones, $cczones) = ({}, {});
    while (defined(my $line = <$ZONE_TAB_FH>)) {
        next if $line =~ m/^\s*(?:#|$)/;
        if ($line =~ m|^([A-Z][A-Z])\s+\S+\s+(([^/]+)/\S+)\s|) {
            my $cc = lc($1);
            $cczones->{$cc}->{$2} = 1;
            $countries->{$cc}->{zone} = $2 if !defined($countries->{$cc}->{zone});
            $zones->{$2} = 1;

        }
    }
    close($ZONE_TAB_FH);

    return ($zones, $cczones);
}

# country codes from:
my $country_codes_file = "/usr/share/iso-codes/json/iso_3166-1.json";

my $iso_3166_codes =
    from_json(PVE::Tools::file_get_contents($country_codes_file, 64 * 1024), { utf8 => 1 });

my $country_codes = { map { lc($_->{'alpha_2'}) => $_->{'common_name'} // $_->{'name'} }
    @{ $iso_3166_codes->{'3166-1'} } };

my $defmap = {
    'us' => 'en-us',
    'be' => 'fr-be',
    'br' => 'pt-br',
    'ca' => 'en-us',
    'dk' => 'dk',
    'nl' => 'en-us', # most Dutch people use US layout
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
    die "undefined country code '$cc'" if !defined($country_codes->{$cc});
}

my ($countries, $countryhash) = generate_country_mappings($country_codes, $defmap, $mirrors);
my ($kmap, $kmaphash) = generate_keymaps($country_codes);
my ($zones, $cczones) = parse_zoneinfo($countries);

my $locale_info = {
    country => $countries,
    countryhash => $countryhash,
    kmap => $kmap,
    kmaphash => $kmaphash,
    zones => $zones,
    cczones => $cczones,
};

my $json = to_json($locale_info, { utf8 => 1, canonical => 1 });
print $json;
