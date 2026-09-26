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
#include <QFileInfo>
#include <QHash>
#include <QJsonDocument>
#include <QJsonObject>
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
#include <QWebEngineUrlRequestInfo>
#include <QWebEngineUrlRequestInterceptor>
#include <QWebEngineView>

#include <cstdio>
#include <cstdlib>
#include <memory>

namespace {

struct Context {
    QMainWindow *window = nullptr;
    QWebEngineView *main_view = nullptr;
    QWebEngineProfile *profile = nullptr;
    /// Off-the-record profile used by incognito tabs.
    QWebEngineProfile *otr_profile = nullptr;
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

// Ad filtering + the Google sign-in User-Agent override (Rust owns the engine).
peakd_filter_cb g_filter_cb = nullptr;
peakd_ua_cb g_ua_cb = nullptr;

// The download manager's Rust side.
peakd_download_cb g_download_cb = nullptr;
void *g_cb_userdata = nullptr;
QString g_downloads_dir;
int g_download_seq = 0;
QHash<QString, QPointer<QWebEngineDownloadRequest>> g_downloads;

/// Intercepts every request in a profile: asks the Rust engine whether to
/// block, and applies the per-host User-Agent override. Runs on QtWebEngine's
/// IO thread, so it only touches thread-safe globals and the Rust engine.
class ShinyRequestInterceptor : public QWebEngineUrlRequestInterceptor {
public:
    void interceptRequest(QWebEngineUrlRequestInfo &info) override {
        const QUrl url = info.requestUrl();
        if (g_ua_cb) {
            const QByteArray host = url.host().toUtf8();
            if (const char *ua = g_ua_cb(g_cb_userdata, host.constData())) {
                info.setHttpHeader(QByteArrayLiteral("User-Agent"), QByteArray(ua));
            }
        }
        if (!g_filter_cb) return;
        const QByteArray u = url.toString(QUrl::FullyEncoded).toUtf8();
        const QByteArray fp = info.firstPartyUrl().toString(QUrl::FullyEncoded).toUtf8();
        const QByteArray method = info.requestMethod();
        const int block = g_filter_cb(g_cb_userdata, u.constData(), fp.constData(),
                                      static_cast<int>(info.resourceType()), method.constData());
        if (block) info.block(true);
    }
};

/// A filesystem-safe download name.
QString sanitize_download_name(const QString &raw) {
    QString name = raw;
    name.replace(QRegularExpression(QStringLiteral("[\\\\/:*?\"<>|\\x00-\\x1f]")),
                 QStringLiteral("_"));
    name = name.trimmed();
    while (name.startsWith(QLatin1Char('.'))) name.remove(0, 1);
    if (name.isEmpty()) name = QStringLiteral("download");
    return name.left(200);
}

/// `dir/name`, with ` (n)` inserted before the extension until it is free.
QString unique_download_path(const QString &dir, const QString &name) {
    QFileInfo info(name);
    const QString stem = info.completeBaseName().isEmpty() ? name : info.completeBaseName();
    const QString suffix = info.suffix();
    QString candidate = QDir(dir).filePath(name);
    for (int n = 1; QFileInfo::exists(candidate) && n < 10000; ++n) {
        const QString next = suffix.isEmpty()
            ? QStringLiteral("%1 (%2)").arg(stem).arg(n)
            : QStringLiteral("%1 (%2).%3").arg(stem).arg(n).arg(suffix);
        candidate = QDir(dir).filePath(next);
    }
    return candidate;
}

void emit_download(QWebEngineDownloadRequest *download, const QString &id, const char *kind,
                   bool incognito, const QString &error) {
    if (!g_download_cb || !download) return;
    QJsonObject o;
    o[QStringLiteral("id")] = id;
    o[QStringLiteral("url")] = download->url().toString();
    o[QStringLiteral("host")] = download->url().host();
    o[QStringLiteral("file")] = download->downloadFileName();
    o[QStringLiteral("path")] =
        QDir(download->downloadDirectory()).filePath(download->downloadFileName());
    o[QStringLiteral("mime")] = download->mimeType();
    o[QStringLiteral("total")] = static_cast<double>(download->totalBytes());
    o[QStringLiteral("received")] = static_cast<double>(download->receivedBytes());
    o[QStringLiteral("incognito")] = incognito;
    QString state = QStringLiteral("downloading");
    switch (download->state()) {
    case QWebEngineDownloadRequest::DownloadRequested:
        state = QStringLiteral("requested");
        break;
    case QWebEngineDownloadRequest::DownloadInProgress:
        state = download->isPaused() ? QStringLiteral("paused") : QStringLiteral("downloading");
        break;
    case QWebEngineDownloadRequest::DownloadCompleted:
        state = QStringLiteral("completed");
        break;
    case QWebEngineDownloadRequest::DownloadCancelled:
        state = QStringLiteral("cancelled");
        break;
    case QWebEngineDownloadRequest::DownloadInterrupted:
        state = QStringLiteral("interrupted");
        break;
    }
    o[QStringLiteral("state")] = state;
    if (!error.isEmpty()) {
        o[QStringLiteral("error")] = error;
    } else if (download->state() == QWebEngineDownloadRequest::DownloadInterrupted) {
        o[QStringLiteral("error")] = download->interruptReasonString();
    }
    const QByteArray body = QJsonDocument(o).toJson(QJsonDocument::Compact);
    g_download_cb(g_cb_userdata, id.toUtf8().constData(), kind, body.constData());
}

/// Take ownership of one download: assign an id, choose a destination, accept
/// it, and stream lifecycle events to Rust.
void handle_download(QWebEngineDownloadRequest *download, bool incognito) {
    if (!download) return;
    const QString id = QStringLiteral("d%1").arg(++g_download_seq);

    QString dir;
    if (incognito) {
        const QString base = g_downloads_dir.isEmpty()
            ? QDir::tempPath() + QStringLiteral("/shiny-incognito")
            : g_downloads_dir + QStringLiteral("/.incognito");
        dir = base;
    } else {
        dir = g_downloads_dir;
    }
    if (dir.isEmpty()) {
        dir = QStandardPaths::writableLocation(QStandardPaths::DownloadLocation);
    }
    QDir().mkpath(dir);

    const QString name = QFileInfo(unique_download_path(dir, sanitize_download_name(
                                                             download->downloadFileName())))
                             .fileName();
    // Qt 6.5+: directory + file name are set before accept().
    download->setDownloadDirectory(dir);
    download->setDownloadFileName(name);
    g_downloads.insert(id, download);

    emit_download(download, id, "download", incognito, QString());

    QObject::connect(download, &QWebEngineDownloadRequest::receivedBytesChanged, download,
                     [download, id, incognito]() {
                         emit_download(download, id, "progress", incognito, QString());
                     });
    QObject::connect(download, &QWebEngineDownloadRequest::totalBytesChanged, download,
                     [download, id, incognito]() {
                         emit_download(download, id, "progress", incognito, QString());
                     });
    QObject::connect(download, &QWebEngineDownloadRequest::isPausedChanged, download,
                     [download, id, incognito]() {
                         emit_download(download, id, "progress", incognito, QString());
                     });
    QObject::connect(
        download, &QWebEngineDownloadRequest::stateChanged, download,
        [download, id, incognito](QWebEngineDownloadRequest::DownloadState state) {
            const char *kind = "progress";
            if (state == QWebEngineDownloadRequest::DownloadCompleted) kind = "completed";
            else if (state == QWebEngineDownloadRequest::DownloadCancelled) kind = "cancelled";
            else if (state == QWebEngineDownloadRequest::DownloadInterrupted) kind = "interrupted";
            emit_download(download, id, kind, incognito, QString());
        });

    std::fprintf(stderr, "peakd: download %s -> %s\n", name.toUtf8().constData(),
                 dir.toUtf8().constData());
    download->accept();
}

/// Copy the settings the app relies on from the persistent profile to another.
void mirror_profile_settings(QWebEngineProfile *to) {
    to->settings()->setAttribute(QWebEngineSettings::PlaybackRequiresUserGesture, false);
    to->settings()->setAttribute(QWebEngineSettings::FullScreenSupportEnabled, true);
    to->settings()->setAttribute(QWebEngineSettings::JavascriptCanOpenWindows, true);
}

/// Install the interceptor and the download handler on a profile.
void install_profile_services(QWebEngineProfile *profile, bool incognito) {
    profile->setUrlRequestInterceptor(new ShinyRequestInterceptor());
    QObject::connect(profile, &QWebEngineProfile::downloadRequested, profile,
                     [incognito](QWebEngineDownloadRequest *download) {
                         handle_download(download, incognito);
                     });
}

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
    // Only the app's own view may drive the shell. A child web view renders a
    // browsing page, so its bridge forwards nothing but the exit message.
    const bool privileged = page->id() == QLatin1String("main");
    auto *bridge = new IpcBridge(g_ctx->ipc_cb, g_ctx->userdata, privileged, page);
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

PeakdPage *make_page(const QString &id, QWebEngineProfile *profile, QObject *parent) {
    auto *page = new PeakdPage(profile, id, parent);
    install_page(page);
    return page;
}

} // namespace

IpcBridge::IpcBridge(peakd_ipc_cb callback, void *userdata, bool privileged, QObject *parent)
    : QObject(parent), callback_(callback), userdata_(userdata), privileged_(privileged) {}

void IpcBridge::postMessage(const QString &body) {
    // A non-privileged view (a browsing page) may only ask to leave the kiosk.
    if (!privileged_ && body != QLatin1String("peakd:exit")) {
        return;
    }
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
    mirror_profile_settings(profile);
    install_profile_services(profile, false);

    // The off-the-record profile backs incognito tabs: no cookies, no cache, no
    // storage. Same settings and the same interceptor (filtering and the Google
    // UA override apply there too).
    auto *otr_profile = new QWebEngineProfile(&app);
    otr_profile->setHttpUserAgent(profile->httpUserAgent());
    mirror_profile_settings(otr_profile);
    install_profile_services(otr_profile, true);
    context.otr_profile = otr_profile;

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

    PeakdWindow window;
    window.setWindowTitle(g_window_title);
    auto *main_page = make_page(QStringLiteral("main"), g_ctx->profile, &window);
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
                          int visible, int incognito) {
    if (!g_ctx || !g_ctx->main_view) return;

    const QString key = QString::fromUtf8(id);
    QWebEngineProfile *profile =
        (incognito && g_ctx->otr_profile) ? g_ctx->otr_profile : g_ctx->profile;
    auto *view = new PeakdView(g_ctx->main_view);
    view->setPage(make_page(key, profile, view));
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

void peakd_qt_set_filter_cb(peakd_filter_cb cb, void *userdata) {
    g_filter_cb = cb;
    if (userdata) g_cb_userdata = userdata;
}

void peakd_qt_set_ua_cb(peakd_ua_cb cb, void *userdata) {
    g_ua_cb = cb;
    if (userdata) g_cb_userdata = userdata;
}

void peakd_qt_set_download_cb(peakd_download_cb cb, void *userdata) {
    g_download_cb = cb;
    if (userdata) g_cb_userdata = userdata;
}

void peakd_qt_set_download_dir(const char *dir) {
    g_downloads_dir = QString::fromUtf8(dir);
}

void peakd_qt_download_action(const char *id, const char *action) {
    const QString key = QString::fromUtf8(id);
    auto download = g_downloads.value(key);
    if (!download) return;
    const QString verb = QString::fromUtf8(action);
    if (verb == QLatin1String("pause")) {
        download->pause();
    } else if (verb == QLatin1String("resume")) {
        download->resume();
    } else if (verb == QLatin1String("cancel")) {
        download->cancel();
    } else if (verb == QLatin1String("forget")) {
        g_downloads.remove(key);
        if (g_download_cb) {
            g_download_cb(g_cb_userdata, id, "removed", "{}");
        }
    }
}
