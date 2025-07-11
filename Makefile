include /usr/share/dpkg/default.mk

PACKAGE = proxmox-installer
BUILDDIR ?= $(PACKAGE)-$(DEB_VERSION_UPSTREAM)

DEB=$(PACKAGE)_$(DEB_VERSION)_$(DEB_HOST_ARCH).deb
ASSISTANT_DEB=proxmox-auto-install-assistant_$(DEB_VERSION)_$(DEB_HOST_ARCH).deb
FIRST_BOOT_DEB=proxmox-first-boot_$(DEB_VERSION)_$(DEB_HOST_ARCH).deb

ALL_DEBS = $(DEB) $(ASSISTANT_DEB) $(FIRST_BOOT_DEB)

DSC=$(PACKAGE)_$(DEB_VERSION).dsc

CARGO ?= cargo
ifeq ($(BUILD_MODE), release)
CARGO_BUILD_ARGS += --release
CARGO_COMPILEDIR := target/release
else
CARGO_COMPILEDIR := target/debug
endif

INSTALLER_SOURCES=$(shell git ls-files) locale-info.json

PREFIX = /usr
BINDIR = $(PREFIX)/bin
USR_BIN := \
	   proxmox-chroot\
	   proxmox-tui-installer\
	   proxmox-fetch-answer\
	   proxmox-auto-install-assistant \
	   proxmox-auto-installer \
	   proxmox-post-hook

COMPILED_BINS := \
	$(addprefix $(CARGO_COMPILEDIR)/,$(USR_BIN))

SHELL_SCRIPTS := \
	fake-start-stop-daemon \
	policy-disable-rc.d \
	spice-vdagent.sh \
	unconfigured.sh \
	xinitrc

all:

$(BUILDDIR):
	rm -rf $@ $@.tmp; mkdir $@.tmp
	cp -a \
	  .cargo \
	  Cargo.toml \
	  Makefile \
	  Proxmox/ \
	  Xdefaults \
	  banner/ \
	  checktime \
	  country.pl \
	  html/ \
	  interfaces \
	  proxinstall \
	  proxmox-low-level-installer \
	  proxmox-auto-installer/ \
	  proxmox-auto-install-assistant/ \
	  proxmox-fetch-answer/ \
	  proxmox-chroot \
	  proxmox-tui-installer/ \
	  proxmox-installer-common/ \
	  proxmox-post-hook \
	  proxmox-first-boot \
	  test/ \
	  $(SHELL_SCRIPTS) \
	  $@.tmp
	cp -a debian $@.tmp/
	mv $@.tmp $@

locale-info.json: country.pl
	./country.pl > $@.tmp
	mv $@.tmp $@

.PHONY: tidy
tidy:
	git ls-files ':*.p[ml]'| xargs -n4 -P0 proxmox-perltidy
	proxmox-perltidy proxmox-low-level-installer proxinstall

deb: $(DEB)
$(ASSISTANT_DEB): $(DEB)
$(FIRST_BOOT_DEB): $(DEB)
$(DEB): $(BUILDDIR)
	cd $(BUILDDIR); dpkg-buildpackage -b -us -uc
	lintian $(ALL_DEBS)

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

.PHONY: prepare-test-env
prepare-test-env: cd-info.test locale-info.json test.img
	rm -rf testdir
	mkdir -p testdir/var/lib/proxmox-installer/
	cp -v locale-info.json testdir/var/lib/proxmox-installer/
	./proxmox-low-level-installer -t test.img dump-env

.PHONY: test
test: prepare-test-env
	shellcheck $(SHELL_SCRIPTS)
	$(MAKE) -C test check
	$(CARGO) test --workspace $(CARGO_BUILD_ARGS)

DESTDIR=
VARLIBDIR=$(DESTDIR)/var/lib/proxmox-installer
HTMLDIR=$(VARLIBDIR)/html/common

.PHONY: install
install: $(INSTALLER_SOURCES) $(COMPILED_BINS)
	$(MAKE) -C banner install
	$(MAKE) -C Proxmox install
	$(MAKE) -C proxmox-first-boot install
	install -D -m 644 interfaces $(DESTDIR)/etc/network/interfaces
	install -D -m 755 fake-start-stop-daemon $(VARLIBDIR)/fake-start-stop-daemon
	install -D -m 755 policy-disable-rc.d $(VARLIBDIR)/policy-disable-rc.d
	install -D -m 644 locale-info.json $(VARLIBDIR)/locale-info.json
	install -D -m 755 unconfigured.sh $(DESTDIR)/usr/sbin/unconfigured.sh
	install -D -m 755 proxinstall $(DESTDIR)/usr/bin/proxinstall
	install -D -m 755 proxmox-low-level-installer $(DESTDIR)/$(BINDIR)/proxmox-low-level-installer
	$(foreach i,$(USR_BIN), install -m755 $(CARGO_COMPILEDIR)/$(i) $(DESTDIR)$(BINDIR)/ ;)
	install -D -m 755 checktime $(DESTDIR)/usr/bin/checktime
	install -D -m 644 xinitrc $(DESTDIR)/.xinitrc
	install -D -m 755 spice-vdagent.sh $(DESTDIR)/.spice-vdagent.sh
	install -D -m 644 Xdefaults $(DESTDIR)/.Xdefaults

