include /usr/share/dpkg/pkg-info.mk

DEB=proxmox-installer_${DEB_VERSION_UPSTREAM_REVISION}_all.deb

INSTALLER_SOURCES=$(shell git ls-files) country.dat

all:

country.dat: country.pl
	./country.pl > country.dat.tmp
	mv country.dat.tmp country.dat

deb: ${DEB}
${DEB}: ${INSTALLER_SOURCES}
	rsync --exclude='test*.img' -a * build
	cd build; dpkg-buildpackage -b -us -uc
	lintian ${DEB}

DESTDIR=
VARLIBDIR=$(DESTDIR)/var/lib/proxmox-installer
HTMLDIR=$(VARLIBDIR)/html/common

.PHONY: install
install: ${INSTALLER_SOURCES}
	$(MAKE) -C banner install
	$(MAKE) -C Proxmox install
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
upload: UPLOAD_DIST ?= $(DEB_DISTRIBUTION)
upload: $(DEB)
	tar cf - ${DEB} | ssh -X repoman@repo.proxmox.com -- upload --product pve,pmg,pbs --dist $(UPLOAD_DIST)

%.img:
	truncate -s 2G $@

%.big.img:
	truncate -s 8G $@

.PHONY: prepare-check-env
prepare-check-env: $(DEB)
	umount -Rd testdir || true
	rm -rf testdir
	dpkg -X $(DEB) testdir

cd-info.test: PRODUCT ?= pve
cd-info.test:
	printf '%s\n' "PRODUCT='$(or $(PRODUCT), pve)'" >$@.tmp
	printf '%s\n' "PRODUCTLONG='$(or $(PRODUCTLONG), Proxmox VE)'" >>$@.tmp
	printf '%s\n' "RELEASE='$(or $(RELEASE), 42.1)'" >>$@.tmp
	printf '%s\n' "ISORELEASE='$(or $(ISORELEASE), 1)'" >>$@.tmp
	printf '%s\n' "ISONAME='$(or $(ISONAME), proxmox-ve)'" >>$@.tmp
	mv $@.tmp $@

check-pve: prepare-check-env test.img
	rm -f cd-info.test; $(MAKE) cd-info.test
	G_SLICE=always-malloc perl -I testdir/usr/share/perl5 testdir/usr/bin/proxinstall -t test.img

check-pve-multidisks: prepare-check-env test.img test2.img test3.img test4.img test5.big.img
	rm -f cd-info.test; $(MAKE) cd-info.test
	G_SLICE=always-malloc perl -I testdir/usr/share/perl5 testdir/usr/bin/proxinstall -t test.img,test2.img,test3.img,test4.img,test5.big.img

check-pmg: prepare-check-env test.img
	rm -f cd-info.test; $(MAKE) \
	    PRODUCT=pmg \
	    PRODUCTLONG="Proxmox Mail Gateway" \
	    ISONAME='proxmox-mail-gateway' \
	    cd-info.test
	G_SLICE=always-malloc perl -I testdir/usr/share/perl5 testdir/usr/bin/proxinstall -t test.img

check-pbs: prepare-check-env test.img
	rm -f cd-info.test; $(MAKE) \
	    PRODUCT='pbs' \
	    PRODUCTLONG='Proxmox Backup Server' \
	    ISONAME='proxmox-backup-server' \
	    cd-info.test
	G_SLICE=always-malloc perl -I testdir/usr/share/perl5 testdir/usr/bin/proxinstall -t test.img


.phony: clean
clean:
	umount -Rd testdir || true
	rm -rf *~ *.deb target build packages packages.tmp testdir test*.img pve-final.pkglist \
	  *.buildinfo *.changes country.dat final.pkglist cd-info.test
	find . -name '*~' -exec rm {} ';'
