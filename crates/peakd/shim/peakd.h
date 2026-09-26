// Qt shim API for the PEAK'D! kiosk shell.
//
// Plain C ABI on purpose: all Qt objects stay behind it, Rust owns the policy
// (config, command protocol, queues, events, scale, gestures, benchmark).

#ifndef PEAKD_QT_H
#define PEAKD_QT_H

#ifdef __cplusplus
extern "C" {
#endif

/// Page -> shell: one `window.ipc.postMessage` body.
typedef void (*peakd_ipc_cb)(void *userdata, const char *body);

/// View lifecycle. `id` is `main`, a child view id, or `window` (kind `move`,
/// payload `x,y`). Kinds: `load` (`finished`/`failed`), `title`, `url`,
/// `new-window`, `crashed`.
typedef void (*peakd_view_cb)(void *userdata, const char *id, const char *kind,
                              const char *payload);

/// Called every ~100 ms on the Qt main thread: the shell's pump.
typedef void (*peakd_pump_cb)(void *userdata);

/// Result of `peakd_qt_main_run_js`.
typedef void (*peakd_js_cb)(void *userdata, int callback_id, const char *value);

/// Ad-filter verdict for one request: 1 = block, 0 = allow.
typedef int (*peakd_filter_cb)(void *userdata, const char *url, const char *first_party,
                               int resource_type, const char *method);

/// Per-host User-Agent override (null when none).
typedef const char *(*peakd_ua_cb)(void *userdata, const char *host);

/// One download lifecycle event: `(userdata, id, kind, payload_json)`.
typedef void (*peakd_download_cb)(void *userdata, const char *id, const char *kind,
                                  const char *payload);

/// Runs the Qt event loop. Returns the process exit code the shell decided on
/// (`peakd_qt_quit` after setting it).
int peakd_qt_run(const char *url, const char *data_dir, int probe, peakd_ipc_cb ipc,
                 peakd_view_cb view, peakd_pump_cb pump, peakd_js_cb js, void *userdata);

/// Install a DocumentCreation script into every page (main, child, future).
/// Must be called before `peakd_qt_run`.
void peakd_qt_inject_script(const char *name, const char *source);

/// Window title and size for `peakd_qt_run` (defaults when unset).
void peakd_qt_set_window(const char *title, int width, int height);

/// Leave the event loop. The caller decides the process status afterwards.
void peakd_qt_quit(void);

/// Main view.
void peakd_qt_main_load(const char *url);
void peakd_qt_main_zoom(double factor);
void peakd_qt_main_run_js(const char *script, int callback_id);

/// Physical DPI of the primary screen (0 when unknown), and its size in px.
double peakd_qt_screen_dpi(void);
void peakd_qt_screen_size(int *width, int *height);

/// Browser-plugin child views, geometry in Qt logical pixels.
void peakd_qt_view_create(const char *id, const char *url, int x, int y, int w, int h,
                          int visible, int incognito);
void peakd_qt_view_navigate(const char *id, const char *url);
void peakd_qt_view_bounds(const char *id, int x, int y, int w, int h);
void peakd_qt_view_visible(const char *id, int visible);
void peakd_qt_view_back(const char *id);
void peakd_qt_view_forward(const char *id);
void peakd_qt_view_reload(const char *id);
void peakd_qt_view_focus(const char *id);
void peakd_qt_view_close(const char *id);

/// Ad filtering + the Google sign-in User-Agent override. Callbacks are global
/// (the engine state is global), so a null userdata is fine.
void peakd_qt_set_filter_cb(peakd_filter_cb cb, void *userdata);
void peakd_qt_set_ua_cb(peakd_ua_cb cb, void *userdata);

/// Download management.
void peakd_qt_set_download_cb(peakd_download_cb cb, void *userdata);
void peakd_qt_set_download_dir(const char *dir);
void peakd_qt_download_action(const char *id, const char *action);

#ifdef __cplusplus
}
#endif

#endif // PEAKD_QT_H
