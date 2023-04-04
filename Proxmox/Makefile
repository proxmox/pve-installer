all:

DESTDIR=
VARLIBDIR=$(DESTDIR)/var/lib/proxmox-installer
PERL5DIR=$(DESTDIR)/usr/share/perl5/Proxmox

#PERL_MODULES=$(shell git ls-files . | grep '.pm$')
PERL_MODULES=\
    Install/Setup.pm \
    Log.pm \
    Sys/Command.pm \
    Sys/File.pm \
    Sys/Net.pm \

.PHONY: install
install: $(PERL_MODULES)
	for f in $(PERL_MODULES); \
	    do install -D -m 644 "$$f" "$(PERL5DIR)/$$f"; \
	done

.PHONY: clean
clean:
