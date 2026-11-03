// SPDX-License-Identifier: MIT
// A core's side of Specular's native core protocol (specular_native.h), for cores to copy along
// with that header: the handshake, the shared memory, the buttons, the sound ring and messages.
// What the core does with Specular's requests (pausing, states) is its own: it receives them
// with spn_receive, on a thread of its choosing, and answers with spn_send.
#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "specular_native.h"

typedef struct SpnClient {
  int socket;
  SpnShared *shared;
  // SPN_START's keys, as given (NULL where missing).
  const char *package, *saves, *cache;
  uint64_t clock;
  char start_text[SPN_MESSAGE_MAX];
} SpnClient;

// Connects on `fd` (the number after --specular): says hello, waits for SPN_START, maps the
// shared memory. False, with why in `error`, if any of it fails.
bool spn_connect(SpnClient *c, int fd, const char *name, const char *version, char *error, size_t error_size);

// Sends a message (text may be NULL). False if the socket is gone (Specular has stopped).
bool spn_send(SpnClient *c, uint32_t type, uint64_t a, uint64_t b, const char *text);

// Waits for Specular's next message; `text` gets its text, NUL-terminated (cut to `text_size`).
// False once the socket has closed.
bool spn_receive(SpnClient *c, SpnMessage *message, char *text, size_t text_size);

// The buttons held now (1 << SpnButton), and when they last changed (CLOCK_MONOTONIC ns).
uint32_t spn_buttons(const SpnClient *c, uint64_t *time_ns);

// Sets the sound's rate (before the first spn_play, and whenever it changes).
void spn_sound_rate(SpnClient *c, uint32_t hz);
// Queues `count` stereo frames; returns how many fit (the rest are dropped).
size_t spn_play(SpnClient *c, const int16_t (*frames)[2], size_t count);

// Counts a frame presented, which took `time_ns` on the game's thread.
void spn_frame_presented(SpnClient *c, uint64_t time_ns);
