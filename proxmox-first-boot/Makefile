all:

DESTDIR =
LIBSYSTEMD_DIR = $(DESTDIR)/usr/lib/systemd/system

.PHONY: install
install:
	install -D -m 644 etc/proxmox-first-boot-network-pre.service $(LIBSYSTEMD_DIR)/proxmox-first-boot-network-pre.service
	install -D -m 644 etc/proxmox-first-boot-network-online.service $(LIBSYSTEMD_DIR)/proxmox-first-boot-network-online.service
	install -D -m 644 etc/proxmox-first-boot-multi-user.service $(LIBSYSTEMD_DIR)/proxmox-first-boot-multi-user.service
