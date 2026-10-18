# Builds Specular and its emulator cores, and installs them: how a system (such as Crystaline OS)
# puts Specular on a device. For this PC by default; for a device, for example:
#   make install DEVICE=brick TARGET=aarch64-unknown-linux-gnu \
#     CROSS_COMPILE=aarch64-linux-gnu- CORE_CFLAGS=-mcpu=cortex-a53 DESTDIR=<root> PREFIX=/usr/lib/specular
# Working on Specular itself needs only cargo (README.md); `make cores` builds the cores to play
# games with, and `make test` runs the tests with them (both for this PC only).

# The device's crate (devices/<DEVICE>).
DEVICE = desktop
# Rust's name for the device's platform; empty for this PC.
TARGET =
# The device's compiler prefix (gcc and g++ are added); empty for this PC's.
CROSS_COMPILE =
# Extra C flags for the cores, e.g. the CPU to tune for.
CORE_CFLAGS =
# Where everything is built: cargo's output, and the cores in $(OUT)/cores/<TARGET, or host>.
OUT = $(CURDIR)/target
# install puts Specular in $(DESTDIR)$(PREFIX): the specular binary, with cores/, fonts/ and
# licences.txt next to it, where it looks for them.
DESTDIR =
PREFIX = /usr/lib/specular

BIN := $(OUT)/$(if $(TARGET),$(TARGET)/)release
CORES := $(OUT)/cores/$(or $(TARGET),host)
# Cross builds: cargo links with the device's compiler, and crates that build C code (zstd) use it
# too. CC_<target> rather than CC, which would also reach the build scripts that run on this PC.
CROSS_ENV := $(if $(TARGET),CC_$(subst -,_,$(TARGET))=$(CROSS_COMPILE)gcc)
CROSS_ARGS := $(if $(TARGET),--target $(TARGET) --config 'target.$(TARGET).linker="$(CROSS_COMPILE)gcc"')

.PHONY: build cores install test

# Specular, the device's hardware checks (devices/<DEVICE>/examples) and the runtime's headless
# player (play). Always run: cargo decides what is out of date.
build:
	$(CROSS_ENV) CARGO_TARGET_DIR=$(OUT) cargo build --release $(CROSS_ARGS) \
		-p specular-$(DEVICE) -p specular-runtime --bins --examples

# Always run: the cores' own builds decide what is out of date.
cores:
	CROSS_COMPILE=$(CROSS_COMPILE) CORE_CFLAGS='$(CORE_CFLAGS)' tools/build-cores.sh $(CORES)

install: build cores
	install -D -m 755 $(BIN)/specular $(DESTDIR)$(PREFIX)/specular
	install -D -m 644 $(BIN)/licences.txt $(DESTDIR)$(PREFIX)/licences.txt	# see crates/specular-credits
	install -D -m 644 -t $(DESTDIR)$(PREFIX)/cores $(CORES)/*.so
	install -D -m 644 -t $(DESTDIR)$(PREFIX)/fonts assets/fonts/NotoEmoji.ttf	# see specular_ui::use_fallback_fonts

test: cores
	SPECULAR_CORES=$(CORES) CARGO_TARGET_DIR=$(OUT) cargo test --workspace
