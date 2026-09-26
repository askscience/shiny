// Qt 6 + QtWebEngine shim for the PEAK'D! kiosk shell. See peakd.h.
//
// Everything Qt lives here: the window, the main view, the child views, the
// QWebChannel IPC bridge, permissions, signals -> Rust callbacks. All policy
// (config, command protocol, queues, scale, gestures, benchmark) is Rust.

#include "peakd.h"

#include "ipc_bridge.h"

#include <QApplication>
#include <QContextMenuEvent>
#include <QCoreApplication>
#include <QDir>
#include <QFile>
#include <QFileDialog>
#include <QHash>
#include <QJsonDocument>
#include <QList>
#include <QMainWindow>
#include <QMenu>
#include <QMoveEvent>
#include <QPixmap>
#include <QPointer>
#include <QRegularExpression>
#include <QScreen>
#include <QStandardPaths>
#include <QTimer>
#include <QUrl>
#include <QVariant>
#include <QWebChannel>
#include <QWebEngineDownloadRequest>
#include <QWebEngineNewWindowRequest>
#include <QWebEnginePage>
#include <QWebEngineProfile>
#include <QWebEngineScript>
#include <QWebEngineScriptCollection>
#include <QWebEngineSettings>
#include <QWebEngineView>

#include <cstdio>
#include <cstdlib>
#include <memory>

namespace {

struct Context {
    QMainWindow *window = nullptr;
    QWebEngineView *main_view = nullptr;
    QWebEngineProfile *profile = nullptr;
    peakd_ipc_cb ipc_cb = nullptr;
    peakd_view_cb view_cb = nullptr;
    peakd_pump_cb pump_cb = nullptr;
    peakd_js_cb js_cb = nullptr;
    void *userdata = nullptr;
    QHash<QString, QPointer<QWebEngineView>> children;
    QString bootstrap;
};

// One shell per process (the kiosk); free functions reach it through this.
Context *g_ctx = nullptr;

// Scripts injected into every page, registered before the shell starts.
QList<QWebEngineScript> g_pending_scripts;

// Window configuration, set before the shell starts.
QString g_window_title = QStringLiteral("peakd");
int g_window_width = 1280;
int g_window_height = 860;

void emit_view(const QString &id, const char *kind, const QString &payload) {
    if (g_ctx && g_ctx->view_cb) {
        g_ctx->view_cb(g_ctx->userdata, id.toUtf8().constData(), kind,
                       payload.toUtf8().constData());
    }
}

/// The JavaScript injected before any page script runs.
///
/// `window.ipc` must exist from the first line of the document, because the
/// app's modules read it during import (plugins/browser/web/plugin.js). The
/// QWebChannel handshake is asynchronous, so `postMessage` starts as a queue
/// and is re-pointed once the channel opens.
QString bootstrap_script(const QString &qwebchannel_js) {
    return qwebchannel_js + QStringLiteral(R"JS(
(function () {
  if (window.ipc) return;
  var queue = [];
  window.ipc = { postMessage: function (s) { queue.push(String(s)); } };
  // Only the top-level frame gets the WebChannel transport; in a subframe the
  // queue simply stays local (nothing there needs to reach the shell).
  if (typeof qt === 'undefined' || !qt.webChannelTransport) return;
  new QWebChannel(qt.webChannelTransport, function (channel) {
    var target = channel.objects.ipc;
    window.ipc = { postMessage: function (s) { target.postMessage(String(s)); } };
    while (queue.length) target.postMessage(queue.shift());
  });
})();
)JS");
}

QString load_qwebchannel_js() {
    QFile file(QStringLiteral(":/qtwebchannel/qwebchannel.js"));
    if (!file.open(QIODevice::ReadOnly)) {
        std::fprintf(stderr, "peakd: could not open :/qtwebchannel/qwebchannel.js\n");
        return {};
    }
    return QString::fromUtf8(file.readAll());
}

/// Name filters for the file dialog from the `accept` attribute's MIME types.
QStringList mime_filters(const QStringList &mime_types) {
    QStringList filters;
    for (const QString &mime : mime_types) {
        if (mime == QLatin1String("image/*")) {
            filters << QStringLiteral("Images (*.png *.jpg *.jpeg *.gif *.webp *.bmp *.svg *.avif)");
        } else if (mime == QLatin1String("video/*")) {
            filters << QStringLiteral("Videos (*.mp4 *.webm *.mkv *.mov *.avi)");
        } else if (mime == QLatin1String("audio/*")) {
            filters << QStringLiteral("Audio (*.mp3 *.ogg *.wav *.flac *.m4a *.opus)");
        } else if (mime == QLatin1String("application/pdf")) {
            filters << QStringLiteral("PDF (*.pdf)");
        } else if (mime == QLatin1String("text/*")) {
            filters << QStringLiteral("Text (*.txt *.md *.csv *.json *.html)");
        } else if (mime.contains(QLatin1Char('/'))) {
            filters << QStringLiteral("%1 (*)").arg(mime);
        }
    }
    filters << QStringLiteral("All files (*)");
    return filters;
}

/// QWebEnginePage with the shell's id, reporting navigation and console, and
/// answering the file pickers the app's plugins use (`web/js/files.js`
/// `pickFiles`, image import, uploads).
class PeakdPage : public QWebEnginePage {
public:
    PeakdPage(QWebEngineProfile *profile, QString id, QObject *parent)
        : QWebEnginePage(profile, parent), id_(std::move(id)) {}

