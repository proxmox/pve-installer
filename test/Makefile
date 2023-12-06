all:

export PERLLIB=..

.PHONY: check
check: test-zfs-arc-max test-run-command test-parse-fqdn test-ui2-stdio

.PHONY: test-zfs-arc-max
test-zfs-arc-max:
	./zfs-arc-max.pl

test-run-command:
	./run-command.pl

.PHONY: test-parse-fqdn
test-parse-fqdn:
	./parse-fqdn.pl

.PHONY: test-ui2-stdio
test-ui2-stdio:
	./ui2-stdio.pl
