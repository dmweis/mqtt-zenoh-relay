DEB_BUILD_PATH ?= target/debian/mqtt-zenoh-relay*.deb


.PHONY: build
build:
	cargo build --release

.PHONY: build-deb
build-deb: build
	cargo deb --no-build

.PHONE: install
install: build-deb
	sudo dpkg -i $(DEB_BUILD_PATH)

.PHONY: install-deps
install-deps:
	cargo install cargo-deb