    QString id() const { return id_; }

protected:
    bool acceptNavigationRequest(const QUrl &url, NavigationType, bool isMainFrame) override {
        if (isMainFrame) emit_view(id_, "url", url.toString());
        return true;
    }

    QStringList chooseFiles(FileSelectionMode mode, const QStringList &oldFiles,
                            const QStringList &acceptedMimeTypes) override {
        const QString filters = mime_filters(acceptedMimeTypes).join(QStringLiteral(";;"));
        QWidget *window = QApplication::activeWindow();
        switch (mode) {
        case FileSelectOpen: {
            const QString file = QFileDialog::getOpenFileName(
                window, QStringLiteral("Open"), QString(), filters);
            return file.isEmpty() ? QStringList() : QStringList{file};
        }
        case FileSelectOpenMultiple:
            return QFileDialog::getOpenFileNames(window, QStringLiteral("Open"), QString(),
                                                 filters);
        case FileSelectUploadFolder: {
            // `<input webkitdirectory>` / directory upload.
            const QString dir =
                QFileDialog::getExistingDirectory(window, QStringLiteral("Choose folder"));
            return dir.isEmpty() ? QStringList() : QStringList{dir};
        }
        case FileSelectSave: {
            const QString suggested = oldFiles.isEmpty() ? QString() : oldFiles.first();
            const QString file = QFileDialog::getSaveFileName(
                window, QStringLiteral("Save"), suggested, filters);
            return file.isEmpty() ? QStringList() : QStringList{file};
        }
        }
        return {};
    }

