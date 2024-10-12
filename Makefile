DOCKER = podman
IMAGE_TAG = apt-cache

.PHONY: clean

all: approx_host.bin Dockerfile init.sh approx.conf
	$(DOCKER) build -t $(IMAGE_TAG) .

clean:
	rm -f approx_host.bin
	cd approx_host && cargo clean

approx_host.bin: approx_host/src/main.rs
	cd approx_host && cargo build -r && cp target/release/approx_host ../approx_host.bin
