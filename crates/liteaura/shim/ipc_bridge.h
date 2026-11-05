// QWebChannel object exposed to the page as `ipc`. One instance per page.
// The meta-object (Q_INVOKABLE) is what QWebChannel calls, so this one file is
// run through moc by build.rs.

#ifndef LITEAURA_IPC_BRIDGE_H
#define LITEAURA_IPC_BRIDGE_H

#include <QObject>
#include <QString>

#include "liteaura.h"

class IpcBridge : public QObject {
    Q_OBJECT

public:
    // `privileged` is true only for the app's own view (`id == "main"`). A
    // browsing page in a child web view gets a bridge that forwards nothing but
    // `liteaura:exit`: otherwise any site could drive the view manager or toggle
    // the ad blocker.
    IpcBridge(liteaura_ipc_cb callback, void *userdata, bool privileged,
              QObject *parent = nullptr);

public slots:
    void postMessage(const QString &body);

private:
    liteaura_ipc_cb callback_;
    void *userdata_;
    bool privileged_;
};

#endif // LITEAURA_IPC_BRIDGE_H
