DESTFILE=
INSTALLER_SOURCES=		\
	pmg-banner.png		\
	itPVE-banner.png		\
	pbs-banner.png		\
	pdm-banner.png		\

%-banner.png: %-banner.svg
	rsvg-convert -o $@ $<

# just for triggering a "build", the actual installation is handled by debhelper
.PHONY: install
install: $(INSTALLER_SOURCES)
