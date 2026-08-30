/* Compatibility shims for macOS system libsqlite3.
 *
 * Apple's libsqlite3.dylib is compiled with SQLITE_OMIT_LOAD_EXTENSION and
 * without SQLITE_ENABLE_UNLOCK_NOTIFY, so these two symbols are absent from the
 * system dylib. sqlx-sqlite references them unconditionally, so we supply
 * no-op stubs to let the app link against the system SQLite (a single shared
 * library) instead of bundling a private SQLite into every dlopen'd plugin.
 */

#include "sqlite3.h"

int sqlite3_load_extension(
    sqlite3 *db,
    const char *zFile,
    const char *zProc,
    char **pzErrMsg
) {
    (void)db;
    (void)zFile;
    (void)zProc;
    if (pzErrMsg) {
        *pzErrMsg = 0;
    }
    return SQLITE_ERROR;
}

int sqlite3_unlock_notify(
    sqlite3 *db,
    void (*xNotify)(void **apArg, int nArg),
    void *pNotifyArg
) {
    (void)db;
    (void)xNotify;
    (void)pNotifyArg;
    return SQLITE_OK;
}
