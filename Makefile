# Install claudin onto a directory that is already on PATH.
#
# Search PATH from front to back for a writable directory inside $(HOME):
#   $(HOME)/.local/bin, $(HOME)/.cargo/bin, or $(HOME)/bin.
# The first match is used, so the shell finds the new binary.
#
# When none of those are on PATH, ask before copying to /usr/local/bin.
# `make install PREFIX=/some/prefix` skips the search and uses $(PREFIX)/bin.

.PHONY: all build install

CARGO      ?= cargo
BIN        := claudin
TARGET     := target/release/$(BIN)
GLOBAL_BIN := /usr/local/bin

all: build

build:
	$(CARGO) build --release

install: build
	@set -eu; \
	bin='$(BIN)'; \
	target='$(TARGET)'; \
	global='$(GLOBAL_BIN)'; \
	prefix='$(PREFIX)'; \
	home="$$HOME"; \
	dest=""; \
	if [ -n "$$prefix" ]; then \
		dest="$$prefix/bin"; \
	else \
		rest="$$PATH"; \
		while [ -n "$$rest" ]; do \
			dir="$${rest%%:*}"; \
			case "$$rest" in \
				*:*) rest="$${rest#*:}" ;; \
				*) rest="" ;; \
			esac; \
			while [ -n "$$dir" ] && [ "$$dir" != "/" ] && [ "$${dir%/}" != "$$dir" ]; do \
				dir="$${dir%/}"; \
			done; \
			case "$$dir" in \
				"$$home/.local/bin"|"$$home/.cargo/bin"|"$$home/bin") ;; \
				*) continue ;; \
			esac; \
			if [ -d "$$dir" ] && [ -w "$$dir" ]; then \
				dest="$$dir"; \
				break; \
			fi; \
			if [ ! -e "$$dir" ]; then \
				parent="$$dir"; \
				while [ ! -d "$$parent" ] && [ "$$parent" != "/" ]; do \
					parent=$$(dirname "$$parent"); \
				done; \
				if [ -w "$$parent" ]; then \
					dest="$$dir"; \
					break; \
				fi; \
			fi; \
		done; \
	fi; \
	if [ -z "$$dest" ]; then \
		if [ ! -r /dev/tty ] || [ ! -w /dev/tty ]; then \
			echo "claudin: no safe directory under $$home is on PATH." >&2; \
			echo "Run make install in a terminal to confirm $$global, or set PREFIX." >&2; \
			exit 1; \
		fi; \
		printf 'No safe install directory under %s is on PATH.\n' "$$home" >/dev/tty; \
		printf 'Install %s to %s? This may ask for your password. [y/N] ' "$$bin" "$$global" >/dev/tty; \
		read ans </dev/tty || ans=""; \
		case "$$ans" in \
			y|Y|yes|YES) dest="$$global" ;; \
			*) echo "Install cancelled."; exit 1 ;; \
		esac; \
	fi; \
	if [ -d "$$dest" ] && [ -w "$$dest" ]; then \
		install -m 755 "$$target" "$$dest/$$bin"; \
	else \
		parent="$$dest"; \
		while [ ! -e "$$parent" ] && [ "$$parent" != "/" ]; do \
			parent=$$(dirname "$$parent"); \
		done; \
		if [ -d "$$parent" ] && [ -w "$$parent" ] && [ ! -e "$$dest" ]; then \
			mkdir -p "$$dest"; \
			install -m 755 "$$target" "$$dest/$$bin"; \
		else \
			sudo install -d -m 755 "$$dest"; \
			sudo install -m 755 "$$target" "$$dest/$$bin"; \
		fi; \
	fi; \
	echo "Installed $$bin to $$dest/$$bin"
