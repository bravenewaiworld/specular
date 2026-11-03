// SPDX-License-Identifier: MIT
// A native core for specular-runtime's tests (src/native/tests.rs builds it): plays a "game"
// whose picture and sound are the buttons held, so the tests can see both sides of the protocol.
//   - While running, presents a frame every 16 ms and plays 512 frames of sound at 32000 Hz,
//     each sample the buttons' bits.
//   - Paused, its frame is width x height pixels of (buttons, frames presented % 256, 0, 255).
//   - A state is a file holding the frames presented; loading one sets that count.
//   - On quit, writes "quit" to SAVES/quit.txt and sends SPN_SAVED first.
//   - With MOCK_FAIL=resume, says SPN_ERROR "the disc can't be read" on the first resume and exits;
//     with MOCK_FAIL=start, says it instead of SPN_STARTED.
#define _GNU_SOURCE
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#include "spn_client.h"

static SpnClient g_client;
static pthread_mutex_t g_lock = PTHREAD_MUTEX_INITIALIZER;
static int g_running;
static uint64_t g_frames;

static void *game(void *unused) {
  (void)unused;
  for (;;) {
    usleep(16000);
    pthread_mutex_lock(&g_lock);
    if (g_running) {
      int16_t sound[512][2];
      const uint32_t buttons = spn_buttons(&g_client, NULL);
      for (int i = 0; i < 512; i++) sound[i][0] = sound[i][1] = (int16_t)buttons;
      spn_play(&g_client, (const int16_t (*)[2])sound, 512);
      g_frames++;
      spn_frame_presented(&g_client, 1000000);
    }
    pthread_mutex_unlock(&g_lock);
  }
  return NULL;
}

int main(int argc, char **argv) {
  if (argc != 3 || strcmp(argv[1], "--specular")) return 2;
  char error[256];
  if (!spn_connect(&g_client, atoi(argv[2]), "Mock", "1.0", error, sizeof error)) {
    fprintf(stderr, "mock: %s\n", error);
    return 1;
  }
  if (getenv("MOCK_FAIL") && !strcmp(getenv("MOCK_FAIL"), "start")) {
    spn_send(&g_client, SPN_ERROR, 0, 0, "the disc can't be read");
    return 1;
  }
  spn_sound_rate(&g_client, 32000);
  spn_send(&g_client, SPN_STARTED, 320, 240, "60");
  pthread_t thread;
  pthread_create(&thread, NULL, game, NULL);
  SpnMessage m;
  char text[SPN_MESSAGE_MAX];
  while (spn_receive(&g_client, &m, text, sizeof text)) {
    pthread_mutex_lock(&g_lock);
    switch (m.type) {
    case SPN_RESUME:
      if (getenv("MOCK_FAIL") && !strcmp(getenv("MOCK_FAIL"), "resume")) {
        spn_send(&g_client, SPN_ERROR, 0, 0, "the disc can't be read");
        return 1;
      }
      g_running = 1;
      spn_send(&g_client, SPN_RESUMED, 0, 0, NULL);
      break;
    case SPN_PAUSE: {
      g_running = 0;
      SpnShared *s = g_client.shared;
      s->frame_width = 320, s->frame_height = 240;
      const uint8_t pixel[4] = {(uint8_t)spn_buttons(&g_client, NULL), (uint8_t)g_frames, 0, 255};
      for (int i = 0; i < 320 * 240; i++) memcpy(s->frame + 4 * i, pixel, 4);
      spn_send(&g_client, SPN_PAUSED, 0, 0, NULL);
      break;
    }
    case SPN_SAVE_STATE: case SPN_LOAD_STATE: {
      FILE *f = fopen(text, m.type == SPN_SAVE_STATE ? "w" : "r");
      int ok = f && (m.type == SPN_SAVE_STATE ? fprintf(f, "%llu", (unsigned long long)g_frames) > 0
                                              : fscanf(f, "%llu", (unsigned long long *)&g_frames) == 1);
      if (f) fclose(f);
      spn_send(&g_client, SPN_STATE_DONE, ok ? 0 : 2, 0, ok ? NULL : "no such state");
      break;
    }
    case SPN_QUIT: {
      char path[4096];
      snprintf(path, sizeof path, "%s/quit.txt", g_client.saves);
      FILE *f = fopen(path, "w");
      if (f) fputs("quit", f), fclose(f);
      spn_send(&g_client, SPN_SAVED, 0, 0, NULL);
      return 0;
    }
    }
    pthread_mutex_unlock(&g_lock);
  }
  return 0;
}
