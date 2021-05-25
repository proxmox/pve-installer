include /usr/share/dpkg/pkg-info.mk

DEB=proxmox-installer_${DEB_VERSION_UPSTREAM_REVISION}_all.deb

INSTALLER_SOURCES=		\
	unconfigured.sh 	\
	fake-start-stop-daemon	\
	policy-disable-rc.d	\
	interfaces		\
	checktime		\
	xinitrc			\
	spice-vdagent.sh	\
	Xdefaults		\
	country.dat		\
	proxinstall

HTML_SOURCES=$(wildcard html/*.htm)
HTML_COMMON_SOURCES=$(wildcard html-common/*.htm) $(wildcard html-common/*.css) $(wildcard html-common/*.png)

all: ${INSTALLER_SOURCES} ${HTML_COMMON_SOURCES} ${HTML_SOURCES}

country.dat: country.pl
	./country.pl > country.dat

deb: ${DEB}
${DEB}: ${INSTALLER_SOURCES} ${HTML_COMMON_SOURCES} ${HTML_SOURCES}
	rsync --exclude='test*.img' -a * build
	cd build; dpkg-buildpackage -b -us -uc
	lintian ${DEB}

DESTDIR=
VARLIBDIR=$(DESTDIR)/var/lib/proxmox-installer
HTMLDIR=$(VARLIBDIR)/html/common

.PHONY: install
install: ${INSTALLER_SOURCES} ${HTML_COMMON_SOURCES} ${HTML_SOURCES}
	$(MAKE) -C banner install
	install -D -m 644 interfaces ${DESTDIR}/etc/network/interfaces
	install -D -m 755 fake-start-stop-daemon ${VARLIBDIR}/fake-start-stop-daemon
	install -D -m 755 policy-disable-rc.d $(VARLIBDIR)/policy-disable-rc.d
	install -D -m 644 country.dat $(VARLIBDIR)/country.dat
	install -D -m 755 unconfigured.sh ${DESTDIR}/sbin/unconfigured.sh
	install -D -m 755 proxinstall ${DESTDIR}/usr/bin/proxinstall
	install -D -m 755 checktime ${DESTDIR}/usr/bin/checktime
	install -D -m 644 xinitrc ${DESTDIR}/.xinitrc
	install -D -m 755 spice-vdagent.sh ${DESTDIR}/.spice-vdagent.sh
	install -D -m 644 Xdefaults ${DESTDIR}/.Xdefaults

%-banner.png: %-banner.svg
	rsvg-convert -o $@ $<

.PHONY: upload
upload: ${DEB}
	tar cf - ${PMG_DEB} | ssh -X repoman@repo.proxmox.com -- upload --product pve,pmg,pbs --dist bullseye

%.img:
	truncate -s 2G $@

.PHONY: prepare-check-env
prepare-check-env: $(DEB)
	umount -Rd testdir || true
	rm -rf testdir
	dpkg -X $(DEB) testdir

check-pve: prepare-check-env test.img
	printf '%s\n' "PRODUCT='pve'" >cd-info.test
	G_SLICE=always-malloc perl -I testdir/usr/share/perl5 testdir/usr/bin/proxinstall -t test.img

check-pve-multidisks: prepare-check-env test.img test2.img test3.img test4.img
	printf '%s\n' "PRODUCT='pve'" >cd-info.test
	G_SLICE=always-malloc perl -I testdir/usr/share/perl5 testdir/usr/bin/proxinstall -t test.img,test2.img,test3.img,test4.img

check-pmg: prepare-check-env test.img
	printf '%s\n' "PRODUCT='pmg'" >cd-info.test
	G_SLICE=always-malloc perl -I testdir/usr/share/perl5 testdir/usr/bin/proxinstall -t test.img

check-pbs: prepare-check-env test.img
	printf '%s\n' "PRODUCT='pbs'" >cd-info.test
	G_SLICE=always-malloc perl -I testdir/usr/share/perl5 testdir/usr/bin/proxinstall -t test.img


.phony: clean
clean:
	umount -Rd testdir || true
	rm -rf *~ *.deb target build packages packages.tmp testdir test*.img pve-final.pkglist \
	  *.buildinfo *.changes country.dat final.pkglist cd-info.test
	find . -name '*~' -exec rm {} ';'
