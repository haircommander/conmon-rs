MAKEFILE_PATH := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))
RUNTIME_PATH ?= "/usr/bin/runc"
PROTO_PATH ?= "conmon-rs/common/proto"
BINARY := conmonrs
CONTAINER_RUNTIME ?= $(if $(shell which podman 2>/dev/null),podman,docker)
BUILD_DIR ?= .build
GOTOOLS_GOPATH ?= $(BUILD_DIR)/gotools
GOTOOLS_BINDIR ?= $(GOTOOLS_GOPATH)/bin
GINKGO_FLAGS ?= -vv --trace --race --randomize-all --flake-attempts 3 --progress --timeout 5m -r pkg/client
PACKAGE_NAME ?= $(shell cargo metadata --no-deps --format-version 1 | jq -r '.packages[2] | [ .name, .version ] | join("-")')
PREFIX ?= /usr
CI_TAG ?=

default:
	cargo build

release:
	cargo build --release

.PHONY: release-static
release-static:
	mkdir -p ~/.cargo/git
	$(CONTAINER_RUNTIME) run -it \
		--pull always \
		-v "$(shell pwd)":/volume \
		-v ~/.cargo/registry:/root/.cargo/registry \
		-v ~/.cargo/git:/root/.cargo/git \
		clux/muslrust:stable \
		bash -c "\
			apt-get update && \
			apt-get install -y capnproto && \
			rustup component add rustfmt && \
			make release && \
			strip -s target/x86_64-unknown-linux-musl/release/$(BINARY)"

lint: .install.golangci-lint
	cargo fmt && git diff --exit-code
	cargo clippy --all-targets -- -D warnings
	$(GOTOOLS_BINDIR)/golangci-lint run

unit:
	cargo test --bins --no-fail-fast

integration: .install.ginkgo release # It needs to be release so we correctly test the RSS usage
	export CONMON_BINARY="$(MAKEFILE_PATH)target/release/$(BINARY)" && \
	export RUNTIME_BINARY="$(RUNTIME_PATH)" && \
	export MAX_RSS_KB=10240 && \
	sudo -E "$(GOTOOLS_BINDIR)/ginkgo" $(GINKGO_FLAGS)

integration-static: .install.ginkgo # It needs to be release so we correctly test the RSS usage
	export CONMON_BINARY="$(MAKEFILE_PATH)target/x86_64-unknown-linux-musl/release/$(BINARY)" && \
	if [ ! -f "$$CONMON_BINARY" ]; then \
		$(MAKE) release-static; \
	fi && \
	export RUNTIME_BINARY="$(RUNTIME_PATH)" && \
	export MAX_RSS_KB=3500 && \
	sudo -E "$(GOTOOLS_BINDIR)/ginkgo" $(GINKGO_FLAGS)

.install.ginkgo:
	GOBIN=$(abspath $(GOTOOLS_BINDIR)) go install github.com/onsi/ginkgo/v2/ginkgo@latest

.install.golangci-lint:
	curl -sSfL https://raw.githubusercontent.com/golangci/golangci-lint/master/install.sh | BINDIR=$(abspath $(GOTOOLS_BINDIR)) sh -s v1.46.2

clean:
	rm -rf target/

update-proto:
	go install capnproto.org/go/capnp/v3/capnpc-go@latest
	cat $(PROTO_PATH)/go-patch >> $(PROTO_PATH)/conmon.capnp
	capnp compile \
		-I$$GOPATH/src/capnproto.org/go/capnp/std \
		-ogo $(PROTO_PATH)/conmon.capnp
	mv $(PROTO_PATH)/conmon.capnp.go internal/proto/
	git checkout $(PROTO_PATH)/conmon.capnp

.PHONY: lint clean unit integration update-proto

.PHONY: create-release-packages
create-release-packages: release
	if [ "$(PACKAGE_NAME)" != "conmonrs-$(CI_TAG)" ]; then \
		echo "crate version and tag mismatch" ; \
		exit 1 ; \
	fi
	cargo vendor -q && tar zcf $(PACKAGE_NAME)-vendor.tar.gz vendor && rm -rf vendor
	git archive --format tar --prefix=conmonrs-$(CI_TAG)/ $(CI_TAG) | gzip >$(PACKAGE_NAME).tar.gz


.PHONY: install
install:
	mkdir -p "${DESTDIR}$(PREFIX)/bin"
	install -D -t "${DESTDIR}$(PREFIX)/bin" target/release/conmonrs

# Only meant to build the latest HEAD commit + any uncommitted changes
# Not a replacement for the distro package
.PHONY: rpm
rpm:
	rpkg local
