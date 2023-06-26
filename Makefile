include /usr/share/dpkg/default.mk

PACKAGE = proxmox-installer
BUILDDIR ?= $(PACKAGE)-$(DEB_VERSION_UPSTREAM)

DEB=$(PACKAGE)_$(DEB_VERSION)_$(DEB_HOST_ARCH).deb
DSC=$(PACKAGE)_$(DEB_VERSION).dsc

CARGO ?= cargo
ifeq ($(BUILD_MODE), release)
CARGO_BUILD_ARGS += --release
CARGO_COMPILEDIR := target/release
else
CARGO_COMPILEDIR := target/debug
endif

INSTALLER_SOURCES=$(shell git ls-files) country.dat

PREFIX = /usr
BINDIR = $(PREFIX)/bin
USR_BIN := proxmox-tui-installer

COMPILED_BINS := \
	$(addprefix $(CARGO_COMPILEDIR)/,$(USR_BIN))

all:

$(BUILDDIR):
	rm -rf $@ $@.tmp; mkdir $@.tmp
	cp -a \
	  Cargo.toml \
	  Makefile \
	  Proxmox/ \
	  Xdefaults \
	  banner/ \
	  checktime \
	  country.pl \
	  fake-start-stop-daemon \
	  html/ \
	  interfaces \
	  policy-disable-rc.d \
	  proxinstall \
	  proxmox-low-level-installer \
	  proxmox-tui-installer/ \
	  spice-vdagent.sh \
	  unconfigured.sh \
	  xinitrc \
	  $@.tmp
	cp -a debian $@.tmp/
	mv $@.tmp $@

country.dat: country.pl
	./country.pl > country.dat.tmp
	mv country.dat.tmp country.dat

deb: $(DEB)
$(DEB): $(BUILDDIR)
	cd $(BUILDDIR); dpkg-buildpackage -b -us -uc
	lintian $(DEB)

test-$(DEB): $(INSTALLER_SOURCES)
	rsync --exclude='test*.img' --exclude='*.deb' --exclude='build' -a * build
	cd build; dpkg-buildpackage -b -us -uc -nc
	mv $(DEB) test-$(DEB)

dsc: $(DSC)
	$(MAKE) clean
	$(MAKE) $(DSC)
	lintian $(DSC)

$(DSC): $(BUILDDIR)
	cd $(BUILDDIR); dpkg-buildpackage -S -us -uc -d

sbuild: $(DSC)
	sbuild $(DSC)

DESTDIR=
VARLIBDIR=$(DESTDIR)/var/lib/proxmox-installer
HTMLDIR=$(VARLIBDIR)/html/common

.PHONY: install
install: $(INSTALLER_SOURCES) $(CARGO_COMPILEDIR)/proxmox-tui-installer
	$(MAKE) -C banner install
	$(MAKE) -C Proxmox install
	install -D -m 644 interfaces $(DESTDIR)/etc/network/interfaces
	install -D -m 755 fake-start-stop-daemon $(VARLIBDIR)/fake-start-stop-daemon
	install -D -m 755 policy-disable-rc.d $(VARLIBDIR)/policy-disable-rc.d
	install -D -m 644 country.dat $(VARLIBDIR)/country.dat
	install -D -m 755 unconfigured.sh $(DESTDIR)/sbin/unconfigured.sh
	install -D -m 755 proxinstall $(DESTDIR)/usr/bin/proxinstall
	install -D -m 755 proxmox-low-level-installer $(DESTDIR)/$(BINDIR)/proxmox-low-level-installer
	$(foreach i,$(USR_BIN), install -m755 $(CARGO_COMPILEDIR)/$(i) $(DESTDIR)$(BINDIR)/)
	install -D -m 755 checktime $(DESTDIR)/usr/bin/checktime
	install -D -m 644 xinitrc $(DESTDIR)/.xinitrc
	install -D -m 755 spice-vdagent.sh $(DESTDIR)/.spice-vdagent.sh
	install -D -m 644 Xdefaults $(DESTDIR)/.Xdefaults

$(COMPILED_BINS): cargo-build
.PHONY: cargo-build
cargo-build:
	$(CARGO) build --package proxmox-tui-installer --bin proxmox-tui-installer $(CARGO_BUILD_ARGS)

%-banner.png: %-banner.svg
	rsvg-convert -o $@ $<

.PHONY: upload
upload: UPLOAD_DIST ?= $(DEB_DISTRIBUTION)
upload: $(DEB)
	tar cf - $(DEB) | ssh -X repoman@repo.proxmox.com -- upload --product pve,pmg,pbs --dist $(UPLOAD_DIST)

%.img:
	truncate -s 2G $@

%.big.img:
	truncate -s 8G $@

.PHONY: prepare-check-env
prepare-check-env: test-$(DEB)
	rm -rf testdir
	dpkg -X test-$(DEB) testdir
	mkdir -p testdir/run/proxmox-installer/

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
	./proxmox-low-level-installer dump-env -t
	G_SLICE=always-malloc perl -I testdir/usr/share/perl5 testdir/usr/bin/proxinstall -t test.img

check-pve-multidisks: prepare-check-env test.img test2.img test3.img test4.img test5.big.img
	rm -f cd-info.test; $(MAKE) cd-info.test
	./proxmox-low-level-installer dump-env -t
	G_SLICE=always-malloc perl -I testdir/usr/share/perl5 testdir/usr/bin/proxinstall -t test.img,test2.img,test3.img,test4.img,test5.big.img

check-pve-tui: prepare-check-env test.img
	rm -f cd-info.test; $(MAKE) cd-info.test
	./proxmox-low-level-installer dump-env -t
	testdir/usr/bin/proxmox-tui-installer -t test.img

prepare-check-pmg: prepare-check-env test.img
	rm -f cd-info.test; $(MAKE) \
	    PRODUCT=pmg \
	    PRODUCTLONG="Proxmox Mail Gateway" \
	    ISONAME='proxmox-mail-gateway' \
	    cd-info.test
	./proxmox-low-level-installer dump-env -t

check-pmg: prepare-check-pmg
	G_SLICE=always-malloc perl -I testdir/usr/share/perl5 testdir/usr/bin/proxinstall -t test.img

check-pmg-tui: prepare-check-pmg
	testdir/usr/bin/proxmox-tui-installer -t test.img

prepare-check-pbs: prepare-check-env test.img
	rm -f cd-info.test; $(MAKE) \
	    PRODUCT='pbs' \
	    PRODUCTLONG='Proxmox Backup Server' \
	    ISONAME='proxmox-backup-server' \
	    cd-info.test
	./proxmox-low-level-installer dump-env -t

check-pbs: prepare-check-pbs
	G_SLICE=always-malloc perl -I testdir/usr/share/perl5 testdir/usr/bin/proxinstall -t test.img

check-pbs-tui: prepare-check-pbs
	testdir/usr/bin/proxmox-tui-installer -t test.img

.phony: clean
clean:
	rm -rf target build $(PACKAGE)-[0-9]* testdir
	rm -f $(PACKAGE)*.tar* *.deb packages packages.tmp *.build *.dsc *.buildinfo *.changes
	rm -f test*.img pve-final.pkglist country.dat final.pkglist cd-info.test
	find . -name '*~' -exec rm {} ';'
