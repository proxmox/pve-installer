all:

export PERLLIB=..

.PHONY: check
check: test-zfs-arc-max test-run-command test-parse-fqdn

.PHONY: test-zfs-arc-max
test-zfs-arc-max:
	./zfs-arc-max.pl

test-run-command:
	./run-command.pl

.PHONY: test-parse-fqdn
test-parse-fqdn:
	./parse-fqdn.pl
