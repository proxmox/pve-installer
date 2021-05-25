DESTDIR=
VARLIBDIR=$(DESTDIR)/var/lib/proxmox-installer
HTMLDIR=$(VARLIBDIR)/html/common

all:

.phony: install
install:
	install -D -m 644 pve-installer.css $(HTMLDIR)/pve-installer.css
	install -D -m 644 plus.png $(HTMLDIR)/plus.png
	install -D -m 644 country.htm $(HTMLDIR)/country.htm
	install -D -m 644 license.htm $(HTMLDIR)/license.htm
	install -D -m 644 ipconf.htm $(HTMLDIR)/ipconf.htm
	install -D -m 644 nohds.htm $(HTMLDIR)/nohds.htm
	install -D -m 644 nonics.htm $(HTMLDIR)/nonics.htm
	install -D -m 644 ack_template.htm $(HTMLDIR)/ack_template.htm
	install -D -m 644 success.htm $(HTMLDIR)/success.htm
	install -D -m 644 fail.htm $(HTMLDIR)/fail.htm

.PHONY: clean
clean:
	rm -rf *~
