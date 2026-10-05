/* PEAK'D! web client — M0 service worker.
 *
 * This milestone only proves that the origin can register and activate a
 * service worker. No fetch handler yet: every request goes to the network as
 * usual. M1+ adds the Iroh tunnel and the request interception.
 */

const VERSION = "m0";

self.addEventListener("install", () => {
  self.skipWaiting();
});

self.addEventListener("activate", (event) => {
  event.waitUntil(self.clients.claim());
});

self.addEventListener("message", (event) => {
  if (event.data === "version") {
    event.source && event.source.postMessage({ version: VERSION });
  }
});
