// capture-render shim: one HTML graphic, offscreen, frame by frame.
#include <QApplication>
#include <QBuffer>
#include <QFile>
#include <QImage>
#include <QTimer>
#include <QWebEnginePage>
#include <QWebEngineProfile>
#include <QWebEngineSettings>
#include <QWebEngineUrlRequestInterceptor>
#include <QWebEngineView>
#include <cstdio>
#include <cstring>
#include <unistd.h>

namespace {

// Nothing leaves the page: only inline content may load.
class Offline : public QWebEngineUrlRequestInterceptor {
public:
    void interceptRequest(QWebEngineUrlRequestInfo &info) override {
        const QString s = info.requestUrl().scheme();
        if (s != "data" && s != "blob" && s != "about" && s != "capture") info.block(true);
    }
};

struct Args {
    QString html;
    int width = 640, height = 360, fps = 30;
    double duration = 4.0, at = -1.0;
    bool png = false;
};

bool parse(int argc, char **argv, Args &a) {
    for (int i = 1; i < argc; ++i) {
        auto next = [&](void) -> const char * { return i + 1 < argc ? argv[++i] : ""; };
        if (!strcmp(argv[i], "--html")) a.html = QString::fromLocal8Bit(next());
        else if (!strcmp(argv[i], "--width")) a.width = atoi(next());
        else if (!strcmp(argv[i], "--height")) a.height = atoi(next());
        else if (!strcmp(argv[i], "--fps")) a.fps = atoi(next());
        else if (!strcmp(argv[i], "--duration")) a.duration = atof(next());
        else if (!strcmp(argv[i], "--at")) a.at = atof(next());
        else if (!strcmp(argv[i], "--png")) a.png = true;
    }
    a.width = qBound(16, a.width, 3840);
    a.height = qBound(16, a.height, 3840);
    a.fps = qBound(1, a.fps, 60);
    a.duration = qBound(0.04, a.duration, 3600.0);
    return !a.html.isEmpty();
}

bool writeAll(const char *p, size_t n) {
    while (n) {
        ssize_t w = ::write(1, p, n);
        if (w <= 0) return false;
        p += w;
        n -= size_t(w);
    }
    return true;
}

class Renderer : public QObject {
public:
    Renderer(const Args &a, QWebEngineView *v) : args(a), view(v) {}

    void start() {
        total = args.at >= 0 ? 1 : qMax(1, int(args.duration * args.fps + 0.5));
        step();
    }

private:
    Args args;
    QWebEngineView *view;
    int frame = 0, total = 1, polls = 0;

    double timeOf(int i) const { return args.at >= 0 ? args.at : double(i) / args.fps; }

    void step() {
        if (frame >= total) { QApplication::exit(0); return; }
        const double t = timeOf(frame);
        polls = 0;
        view->page()->runJavaScript(QStringLiteral(
            "window.__capDone=false;try{capture.seek(%1)}catch(e){};"
            "requestAnimationFrame(()=>requestAnimationFrame(()=>{window.__capDone=true}));").arg(t, 0, 'f', 4));
        QTimer::singleShot(4, this, [this] { poll(); });
    }

    void poll() {
        view->page()->runJavaScript(QStringLiteral("window.__capDone===true"), [this](const QVariant &v) {
            if (v.toBool() || ++polls > 200) {
                // One more turn of the event loop so the compositor delivers.
                QTimer::singleShot(8, this, [this] { grab(); });
            } else {
                QTimer::singleShot(2, this, [this] { poll(); });
            }
        });
    }

    void grab() {
        QImage img = view->grab().toImage().convertToFormat(QImage::Format_RGBA8888);
        if (img.width() != args.width || img.height() != args.height)
            img = img.scaled(args.width, args.height, Qt::IgnoreAspectRatio, Qt::SmoothTransformation);
        bool ok;
        if (args.png) {
            QByteArray bytes;
            QBuffer buf(&bytes);
            buf.open(QIODevice::WriteOnly);
            img.save(&buf, "PNG");
            ok = writeAll(bytes.constData(), size_t(bytes.size()));
        } else {
            ok = true;
            for (int y = 0; y < img.height() && ok; ++y)
                ok = writeAll(reinterpret_cast<const char *>(img.constScanLine(y)), size_t(img.width()) * 4);
        }
        if (!ok) { QApplication::exit(3); return; }
        ++frame;
        step();
    }
};

} // namespace

extern "C" int capture_render_main(int argc, char **argv) {
    Args a;
    if (!parse(argc, argv, a)) {
        fprintf(stderr, "usage: liteaura-capture-render --html FILE --width W --height H [--fps F --duration S | --at T --png]\n");
        return 2;
    }
    if (qEnvironmentVariableIsEmpty("QT_QPA_PLATFORM")) qputenv("QT_QPA_PLATFORM", "offscreen");
    qputenv("QTWEBENGINE_CHROMIUM_FLAGS",
            "--disable-gpu --disable-gpu-compositing --force-device-scale-factor=1 --disable-features=Translate --mute-audio");
    qputenv("QT_SCALE_FACTOR", "1");
    QFile f(a.html);
    if (!f.open(QIODevice::ReadOnly)) { fprintf(stderr, "cannot read %s\n", qPrintable(a.html)); return 2; }
    const QString html = QString::fromUtf8(f.readAll());

    static int qargc = 1;
    static char name[] = "liteaura-capture-render";
    static char *qargv[] = {name, nullptr};
    QApplication app(qargc, qargv);

    auto *profile = new QWebEngineProfile(&app); // off the record: nothing on disk
    profile->setHttpCacheType(QWebEngineProfile::NoCache);
    profile->setPersistentCookiesPolicy(QWebEngineProfile::NoPersistentCookies);
    auto *interceptor = new Offline;
    profile->setUrlRequestInterceptor(interceptor);
    auto *page = new QWebEnginePage(profile, &app);
    page->setBackgroundColor(Qt::transparent);
    auto *s = page->settings();
    s->setAttribute(QWebEngineSettings::LocalContentCanAccessRemoteUrls, false);
    s->setAttribute(QWebEngineSettings::LocalContentCanAccessFileUrls, false);
    s->setAttribute(QWebEngineSettings::JavascriptCanOpenWindows, false);
    s->setAttribute(QWebEngineSettings::JavascriptCanAccessClipboard, false);
    s->setAttribute(QWebEngineSettings::PluginsEnabled, false);
    s->setAttribute(QWebEngineSettings::PdfViewerEnabled, false);
    s->setAttribute(QWebEngineSettings::ShowScrollBars, false);

    QWebEngineView view;
    view.setPage(page);
    view.setAttribute(Qt::WA_TranslucentBackground);
    view.setStyleSheet("background: transparent");
    view.resize(a.width, a.height);
    view.show();

    Renderer r(a, &view);
    bool started = false;
    QObject::connect(page, &QWebEnginePage::loadFinished, &app, [&](bool) {
        if (started) return;
        started = true;
        // Let fonts and the first layout settle.
        QTimer::singleShot(60, &app, [&] { r.start(); });
    });
    // A page that never finishes loading still renders what it has.
    QTimer::singleShot(8000, &app, [&] { if (!started) { started = true; r.start(); } });
    // Hard cap: a hostile page cannot keep the export waiting forever.
    QTimer::singleShot(int(qMax(30.0, a.duration * 4 + 30) * 1000), &app, [] {
        fprintf(stderr, "render timed out\n");
        QApplication::exit(4);
    });
    page->setHtml(html, QUrl("capture://graphic/"));
    const int rc = app.exec();
    // Everything is written; tearing Qt WebEngine down only risks a crash
    // in its own shutdown (profile, GPU thread). Leave at once.
    fflush(stdout);
    _exit(rc);
}
