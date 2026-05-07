all:

DESTDIR =
LIBSYSTEMD_DIR = $(DESTDIR)/usr/lib/systemd/system
LIBEXECDIR = $(DESTDIR)/usr/libexec/proxmox

.PHONY: install
install:
	install -D -m 644 etc/proxmox-first-boot-network-pre.service $(LIBSYSTEMD_DIR)/proxmox-first-boot-network-pre.service
	install -D -m 644 etc/proxmox-first-boot-network-online.service $(LIBSYSTEMD_DIR)/proxmox-first-boot-network-online.service
	install -D -m 644 etc/proxmox-first-boot-multi-user.service $(LIBSYSTEMD_DIR)/proxmox-first-boot-multi-user.service
	install -D -m 644 etc/proxmox-first-boot-subscription.service $(LIBSYSTEMD_DIR)/proxmox-first-boot-subscription.service
	install -D -m 755 proxmox-first-boot-subscription $(LIBEXECDIR)/proxmox-first-boot-subscription
