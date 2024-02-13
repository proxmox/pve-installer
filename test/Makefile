all:

export PERLLIB=..

.PHONY: check
check: test-zfs-arc-max test-run-command

.PHONY: test-zfs-arc-max
test-zfs-arc-max:
	./zfs-arc-max.pl

test-run-command:
	./run-command.pl
