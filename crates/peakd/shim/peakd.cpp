// Qt 6 + QtWebEngine **Quick** shim for the PEAK'D! kiosk shell. See peakd.h.
//
// Everything Qt lives here: the window, the app view, the Browser plugin's page
// views, the QWebChannel IPC bridge, permissions, signals -> Rust callbacks.
// All policy (config, command protocol, queues, scale, gestures, benchmark) is
// Rust.
//
// The app view and every page view are **items in one QQuickWidget scene**, not
// separate native windows. That is the point: the page is composited by the
// same renderer as the app's own DOM, so it moves in the same frame as its
// window, never blanks, and needs no X Shape masking.
//
//   geometry   -- item x/y/w/h (there is no separate window to move)
//   holes      -- where an HTML popup or a higher window overlaps the page, the
//                 page item is cut: a MultiEffect alpha mask for the visuals,
//                 `enabled` for input (a disabled item passes the press to the
//                 app view below, like an X Shape input region)
//   visibility -- item visible (hidden tile, overview, launcher)
//
// Scene: root -> [ appView (WebEngineView), pagesLayer -> page items, maskImage ]

#include "peakd.h"

#include "ipc_bridge.h"

#include <QAction>
#include <QApplication>
#include <QClipboard>
#include <QCoreApplication>
#include <QDir>
#include <QFile>
#include <QFileDialog>
#include <QFileInfo>
#include <QGuiApplication>
#include <QHash>
#include <QImage>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QKeyEvent>
#include <QMainWindow>
#include <QMenu>
#include <QMoveEvent>
#include <QPainter>
#include <QPalette>
#include <QPointer>
#include <QQmlComponent>
#include <QQmlContext>
#include <QQmlEngine>
#include <QQuickImageProvider>
#include <QQuickItem>
#include <QQuickWidget>
#include <QRegularExpression>
#include <QScreen>
#include <QStandardPaths>
#include <QStyle>
#include <QTimer>
#include <QUrl>
#include <QVariant>
#include <QtWebChannelQuick/qqmlwebchannel.h>
#include <QWebEngineDownloadRequest>
#include <QWebEnginePermission>
#include <QWebEngineProfile>
#include <QWebEngineSettings>
#include <QWebEngineUrlRequestInfo>
#include <QWebEngineUrlRequestInterceptor>
#include <QtQml/qqml.h>
#include <QtWebEngineQuick/qquickwebenginedownloadrequest.h>
#include <QtWebEngineQuick/qquickwebengineprofile.h>
#include <QtWebEngineQuick/qtwebenginequickglobal.h>

#include <cstdio>
#include <cstdlib>

/// Origin of the kiosk app view, captured from `peakd_qt_run`'s start URL.
/// Main-frame navigation and the privileged IPC bridge only trust this origin;
/// every other page is a browsing child view.
static QUrl g_app_origin;

static QString origin_of(const QUrl &url) {
    QString origin = url.scheme() + QStringLiteral("://") + url.host();
    if (url.port() != -1) {
        origin += QStringLiteral(":") + QString::number(url.port());
    }
    return origin;
}

static bool is_app_origin(const QUrl &url) {
    return g_app_origin.isValid() && url.isValid() &&
           origin_of(url).compare(origin_of(g_app_origin), Qt::CaseInsensitive) == 0;
}

namespace {

/// Scripts injected into every page (bootstrap + `peakd_qt_inject_script`).
struct InjectedScript {
    QString name;
    QString source;
};

struct Context {
    QQuickWidget *scene = nullptr;
    QQuickItem *scene_root = nullptr;
    QObject *app_view = nullptr;
    QObject *mask_image = nullptr;
    QQuickWebEngineProfile *profile = nullptr;
    QQuickWebEngineProfile *otr_profile = nullptr;
    peakd_ipc_cb ipc_cb = nullptr;
    peakd_view_cb view_cb = nullptr;
    peakd_pump_cb pump_cb = nullptr;
    peakd_js_cb js_cb = nullptr;
    void *userdata = nullptr;
    QHash<QString, QObject *> children;
    QString bootstrap;
};

// One shell per process (the kiosk); free functions reach it through this.
Context *g_ctx = nullptr;

// Scripts injected into every page, registered before the shell starts.
QList<InjectedScript> g_pending_scripts;

// Ad filtering + the Google sign-in User-Agent override (Rust owns the engine).
peakd_filter_cb g_filter_cb = nullptr;
peakd_ua_cb g_ua_cb = nullptr;

// The download manager's Rust side.
peakd_download_cb g_download_cb = nullptr;
void *g_cb_userdata = nullptr;
QString g_downloads_dir;
int g_download_seq = 0;
QHash<QString, QPointer<QQuickWebEngineDownloadRequest>> g_downloads;

// Copy events from Browser-plugin child views (the app's DOM cannot see them).
peakd_clipboard_cb g_clipboard_cb = nullptr;

// Debug hooks (probe / PEAKD_QT_EVAL) and their result routing.
bool g_probe = false;
bool g_eval_exit = false;

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

/// image://peakdmask/<key>: opaque white with the hole rects cleared -- what
/// MultiEffect.maskSource expects (it masks by alpha).
class MaskProvider : public QQuickImageProvider {
public:
    MaskProvider() : QQuickImageProvider(QQuickImageProvider::Image) {}

    QString add(const QList<QRectF> &holes, int w, int h) {
        QImage image(qMax(1, w), qMax(1, h), QImage::Format_ARGB32_Premultiplied);
        image.fill(Qt::white);
        QPainter painter(&image);
        painter.setCompositionMode(QPainter::CompositionMode_Clear);
        for (const QRectF &rect : holes) painter.fillRect(rect, Qt::transparent);
        painter.end();
        const QString key = QString::number(++seq_);
        images_.insert(key, image);
        // One page is in view at a time; a small cache is plenty.
        while (images_.size() > 24) images_.remove(images_.constBegin().key());
        return QStringLiteral("image://peakdmask/") + key;
    }

