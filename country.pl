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

my $keymaps = {
    'dk' => ['Danish', 'da', 'qwerty/dk-latin1.kmap.gz', 'dk', 'nodeadkeys'],
    'de' => ['German', 'de', 'qwertz/de-latin1-nodeadkeys.kmap.gz', 'de', 'nodeadkeys'],
    'de-ch' => ['Swiss-German', 'de-ch', 'qwertz/sg-latin1.kmap.gz', 'ch', 'de_nodeadkeys'],
    'en-gb' => ['United Kingdom', 'en-gb', 'qwerty/uk.kmap.gz', 'gb', undef],
    'en-us' => ['U.S. English', 'en-us', 'qwerty/us-latin1.kmap.gz', 'us', undef],
    'es' => ['Spanish', 'es', 'qwerty/es.kmap.gz', 'es', 'nodeadkeys'],
    #'et'     => [], # Ethopia or Estonia ??
    'fi' => ['Finnish', 'fi', 'qwerty/fi-latin1.kmap.gz', 'fi', 'nodeadkeys'],
    #'fo'     => ['Faroe Islands', 'fo', ???, 'fo', 'nodeadkeys'],
    'fr' => ['French', 'fr', 'azerty/fr-latin1.kmap.gz', 'fr', 'nodeadkeys'],
    'fr-be' => ['Belgium-French', 'fr-be', 'azerty/be2-latin1.kmap.gz', 'be', 'nodeadkeys'],
    'fr-ca' => ['Canada-French', 'fr-ca', 'qwerty/cf.kmap.gz', 'ca', 'fr-legacy'],
    'fr-ch' => ['Swiss-French', 'fr-ch', 'qwertz/fr_CH-latin1.kmap.gz', 'ch', 'fr_nodeadkeys'],
    #'hr'     => ['Croatia', 'hr', 'qwertz/croat.kmap.gz', 'hr', ??], # latin2?
    'hu' => ['Hungarian', 'hu', 'qwertz/hu.kmap.gz', 'hu', undef],
    'is' => ['Icelandic', 'is', 'qwerty/is-latin1.kmap.gz', 'is', 'nodeadkeys'],
    'it' => ['Italian', 'it', 'qwerty/it2.kmap.gz', 'it', 'nodeadkeys'],
    'jp' => ['Japanese', 'ja', 'qwerty/jp106.kmap.gz', 'jp', undef],
    'lt' => ['Lithuanian', 'lt', 'qwerty/lt.kmap.gz', 'lt', 'std'],
    #'lv'     => ['Latvian', 'lv', 'qwerty/lv-latin4.kmap.gz', 'lv', ??], # latin4 or latin7?
    'mk' => ['Macedonian', 'mk', 'qwerty/mk.kmap.gz', 'mk', 'nodeadkeys'],
    'nl' => ['Dutch', 'nl', 'qwerty/nl.kmap.gz', 'nl', undef],
    #'nl-be'  => ['Belgium-Dutch', 'nl-be', ?, ?, ?],
    'no' => ['Norwegian', 'no', 'qwerty/no-latin1.kmap.gz', 'no', 'nodeadkeys'],
    'pl' => ['Polish', 'pl', 'qwerty/pl.kmap.gz', 'pl', undef],
    'pt' => ['Portuguese', 'pt', 'qwerty/pt-latin1.kmap.gz', 'pt', 'nodeadkeys'],
    'pt-br' => ['Brazil-Portuguese', 'pt-br', 'qwerty/br-latin1.kmap.gz', 'br', 'nodeadkeys'],
    #'ru'     => ['Russian', 'ru', 'qwerty/ru.kmap.gz', 'ru', undef], # don't know?
    'si' => ['Slovenian', 'sl', 'qwertz/slovene.kmap.gz', 'si', undef],
    'se' => ['Swedish', 'sv', 'qwerty/se-latin1.kmap.gz', 'se', 'nodeadkeys'],
    #'th'     => [],
    'tr' => ['Turkish', 'tr', 'qwerty/trq.kmap.gz', 'tr', undef],
};

# we need mappings for X11, console, and kvm vnc
# LC(-LC)? => [DESC, kvm, console, X11, X11variant]
my sub generate_keymaps {
    my ($country_codes) = @_;

    my ($kmap, $kmaphash) = ({}, {});
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

sub debmirrors {
    return {
        'at' => 'ftp.at.debian.org',
        'au' => 'ftp.au.debian.org',
        'be' => 'ftp.be.debian.org',
        'bg' => 'ftp.bg.debian.org',
        'br' => 'ftp.br.debian.org',
        'ca' => 'ftp.ca.debian.org',
        'ch' => 'ftp.ch.debian.org',
        'cl' => 'ftp.cl.debian.org',
        'cz' => 'ftp.cz.debian.org',
        'de' => 'ftp.de.debian.org',
        'dk' => 'ftp.dk.debian.org',
        'ee' => 'ftp.ee.debian.org',
        'es' => 'ftp.es.debian.org',
        'fi' => 'ftp.fi.debian.org',
        'fr' => 'ftp.fr.debian.org',
        'gr' => 'ftp.gr.debian.org',
        'hk' => 'ftp.hk.debian.org',
        'hr' => 'ftp.hr.debian.org',
        'hu' => 'ftp.hu.debian.org',
        'ie' => 'ftp.ie.debian.org',
        'is' => 'ftp.is.debian.org',
        'it' => 'ftp.it.debian.org',
        'jp' => 'ftp.jp.debian.org',
        'kr' => 'ftp.kr.debian.org',
        'mx' => 'ftp.mx.debian.org',
        'nl' => 'ftp.nl.debian.org',
        'no' => 'ftp.no.debian.org',
        'nz' => 'ftp.nz.debian.org',
        'pl' => 'ftp.pl.debian.org',
        'pt' => 'ftp.pt.debian.org',
        'ro' => 'ftp.ro.debian.org',
        'ru' => 'ftp.ru.debian.org',
        'se' => 'ftp.se.debian.org',
        'si' => 'ftp.si.debian.org',
        'sk' => 'ftp.sk.debian.org',
        'tr' => 'ftp.tr.debian.org',
        'tw' => 'ftp.tw.debian.org',
        'gb' => 'ftp.uk.debian.org',
        'us' => 'ftp.us.debian.org',
    };
}

my $mirrors = debmirrors();
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
