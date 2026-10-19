// The log callback libretro cores are given (retro_log_printf_t). It is C-variadic, which stable
// Rust can't define: this formats the message, then hands it to Specular's sink (`log.rs`).
#include <stdarg.h>
#include <stdio.h>

// Set once, by log.rs's `start` as the first game loads (before its core is initialised); until
// then messages are dropped.
static void (*sink)(unsigned level, const char *line);

void specular_log_to(void (*s)(unsigned, const char *)) { sink = s; }

void specular_log(unsigned level, const char *fmt, ...) {
    // RETRO_LOG_DEBUG: dropped before formatting, so a chatty core costs next to nothing.
    if (level == 0 || !sink || !fmt) return;
    char line[1024]; // longer messages are cut short
    va_list args;
    va_start(args, fmt);
    vsnprintf(line, sizeof line, fmt, args);
    va_end(args);
    sink(level, line);
}
