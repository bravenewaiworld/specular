// SPDX-License-Identifier: MIT
// Specular's native core protocol, version 1: how Specular plays a game with a core that is a
// program of its own (docs/native-cores.md). This header is the protocol's definition; cores copy
// it (it is MIT so that any core can), and Specular's side (specular-runtime's `native` module)
// mirrors it and checks the layout in its tests.
//
// Specular starts the core's program as `PROGRAM --specular FD`. FD is the core's end of a
// SOCK_SEQPACKET socket pair: each message is one packet, an SpnMessage followed by `text_len`
// bytes of UTF-8 (no NUL). The first message is the core's SPN_HELLO; Specular answers with
// SPN_START, which carries the shared memory (an SpnShared) as an SCM_RIGHTS file descriptor.
//
// The screen: the core draws to it only between its SPN_RESUMED and its SPN_PAUSED (Specular has
// stopped drawing by the time it sends SPN_RESUME, and draws again only after SPN_PAUSED). The
// core starts paused: it boots the game after SPN_START, but presents nothing before the first
// SPN_RESUME.
#pragma once

#include <stdint.h>

#define SPN_PROTOCOL 1u
// The largest packet, header included.
#define SPN_MESSAGE_MAX 4096u

enum SpnType {
  // core -> Specular
  SPN_HELLO = 1,       // a = SPN_PROTOCOL; text = "NAME\tVERSION"
  SPN_STARTED = 2,     // a = the picture's width, b = its height (pixels, as the game makes it); text = frames per second
  SPN_PAUSED = 3,      // stopped, screen let go; the frame on screen is in SpnShared.frame
  SPN_RESUMED = 4,     // running, drawing to the screen
  SPN_STATE_DONE = 5,  // a SPN_SAVE_STATE or SPN_LOAD_STATE is done: a = 0, or an errno; text = why it failed, for the player
  SPN_SAVED = 6,       // the game wrote its save (a memory card): for Specular's effects
  SPN_RUMBLE = 7,      // a = the motor's strength, 0 (still) to 65535
  SPN_ERROR = 8,       // the game can't go on: text = why, for the player; the core then exits

  // Specular -> core
  SPN_START = 64,   // SCM_RIGHTS: the SpnShared memfd; text = "key=value" lines (SPN_KEY_*)
  SPN_PAUSE = 65,   // answered by SPN_PAUSED
  SPN_RESUME = 66,  // only while paused; answered by SPN_RESUMED
  SPN_SAVE_STATE = 67,  // only while paused; text = the file to write; answered by SPN_STATE_DONE
  SPN_LOAD_STATE = 68,  // only while paused; text = the file to read; answered by SPN_STATE_DONE
  SPN_QUIT = 69,        // only while paused: write the saves, then exit with status 0
};

// SPN_START's keys. Paths are absolute; a key may be missing where said.
#define SPN_KEY_PACKAGE "package"      // the game's package (the ROM)
#define SPN_KEY_SAVES "saves"          // a folder for the game's saves (memory cards), made by Specular
#define SPN_KEY_CACHE "cache"          // a folder for what the core may rebuild (shader caches), made by Specular
#define SPN_KEY_CLOCK "clock"          // the local time at launch, in seconds since 2000-01-01 00:00
#define SPN_KEY_OPTION_PREFIX "option."  // "option.KEY=VALUE": the core's options (none yet)

typedef struct SpnMessage {
  uint32_t type;      // SpnType
  uint32_t text_len;  // bytes of text after this header
  uint64_t a, b;
} SpnMessage;

// Specular's buttons, bits of SpnShared.buttons. Menu, the volume keys and Power never reach the core.
enum SpnButton {
  SPN_UP, SPN_DOWN, SPN_LEFT, SPN_RIGHT, SPN_A, SPN_B, SPN_X, SPN_Y,
  SPN_L1, SPN_R1, SPN_L2, SPN_R2, SPN_SELECT, SPN_START_BUTTON, SPN_F1, SPN_F2,
};

#define SPN_SOUND_FRAMES 8192u           // a power of two
#define SPN_FRAME_PIXELS (1024u * 1024u)  // the largest picture

// One memfd, made by Specular, mapped by both. Fields marked "seq" follow a sequence lock: the
// writer makes the sequence odd, writes, then makes it even; a reader retries while it is odd or
// changed. Counters only grow. Atomics are plain integers here, accessed with the compilers'
// __atomic builtins (C) and AtomicU32/AtomicU64 (Rust).
typedef struct SpnShared {
  uint32_t protocol;  // SPN_PROTOCOL, set by Specular
  uint32_t size;      // sizeof(SpnShared), set by Specular

  // Buttons, written by Specular as they change; the core reads them when the game polls its pads.
  uint32_t buttons_seq;       // seq
  uint32_t buttons;           // bits (1 << SpnButton)
  uint64_t buttons_time_ns;   // seq: CLOCK_MONOTONIC time of the latest change

  // Sound, written by the core, played by Specular: stereo frames (left, right) in a ring of
  // SPN_SOUND_FRAMES; frame n is at sound[n % SPN_SOUND_FRAMES]. The core doesn't overwrite
  // frames Specular hasn't read (it drops what doesn't fit).
  uint32_t sound_rate;        // Hz, set before the first frame is written
  uint32_t reserved0;
  uint64_t sound_written;     // frames written so far (store-release after the frames)
  uint64_t sound_read;        // frames read so far (by Specular)
  int16_t sound[SPN_SOUND_FRAMES][2];

  // Progress, written by the core: frames presented so far. The watchdog and the CPU minimum
  // follow it.
  uint64_t frames;
  uint64_t frame_time_ns;     // the latest frame's time on the game's thread

  // The picture on screen when the core paused, written before SPN_PAUSED: RGBA, rows packed.
  uint32_t frame_width, frame_height;
  uint8_t frame[SPN_FRAME_PIXELS * 4];
} SpnShared;