    void javaScriptConsoleMessage(JavaScriptConsoleMessageLevel level, const QString &message,
                                  int line, const QString &source) override {
        const char *tag = "log";
        switch (level) {
        case InfoMessageLevel:
            tag = "info";
            break;
        case WarningMessageLevel:
            tag = "warn";
            break;
        case ErrorMessageLevel:
            tag = "error";
            break;
        default:
            break;
        }
        std::fprintf(stderr, "peakd: console[%s] %s:%d %s\n", tag, source.toUtf8().constData(),
                     line, message.toUtf8().constData());
    }

private:
    QString id_;
};

/// A view with the standard browser context menu. Without this QtWebEngine
/// shows nothing on right-click, while WebKitGTK showed its own link/image
/// menu — the Browser plugin's pages rely on the engine for that.
class PeakdView : public QWebEngineView {
public:
    using QWebEngineView::QWebEngineView;

protected:
    void contextMenuEvent(QContextMenuEvent *event) override {
        QMenu *menu = createStandardContextMenu();
        if (!menu) {
            QWebEngineView::contextMenuEvent(event);
            return;
        }
        menu->popup(event->globalPos());
        QObject::connect(menu, &QMenu::aboutToHide, menu, &QObject::deleteLater);
    }
};

/// The top-level window, reporting moves (the benchmark's dragging signal).
class PeakdWindow : public QMainWindow {
protected:
    void moveEvent(QMoveEvent *event) override {
        const QPoint position = event->pos();
        emit_view(QStringLiteral("window"), "move",
                  QStringLiteral("%1,%2").arg(position.x()).arg(position.y()));
        QMainWindow::moveEvent(event);
    }
};

/// Connect the per-page signals the shell cares about. Lambdas only: no new
/// signals/slots, so this file needs moc solely for IpcBridge.
void connect_page(PeakdPage *page, const QString &id) {
    QObject::connect(page, &QWebEnginePage::titleChanged, page,
                     [id](const QString &title) { emit_view(id, "title", title); });
    QObject::connect(page, &QWebEnginePage::loadStarted, page, [id]() { emit_view(id, "load", "started"); });
    QObject::connect(page, &QWebEnginePage::loadFinished, page, [id](bool ok) {
        emit_view(id, "load", ok ? "finished" : "failed");
    });
    QObject::connect(page, &QWebEnginePage::renderProcessTerminated, page,
                     [id](QWebEnginePage::RenderProcessTerminationStatus status, int code) {
                         emit_view(id, "crashed", QStringLiteral("%1,%2").arg(status).arg(code));
                     });
    QObject::connect(page, &QWebEnginePage::newWindowRequested, page,
                     [id](QWebEngineNewWindowRequest &request) {
                         // The Browser plugin opens a tab instead of a popup; not
                         // calling openIn() rejects the request.
                         emit_view(id, "new-window", request.requestedUrl().toString());
                     });
    QObject::connect(
        page, &QWebEnginePage::featurePermissionRequested, page,
        [page](const QUrl &origin, QWebEnginePage::Feature feature) {
            switch (feature) {
            case QWebEnginePage::MediaAudioCapture:
            case QWebEnginePage::MediaVideoCapture:
            case QWebEnginePage::MediaAudioVideoCapture:
                // The app's own voice input and the pages the user browses
                // get mic/camera, exactly like the wry shell granted.
                page->setFeaturePermission(origin, feature,
                                           QWebEnginePage::PermissionGrantedByUser);
                break;
            default:
                break;
            }
        });
}

/// Install the IPC bridge, the bootstrap and any injected scripts into a page.
void install_page(PeakdPage *page) {
    auto *bridge = new IpcBridge(g_ctx->ipc_cb, g_ctx->userdata, page);
    auto *channel = new QWebChannel(page);
    channel->registerObject(QStringLiteral("ipc"), bridge);
    page->setWebChannel(channel);

    if (!g_ctx->bootstrap.isEmpty()) {
        QWebEngineScript script;
        script.setName(QStringLiteral("peakd:ipc"));
        script.setInjectionPoint(QWebEngineScript::DocumentCreation);
        script.setWorldId(QWebEngineScript::MainWorld);
        script.setRunsOnSubFrames(true);
        script.setSourceCode(g_ctx->bootstrap);
        page->scripts().insert(script);
    }

    for (const QWebEngineScript &script : g_pending_scripts) {
        page->scripts().insert(script);
    }

    connect_page(page, page->id());
}

PeakdPage *make_page(const QString &id, QObject *parent) {
    auto *page = new PeakdPage(g_ctx->profile, id, parent);
    install_page(page);
    return page;
}

} // namespace

IpcBridge::IpcBridge(peakd_ipc_cb callback, void *userdata, QObject *parent)
    : QObject(parent), callback_(callback), userdata_(userdata) {}

void IpcBridge::postMessage(const QString &body) {
    if (callback_) callback_(userdata_, body.toUtf8().constData());
}

void peakd_qt_inject_script(const char *name, const char *source) {
    QWebEngineScript script;
    script.setName(QString::fromUtf8(name));
    script.setInjectionPoint(QWebEngineScript::DocumentCreation);
    script.setWorldId(QWebEngineScript::MainWorld);
    script.setRunsOnSubFrames(true);
    script.setSourceCode(QString::fromUtf8(source));
    g_pending_scripts.append(script);
}

void peakd_qt_set_window(const char *title, int width, int height) {
    g_window_title = QString::fromUtf8(title);
    if (width > 0) g_window_width = width;
    if (height > 0) g_window_height = height;
}

int peakd_qt_run(const char *url, const char *data_dir, int probe, peakd_ipc_cb ipc,
                 peakd_view_cb view_cb, peakd_pump_cb pump, peakd_js_cb js, void *userdata) {
    // QtWebEngine wants a shared OpenGL context group; set before QApplication.
    QCoreApplication::setAttribute(Qt::AA_ShareOpenGLContexts);

    int argc = 1;
    char name[] = "peakd";
    char *argv[] = {name, nullptr};
    QApplication app(argc, argv);
    QApplication::setApplicationName("peakd");
    QApplication::setOrganizationName("shiny");

    Context context;
    context.ipc_cb = ipc;
    context.view_cb = view_cb;
    context.pump_cb = pump;
    context.js_cb = js;
    context.userdata = userdata;
    context.bootstrap = bootstrap_script(load_qwebchannel_js());
    g_ctx = &context;

    // One explicit, persistent profile shared by the main view and every
    // Browser-plugin child view: same cookie jar, same cache, same UA. A named
    // profile (not defaultProfile()) so the storage paths below are honoured
    // from the start.
    const QString root = QString::fromLocal8Bit(data_dir);
    auto *profile = new QWebEngineProfile(QStringLiteral("peakd"), &app);
    context.profile = profile;
    profile->setPersistentStoragePath(root + QStringLiteral("/storage"));
    profile->setCachePath(root + QStringLiteral("/cache"));
    profile->setHttpCacheType(QWebEngineProfile::DiskHttpCache);
    profile->setPersistentCookiesPolicy(QWebEngineProfile::ForcePersistentCookies);
    // The app plays TTS/radio/YouTube without a click on some paths.
    profile->settings()->setAttribute(QWebEngineSettings::PlaybackRequiresUserGesture, false);
    // web/js/fullscreen.js uses the Fullscreen API on plugin windows.
    profile->settings()->setAttribute(QWebEngineSettings::FullScreenSupportEnabled, true);
    // The Browser plugin opens links via a tab, never a popup.
    profile->settings()->setAttribute(QWebEngineSettings::JavascriptCanOpenWindows, true);

    std::printf("peakd: profile storage %s\n",
                profile->persistentStoragePath().toUtf8().constData());
    std::printf("peakd: profile cache   %s\n", profile->cachePath().toUtf8().constData());

    // Present as plain Chromium. The QtWebEngine token is an unusual
    // fingerprint, and the app never parses the UA. Stripping the token keeps
    // Chrome/<engine version> truthful — unlike the old WebKitGTK shell, which
    // claimed Safari while its TLS/JS fingerprint was WebKitGTK's.
    QString user_agent = profile->httpUserAgent();
    user_agent.remove(QRegularExpression(QStringLiteral("QtWebEngine/[0-9.]+\\s*")));
    profile->setHttpUserAgent(user_agent);
    std::printf("peakd: user agent   %s\n", user_agent.toUtf8().constData());

    // Downloads (a plain link with `download`, or Content-Disposition) land in
    // the user's Downloads folder; the app's own exports go through the Files
    // plugin's upload path instead.
    QObject::connect(
        profile, &QWebEngineProfile::downloadRequested, profile,
        [](QWebEngineDownloadRequest *download) {
            const QString dir =
                QStandardPaths::writableLocation(QStandardPaths::DownloadLocation);
            QDir().mkpath(dir);
            download->setDownloadDirectory(dir);
            download->accept();
            std::fprintf(stderr, "peakd: download %s -> %s\n",
                         download->downloadFileName().toUtf8().constData(),
                         dir.toUtf8().constData());
            QObject::connect(download, &QWebEngineDownloadRequest::stateChanged, download,
                             [download](QWebEngineDownloadRequest::DownloadState state) {
                                 if (state == QWebEngineDownloadRequest::DownloadCompleted) {
                                     std::fprintf(stderr, "peakd: download finished: %s\n",
                                                  QDir(download->downloadDirectory())
                                                      .filePath(download->downloadFileName())
                                                      .toUtf8()
                                                      .constData());
                                 } else if (state == QWebEngineDownloadRequest::DownloadInterrupted) {
                                     std::fprintf(stderr, "peakd: download interrupted: %s\n",
                                                  download->interruptReasonString().toUtf8().constData());
                                 }
                             });
        });

    PeakdWindow window;
    window.setWindowTitle(g_window_title);
    auto *main_page = make_page(QStringLiteral("main"), &window);
    auto *view = new PeakdView(&window);
    view->setPage(main_page);
    window.setCentralWidget(view);
    window.resize(g_window_width, g_window_height);
    context.window = &window;
    context.main_view = view;

    // The shell's pump: queues are drained on this 100 ms tick.
    auto *timer = new QTimer(&app);
    QObject::connect(timer, &QTimer::timeout, &app, [&context]() {
        if (context.pump_cb) context.pump_cb(context.userdata);
    });
    timer->start(100);

    std::printf("peakd: Qt %s\n", qVersion());
    std::fflush(stdout);

    if (probe) {
        QObject::connect(view, &QWebEngineView::loadFinished, [view](bool ok) {
            QTimer::singleShot(9000, view, [view, ok]() {
                const QString script = QStringLiteral(R"JS(
(() => {
  const out = {};
  try {
    out.optionalChaining = ({ a: { b: 1 } })?.a?.b === 1;
    out.nullish = (null ?? 'x') === 'x';
    out.modules = (() => { try { eval('import("data:text/javascript,")'); return true; } catch (e) { return String(e); } })();
  } catch (e) { out.syntax = String(e); }
  out.ua = navigator.userAgent;
  out.secureContext = window.isSecureContext;
  out.getUserMedia = !!(navigator.mediaDevices && navigator.mediaDevices.getUserMedia);
  out.devicePixelRatio = window.devicePixelRatio;
  out.hidden = document.hidden;
  out.title = document.title;
  out.readyState = document.readyState;
  out.ipc = typeof window.ipc;
  out.tiles = document.querySelectorAll('.tile').length;
  out.canvases = document.querySelectorAll('canvas').length;
  out.htmlLength = document.body ? document.body.innerHTML.length : 0;
  out.bodyChildren = document.body
    ? Array.from(document.body.children).slice(0, 8)
        .map(e => e.tagName + (e.id ? '#' + e.id : '') + (e.className ? '.' + String(e.className).split(' ')[0] : ''))
        .join('|')
    : null;
  out.shinyKeys = Object.keys(window).filter(k => k.indexOf('__shiny') === 0).join(',');
  return JSON.stringify(out);
})()
)JS");
                view->page()->runJavaScript(script, [view, ok](const QVariant &value) {
                    std::printf("peakd: loadFinished=%s probe=%s\n", ok ? "true" : "false",
                                value.toString().toUtf8().constData());
                    std::fflush(stdout);
                    if (const char *path = std::getenv("PEAKD_QT_SCREENSHOT")) {
                        const QString file = QString::fromLocal8Bit(path);
                        view->grab().save(file, "PNG");
                        std::printf("peakd: screenshot -> %s\n", path);
                    }
                    QCoreApplication::quit();
                });
            });
        });
    }

    // Debug hook: run one script on the main view after the page has settled.
    // Used by the migration checks; useful on the kiosk when something needs
    // inspecting. The script reports through console.log (printed below).
    if (const char *eval_env = std::getenv("PEAKD_QT_EVAL")) {
        const QString script = QString::fromUtf8(eval_env);
        const int delay_ms = [] {
            const char *value = std::getenv("PEAKD_QT_EVAL_DELAY_MS");
            return value ? std::atoi(value) : 6000;
        }();
        const bool exit_after = std::getenv("PEAKD_QT_EVAL_EXIT") != nullptr;
        auto ran = std::make_shared<bool>(false);
        QObject::connect(
            view, &QWebEngineView::loadFinished,
            [view, script, delay_ms, exit_after, ran](bool ok) {
                if (!ok || *ran) return;
                *ran = true;
                QTimer::singleShot(delay_ms, view, [view, script, exit_after]() {
                    view->page()->runJavaScript(script, [exit_after](const QVariant &value) {
                        std::printf("peakd: eval %s\n", value.toString().toUtf8().constData());
                        std::fflush(stdout);
                        if (exit_after) QCoreApplication::quit();
                    });
                });
            });
    }

    view->load(QUrl(QString::fromLocal8Bit(url)));
    window.show();
    return app.exec();
}

void peakd_qt_quit(void) { QCoreApplication::quit(); }

void peakd_qt_main_load(const char *url) {
    if (g_ctx && g_ctx->main_view) g_ctx->main_view->load(QUrl(QString::fromUtf8(url)));
}

void peakd_qt_main_zoom(double factor) {
    if (g_ctx && g_ctx->main_view) g_ctx->main_view->setZoomFactor(factor);
}

void peakd_qt_main_run_js(const char *script, int callback_id) {
    if (!g_ctx || !g_ctx->main_view) return;
    const int id = callback_id;
    g_ctx->main_view->page()->runJavaScript(QString::fromUtf8(script), [id](const QVariant &value) {
        if (!g_ctx || !g_ctx->js_cb) return;
        QString text;
        if (value.userType() == QMetaType::QString) {
            text = value.toString();
        } else if (value.isValid()) {
            text = QString::fromUtf8(QJsonDocument::fromVariant(value).toJson(QJsonDocument::Compact));
        }
        g_ctx->js_cb(g_ctx->userdata, id, text.toUtf8().constData());
    });
}

double peakd_qt_screen_dpi(void) {
    QScreen *screen = QApplication::primaryScreen();
    return screen ? screen->physicalDotsPerInch() : 0.0;
}

void peakd_qt_screen_size(int *width, int *height) {
    QScreen *screen = QApplication::primaryScreen();
    const QSize size = screen ? screen->geometry().size() : QSize();
    if (width) *width = size.width();
    if (height) *height = size.height();
}

