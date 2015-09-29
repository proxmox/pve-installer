# achtung: also set release in proxinstall!
RELEASE=4.0

DEB=pve-installer_4.0-3_all.deb

INSTALLER_SOURCES=		\
	unconfigured.sh 	\
	fake-start-stop-daemon	\
	policy-disable-rc.d	\
	interfaces		\
	proxlogo.xpm		\
	checktime		\
	proxinstall

HTML_SOURCES=$(wildcard html/*.htm) $(wildcard html/*.css) $(wildcard html/*.png)

${DEB}: ${INSTALLER_SOURCES} ${HTML_SOURCES} Makefile html/Makefile
	rm -rf debian
	mkdir debian
	make DESTDIR=`pwd`/debian install
	mkdir debian/DEBIAN
	install -m 0644 control debian/DEBIAN
	install -m 0644 conffiles debian/DEBIAN
	dpkg-deb --build debian ${DEB}
	rm -rf debian
	lintian -X man ${DEB}

.phony: install
install: ${INSTALLER_SOURCES} ${HTML_SOURCES}
	make DESTDIR=`pwd`/debian -C html install
	install -D -m 644 interfaces ${DESTDIR}/etc/network/interfaces
	mkdir -p ${DESTDIR}/var/lib/dhcp3/
	ln -s /tmp/dhclient.eth0.leases ${DESTDIR}/var/lib/dhcp3
	ln -s /tmp/resolv.conf.dhclient-new ${DESTDIR}/etc/resolv.conf
	ln -s /tmp/resolv.conf.dhclient-new ${DESTDIR}/etc/resolv.conf.dhclient-new
	install -D -m 755 fake-start-stop-daemon ${DESTDIR}/var/lib/pve-installer/fake-start-stop-daemon
	install -D -m 755 policy-disable-rc.d ${DESTDIR}/var/lib/pve-installer/policy-disable-rc.d
	install -D -m 644 proxlogo.xpm  ${DESTDIR}/var/lib/pve-installer/proxlogo.xpm
	install -D -m 755 unconfigured.sh ${DESTDIR}/sbin/unconfigured.sh
	install -D -m 755 proxinstall ${DESTDIR}/usr/bin/proxinstall
	install -D -m 755 checktime ${DESTDIR}/usr/bin/checktime
	install -D -m 644 xinitrc ${DESTDIR}/.xinitrc
	install -D -m 644 Xdefaults ${DESTDIR}/.Xdefaults
	install -D -m 644 copyright ${DESTDIR}/usr/share/doc/pve-installer/copyright
	install -D -m 644 changelog.Debian  ${DESTDIR}/usr/share/doc/pve-installer/changelog.Debian
	gzip --best  ${DESTDIR}/usr/share/doc/pve-installer/changelog.Debian

.phony: upload
upload: ${DEB}
	umount /pve/${RELEASE}; mount -o rw /pve/${RELEASE}
	mkdir -p /pve/${RELEASE}/extra
	rm -rf /pve/${RELEASE}/extra/pve-installer_*.deb
	rm -f /pve/${RELEASE}/extra/Packages*
	cp ${DEB} /pve/${RELEASE}/extra
	cd /pve/${RELEASE}/extra; dpkg-scanpackages . /dev/null  > Packages; gzip -9c Packages > Packages.gz
	umount /pve/${RELEASE}; mount -o ro /pve/${RELEASE}

packages: /pve/${RELEASE}/install/pve.files
	rm -rf packages packages.tmp; mkdir packages.tmp
	for i in `cat $<`; do install -m 644 $$i  packages.tmp/; done
	mv packages.tmp packages

test.img:
	dd if=/dev/zero of=test.img bs=2048 count=1M

check: packages test.img
	G_SLICE=always-malloc ./proxinstall -t test.img

.phony: clean
clean:
	make -C html clean
	rm -rf *~ ${DEB} target debian packages packages.tmp test.img pve-final.pkglist