$(COMPILED_BINS): cargo-build
.PHONY: cargo-build
cargo-build:
	$(CARGO) build --package proxmox-tui-installer --bin proxmox-tui-installer \
		--package proxmox-auto-installer --bin proxmox-auto-installer \
		--package proxmox-fetch-answer --bin proxmox-fetch-answer \
		--package proxmox-auto-install-assistant --bin proxmox-auto-install-assistant \
		--package proxmox-chroot --bin proxmox-chroot \
		--package proxmox-post-hook --bin proxmox-post-hook \
		$(CARGO_BUILD_ARGS)

%-banner.png: %-banner.svg
	rsvg-convert -o $@ $<

.PHONY: upload
upload: UPLOAD_DIST ?= $(DEB_DISTRIBUTION)
upload: $(ALL_DEBS)
	tar cf - $(ALL_DEBS) | ssh -X repoman@repo.proxmox.com -- upload --product pve,pmg,pbs,pdm --dist $(UPLOAD_DIST)

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
	./proxmox-low-level-installer dump-env -t test.img
	G_SLICE=always-malloc perl -I testdir/usr/share/perl5 testdir/usr/bin/proxinstall -t test.img

check-pve-multidisks: prepare-check-env test.img test2.img test3.img test4.img test5.big.img
	rm -f cd-info.test; $(MAKE) cd-info.test
	./proxmox-low-level-installer dump-env -t test.img,test2.img,test3.img,test4.img,test5.big.img
	G_SLICE=always-malloc perl -I testdir/usr/share/perl5 testdir/usr/bin/proxinstall -t test.img,test2.img,test3.img,test4.img,test5.big.img

check-pve-tui: prepare-check-env test.img
	rm -f cd-info.test; $(MAKE) cd-info.test
	./proxmox-low-level-installer dump-env -t test.img
	testdir/usr/bin/proxmox-tui-installer -t test.img 2>testdir/run/stderr

check-pve-tui-multidisks: prepare-check-env test.img test2.img test3.img test4.img test5.big.img
	rm -f cd-info.test; $(MAKE) cd-info.test
	./proxmox-low-level-installer dump-env -t test.img,test2.img,test3.img,test4.img,test5.big.img
	testdir/usr/bin/proxmox-tui-installer -t test.img,test2.img,test3.img,test4.img,test5.big.img

prepare-check-pmg: prepare-check-env test.img
	rm -f cd-info.test; $(MAKE) \
	    PRODUCT=pmg \
	    PRODUCTLONG="Proxmox Mail Gateway" \
	    ISONAME='proxmox-mail-gateway' \
	    cd-info.test
	./proxmox-low-level-installer dump-env -t test.img

check-pmg: prepare-check-pmg
	G_SLICE=always-malloc perl -I testdir/usr/share/perl5 testdir/usr/bin/proxinstall -t test.img

check-pmg-tui: prepare-check-pmg
	testdir/usr/bin/proxmox-tui-installer -t test.img 2>testdir/run/stderr

prepare-check-pbs: prepare-check-env test.img
	rm -f cd-info.test; $(MAKE) \
	    PRODUCT='pbs' \
	    PRODUCTLONG='Proxmox Backup Server' \
	    ISONAME='proxmox-backup-server' \
	    cd-info.test
	./proxmox-low-level-installer dump-env -t test.img

check-pbs: prepare-check-pbs
	G_SLICE=always-malloc perl -I testdir/usr/share/perl5 testdir/usr/bin/proxinstall -t test.img

check-pbs-tui: prepare-check-pbs
	testdir/usr/bin/proxmox-tui-installer -t test.img 2>testdir/run/stderr

prepare-check-pdm: prepare-check-env test.img
	rm -f cd-info.test; $(MAKE) \
	    PRODUCT='pdm' \
	    PRODUCTLONG='Proxmox Datacenter Manager' \
	    ISONAME='proxmox-datacenter-manager' \
	    cd-info.test
	./proxmox-low-level-installer dump-env -t test.img

check-pdm: prepare-check-pdm
	G_SLICE=always-malloc perl -I testdir/usr/share/perl5 testdir/usr/bin/proxinstall -t test.img

check-pdm-tui: prepare-check-pdm
	testdir/usr/bin/proxmox-tui-installer -t test.img 2>testdir/run/stderr

.phony: clean
clean:
	rm -rf target build $(PACKAGE)-[0-9]* testdir
	rm -f $(PACKAGE)*.tar* *.deb packages packages.tmp *.build *.dsc *.buildinfo *.changes
	rm -f test*.img pve-final.pkglist locale-info.json final.pkglist cd-info.test
	find . -name '*~' -exec rm {} ';'
