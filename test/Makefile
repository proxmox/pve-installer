all:

export PERLLIB=..

.PHONY: check
check: test-zfs-arc-max test-run-command test-parse-fqdn test-ui2-stdio \
       test-zfs-get-pool-list test-parse-kernel-cmdline

.PHONY: test-zfs-arc-max
test-zfs-arc-max:
	./zfs-arc-max.pl

.PHONY: test-run-command
test-run-command:
	./run-command.pl

.PHONY: test-parse-fqdn
test-parse-fqdn:
	./parse-fqdn.pl

.PHONY: test-ui2-stdio
test-ui2-stdio:
	./ui2-stdio.pl

.PHONY: test-zfs-get-pool-list
test-zfs-get-pool-list:
	./zfs-get-pool-list.pl

.PHONY: test-parse-kernel-cmdline
test-parse-kernel-cmdline:
	./parse-kernel-cmdline.pl
