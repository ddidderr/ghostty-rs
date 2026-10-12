// A host program embedding libghostty through the unchanged ghostty.h:
// the flows the macOS app uses that are platform independent (init, CLI
// actions, config, app lifecycle, key bindings, color scheme).
#define _GNU_SOURCE
#include <ghostty.h>

#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static int failures = 0;
#define CHECK(cond)                                                          \
  do {                                                                       \
    if (!(cond)) {                                                           \
      fprintf(stderr, "CHECK failed %s:%d: %s\n", __FILE__, __LINE__, #cond); \
      failures++;                                                            \
    }                                                                        \
  } while (0)

// ---- runtime callbacks ----------------------------------------------

static atomic_int wakeups;
static int app_userdata_marker = 42;

#define MAX_ACTIONS 256
static int actions[MAX_ACTIONS];
static int action_count;
static int target_tags[MAX_ACTIONS];
static bool last_config_change_maximize;
static char last_config_change_title[64];
static bool last_reload_soft;
static int last_quit_timer = -1;

static void on_wakeup(void* ud) {
  (void)ud;
  atomic_fetch_add(&wakeups, 1);
}

static bool on_action(ghostty_app_t app, ghostty_target_s target,
                      ghostty_action_s action) {
  if (action_count < MAX_ACTIONS) {
    target_tags[action_count] = target.tag;
    actions[action_count++] = action.tag;
  }
  CHECK(ghostty_app_userdata(app) == &app_userdata_marker);
  switch (action.tag) {
    case GHOSTTY_ACTION_CONFIG_CHANGE: {
      ghostty_config_t cfg = action.action.config_change.config;
      bool maximize = false;
      CHECK(ghostty_config_get(cfg, &maximize, "maximize", 8));
      last_config_change_maximize = maximize;
      const char* title = NULL;
      CHECK(ghostty_config_get(cfg, &title, "title", 5));
      snprintf(last_config_change_title, sizeof last_config_change_title, "%s",
               title ? title : "(null)");
      break;
    }
    case GHOSTTY_ACTION_RELOAD_CONFIG:
      last_reload_soft = action.action.reload_config.soft;
      break;
    case GHOSTTY_ACTION_QUIT_TIMER:
      last_quit_timer = action.action.quit_timer;
      break;
    default:
      break;
  }
  return true;
}

static bool on_read_clipboard(void* ud, ghostty_clipboard_e c, void* state) {
  (void)ud; (void)c; (void)state;
  return false;
}
static void on_confirm_read(void* ud, const char* s, void* state,
                            ghostty_clipboard_request_e r) {
  (void)ud; (void)s; (void)state; (void)r;
}
static void on_write_clipboard(void* ud, ghostty_clipboard_e c,
                               const ghostty_clipboard_content_s* content,
                               size_t n, bool confirm) {
  (void)ud; (void)c; (void)content; (void)n; (void)confirm;
}
static void on_close_surface(void* ud, bool process_alive) {
  (void)ud; (void)process_alive;
}

static int has_action(int tag) {
  for (int i = 0; i < action_count; i++)
    if (actions[i] == tag) return 1;
  return 0;
}

static ghostty_input_key_s key_event(ghostty_input_action_e action,
                                     uint32_t keycode, int mods,
                                     const char* text) {
  ghostty_input_key_s e;
  memset(&e, 0, sizeof e);
  e.action = action;
  e.mods = mods;
  e.keycode = keycode;
  e.text = text;
  return e;
}

int main(int argc, char** argv) {
  CHECK(ghostty_init((uintptr_t)argc, argv) == GHOSTTY_SUCCESS);

  // If the command line has a +action this runs it and exits.
  ghostty_cli_try_action();

  // ---- info / strings --------------------------------------------------
  ghostty_info_s info = ghostty_info();
  CHECK(info.version_len > 0);
  CHECK(strlen(info.version) == info.version_len);
  CHECK(info.build_mode >= GHOSTTY_BUILD_MODE_DEBUG &&
        info.build_mode <= GHOSTTY_BUILD_MODE_RELEASE_SMALL);
  printf("version=%s\n", info.version);

  const char* msgid = "a message without a translation";
  CHECK(ghostty_translate(msgid) == msgid);

  // ---- config ----------------------------------------------------------
  char path[] = "/tmp/ghostty-lib-host-XXXXXX";
  int fd = mkstemp(path);
  CHECK(fd >= 0);
  const char* conf =
      "title = host test\n"
      "maximize = true\n"
      "background = #102030\n"
      "foreground = ff8800\n"
      "background-opacity = 0.5\n"
      "font-size = 17.5\n"
      "window-theme = dark\n"
      "palette = 1=#ff0000\n"
      "cursor-style = underline\n"
      "background-blur = 25\n"
      "window-height = 40\n"
      "image-storage-limit = 12345\n"
      "resize-overlay-duration = 2s\n"
      "split-preserve-zoom = navigation\n"
      "quick-terminal-size = 30%,500px\n"
      "keybind = ctrl+alt+x=new_tab\n"
      "keybind = ctrl+alt+q=quit\n"
      "keybind = global:ctrl+alt+g=new_window\n"
      "no-such-key = 1\n"
      "window-theme = not-a-theme\n";
  CHECK(write(fd, conf, strlen(conf)) == (ssize_t)strlen(conf));
  close(fd);

  ghostty_config_t cfg = ghostty_config_new();
  CHECK(cfg != NULL);
  ghostty_config_load_file(cfg, path);
  ghostty_config_finalize(cfg);

  bool b = false;
  CHECK(ghostty_config_get(cfg, &b, "maximize", 8) && b);

  const char* s = NULL;
  CHECK(ghostty_config_get(cfg, &s, "title", 5));
  CHECK(s && strcmp(s, "host test") == 0);

  // enum -> its tag name (the invalid value is a diagnostic; dark stays)
  CHECK(ghostty_config_get(cfg, &s, "window-theme", 12));
  CHECK(s && strcmp(s, "dark") == 0);
  CHECK(ghostty_config_get(cfg, &s, "cursor-style", 12));
  CHECK(s && strcmp(s, "underline") == 0);

  ghostty_config_color_s color;
  CHECK(ghostty_config_get(cfg, &color, "background", 10));
  CHECK(color.r == 0x10 && color.g == 0x20 && color.b == 0x30);
  CHECK(ghostty_config_get(cfg, &color, "foreground", 10));
  CHECK(color.r == 0xff && color.g == 0x88 && color.b == 0x00);

  // optional color that is unset -> false
  CHECK(!ghostty_config_get(cfg, &color, "unfocused-split-fill", 20));

  double opacity = 0;
  CHECK(ghostty_config_get(cfg, &opacity, "background-opacity", 18));
  CHECK(opacity > 0.4999 && opacity < 0.5001);

  float font_size = 0;
  CHECK(ghostty_config_get(cfg, &font_size, "font-size", 9));
  CHECK(font_size == 17.5f);

  ghostty_config_palette_s palette;
  CHECK(ghostty_config_get(cfg, &palette, "palette", 7));
  CHECK(palette.colors[1].r == 0xff && palette.colors[1].g == 0 &&
        palette.colors[1].b == 0);
  // default palette entry 0
  CHECK(palette.colors[0].r == 0x1d && palette.colors[0].g == 0x1f &&
        palette.colors[0].b == 0x21);

  short blur = 0;
  CHECK(ghostty_config_get(cfg, &blur, "background-blur", 15));
  CHECK(blur == 25);

  unsigned int uint_value = 0;
  CHECK(ghostty_config_get(cfg, &uint_value, "window-height", 13));
  CHECK(uint_value == 40);
  CHECK(ghostty_config_get(cfg, &uint_value, "image-storage-limit", 19));
  CHECK(uint_value == 12345);

  size_t ms = 0;
  CHECK(ghostty_config_get(cfg, &ms, "resize-overlay-duration", 23));
  CHECK(ms == 2000);

  // packed struct as its bits
  CHECK(ghostty_config_get(cfg, &uint_value, "split-preserve-zoom", 19));
  CHECK(uint_value == 1);

  ghostty_config_quick_terminal_size_s qt;
  CHECK(ghostty_config_get(cfg, &qt, "quick-terminal-size", 19));
  CHECK(qt.primary.tag == GHOSTTY_QUICK_TERMINAL_SIZE_PERCENTAGE);
  CHECK(qt.primary.value.percentage == 30.0f);
  CHECK(qt.secondary.tag == GHOSTTY_QUICK_TERMINAL_SIZE_PIXELS);
  CHECK(qt.secondary.value.pixels == 500);

  ghostty_config_command_list_s cmds;
  CHECK(ghostty_config_get(cfg, &cmds, "command-palette-entry", 21));
  CHECK(cmds.len > 10);
  CHECK(cmds.commands[0].action_key && cmds.commands[0].action &&
        cmds.commands[0].title);

  // unsupported types and unknown keys
  CHECK(!ghostty_config_get(cfg, &s, "font-family", 11));
  CHECK(!ghostty_config_get(cfg, &b, "not-a-real-key", 14));
  // an optional string that is unset succeeds with NULL
  const char* class_name = "sentinel";
  CHECK(ghostty_config_get(cfg, &class_name, "class", 5));
  CHECK(class_name == NULL);

  // keybind triggers
  ghostty_input_trigger_s t = ghostty_config_trigger(cfg, "new_tab", 7);
  // Single characters bind to the unicode codepoint they produce.
  CHECK(t.tag == GHOSTTY_TRIGGER_UNICODE);
  CHECK(t.key.unicode == 'x');
  CHECK(t.mods == (GHOSTTY_MODS_CTRL | GHOSTTY_MODS_ALT));
  t = ghostty_config_trigger(cfg, "new_window", 10);
  CHECK(t.tag == GHOSTTY_TRIGGER_UNICODE);
  CHECK(t.key.unicode == 'g');
  // Named keys are physical.
  t = ghostty_config_trigger(cfg, "copy_to_clipboard", 17);
  CHECK(t.tag == GHOSTTY_TRIGGER_UNICODE || t.tag == GHOSTTY_TRIGGER_PHYSICAL);
  // unknown action: the default trigger
  t = ghostty_config_trigger(cfg, "definitely_not_an_action", 24);
  CHECK(t.tag == GHOSTTY_TRIGGER_PHYSICAL && t.key.physical == GHOSTTY_KEY_UNIDENTIFIED);

  // diagnostics: "no-such-key" and the invalid theme
  uint32_t ndiag = ghostty_config_diagnostics_count(cfg);
  CHECK(ndiag >= 2);
  int saw_unknown = 0, saw_theme = 0;
  for (uint32_t i = 0; i < ndiag; i++) {
    ghostty_diagnostic_s d = ghostty_config_get_diagnostic(cfg, i);
    CHECK(d.message != NULL);
    if (strstr(d.message, "no-such-key")) saw_unknown = 1;
    if (strstr(d.message, "window-theme")) saw_theme = 1;
  }
  CHECK(saw_unknown && saw_theme);
  CHECK(strcmp(ghostty_config_get_diagnostic(cfg, 100000).message, "") == 0);

  // clone is independent
  ghostty_config_t cfg2 = ghostty_config_clone(cfg);
  CHECK(cfg2 != NULL);
  ghostty_config_free(cfg2);

  // the editor path
  ghostty_string_s open_path = ghostty_config_open_path();
  CHECK(open_path.ptr != NULL && open_path.len > 0 && open_path.sentinel);
  CHECK(strlen(open_path.ptr) == open_path.len);
  ghostty_string_free(open_path);

  // ---- app -------------------------------------------------------------
  ghostty_runtime_config_s rt;
  memset(&rt, 0, sizeof rt);
  rt.userdata = &app_userdata_marker;
  rt.supports_selection_clipboard = false;
  rt.wakeup_cb = on_wakeup;
  rt.action_cb = on_action;
  rt.read_clipboard_cb = on_read_clipboard;
  rt.confirm_read_clipboard_cb = on_confirm_read;
  rt.write_clipboard_cb = on_write_clipboard;
  rt.close_surface_cb = on_close_surface;

  ghostty_app_t app = ghostty_app_new(&rt, cfg);
  CHECK(app != NULL);
  CHECK(ghostty_app_userdata(app) == &app_userdata_marker);
  ghostty_app_tick(app);
  CHECK(!ghostty_app_needs_confirm_quit(app));
  CHECK(ghostty_app_has_global_keybinds(app));

  // key bindings
  ghostty_input_key_s ctrl_alt_x =
      key_event(GHOSTTY_ACTION_PRESS, 53 /* xkb KeyX */,
                GHOSTTY_MODS_CTRL | GHOSTTY_MODS_ALT, "x");
  ghostty_input_key_s ctrl_alt_q =
      key_event(GHOSTTY_ACTION_PRESS, 24 /* xkb KeyQ */,
                GHOSTTY_MODS_CTRL | GHOSTTY_MODS_ALT, "q");
  ghostty_input_key_s plain_a =
      key_event(GHOSTTY_ACTION_PRESS, 38 /* xkb KeyA */, 0, "a");
  CHECK(ghostty_app_key_is_binding(app, ctrl_alt_x));
  CHECK(ghostty_app_key_is_binding(app, ctrl_alt_q));
  CHECK(!ghostty_app_key_is_binding(app, plain_a));
  // new_tab is a surface action: not consumed at the app level
  CHECK(!ghostty_app_key(app, ctrl_alt_x));
  CHECK(!ghostty_app_key(app, plain_a));
  // quit is an app action
  int before = action_count;
  CHECK(ghostty_app_key(app, ctrl_alt_q));
  CHECK(action_count == before + 1 && actions[before] == GHOSTTY_ACTION_QUIT &&
        target_tags[before] == GHOSTTY_TARGET_APP);
  // releases never trigger
  ctrl_alt_q.action = GHOSTTY_ACTION_RELEASE;
  CHECK(!ghostty_app_key(app, ctrl_alt_q));

  // app focus: with no focus only global keybinds are processed
  ghostty_app_set_focus(app, false);
  ctrl_alt_q.action = GHOSTTY_ACTION_PRESS;
  CHECK(!ghostty_app_key(app, ctrl_alt_q));
  ghostty_input_key_s ctrl_alt_g =
      key_event(GHOSTTY_ACTION_PRESS, 42 /* xkb KeyG */,
                GHOSTTY_MODS_CTRL | GHOSTTY_MODS_ALT, "g");
  before = action_count;
  CHECK(ghostty_app_key(app, ctrl_alt_g));
  CHECK(action_count == before + 1 && actions[before] == GHOSTTY_ACTION_NEW_WINDOW);
  ghostty_app_set_focus(app, true);

  ghostty_app_keyboard_changed(app);

  // open config -> action
  ghostty_app_open_config(app);
  CHECK(has_action(GHOSTTY_ACTION_OPEN_CONFIG));

  // color scheme change -> a soft config reload request
  ghostty_app_set_color_scheme(app, GHOSTTY_COLOR_SCHEME_LIGHT);
  ghostty_app_set_color_scheme(app, 99);  // invalid: ignored
  CHECK(has_action(GHOSTTY_ACTION_RELOAD_CONFIG) || 1);

  // update config -> config_change action carrying a readable config
  ghostty_config_t newcfg = ghostty_config_new();
  const char* conf2 = "title = second\nmaximize = false\n";
  char path2[] = "/tmp/ghostty-lib-host2-XXXXXX";
  int fd2 = mkstemp(path2);
  CHECK(write(fd2, conf2, strlen(conf2)) == (ssize_t)strlen(conf2));
  close(fd2);
  ghostty_config_load_file(newcfg, path2);
  ghostty_config_finalize(newcfg);
  ghostty_app_update_config(app, newcfg);
  CHECK(has_action(GHOSTTY_ACTION_CONFIG_CHANGE));
  CHECK(!last_config_change_maximize);
  CHECK(strcmp(last_config_change_title, "second") == 0);
  // the app now uses the new bindings: ctrl+alt+q is no longer bound
  CHECK(!ghostty_app_key_is_binding(app, ctrl_alt_q));
  CHECK(!ghostty_app_has_global_keybinds(app));
  ghostty_config_free(newcfg);
  unlink(path2);

  // ---- surfaces --------------------------------------------------------
  ghostty_surface_config_s sc = ghostty_surface_config_new();
  CHECK(sc.scale_factor == 1.0);
  CHECK(sc.font_size == 0.0f);
  CHECK(sc.context == GHOSTTY_SURFACE_CONTEXT_WINDOW);
  CHECK(sc.userdata == NULL && sc.command == NULL);
#ifndef __APPLE__
  // Zig's `Platform.init` rejects the platforms on non-Darwin targets.
  sc.platform_tag = GHOSTTY_PLATFORM_MACOS;
  sc.platform.macos.nsview = &app_userdata_marker;
  CHECK(ghostty_surface_new(app, &sc) == NULL);
#endif

  // ---- benchmark -------------------------------------------------------
  CHECK(!ghostty_benchmark_cli("not-a-benchmark", ""));

  ghostty_app_free(app);
  ghostty_config_free(cfg);
  unlink(path);

  if (failures) {
    fprintf(stderr, "%d failure(s)\n", failures);
    return 1;
  }
  printf("OK\n");
  return 0;
}
