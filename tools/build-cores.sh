#!/bin/sh
# Build the libretro cores Specular runs: mGBA, for GB, GBC and GBA. Run by the Makefile's `cores`.
# Usage: build-cores.sh <out-dir>
# From the environment: CROSS_COMPILE, the device's compiler prefix (empty: this PC's compiler),
# and CORE_CFLAGS, extra C flags (e.g. the CPU to tune for).
# Each core's source is fetched once, at a pinned commit, into <out-dir>/src/<core>. Every run puts
# back the files our patches (tools/core-patches/<core>) changed and applies the patches again, so
# each applies cleanly; files they leave alone keep their times, so rebuilds stay incremental. The
# core is built in <out-dir>/build/<core> and copied to <out-dir>.
set -e
ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT=$(mkdir -p "$1" && cd "$1" && pwd)
CC=${CROSS_COMPILE:-}gcc CXX=${CROSS_COMPILE:-}g++
# The device's processor (aarch64, ...) from its compiler; CMake needs it to cross-compile.
[ -z "${CROSS_COMPILE:-}" ] ||
	CMAKE_CROSS="-DCMAKE_SYSTEM_NAME=Linux -DCMAKE_SYSTEM_PROCESSOR=$("$CC" -dumpmachine | cut -d- -f1)"

# patched_src CORE URL COMMIT: CORE's source at COMMIT in $OUT/src/CORE, with our patches applied.
patched_src() {
	d=$OUT/src/$1
	git -C "$d" cat-file -e "$3^{commit}" 2>/dev/null ||
		{ git init -q "$d" && git -C "$d" fetch -q --depth 1 "$2" "$3"; }
	git -C "$d" checkout -q -f "$3" && git -C "$d" clean -q -f -d
	for p in "$ROOT/tools/core-patches/$1"/*.patch; do
		[ -e "$p" ] || continue	# no patches: the pattern stays unexpanded
		git -C "$d" apply "$p"
	done
}

# mGBA (MPL-2.0): CMake, with everything but the libretro core switched off (no GUI, no optional
# libraries).
patched_src mgba https://github.com/libretro/mgba.git 7a12d6d4b9acb14c0ae62c9166b6a2f3d08007f6
cmake -S "$OUT/src/mgba" -B "$OUT/build/mgba" -DCMAKE_BUILD_TYPE=Release ${CMAKE_CROSS:-} \
	-DCMAKE_C_COMPILER="$CC" -DCMAKE_CXX_COMPILER="$CXX" -DCMAKE_C_FLAGS="${CORE_CFLAGS:-}" \
	-DBUILD_LIBRETRO=ON -DSKIP_LIBRARY=ON -DBUILD_QT=OFF -DBUILD_SDL=OFF -DBUILD_SHARED=OFF -DBUILD_STATIC=OFF \
	-DBUILD_GL=OFF -DBUILD_GLES2=OFF -DBUILD_GLES3=OFF -DENABLE_SCRIPTING=OFF -DUSE_DEBUGGERS=OFF -DUSE_GDB_STUB=OFF \
	-DUSE_FFMPEG=OFF -DUSE_ZLIB=OFF -DUSE_PNG=OFF -DUSE_LIBZIP=OFF -DUSE_MINIZIP=OFF -DUSE_SQLITE3=OFF -DUSE_ELF=OFF \
	-DUSE_LZMA=OFF -DUSE_EPOXY=OFF -DUSE_DISCORD_RPC=OFF -DUSE_EDITLINE=OFF -DUSE_LUA=OFF -DUSE_JSON_C=OFF >/dev/null
cmake --build "$OUT/build/mgba" --target mgba_libretro -j"$(nproc)" >/dev/null
cp "$OUT/build/mgba/mgba_libretro.so" "$OUT/"
