// Counting / failing / limiting GhosttyAllocator exercised through the OSC
// parser: every byte the parser owns must come from (and return to) the
// caller's allocator, and its failures must surface like Zig's.
#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <ghostty/vt.h>

typedef struct {
  size_t allocs, frees;
  size_t live;          // bytes currently allocated
  size_t limit;         // max live bytes (0 = unlimited)
  size_t fail_after;    // fail every allocation after this many succeed (SIZE_MAX = never)
} Counter;

static void *c_alloc(void *ctx, size_t len, uint8_t alignment, uintptr_t ret) {
  (void)alignment; (void)ret;
  Counter *c = ctx;
  if (c->allocs >= c->fail_after) return NULL;
  if (c->limit && c->live + len > c->limit) return NULL;
  void *p = malloc(len ? len : 1);
  if (!p) return NULL;
  c->allocs++;
  c->live += len;
  return p;
}
static bool c_resize(void *ctx, void *m, size_t ml, uint8_t a, size_t nl, uintptr_t r) {
  (void)ctx; (void)m; (void)ml; (void)a; (void)nl; (void)r;
  return false;
}
static void *c_remap(void *ctx, void *m, size_t ml, uint8_t a, size_t nl, uintptr_t r) {
  (void)ctx; (void)m; (void)ml; (void)a; (void)nl; (void)r;
  return NULL;
}
static void c_free(void *ctx, void *m, size_t len, uint8_t a, uintptr_t r) {
  (void)a; (void)r;
  Counter *c = ctx;
  c->frees++;
  c->live -= len;
  free(m);
}
static const GhosttyAllocatorVtable VT = {c_alloc, c_resize, c_remap, c_free};

static Counter counter(size_t limit, size_t fail_after) {
  Counter c = {0, 0, 0, limit, fail_after};
  return c;
}

static void feed(GhosttyOscParser p, const char *s, size_t n) {
  for (size_t i = 0; i < n; i++) ghostty_osc_next(p, (uint8_t)s[i]);
}

static int parse_clipboard(Counter *c, size_t payload_len, int *type_out) {
  GhosttyAllocator a = {.ctx = c, .vtable = &VT};
  GhosttyOscParser p;
  if (ghostty_osc_new(&a, &p) != GHOSTTY_SUCCESS) return -1;
  feed(p, "52;c;", 5);
  for (size_t i = 0; i < payload_len; i++) ghostty_osc_next(p, 'A');
  *type_out = ghostty_osc_command_type(ghostty_osc_end(p, 0x07));
  ghostty_osc_free(p);
  return 0;
}

int main(void) {
  // 1. Everything the parser allocates goes through the allocator and is
  //    returned on free.
  {
    Counter c = counter(0, SIZE_MAX);
    GhosttyAllocator a = {.ctx = &c, .vtable = &VT};
    GhosttyOscParser p;
    assert(ghostty_osc_new(&a, &p) == GHOSTTY_SUCCESS);
    assert(c.allocs >= 2);            // the parser object and its buffer
    assert(c.live >= 2048);           // the fixed payload buffer is ours
    feed(p, "0;hello", 7);
    GhosttyOscCommand cmd = ghostty_osc_end(p, 0x07);
    assert(ghostty_osc_command_type(cmd) == GHOSTTY_OSC_COMMAND_CHANGE_WINDOW_TITLE);
    const char *title = NULL;
    assert(ghostty_osc_command_data(cmd, GHOSTTY_OSC_DATA_CHANGE_WINDOW_TITLE_STR, &title));
    assert(strcmp(title, "hello") == 0);
    size_t before_free = c.live;
    assert(before_free > 2048);       // + the title copy
    ghostty_osc_free(p);
    assert(c.live == 0);
    assert(c.allocs == c.frees);
  }

  // 2. Creation failure at each allocation point is OUT_OF_MEMORY and leaks
  //    nothing.
  for (size_t fail_after = 0; fail_after < 2; fail_after++) {
    Counter c = counter(0, fail_after);
    GhosttyAllocator a = {.ctx = &c, .vtable = &VT};
    GhosttyOscParser p = NULL;
    assert(ghostty_osc_new(&a, &p) == GHOSTTY_OUT_OF_MEMORY);
    assert(c.live == 0);
    assert(c.allocs == c.frees);
  }

  // 3. A live-bytes limit governs payload growth: a 1 KiB clipboard payload
  //    fits; a 64 KiB one does not (the OSC becomes invalid, like Zig's
  //    WriteFailed) and nothing leaks.
  {
    Counter c = counter(0, SIZE_MAX);
    int unlimited = -1;
    assert(parse_clipboard(&c, 65536, &unlimited) == 0);
    assert(unlimited == GHOSTTY_OSC_COMMAND_CLIPBOARD_CONTENTS);
    assert(c.live == 0);
    size_t peak_needed = 65536;

    Counter small = counter(1024 * 1024, SIZE_MAX);
    int ok = -1;
    assert(parse_clipboard(&small, 1024, &ok) == 0);
    assert(ok == GHOSTTY_OSC_COMMAND_CLIPBOARD_CONTENTS);
    assert(small.live == 0);

    Counter tight = counter(16 * 1024, SIZE_MAX);
    int limited = -1;
    assert(parse_clipboard(&tight, peak_needed, &limited) == 0);
    assert(limited == GHOSTTY_OSC_COMMAND_INVALID);
    assert(tight.live == 0);
    assert(tight.allocs == tight.frees);
  }

  // 4. If the allocator fails after creation, the title copy cannot be made
  //    and the command is reported invalid.
  {
    Counter c = counter(0, SIZE_MAX);
    GhosttyAllocator a = {.ctx = &c, .vtable = &VT};
    GhosttyOscParser p;
    assert(ghostty_osc_new(&a, &p) == GHOSTTY_SUCCESS);
    c.fail_after = c.allocs;          // no further allocations succeed
    feed(p, "0;hello", 7);
    GhosttyOscCommand cmd = ghostty_osc_end(p, 0x07);
    assert(ghostty_osc_command_type(cmd) == GHOSTTY_OSC_COMMAND_INVALID);
    const char *title = NULL;
    assert(!ghostty_osc_command_data(cmd, GHOSTTY_OSC_DATA_CHANGE_WINDOW_TITLE_STR, &title));
    ghostty_osc_free(p);
    assert(c.live == 0);
  }

  // 5. A failing growth in the middle of a payload invalidates only that OSC;
  //    the parser is reusable afterwards.
  {
    Counter c = counter(0, SIZE_MAX);
    GhosttyAllocator a = {.ctx = &c, .vtable = &VT};
    GhosttyOscParser p;
    assert(ghostty_osc_new(&a, &p) == GHOSTTY_SUCCESS);
    c.fail_after = c.allocs;
    feed(p, "52;c;", 5);
    for (int i = 0; i < 5000; i++) ghostty_osc_next(p, 'A');
    assert(ghostty_osc_command_type(ghostty_osc_end(p, 0x07)) == GHOSTTY_OSC_COMMAND_INVALID);
    c.fail_after = SIZE_MAX;
    ghostty_osc_reset(p);
    feed(p, "0;ok", 4);
    GhosttyOscCommand cmd = ghostty_osc_end(p, 0x07);
    assert(ghostty_osc_command_type(cmd) == GHOSTTY_OSC_COMMAND_CHANGE_WINDOW_TITLE);
    ghostty_osc_free(p);
    assert(c.live == 0);
  }

  puts("ok");
  return 0;
}
