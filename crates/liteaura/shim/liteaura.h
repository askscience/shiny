// Qt shim API for the LiteAura kiosk shell.
//
// Plain C ABI on purpose: all Qt objects stay behind it, Rust owns the policy
// (config, command protocol, queues, events, scale, gestures, benchmark).

#ifndef LITEAURA_QT_H
#define LITEAURA_QT_H

#ifdef __cplusplus
extern "C" {
#endif

/// Page -> shell: one `window.ipc.postMessage` body.
typedef void (*liteaura_ipc_cb)(void *userdata, const char *body);

/// View lifecycle. `id` is `main`, a child view id, or `window` (kind `move`,
/// payload `x,y`). Kinds: `load` (`finished`/`failed`), `title`, `url`,
/// `new-window`, `crashed`, `context-menu` (a page right-click; payload JSON
/// `{x,y,linkUrl,editable,selection}`, position local to the view item).
typedef void (*liteaura_view_cb)(void *userdata, const char *id, const char *kind,
                              const char *payload);

/// Called every ~100 ms on the Qt main thread (faster while a trackpad gesture
/// is live, see `liteaura_qt_pump_rate`): the shell's pump.
typedef void (*liteaura_pump_cb)(void *userdata);

/// Result of `liteaura_qt_main_run_js`.
typedef void (*liteaura_js_cb)(void *userdata, int callback_id, const char *value);

/// Ad-filter verdict for one request: 1 = block, 0 = allow.
typedef int (*liteaura_filter_cb)(void *userdata, const char *url, const char *first_party,
                               int resource_type, const char *method);

/// Per-host User-Agent override (null when none).
typedef const char *(*liteaura_ua_cb)(void *userdata, const char *host);

/// One download lifecycle event: `(userdata, id, kind, payload_json)`.
typedef void (*liteaura_download_cb)(void *userdata, const char *id, const char *kind,
                                  const char *payload);

/// One clipboard event from a child view: `kind` is `copy`, `text` the copied
/// text. Only copies made inside a Browser-plugin page are reported — the
/// app's own view handles its clipboard in its DOM.
typedef void (*liteaura_clipboard_cb)(void *userdata, const char *kind, const char *text);

/// Runs the Qt event loop. Returns the process exit code the shell decided on
/// (`liteaura_qt_quit` after setting it).
int liteaura_qt_run(const char *url, const char *data_dir, int probe, liteaura_ipc_cb ipc,
                 liteaura_view_cb view, liteaura_pump_cb pump, liteaura_js_cb js, void *userdata);

/// Install a DocumentCreation script into every page (main, child, future).
/// Must be called before `liteaura_qt_run`.
void liteaura_qt_inject_script(const char *name, const char *source);

/// Include the ad-filter scriptlets in every view's script collection, and
/// re-apply the collections of the views that already exist. Safe to call
/// before `liteaura_qt_run` (it only records the state until the scene is up).
void liteaura_qt_set_filter_scripts(int on);

/// Window title and size for `liteaura_qt_run` (defaults when unset).
void liteaura_qt_set_window(const char *title, int width, int height);

/// Show the main window fullscreen on the primary screen.
void liteaura_qt_set_fullscreen(int fullscreen);

/// The shell's own strings (file-dialog titles, name filters) as one
/// `{ "key": "text" }` JSON map from the shared catalogues; keys that are
/// absent keep the English literals compiled in here.
void liteaura_qt_set_strings(const char *strings_json);

/// Leave the event loop. The caller decides the process status afterwards.
void liteaura_qt_quit(void);

/// Main view.
void liteaura_qt_main_load(const char *url);
void liteaura_qt_main_zoom(double factor);
void liteaura_qt_main_run_js(const char *script, int callback_id);
/// Print a complete HTML document to PDF with Chromium's print pipeline (A4 or
/// Letter, margins in mm: `{"size":"A4","landscape":false,"margins":[t,r,b,l]}`).
/// The result reaches the app view as `window.__liteauraPrinted({id, ok, data|error})`.
void liteaura_qt_print_html(const char *id, const char *html, const char *options_json);

/// Set the pump timer's interval (ms, > 0) and, when `wake` is non-zero, run
/// the pump once as soon as the GUI thread is free. The one shim call that is
/// safe from any thread: both are queued onto the GUI thread. A no-op before
/// `liteaura_qt_run` has started the pump.
void liteaura_qt_pump_rate(int interval_ms, int wake);

/// Colour scheme the pages should render for (`"dark"` / `"light"`), mirrored
/// into Qt's style hint so `prefers-color-scheme` follows the app theme.
void liteaura_qt_set_color_scheme(const char *scheme);

/// Physical DPI of the primary screen (0 when unknown), and its size in px.
double liteaura_qt_screen_dpi(void);
void liteaura_qt_screen_size(int *width, int *height);

/// Browser-plugin child views, geometry in Qt logical pixels.
void liteaura_qt_view_create(const char *id, const char *url, int x, int y, int w, int h,
                          int visible, int incognito);
void liteaura_qt_view_navigate(const char *id, const char *url);
void liteaura_qt_view_bounds(const char *id, int x, int y, int w, int h);
void liteaura_qt_view_visible(const char *id, int visible);
/// Clip a view to everything but the CSS rects in `holes_json` (a JSON array of
/// `{x,y,w,h,dpr}`, already in the view's logical space). Empty restores it.
void liteaura_qt_view_mask(const char *id, const char *holes_json);
void liteaura_qt_view_back(const char *id);
void liteaura_qt_view_forward(const char *id);
void liteaura_qt_view_reload(const char *id);
void liteaura_qt_view_focus(const char *id);
/// Put `text` on the system clipboard and run the engine's Paste action on the
/// view (the Clipboard menu picked an entry while a page had focus).
void liteaura_qt_view_paste(const char *id, const char *text);
/// Run one named engine action on the view (the app's context menu over a page:
/// Cut / Copy / Paste / SelectAll), through the engine's own action so the
/// page's handlers still win.
void liteaura_qt_view_action(const char *id, const char *verb);
/// Evaluate `script` in the view's page. The result comes back through the
/// view callback as kind `page-js` with payload `{token, result}`.
void liteaura_qt_view_run_js(const char *id, const char *token, const char *script);
void liteaura_qt_view_close(const char *id);

/// Ad filtering + the Google sign-in User-Agent override. Callbacks are global
/// (the engine state is global), so a null userdata is fine.
void liteaura_qt_set_filter_cb(liteaura_filter_cb cb, void *userdata);
void liteaura_qt_set_ua_cb(liteaura_ua_cb cb, void *userdata);

/// Download management.
void liteaura_qt_set_download_cb(liteaura_download_cb cb, void *userdata);
void liteaura_qt_set_download_dir(const char *dir);
void liteaura_qt_download_action(const char *id, const char *action);

/// Clipboard copies made inside Browser child views (a Ctrl/Cmd+C the app DOM
/// can never see). Registered before `liteaura_qt_run`.
void liteaura_qt_set_clipboard_cb(liteaura_clipboard_cb cb, void *userdata);

#ifdef __cplusplus
}
#endif

#endif // LITEAURA_QT_H
