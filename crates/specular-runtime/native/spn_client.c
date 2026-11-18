// SPDX-License-Identifier: MIT
// See spn_client.h.
#define _GNU_SOURCE
#include "spn_client.h"

#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <unistd.h>

bool spn_send(SpnClient *c, uint32_t type, uint64_t a, uint64_t b, const char *text) {
  char packet[SPN_MESSAGE_MAX];
  const size_t len = text ? strlen(text) : 0;
  if (sizeof(SpnMessage) + len > sizeof packet) return false;
  const SpnMessage m = {.type = type, .text_len = (uint32_t)len, .a = a, .b = b};
  memcpy(packet, &m, sizeof m);
  if (len) memcpy(packet + sizeof m, text, len);
  return send(c->socket, packet, sizeof m + len, MSG_NOSIGNAL) == (ssize_t)(sizeof m + len);
}

// Receives one packet, and the descriptor it carries if `fd` isn't NULL (-1 if none).
static bool receive(int socket, SpnMessage *m, char *text, size_t text_size, int *fd) {
  char packet[SPN_MESSAGE_MAX];
  struct iovec iov = {.iov_base = packet, .iov_len = sizeof packet};
  union { struct cmsghdr align; char bytes[CMSG_SPACE(sizeof(int))]; } control;
  struct msghdr h = {.msg_iov = &iov, .msg_iovlen = 1, .msg_control = &control, .msg_controllen = sizeof control};
  ssize_t n;
  do n = recvmsg(socket, &h, MSG_CMSG_CLOEXEC); while (n < 0 && errno == EINTR);
  if (n < (ssize_t)sizeof *m) return false;
  if (fd) {
    *fd = -1;
    for (struct cmsghdr *c = CMSG_FIRSTHDR(&h); c; c = CMSG_NXTHDR(&h, c))
      if (c->cmsg_level == SOL_SOCKET && c->cmsg_type == SCM_RIGHTS) memcpy(fd, CMSG_DATA(c), sizeof(int));
  }
  memcpy(m, packet, sizeof *m);
  size_t len = (size_t)n - sizeof *m;
  if (len > m->text_len) len = m->text_len;
  if (text_size) {
    if (len >= text_size) len = text_size - 1;
    memcpy(text, packet + sizeof *m, len);
    text[len] = 0;
  }
  return true;
}

bool spn_receive(SpnClient *c, SpnMessage *message, char *text, size_t text_size) {
  return receive(c->socket, message, text, text_size, NULL);
}

bool spn_connect(SpnClient *c, int fd, const char *name, const char *version, char *error, size_t error_size) {
  memset(c, 0, sizeof *c);
  c->socket = fd;
  char hello[256];
  snprintf(hello, sizeof hello, "%s\t%s", name, version);
  if (!spn_send(c, SPN_HELLO, SPN_PROTOCOL, 0, hello)) {
    snprintf(error, error_size, "cannot reach Specular on descriptor %d: %s", fd, strerror(errno));
    return false;
  }
  SpnMessage m;
  int memfd;
  if (!receive(fd, &m, c->start_text, sizeof c->start_text, &memfd) || m.type != SPN_START || memfd < 0) {
    snprintf(error, error_size, "Specular didn't send the game");
    return false;
  }
  struct stat st;
  if (fstat(memfd, &st) != 0 || (size_t)st.st_size < sizeof(SpnShared)) {
    snprintf(error, error_size, "the shared memory is %lld bytes, not %zu (a different protocol build?)", (long long)st.st_size, sizeof(SpnShared));
    close(memfd);
    return false;
  }
  void *p = mmap(NULL, sizeof(SpnShared), PROT_READ | PROT_WRITE, MAP_SHARED, memfd, 0);
  close(memfd);
  if (p == MAP_FAILED) {
    snprintf(error, error_size, "cannot map the shared memory: %s", strerror(errno));
    return false;
  }
  c->shared = p;
  if (c->shared->protocol != SPN_PROTOCOL || c->shared->size != sizeof(SpnShared)) {
    snprintf(error, error_size, "the shared memory is for protocol %u, %u bytes; this core's is %u, %zu bytes",
             c->shared->protocol, c->shared->size, SPN_PROTOCOL, sizeof(SpnShared));
    return false;
  }
  // Lines of "key=value", cut in place.
  for (char *line = c->start_text; *line;) {
    char *next = strchr(line, '\n'), *value = strchr(line, '=');
    if (next) *next = 0;
    if (value && (!next || value < next)) {
      *value++ = 0;
      if (!strcmp(line, SPN_KEY_PACKAGE)) c->package = value;
      else if (!strcmp(line, SPN_KEY_SAVES)) c->saves = value;
      else if (!strcmp(line, SPN_KEY_CACHE)) c->cache = value;
      else if (!strcmp(line, SPN_KEY_CLOCK)) c->clock = strtoull(value, NULL, 10);
    }
    if (!next) break;
    line = next + 1;
  }
  return true;
}

uint32_t spn_buttons(const SpnClient *c, uint64_t *time_ns) {
  const SpnShared *s = c->shared;
  uint32_t seq, buttons;
  uint64_t time;
  do {
    seq = __atomic_load_n(&s->buttons_seq, __ATOMIC_ACQUIRE);
    buttons = __atomic_load_n(&s->buttons, __ATOMIC_RELAXED);
    time = __atomic_load_n(&s->buttons_time_ns, __ATOMIC_RELAXED);
    __atomic_thread_fence(__ATOMIC_ACQUIRE);
  } while ((seq & 1) || seq != __atomic_load_n(&s->buttons_seq, __ATOMIC_RELAXED));
  if (time_ns) *time_ns = time;
  return buttons;
}

void spn_sound_rate(SpnClient *c, uint32_t hz) { __atomic_store_n(&c->shared->sound_rate, hz, __ATOMIC_RELEASE); }

size_t spn_play(SpnClient *c, const int16_t (*frames)[2], size_t count) {
  SpnShared *s = c->shared;
  const uint64_t written = __atomic_load_n(&s->sound_written, __ATOMIC_RELAXED);
  const uint64_t read = __atomic_load_n(&s->sound_read, __ATOMIC_ACQUIRE);
  const uint64_t room = SPN_SOUND_FRAMES - (written - read);
  if (count > room) count = (size_t)room;
  for (size_t i = 0; i < count; i++) memcpy(s->sound[(written + i) % SPN_SOUND_FRAMES], frames[i], sizeof frames[i]);
  __atomic_store_n(&s->sound_written, written + count, __ATOMIC_RELEASE);
  return count;
}

void spn_frame_presented(SpnClient *c, uint64_t time_ns) {
  __atomic_store_n(&c->shared->frame_time_ns, time_ns, __ATOMIC_RELAXED);
  __atomic_fetch_add(&c->shared->frames, 1, __ATOMIC_RELEASE);
}