void peakd_qt_view_create(const char *id, const char *url, int x, int y, int w, int h,
                          int visible) {
    if (!g_ctx || !g_ctx->main_view) return;

    const QString key = QString::fromUtf8(id);
    auto *view = new PeakdView(g_ctx->main_view);
    view->setPage(make_page(key, view));
    view->setGeometry(x, y, w, h);
    g_ctx->children.insert(key, view);
    view->load(QUrl(QString::fromUtf8(url)));
    view->setVisible(visible != 0);
}

void peakd_qt_view_navigate(const char *id, const char *url) {
    if (!g_ctx) return;
    if (auto view = g_ctx->children.value(QString::fromUtf8(id))) {
        view->load(QUrl(QString::fromUtf8(url)));
    }
}

void peakd_qt_view_bounds(const char *id, int x, int y, int w, int h) {
    if (!g_ctx) return;
    if (auto view = g_ctx->children.value(QString::fromUtf8(id))) {
        view->setGeometry(x, y, w, h);
    }
}

void peakd_qt_view_visible(const char *id, int visible) {
    if (!g_ctx) return;
    if (auto view = g_ctx->children.value(QString::fromUtf8(id))) {
        view->setVisible(visible != 0);
    }
}

void peakd_qt_view_back(const char *id) {
    if (!g_ctx) return;
    if (auto view = g_ctx->children.value(QString::fromUtf8(id))) {
        view->page()->triggerAction(QWebEnginePage::Back);
    }
}

void peakd_qt_view_forward(const char *id) {
    if (!g_ctx) return;
    if (auto view = g_ctx->children.value(QString::fromUtf8(id))) {
        view->page()->triggerAction(QWebEnginePage::Forward);
    }
}

void peakd_qt_view_reload(const char *id) {
    if (!g_ctx) return;
    if (auto view = g_ctx->children.value(QString::fromUtf8(id))) {
        view->page()->triggerAction(QWebEnginePage::Reload);
    }
}

void peakd_qt_view_focus(const char *id) {
    if (!g_ctx) return;
    if (auto view = g_ctx->children.value(QString::fromUtf8(id))) {
        view->setFocus();
    }
}

void peakd_qt_view_close(const char *id) {
    if (!g_ctx) return;
    const QString key = QString::fromUtf8(id);
    if (auto view = g_ctx->children.take(key)) {
        view->deleteLater();
    }
}
