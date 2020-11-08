DESTDIR=

all:

.phony: install
install:
	install -D -m 644 pve-installer.css ${DESTDIR}/var/lib/pve-installer/html/pve-installer.css
	install -D -m 644 plus.png ${DESTDIR}/var/lib/pve-installer/html/plus.png
	install -D -m 644 country.htm ${DESTDIR}/var/lib/pve-installer/html/country.htm
	install -D -m 644 license.htm ${DESTDIR}/var/lib/pve-installer/html/license.htm
	install -D -m 644 ipconf.htm ${DESTDIR}/var/lib/pve-installer/html/ipconf.htm
	install -D -m 644 nohds.htm ${DESTDIR}/var/lib/pve-installer/html/nohds.htm
	install -D -m 644 nonics.htm ${DESTDIR}/var/lib/pve-installer/html/nonics.htm
	install -D -m 644 ack_template.htm ${DESTDIR}/var/lib/pve-installer/html/ack_template.htm
	install -D -m 644 success.htm ${DESTDIR}/var/lib/pve-installer/html/success.htm

.phony: clean
clean:
	rm -rf *~
