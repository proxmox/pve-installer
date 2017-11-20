DESTDIR=

all:

.phony: install
install:
	install -D -m 644 page1.htm ${DESTDIR}/var/lib/pve-installer/html/page1.htm
	install -D -m 644 passwd.htm ${DESTDIR}/var/lib/pve-installer/html/passwd.htm
	install -D -m 644 extract1-license.htm ${DESTDIR}/var/lib/pve-installer/html/extract1-license.htm
	install -D -m 644 extract2-rulesystem.htm ${DESTDIR}/var/lib/pve-installer/html/extract2-rulesystem.htm
	install -D -m 644 extract3-spam.htm ${DESTDIR}/var/lib/pve-installer/html/extract3-spam.htm
	install -D -m 644 extract4-virus.htm ${DESTDIR}/var/lib/pve-installer/html/extract4-virus.htm
	install -D -m 644 fail.htm ${DESTDIR}/var/lib/pve-installer/html/fail.htm
	install -D -m 644 success.htm ${DESTDIR}/var/lib/pve-installer/html/success.htm

.phony: clean
clean:
	rm -rf *~