    QImage requestImage(const QString &id, QSize *size, const QSize &) override {
        const QImage image = images_.value(id);
        if (size) *size = image.size();
        return image;
    }

private:
    QHash<QString, QImage> images_;
    int seq_ = 0;
};

MaskProvider *g_masks = nullptr;

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
void handle_download(QQuickWebEngineDownloadRequest *download, bool incognito) {
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

/// Install the interceptor and the download handler on a profile.
void install_profile_services(QQuickWebEngineProfile *profile, bool incognito) {
    profile->setUrlRequestInterceptor(new ShinyRequestInterceptor());
    QObject::connect(profile, &QQuickWebEngineProfile::downloadRequested, profile,
                     [incognito](QQuickWebEngineDownloadRequest *download) {
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

/// The scene: the app view, the page layer, the mask image and the helpers the
/// shim calls. Kept as a string so the build needs no qrc/rcc step.
///
/// Every storage/cookie setting lives on the profile itself, so it is in place
/// before the profile is first used. Qt refuses to change the storage backend of
/// a profile that has already loaded something: it warns that the storage name is
/// empty, then switches the profile from off-the-record to disk-based, which can
/// drop the store the session cookie landed in. The app view is therefore
/// navigated from `Shell.setProfiles` — i.e. only after the request interceptor
/// and the download handler are installed — and not from a binding evaluated
/// while the scene is still being built.
const char *kSceneQml = R"QML(
import QtQuick
import QtWebEngine
import QtWebChannel

Item {
    id: root

    // Profiles are fully configured before first use: storage name, then the
    // off-the-record flag, then the disk paths — set in that order from
    // `Shell.setProfiles`, because the order is load-bearing (see the comment
    // above kSceneQml). Only the User-Agent is a plain property here.
    // (Keep QML comments free of quotes: moc scans this raw string literally.)
    WebEngineProfile {
        id: appProfile
        httpUserAgent: shell.userAgent
    }
    WebEngineProfile {
        id: otrProfile
        storageName: "peakd-otr"
        offTheRecord: true
        httpUserAgent: shell.userAgent
        // No cache/storage/cookie setters here on purpose: any persistent
        // storage setting flips an off-the-record profile to disk-based, which
        // would make an incognito tab keep its cookies after all.
    }

    WebEngineView {
        id: appView
        objectName: "appView"
        anchors.fill: parent
        profile: appProfile
        url: shell.appUrl
        onTitleChanged: shell.viewTitle("main", title)
        onUrlChanged: shell.viewUrl("main", url)
        onLoadingChanged: (info) => shell.viewLoad("main", info.status)
        onRenderProcessTerminated: (status, code) => shell.viewCrashed("main", status, code)
        onNewWindowRequested: (request) => shell.newWindow("main", request.requestedUrl)
        onNavigationRequested: (request) => {
            if (request.isMainFrame && !shell.isAppOrigin(request.url)) request.reject();
            else request.accept();
        }
        onJavaScriptConsoleMessage: (level, message, line, source) => shell.consoleMessage(level, message, line, source)
        onPermissionRequested: (permission) => { if (shell.allowPermission(permission.permissionType, permission.origin)) permission.grant(); else permission.deny(); }
        onContextMenuRequested: (request) => {
            shell.contextMenu("main", request.position.x, request.position.y, request.linkUrl, request.isContentEditable, request.selectedText)
            // QtWebEngine pops its own context menu for every request that stays
            // unaccepted, so the shell's menu would come up twice.
            request.accepted = true
        }
        onFileDialogRequested: (request) => shell.fileDialog(request.mode, request.acceptedMimeTypes, request.defaultFileName, request)
        onFullScreenRequested: (request) => request.accept()
        webChannel: shell.appChannel
    }

    Item { id: pagesLayer; objectName: "pagesLayer"; anchors.fill: parent }

    Image {
        id: maskImage
        objectName: "maskImage"
        visible: false
        asynchronous: false
        cache: false
        source: ""
    }

    function applyScripts(view) {
        var sources = shell.scriptSources();
        var list = [];
        for (var i = 0; i < sources.length; ++i) {
            list.push({
                name: sources[i].name,
                sourceCode: sources[i].source,
                injectionPoint: WebEngineScript.DocumentCreation,
                worldId: WebEngineScript.MainWorld,
                runsOnSubFrames: true
            });
        }
        view.userScripts.collection = list;
    }

    function pageAction(id, name) {
        var view = shell.pageItem(id);
        if (view) view.triggerWebAction(WebEngineView[name]);
    }

    function pageJs(id, token, script) {
        var view = shell.pageItem(id);
        if (!view) return;
        view.runJavaScript(script, function (result) {
            shell.pageJsResult(token, result === undefined ? "" : String(result));
        });
    }

    function runJs(script, id) {
        appView.runJavaScript(script, function (result) {
            shell.jsResult(id, typeof result === "string" ? result : JSON.stringify(result));
        });
    }

    Component.onCompleted: {
        shell.setProfiles(appProfile, otrProfile, maskImage, appView);
        applyScripts(appView);
        // The app plays TTS/radio/YouTube without a click on some paths, and
        // full-screen video is allowed; the app's window never parses the UA.
        appView.settings.playbackRequiresUserGesture = false;
        appView.settings.fullScreenSupportEnabled = true;
        appView.settings.javascriptCanOpenWindows = true;
    }
}
)QML";

/// One page: a WebEngineView item whose holes come from the shell, wired to the
/// same callbacks as the app view but with no privileged IPC bridge.
const char *kPageQml = R"QML(
import QtQuick
import QtQuick.Effects
import QtWebEngine
import QtWebChannel

WebEngineView {
    id: view
    property string pageId: ""
    objectName: "pageView"
    visible: false
    backgroundColor: "#ffffff"
    onTitleChanged: shell.viewTitle(pageId, title)
    onUrlChanged: shell.viewUrl(pageId, url)
    onLoadingChanged: (info) => shell.viewLoad(pageId, info.status)
    onRenderProcessTerminated: (status, code) => shell.viewCrashed(pageId, status, code)
    onNewWindowRequested: (request) => shell.newWindow(pageId, request.requestedUrl)
    onJavaScriptConsoleMessage: (level, message, line, source) => shell.consoleMessage(level, message, line, source)
    onPermissionRequested: (permission) => { if (shell.allowPermission(permission.permissionType, permission.origin)) permission.grant(); else permission.deny(); }
    onContextMenuRequested: (request) => {
        shell.contextMenu(pageId, request.position.x, request.position.y, request.linkUrl, request.isContentEditable, request.selectedText)
        request.accepted = true
    }
    onFileDialogRequested: (request) => shell.fileDialog(request.mode, request.acceptedMimeTypes, request.defaultFileName, request)
    settings.playbackRequiresUserGesture: false
    settings.fullScreenSupportEnabled: true
    settings.javascriptCanOpenWindows: true
    layer.enabled: false
    layer.effect: MultiEffect { maskEnabled: true; maskSource: shell.maskItem }
}
)QML";

/// Everything the scene calls back into. One instance, exposed as `shell`.
class Shell : public QObject {
    Q_OBJECT
    /// The app URL. Empty until `setProfiles` has configured and serviced the
    /// profiles; the scene binds the app view's `url` to it, so the first load
    /// starts only once everything is in place.
    Q_PROPERTY(QUrl appUrl READ appUrl NOTIFY appUrlChanged)
    Q_PROPERTY(QString storagePath READ storagePath CONSTANT)
    Q_PROPERTY(QString cachePath READ cachePath CONSTANT)
    Q_PROPERTY(QString userAgent READ userAgent CONSTANT)
    Q_PROPERTY(QObject *appBridge READ appBridge CONSTANT)
    Q_PROPERTY(QObject *childBridge READ childBridge CONSTANT)
    Q_PROPERTY(QObject *maskItem READ maskItem CONSTANT)
    Q_PROPERTY(QObject *appChannel READ appChannel CONSTANT)

signals:
    /// Emitted when `set_app_url` hands the scene its first URL. Declared before
    /// the setters on purpose: moc parses the class in one pass and rejects
    /// `emit` of a signal it has not seen yet.
    void appUrlChanged();

public:
    QUrl appUrl() const { return app_url_; }
    QString storagePath() const { return storage_path_; }
    QString cachePath() const { return cache_path_; }
    QString userAgent() const { return user_agent_; }
    QObject *appBridge() const { return app_bridge_; }
    QObject *childBridge() const { return child_bridge_; }
    QObject *maskItem() const { return mask_item_; }
    QObject *appChannel() const { return app_channel_; }

    void set_app_url(const QUrl &url) {
        if (app_url_ == url) return;
        app_url_ = url;
        emit appUrlChanged();
    }
    void set_storage(const QString &storage, const QString &cache) {
        storage_path_ = storage;
        cache_path_ = cache;
    }
    void set_user_agent(const QString &agent) { user_agent_ = agent; }
    void set_bridges(QObject *app_bridge, QObject *child_bridge) {
        app_bridge_ = app_bridge;
        child_bridge_ = child_bridge;
    }
    void set_app_channel(QObject *channel) { app_channel_ = channel; }

    /// {name, source} for the bootstrap and every injected script.
    Q_INVOKABLE QVariantList scriptSources() const {
        QVariantList list;
        if (g_ctx && !g_ctx->bootstrap.isEmpty()) {
            list.append(QVariantMap{{QStringLiteral("name"), QStringLiteral("peakd:ipc")},
                                    {QStringLiteral("source"), g_ctx->bootstrap}});
        }
        for (const InjectedScript &script : g_pending_scripts) {
            list.append(QVariantMap{{QStringLiteral("name"), script.name},
                                    {QStringLiteral("source"), script.source}});
        }
        return list;
    }

    Q_INVOKABLE bool isAppOrigin(const QUrl &url) const { return is_app_origin(url); }

    Q_INVOKABLE QObject *pageItem(const QString &id) const {
        return g_ctx ? g_ctx->children.value(id, nullptr) : nullptr;
    }

    /// The profiles, the mask image and the app view, handed over by the scene.
    Q_INVOKABLE void setProfiles(QObject *profile, QObject *otr, QObject *mask_image,
                                 QObject *app_view) {
        if (!g_ctx) return;
        g_ctx->profile = qobject_cast<QQuickWebEngineProfile *>(profile);
        g_ctx->otr_profile = qobject_cast<QQuickWebEngineProfile *>(otr);
        g_ctx->mask_image = mask_image;
        g_ctx->app_view = app_view;
        mask_item_ = mask_image;
        install_profile_services(g_ctx->profile, /*incognito=*/false);
        if (g_ctx->otr_profile) install_profile_services(g_ctx->otr_profile, /*incognito=*/true);
        // Order matters here, and it is deterministic because this is C++ and not
        // a set of QML properties: the storage name must be registered *before*
        // the off-the-record flag is cleared (Qt warns and can drop the store when
        // that switch happens with an empty name), and the cache path has to land
        // before the cache type (setting the type resets the path). All of it
        // happens before the app view is pointed anywhere, so no load can race it.
        if (g_ctx->profile) {
            g_ctx->profile->setStorageName(QStringLiteral("peakd"));
            g_ctx->profile->setOffTheRecord(false);
            g_ctx->profile->setPersistentStoragePath(storage_path_);
            g_ctx->profile->setCachePath(cache_path_);
            g_ctx->profile->setHttpCacheType(QQuickWebEngineProfile::DiskHttpCache);
            g_ctx->profile->setPersistentCookiesPolicy(
                QQuickWebEngineProfile::ForcePersistentCookies);
        }
        std::printf("peakd: user agent   %s\n", user_agent_.toUtf8().constData());
        // First load of the app view. Deliberately here and not in the QML:
        // the profile must be fully configured (storage, cookies, UA, request
        // interceptor) before the first request, which is also what keeps the
        // session cookie durable across a reload.
        set_app_url(g_app_origin);
    }

    Q_INVOKABLE void viewTitle(const QString &id, const QString &title) {
        emit_view(id, "title", title);
    }
    Q_INVOKABLE void viewUrl(const QString &id, const QUrl &url) {
        emit_view(id, "url", url.toString());
    }
    Q_INVOKABLE void viewLoad(const QString &id, int status) {
        // WebEngineView: 0 started, 1 stopped, 2 succeeded, 3 failed.
        const char *phase = "started";
        if (status == 2) phase = "finished";
        else if (status == 3 || status == 1) phase = "failed";
        emit_view(id, "load", QString::fromLatin1(phase));
        if (id == QLatin1String("main") && status == 2) maybe_probe();
    }
    Q_INVOKABLE void viewCrashed(const QString &id, int status, int code) {
        emit_view(id, "crashed", QStringLiteral("%1,%2").arg(status).arg(code));
    }
    Q_INVOKABLE void newWindow(const QString &id, const QUrl &url) {
        emit_view(id, "new-window", url.toString());
    }
    Q_INVOKABLE void consoleMessage(int level, const QString &message, int line,
                                    const QString &source) {
        const char *tag = "log";
        if (level == 1) tag = "info";
        else if (level == 2) tag = "warn";
        else if (level == 3) tag = "error";
        std::fprintf(stderr, "peakd: console[%s] %s:%d %s\n", tag, source.toUtf8().constData(),
                     line, message.toUtf8().constData());
    }

    /// The old QWebEnginePage permission policy: the app's own origin gets
    /// capture devices, clipboard reads and the position the HUD's weather,
    /// the clock's timezone and the traveler GPS all resolve from; a browsed
    /// page does not.
    Q_INVOKABLE bool allowPermission(int type, const QUrl &origin) const {
        const auto kind = static_cast<QWebEnginePermission::PermissionType>(type);
        using Kind = QWebEnginePermission::PermissionType;
        const bool capture = kind == Kind::MediaAudioCapture || kind == Kind::MediaVideoCapture ||
                             kind == Kind::MediaAudioVideoCapture;
        const bool clipboard = kind == Kind::ClipboardReadWrite;
        const bool geolocation = kind == Kind::Geolocation;
        return is_app_origin(origin) && (capture || clipboard || geolocation);
    }

    /// A context menu for a page: the actions the old widgets shim got from
    /// `createStandardContextMenu()`, built here and popped over the scene.
    Q_INVOKABLE void contextMenu(const QString &id, qreal x, qreal y, const QUrl &link_url,
                                 bool editable, const QString &selection) {
        if (!g_ctx || !g_ctx->scene) return;
        auto *menu = new QMenu(g_ctx->scene);
        QObject::connect(menu, &QMenu::aboutToHide, menu, &QObject::deleteLater);
        const auto add_action = [&](const QString &label, const char *action, bool enabled) {
            QAction *item = menu->addAction(label);
            item->setEnabled(enabled);
            const QString view_id = id;
            const QString verb = QString::fromLatin1(action);
            QObject::connect(item, &QAction::triggered, menu, [view_id, verb]() {
                QMetaObject::invokeMethod(g_ctx->scene_root, "pageAction",
                                          Q_ARG(QVariant, view_id), Q_ARG(QVariant, verb));
            });
        };
        if (link_url.isValid() && !link_url.isEmpty()) {
            QAction *copy_link = menu->addAction(QStringLiteral("Copy Link Address"));
            const QUrl url = link_url;
            QObject::connect(copy_link, &QAction::triggered, menu, [url]() {
                QGuiApplication::clipboard()->setText(url.toString());
            });
            menu->addSeparator();
        }
        add_action(QStringLiteral("Back"), "Back", true);
        add_action(QStringLiteral("Forward"), "Forward", true);
        add_action(QStringLiteral("Reload"), "Reload", true);
        menu->addSeparator();
        if (editable) {
            add_action(QStringLiteral("Cut"), "Cut", true);
            add_action(QStringLiteral("Copy"), "Copy", !selection.isEmpty());
            add_action(QStringLiteral("Paste"), "Paste", true);
        } else {
            add_action(QStringLiteral("Copy"), "Copy", !selection.isEmpty());
        }
        add_action(QStringLiteral("Select All"), "SelectAll", true);
        menu->popup(g_ctx->scene->mapToGlobal(QPoint(qRound(x), qRound(y))));
    }

    /// The file pickers the app's plugins use (`web/js/files.js` pickFiles,
    /// image import, uploads) and the ones a page requests.
    Q_INVOKABLE void fileDialog(int mode, const QStringList &mime_types,
                                const QString &default_name, QObject *request) {
        if (!request || !g_ctx) return;
        const QString filters = mime_filters(mime_types).join(QStringLiteral(";;"));
        QWidget *parent = g_ctx->scene;
        QStringList files;
        switch (mode) {
        case 0: {  // FileModeOpen
            const QString file =
                QFileDialog::getOpenFileName(parent, QStringLiteral("Open"), QString(), filters);
            if (!file.isEmpty()) files << file;
            break;
        }
        case 1:  // FileModeOpenMultiple
            files = QFileDialog::getOpenFileNames(parent, QStringLiteral("Open"), QString(), filters);
            break;
        case 2: {  // FileModeUploadFolder
            const QString dir =
                QFileDialog::getExistingDirectory(parent, QStringLiteral("Choose folder"));
            if (!dir.isEmpty()) files << dir;
            break;
        }
        case 3: {  // FileModeSave
            const QString file = QFileDialog::getSaveFileName(parent, QStringLiteral("Save"),
                                                             default_name, filters);
            if (!file.isEmpty()) files << file;
            break;
        }
        default:
            break;
        }
        request->setProperty("accepted", true);
        if (files.isEmpty()) QMetaObject::invokeMethod(request, "dialogReject");
        else QMetaObject::invokeMethod(request, "dialogAccept", Q_ARG(QVariant, QVariant(files)));
    }

    /// The selection a page reported for a Ctrl+C made inside it.
    Q_INVOKABLE void pageJsResult(const QString &, const QString &text) {
        if (!g_clipboard_cb || text.isEmpty()) return;
        g_clipboard_cb(g_cb_userdata, "copy", text.toUtf8().constData());
    }

    /// Results of `runJs`: id 1 is the benchmark, negative ids are the debug
    /// hooks (eval, probe), everything else is ignored like before.
    Q_INVOKABLE void jsResult(int id, const QString &value) {
        if (id == -1) {  // PEAKD_QT_EVAL
            std::printf("peakd: eval %s\n", value.toUtf8().constData());
            std::fflush(stdout);
            if (const char *path = std::getenv("PEAKD_QT_SCREENSHOT")) {
                const QImage image = g_ctx->scene->grabFramebuffer();
                image.save(QString::fromLocal8Bit(path), "PNG");
                std::printf("peakd: screenshot -> %s\n", path);
                std::fflush(stdout);
            }
            if (g_eval_exit) QCoreApplication::quit();
            return;
        }
        if (id == -2) {  // probe
            std::printf("peakd: loadFinished=true probe=%s\n", value.toUtf8().constData());
            std::fflush(stdout);
            if (const char *path = std::getenv("PEAKD_QT_SCREENSHOT")) {
                const QImage image = g_ctx->scene->grabFramebuffer();
                image.save(QString::fromLocal8Bit(path), "PNG");
                std::printf("peakd: screenshot -> %s\n", path);
                std::fflush(stdout);
            }
            QCoreApplication::quit();
            return;
        }
        if (!g_ctx || !g_ctx->js_cb) return;
        g_ctx->js_cb(g_ctx->userdata, id, value.toUtf8().constData());
    }

private:
    /// The probe the migration checks use: one script on the app view, then quit.
    void maybe_probe() {
        if (!g_ctx || !g_probe) return;
        g_probe = false;
        QTimer::singleShot(9000, g_ctx->scene_root, []() {
            const QString script = QStringLiteral(R"JS(
(() => {
  const out = {};
  try {
    out.optionalChaining = ({ a: { b: 1 } })?.a?.b === 1;
    out.nullish = (null ?? 'x') === 'x';
    out.ua = navigator.userAgent;
    out.secureContext = window.isSecureContext;
    out.devicePixelRatio = window.devicePixelRatio;
    out.hidden = document.hidden;
    out.title = document.title;
    out.readyState = document.readyState;
    out.ipc = typeof window.ipc;
    out.tiles = document.querySelectorAll('.tile').length;
    out.canvases = document.querySelectorAll('canvas').length;
    out.htmlLength = document.body ? document.body.innerHTML.length : 0;
  } catch (e) { out.syntax = String(e); }
  return JSON.stringify(out);
})()
)JS");
            QMetaObject::invokeMethod(g_ctx->scene_root, "runJs", Q_ARG(QVariant, script),
                                      Q_ARG(QVariant, -2));
        });
    }

    QUrl app_url_;
    QString storage_path_;
    QString cache_path_;
    QString user_agent_;
    QObject *app_bridge_ = nullptr;
    QObject *child_bridge_ = nullptr;
    QObject *mask_item_ = nullptr;
    QObject *app_channel_ = nullptr;
};

Shell g_shell;

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

/// Ctrl/Cmd+C pressed inside a Browser-plugin page.
///
/// The page is a separate web view, so the app's DOM never sees this key and
/// nothing would record the copy for the clipboard history. The filter runs the
/// engine's own Copy action -- so a site's copy handler still wins -- reports
/// the selection to Rust (which forwards it to the app's clipboard service) and
/// swallows the key so the engine does not copy a second time.
class PeakdClipboardFilter : public QObject {
public:
    using QObject::QObject;

protected:
    bool eventFilter(QObject *watched, QEvent *event) override {
        if (event->type() != QEvent::KeyPress) return QObject::eventFilter(watched, event);
        auto *key = static_cast<QKeyEvent *>(event);
        const Qt::KeyboardModifiers mods = key->modifiers();
        if (!mods.testFlag(Qt::ControlModifier) && !mods.testFlag(Qt::MetaModifier)) {
            return QObject::eventFilter(watched, event);
        }
        // Shift keeps its engine meanings (Ctrl+Shift+C is the inspector), and
        // Alt-combos are the app's own shortcuts.
        if (mods.testFlag(Qt::ShiftModifier) || mods.testFlag(Qt::AltModifier)) {
            return QObject::eventFilter(watched, event);
        }
        if (key->key() != Qt::Key_C) return QObject::eventFilter(watched, event);

        const QString id = focused_page_id();
        if (id.isEmpty()) return QObject::eventFilter(watched, event);

        // Run the engine's Copy, then report whatever was selected (empty means
        // it was not a copy at all).
        QMetaObject::invokeMethod(g_ctx->scene_root, "pageAction", Q_ARG(QVariant, id),
                                  Q_ARG(QVariant, QStringLiteral("Copy")));
        QMetaObject::invokeMethod(g_ctx->scene_root, "pageJs", Q_ARG(QVariant, id),
                                  Q_ARG(QVariant, QStringLiteral("copy")),
                                  Q_ARG(QVariant, QStringLiteral("window.getSelection().toString()")));
        return true;  // handled: the engine must not process the key again
    }

private:
    /// The page view owning the focused item, or an empty string.
    static QString focused_page_id() {
        if (!g_ctx) return QString();
        QQuickItem *item = nullptr;
        if (auto *window = qobject_cast<QQuickWindow *>(QGuiApplication::focusWindow())) {
            item = window->activeFocusItem();
        }
        if (!item) {
            if (auto *widget = qobject_cast<QQuickWidget *>(QGuiApplication::focusObject())) {
                item = widget->quickWindow() ? widget->quickWindow()->activeFocusItem() : nullptr;
            }
        }
        for (QQuickItem *walk = item; walk; walk = walk->parentItem()) {
            for (auto it = g_ctx->children.constBegin(); it != g_ctx->children.constEnd(); ++it) {
                if (static_cast<QObject *>(walk) == it.value()) return it.key();
            }
        }
        return QString();
    }
};

}  // namespace

IpcBridge::IpcBridge(peakd_ipc_cb callback, void *userdata, bool privileged, QObject *parent)
    : QObject(parent), callback_(callback), userdata_(userdata), privileged_(privileged) {}

void IpcBridge::postMessage(const QString &body) {
    // A non-privileged view (a browsing page) may only ask to leave the kiosk.
    if (!privileged_) {
        if (body != QLatin1String("peakd:exit")) {
            return;
        }
    } else {
        // Defense in depth over the navigation handler: the privileged bridge
        // only forwards while the document is still on the app origin.
        if (!g_ctx || !g_ctx->app_view ||
            !is_app_origin(g_ctx->app_view->property("url").toUrl())) {
            return;
        }
    }
    if (callback_) callback_(userdata_, body.toUtf8().constData());
}

void peakd_qt_inject_script(const char *name, const char *source) {
    g_pending_scripts.append(InjectedScript{QString::fromUtf8(name), QString::fromUtf8(source)});
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
    QtWebEngineQuick::initialize();

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
    g_probe = probe != 0;
    // Ctrl/Cmd+C inside a Browser page is invisible to the app; this app-wide
    // filter reports it into the clipboard history.
    app.installEventFilter(new PeakdClipboardFilter(&app));
    // The start URL *is* the app origin for this run; child views may browse
    // anywhere, the main view may not. The app view itself is not pointed at it
    // yet: `Shell.setProfiles` does that once the profile is configured, so no
    // load can race the storage/cookie setup.
    g_app_origin = QUrl(QString::fromLocal8Bit(url));

    // Storage paths and the cache live under the shell's data dir; the scene
    // binds them onto its profiles, which are created (and configured) before
    // anything loads. The UA is normalised here for the same reason: present as
    // plain Chromium, since the QtWebEngine token is an unusual fingerprint and
    // the app never parses it.
    const QString root = QString::fromLocal8Bit(data_dir);
    g_shell.set_storage(root + QStringLiteral("/storage"), root + QStringLiteral("/cache"));
    QString user_agent = QWebEngineProfile::defaultProfile()->httpUserAgent();
    user_agent.remove(QRegularExpression(QStringLiteral("QtWebEngine/[0-9.]+\\s*")));
    g_shell.set_user_agent(user_agent);

    // The IPC bridges: the app view is privileged (it drives the shell), every
    // page view shares one that may only ask to leave the kiosk.
    auto *app_bridge = new IpcBridge(ipc, userdata, /*privileged=*/true, &app);
    auto *child_bridge = new IpcBridge(ipc, userdata, /*privileged=*/false, &app);
    g_shell.set_bridges(app_bridge, child_bridge);
    auto *app_channel = new QQmlWebChannel(&app);
    app_channel->registerObject(QStringLiteral("ipc"), app_bridge);
    g_shell.set_app_channel(app_channel);
    g_masks = new MaskProvider;

    PeakdWindow window;
    window.setWindowTitle(g_window_title);
    auto *scene = new QQuickWidget(&window);
    scene->setResizeMode(QQuickWidget::SizeRootObjectToView);
    scene->engine()->addImageProvider(QStringLiteral("peakdmask"), g_masks);
    scene->rootContext()->setContextProperty(QStringLiteral("shell"), &g_shell);
    scene->engine()->setObjectOwnership(&g_shell, QQmlEngine::CppOwnership);

    // Build the scene from the embedded QML so no qrc/rcc step is needed.
    auto *component = new QQmlComponent(scene->engine(), scene);
    component->setData(QByteArray(kSceneQml), QUrl(QStringLiteral("qrc:/peakd/scene.qml")));
    if (component->isError()) {
        for (const QQmlError &error : component->errors())
            std::fprintf(stderr, "peakd: scene: %s\n", error.toString().toUtf8().constData());
        return 1;
    }
    QObject *root_object = component->create(scene->rootContext());
    if (!root_object) {
        std::fprintf(stderr, "peakd: could not create the scene\n");
        return 1;
    }
    scene->setContent(QUrl(), component, root_object);
    context.scene = scene;
    context.scene_root = qobject_cast<QQuickItem *>(root_object);
    window.setCentralWidget(scene);
    window.resize(g_window_width, g_window_height);

    // Show once the scene is up, and report the profile's final storage state
    // (read back, not echoed: Qt silently refuses some of these paths depending
    // on the order they were set in).
    QTimer::singleShot(0, &app, [&window]() {
        window.show();
        if (g_ctx && g_ctx->profile) {
            std::printf("peakd: profile off-the-record %d\n", g_ctx->profile->isOffTheRecord());
            std::printf("peakd: profile storage %s\n",
                        g_ctx->profile->persistentStoragePath().toUtf8().constData());
            std::printf("peakd: profile cache   %s\n",
                        g_ctx->profile->cachePath().toUtf8().constData());
        }
    });

    // The shell's pump: queues are drained on this 100 ms tick.
    auto *timer = new QTimer(&app);
    QObject::connect(timer, &QTimer::timeout, &app, [&context]() {
        if (context.pump_cb) context.pump_cb(context.userdata);
    });
    timer->start(100);

    // Debug hook: run one script on the app view after the page has settled.
    // Used by the migration checks; useful on the kiosk when something needs
    // inspecting. The script reports through `jsResult` (id -1).
    if (const char *eval_env = std::getenv("PEAKD_QT_EVAL")) {
        const QString script = QString::fromUtf8(eval_env);
        const int delay_ms = [] {
            const char *value = std::getenv("PEAKD_QT_EVAL_DELAY_MS");
            return value ? std::atoi(value) : 6000;
        }();
        g_eval_exit = std::getenv("PEAKD_QT_EVAL_EXIT") != nullptr;
        QTimer::singleShot(delay_ms, &app, [script]() {
            QMetaObject::invokeMethod(g_ctx->scene_root, "runJs", Q_ARG(QVariant, script),
                                      Q_ARG(QVariant, -1));
        });
    }

    std::printf("peakd: Qt %s (Quick scene)\n", qVersion());
    std::fflush(stdout);
    return app.exec();
}

void peakd_qt_quit(void) { QCoreApplication::quit(); }

void peakd_qt_main_load(const char *url) {
    if (g_ctx && g_ctx->app_view) g_ctx->app_view->setProperty("url", QUrl(QString::fromUtf8(url)));
}

void peakd_qt_main_zoom(double factor) {
    if (g_ctx && g_ctx->app_view) g_ctx->app_view->setProperty("zoomFactor", factor);
}

void peakd_qt_main_run_js(const char *script, int callback_id) {
    if (!g_ctx || !g_ctx->scene_root) return;
    QMetaObject::invokeMethod(g_ctx->scene_root, "runJs",
                              Q_ARG(QVariant, QString::fromUtf8(script)),
                              Q_ARG(QVariant, callback_id));
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
    if (!g_ctx || !g_ctx->scene_root) return;

    static QQmlComponent *page_component = nullptr;
    if (!page_component) {
        page_component = new QQmlComponent(g_ctx->scene->engine(), g_ctx->scene);
        page_component->setData(QByteArray(kPageQml), QUrl(QStringLiteral("qrc:/peakd/page.qml")));
        if (page_component->isError()) {
            for (const QQmlError &error : page_component->errors())
                std::fprintf(stderr, "peakd: page: %s\n", error.toString().toUtf8().constData());
            return;
        }
    }

    const QString key = QString::fromUtf8(id);
    QQuickWebEngineProfile *profile =
        (incognito && g_ctx->otr_profile) ? g_ctx->otr_profile : g_ctx->profile;
    QVariantMap initial;
    initial.insert(QStringLiteral("pageId"), key);
    initial.insert(QStringLiteral("profile"), QVariant::fromValue<QObject *>(profile));
    QObject *object =
        page_component->createWithInitialProperties(initial, qmlContext(g_ctx->scene_root));
    auto *view = qobject_cast<QQuickItem *>(object);
    if (!view) {
        std::fprintf(stderr, "peakd: could not create page view %s\n", id);
        return;
    }
    QQuickItem *layer =
        g_ctx->scene_root->findChild<QQuickItem *>(QStringLiteral("pagesLayer"));
    view->setParentItem(layer ? layer : g_ctx->scene_root);
    view->setX(x);
    view->setY(y);
    view->setWidth(w);
    view->setHeight(h);
    view->setVisible(visible != 0);
    g_ctx->children.insert(key, view);

    if (QObject *child_bridge = g_shell.childBridge()) {
        auto *channel = new QQmlWebChannel(view);
        channel->registerObject(QStringLiteral("ipc"), child_bridge);
        view->setProperty("webChannel", QVariant::fromValue<QObject *>(channel));
    }

    // Scripts must exist before the first load: the app's modules read
    // `window.ipc` during import.
    QMetaObject::invokeMethod(g_ctx->scene_root, "applyScripts",
                              Q_ARG(QVariant, QVariant::fromValue(view)));
    view->setProperty("url", QUrl(QString::fromUtf8(url)));
}

void peakd_qt_view_navigate(const char *id, const char *url) {
    if (!g_ctx) return;
    if (QObject *view = g_ctx->children.value(QString::fromUtf8(id)))
        view->setProperty("url", QUrl(QString::fromUtf8(url)));
}

void peakd_qt_view_bounds(const char *id, int x, int y, int w, int h) {
    if (!g_ctx) return;
    if (auto *view = qobject_cast<QQuickItem *>(g_ctx->children.value(QString::fromUtf8(id)))) {
        view->setX(x);
        view->setY(y);
        view->setWidth(w);
        view->setHeight(h);
    }
}

void peakd_qt_view_visible(const char *id, int visible) {
    if (!g_ctx) return;
    if (auto *view = qobject_cast<QQuickItem *>(g_ctx->children.value(QString::fromUtf8(id)))) {
        view->setVisible(visible != 0);
        // Hidden tabs cost nothing to keep alive; the page resumes on switch.
        view->setProperty("lifecycleState", visible == 0 ? 1 /*Frozen*/ : 0 /*Active*/);
    }
}

// Cut a page where the app draws over it: `holes` are the CSS rects of open
// menus, the window's own popovers and any higher floating window, already
// scaled to native pixels. The item's own origin is subtracted here.
//
// The hole list is also the page's input region: a page with holes is disabled,
// so a press inside one reaches the app view below it (a disabled item is
// skipped by Qt Quick's delivery, exactly like an X Shape input region), while
// the page keeps rendering.
void peakd_qt_view_mask(const char *id, const char *holes_json) {
    if (!g_ctx) return;
    const QString key = QString::fromUtf8(id);
    auto *view = qobject_cast<QQuickItem *>(g_ctx->children.value(key));
    if (!view) return;

    const QJsonDocument doc =
        QJsonDocument::fromJson(QByteArray(holes_json ? holes_json : ""));
    const QJsonArray holes = doc.array();
    QList<QRectF> rects;
    for (const QJsonValue &value : holes) {
        const QJsonObject hole = value.toObject();
        const double dpr = hole.value(QStringLiteral("dpr")).toDouble(1.0);
        rects.append(QRectF(hole.value(QStringLiteral("x")).toDouble() * dpr - view->x(),
                            hole.value(QStringLiteral("y")).toDouble() * dpr - view->y(),
                            hole.value(QStringLiteral("w")).toDouble() * dpr,
                            hole.value(QStringLiteral("h")).toDouble() * dpr));
    }

    view->setEnabled(rects.isEmpty());

    const bool cut = !rects.isEmpty();
    if (QObject *layer = view->property("layer").value<QObject *>())
        layer->setProperty("enabled", cut);
    if (g_ctx->mask_image) {
        g_ctx->mask_image->setProperty(
            "source", cut ? g_masks->add(rects, qRound(view->width()), qRound(view->height()))
                          : QString());
    }
}

// Mirror the app's theme into Qt's own chrome (menus, dialogs) with a palette
// swap. Deliberately *not* via QStyleHints::setColorScheme: on X11 that asks the
// platform theme to re-apply itself, which re-configures platform windows, and Qt
// then dereferences a window that is already gone while handling the native event
// that results — a SIGSEGV in QXcbWindow::handleNativeEvent (reproduced on the
// kiosk's display). A palette touches no window, and the app styles its pages
// itself (the Browser plugin pushes the scheme for browsed pages).
void peakd_qt_set_color_scheme(const char *scheme) {
    if (!scheme) return;
    const bool dark =
        QString::fromUtf8(scheme).compare(QStringLiteral("dark"), Qt::CaseInsensitive) == 0;
    static int applied = -1; // -1 unset, 0 light, 1 dark
    if (applied == (dark ? 1 : 0)) return;
    applied = dark ? 1 : 0;

    QPalette palette;
    if (dark) {
        palette.setColor(QPalette::Window, QColor(0x1e, 0x1e, 0x1e));
        palette.setColor(QPalette::WindowText, QColor(0xf0, 0xf0, 0xf0));
        palette.setColor(QPalette::Base, QColor(0x27, 0x27, 0x27));
        palette.setColor(QPalette::AlternateBase, QColor(0x1e, 0x1e, 0x1e));
        palette.setColor(QPalette::Text, QColor(0xf0, 0xf0, 0xf0));
        palette.setColor(QPalette::Button, QColor(0x27, 0x27, 0x27));
        palette.setColor(QPalette::ButtonText, QColor(0xf0, 0xf0, 0xf0));
        palette.setColor(QPalette::ToolTipBase, QColor(0x27, 0x27, 0x27));
        palette.setColor(QPalette::ToolTipText, QColor(0xf0, 0xf0, 0xf0));
        palette.setColor(QPalette::Highlight, QColor(0x2f, 0x6f, 0xd0));
        palette.setColor(QPalette::HighlightedText, QColor(0xff, 0xff, 0xff));
        palette.setColor(QPalette::PlaceholderText, QColor(0x9a, 0x9a, 0x9a));
        palette.setColor(QPalette::Disabled, QPalette::Text, QColor(0x8a, 0x8a, 0x8a));
        palette.setColor(QPalette::Disabled, QPalette::WindowText, QColor(0x8a, 0x8a, 0x8a));
    } else {
        palette = QApplication::style()->standardPalette();
    }
    QApplication::setPalette(palette);
}

void peakd_qt_view_back(const char *id) {
    if (!g_ctx) return;
    QMetaObject::invokeMethod(g_ctx->scene_root, "pageAction",
                              Q_ARG(QVariant, QString::fromUtf8(id)),
                              Q_ARG(QVariant, QStringLiteral("Back")));
}

void peakd_qt_view_forward(const char *id) {
    if (!g_ctx) return;
    QMetaObject::invokeMethod(g_ctx->scene_root, "pageAction",
                              Q_ARG(QVariant, QString::fromUtf8(id)),
                              Q_ARG(QVariant, QStringLiteral("Forward")));
}

void peakd_qt_view_reload(const char *id) {
    if (!g_ctx) return;
    QMetaObject::invokeMethod(g_ctx->scene_root, "pageAction",
                              Q_ARG(QVariant, QString::fromUtf8(id)),
                              Q_ARG(QVariant, QStringLiteral("Reload")));
}

void peakd_qt_view_focus(const char *id) {
    if (!g_ctx) return;
    if (auto *view = qobject_cast<QQuickItem *>(g_ctx->children.value(QString::fromUtf8(id)))) {
        view->forceActiveFocus();
    }
}

// Paste text into a page: the Clipboard menu picked an entry while a Browser
// page was focused. The text goes onto the system clipboard first (the engine
// pastes from there), then the engine's Paste action runs on the view's focused
// element -- a site's paste handler still wins.
void peakd_qt_view_paste(const char *id, const char *text) {
    if (!g_ctx) return;
    if (!g_ctx->children.contains(QString::fromUtf8(id))) return;
    if (text) QGuiApplication::clipboard()->setText(QString::fromUtf8(text));
    peakd_qt_view_focus(id);
    QMetaObject::invokeMethod(g_ctx->scene_root, "pageAction",
                              Q_ARG(QVariant, QString::fromUtf8(id)),
                              Q_ARG(QVariant, QStringLiteral("Paste")));
}

void peakd_qt_view_close(const char *id) {
    if (!g_ctx) return;
    const QString key = QString::fromUtf8(id);
    if (QObject *view = g_ctx->children.take(key)) {
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

void peakd_qt_set_clipboard_cb(peakd_clipboard_cb cb, void *userdata) {
    g_clipboard_cb = cb;
    if (userdata) g_cb_userdata = userdata;
}

void peakd_qt_set_download_dir(const char *dir) { g_downloads_dir = QString::fromUtf8(dir); }

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

#include "peakd.moc"
