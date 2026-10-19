#!/bin/sh
# Build the libretro cores Specular runs: mGBA (GB, GBC, GBA), Nestopia (NES) and SwanStation (PS1).
# Run by the Makefile's `cores`.
# Usage: build-cores.sh <out-dir>
# From the environment: CROSS_COMPILE, the device's compiler prefix (empty: this PC's compiler),
# and CORE_CFLAGS, extra C flags (e.g. the CPU to tune for).
# Each core's source is fetched once, at a pinned commit, into <out-dir>/src/<core>. Every run puts
# back the files our patches (tools/core-patches/<core>) changed and applies the patches again, so
# each applies cleanly; files they leave alone keep their times, so rebuilds stay incremental. The
# core is built in <out-dir>/build/<core> (Nestopia's own Makefile builds in its source tree) and
# copied to <out-dir>.
set -e
# When this is run from our Makefile, the settings on its command line (TARGET=…) would reach the
# cores' own Makefiles through MAKEFLAGS and override theirs.
unset MAKEFLAGS MFLAGS MAKELEVEL
ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT=$(mkdir -p "$1" && cd "$1" && pwd)
CC=${CROSS_COMPILE:-}gcc CXX=${CROSS_COMPILE:-}g++
# The device's processor (aarch64, ...) from its compiler; CMake needs it to cross-compile.
[ -z "${CROSS_COMPILE:-}" ] ||
	CMAKE_CROSS="-DCMAKE_SYSTEM_NAME=Linux -DCMAKE_SYSTEM_PROCESSOR=$("$CC" -dumpmachine | cut -d- -f1)"

# patched_src CORE URL COMMIT: CORE's source at COMMIT in $OUT/src/CORE, with our patches applied.
# `git clean` (without -x) keeps ignored files: Nestopia's objects, built in its source tree, survive
# it only if its .gitignore lists them.
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

# Nestopia (GPL-2.0-or-later): its libretro Makefile, which reads the flags from the environment.
patched_src nestopia https://github.com/libretro/nestopia.git 06a8949e0b3cf60e3a38a374b1426bf40fccb9a0
CFLAGS="${CORE_CFLAGS:-}" CXXFLAGS="${CORE_CFLAGS:-}" make -C "$OUT/src/nestopia/libretro" CC="$CC" CXX="$CXX" -j"$(nproc)" >/dev/null
cp "$OUT/src/nestopia/libretro/nestopia_libretro.so" "$OUT/"

# SwanStation (GPL-3.0): CMake. Only its software renderer is used: Specular gives cores no GL
# context to draw with.
patched_src swanstation https://github.com/libretro/swanstation.git e436e9b85374def58d211df7f78880f0f4a917b4
cmake -S "$OUT/src/swanstation" -B "$OUT/build/swanstation" -DCMAKE_BUILD_TYPE=Release ${CMAKE_CROSS:-} \
	-DCMAKE_C_COMPILER="$CC" -DCMAKE_CXX_COMPILER="$CXX" -DCMAKE_C_FLAGS="${CORE_CFLAGS:-}" -DCMAKE_CXX_FLAGS="${CORE_CFLAGS:-}" >/dev/null
cmake --build "$OUT/build/swanstation" --target swanstation_libretro -j"$(nproc)" >/dev/null
cp "$OUT/build/swanstation/swanstation_libretro.so" "$OUT/"
