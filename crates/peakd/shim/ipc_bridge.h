// QWebChannel object exposed to the page as `ipc`. One instance per page.
// The meta-object (Q_INVOKABLE) is what QWebChannel calls, so this one file is
// run through moc by build.rs.

#ifndef PEAKD_IPC_BRIDGE_H
#define PEAKD_IPC_BRIDGE_H

#include <QObject>
#include <QString>

#include "peakd.h"

class IpcBridge : public QObject {
    Q_OBJECT

public:
    IpcBridge(peakd_ipc_cb callback, void *userdata, QObject *parent = nullptr);

public slots:
    void postMessage(const QString &body);

private:
    peakd_ipc_cb callback_;
    void *userdata_;
};

#endif // PEAKD_IPC_BRIDGE_H
