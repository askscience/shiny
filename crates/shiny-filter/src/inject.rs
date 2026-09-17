//! Injection: the document-start runtime shim, cosmetic CSS, and scriptlets.
//!
//! Static rewriting handles the markup a page ships with. Everything a script
//! does afterwards — `fetch`, `XMLHttpRequest`, `WebSocket`, `EventSource`, a
//! late-injected `<script src>` — would otherwise escape the proxy and load
//! unfiltered. The shim is a small ES5-compatible script installed *before*
//! any page script runs, which rewrites those URLs back through the proxy.
//!
//! It must never throw: an exception here would run before the page's own
//! code and could break the site. Every hook is individually wrapped and
//! falls back to the original implementation on any error.

/// Placeholder the shim uses to learn the proxy + document URL.
///
/// `r##"…"##` rather than `r#"…"#`: the shim legitimately contains `"#`
/// (the fragment check), which would otherwise terminate the raw literal.
const SHIM_TEMPLATE: &str = r##"(function () {
  "use strict";
  var PROXY = "__SHINY_PROXY__";
  var DOC = "__SHINY_DOC__";

  // Leave anything the page should not see us touch.
  function passthrough(u) {
    if (typeof u !== "string") return false;
    var s = u.trim().toLowerCase();
    return (
      s.indexOf("data:") === 0 ||
      s.indexOf("blob:") === 0 ||
      s.indexOf("about:") === 0 ||
      s.indexOf("javascript:") === 0 ||
      s.indexOf("mailto:") === 0 ||
      s.indexOf("tel:") === 0 ||
      u.charAt(0) === "#"
    );
  }

  function rewrite(u) {
    try {
      if (typeof u !== "string" || u === "" || passthrough(u)) return u;
      if (u.indexOf(PROXY) === 0) return u;      // already proxied
      if (u.indexOf("/p/http") === 0) return PROXY + u;
      var abs = new URL(u, DOC);
      if (abs.protocol !== "http:" && abs.protocol !== "https:") return u;
      return PROXY + "/p/" + abs.protocol.slice(0, -1) + "/" + abs.href.slice(abs.protocol.length + 2);
    } catch (e) {
      return u; // never break the request over a rewrite failure
    }
  }

  // ---- fetch -------------------------------------------------------------
  if (typeof window.fetch === "function") {
    var origFetch = window.fetch.bind(window);
    window.fetch = function (input, init) {
      try {
        if (typeof input === "string") {
          input = rewrite(input);
        } else if (input && typeof input === "object" && input.url) {
          var r = rewrite(input.url);
          if (r !== input.url) input = new Request(r, input);
        }
      } catch (e) {}
      return origFetch(input, init);
    };
  }

  // ---- XMLHttpRequest ----------------------------------------------------
  try {
    var origOpen = XMLHttpRequest.prototype.open;
    XMLHttpRequest.prototype.open = function (method, url) {
      var rest = Array.prototype.slice.call(arguments, 2);
      try {
        url = rewrite(url);
      } catch (e) {}
      return origOpen.apply(this, [method, url].concat(rest));
    };
  } catch (e) {}

  // ---- WebSocket ---------------------------------------------------------
  try {
    if (typeof window.WebSocket === "function") {
      var OrigWS = window.WebSocket;
      var WS = function (url, protocols) {
        // WebSockets are tunneled by upgrading a proxied path.
        var target = rewrite(url).replace(/^http/, "ws");
        return protocols === undefined
          ? new OrigWS(target)
          : new OrigWS(target, protocols);
      };
      WS.prototype = OrigWS.prototype;
      WS.CONNECTING = OrigWS.CONNECTING;
      WS.OPEN = OrigWS.OPEN;
      WS.CLOSING = OrigWS.CLOSING;
      WS.CLOSED = OrigWS.CLOSED;
      window.WebSocket = WS;
    }
  } catch (e) {}

  // ---- EventSource -------------------------------------------------------
  try {
    if (typeof window.EventSource === "function") {
      var OrigES = window.EventSource;
      window.EventSource = function (url, config) {
        return new OrigES(rewrite(url), config);
      };
      window.EventSource.prototype = OrigES.prototype;
    }
  } catch (e) {}

  // ---- history / navigation ---------------------------------------------
  // Keep the address bar and history entries free of proxy URLs where the
  // browser allows it, so the user sees the real site.
  try {
    ["pushState", "replaceState"].forEach(function (name) {
      var orig = history[name];
      if (typeof orig !== "function") return;
      history[name] = function (state, title, url) {
        return orig.call(history, state, title, url);
      };
    });
  } catch (e) {}

  // ---- dynamic <script src> / <link href> --------------------------------
  // A MutationObserver catches nodes inserted by page script, which the
  // static rewriter never sees.
  try {
    var observer = new MutationObserver(function (records) {
      for (var i = 0; i < records.length; i++) {
        var nodes = records[i].addedNodes;
        for (var j = 0; j < nodes.length; j++) {
          var el = nodes[j];
          if (!el || el.nodeType !== 1) continue;
          if (el.tagName === "SCRIPT" && el.src) el.src = rewrite(el.src);
          if (el.tagName === "LINK" && el.href) el.href = rewrite(el.href);
          if (el.tagName === "IMG" && el.src) el.src = rewrite(el.src);
          if (el.tagName === "IFRAME" && el.src) el.src = rewrite(el.src);
        }
      }
    });
    observer.observe(document.documentElement || document, {
      childList: true,
      subtree: true,
    });
  } catch (e) {}

  // ---- navigation bridge (the in-app browser window only) ----------------
  // When this document is framed by the browser window, report the real page
  // URL to the parent so its address bar, history and Back/Forward/Reload
  // controls follow in-page navigations (a link click, a redirect, an SPA
  // pushState) rather than only the URL the parent last typed. The parent
  // drives navigation back through `history` so no hop escapes the proxy.
  //
  // The same channel carries the window's link affordances: right-click opens
  // the parent's "open in new tab / copy link" menu, a `target=_blank` link (or
  // `window.open`) opens a new tab instead of leaving the frame, and hovering a
  // link lets the parent show a preview. All of it is best-effort: every hook
  // is wrapped, and a failure here must never break the page.
  //
  // The native shell loads the proxy top-level (`parent === window`), so this
  // whole block is skipped there. Every hook is wrapped; nothing may throw
  // before the page's own scripts run.
  try {
    if (window.parent && window.parent !== window) {
      var realUrl = function () {
        try {
          var marker = PROXY + "/p/";
          var href = location.href;
          if (href.indexOf(marker) === 0) {
            var rest = href.slice(marker.length);
            var slash = rest.indexOf("/");
            if (slash > 0) {
              var scheme = rest.slice(0, slash);
              if (scheme === "http" || scheme === "https") {
                return scheme + "://" + rest.slice(slash + 1);
              }
            }
          }
        } catch (e) {}
        return DOC;
      };
      // The real URL behind a link the page gives us. The rewriter has already
      // turned hrefs into proxy paths, so this undoes `PROXY + "/p/<scheme>/…"`
      // and resolves anything relative against the document.
      var realUrlOf = function (href) {
        try {
          if (!href) return null;
          var abs = new URL(href, DOC).href;
          var marker = PROXY + "/p/";
          if (abs.indexOf(marker) === 0) {
            var rest = abs.slice(marker.length);
            var slash = rest.indexOf("/");
            if (slash > 0) {
              var scheme = rest.slice(0, slash);
              if (scheme === "http" || scheme === "https") {
                return scheme + "://" + rest.slice(slash + 1);
              }
            }
          }
          if (abs.indexOf("http://") === 0 || abs.indexOf("https://") === 0) return abs;
        } catch (e) {}
        return null;
      };
      var post = function (message) {
        try {
          parent.postMessage(message, "*");
        } catch (e) {}
      };
      var announce = function () {
        try {
          parent.postMessage(
            { type: "shiny:location", url: realUrl(), title: document.title || "" },
            "*"
          );
        } catch (e) {}
      };
      var announceSoon = function () {
        setTimeout(announce, 0);
      };
      // The document URL is known at document-start; waiting for
      // DOMContentLoaded gives the page's own scripts a chance to update it.
      if (document.readyState === "complete" || document.readyState === "interactive") {
        announceSoon();
      } else {
        document.addEventListener("DOMContentLoaded", announceSoon, { once: true });
      }
      window.addEventListener("load", announce);
      window.addEventListener("popstate", announceSoon);
      window.addEventListener("hashchange", announceSoon);

      window.addEventListener("message", function (event) {
        try {
          if (event.source !== parent) return;
          var d = event.data;
          if (!d || d.type !== "shiny:cmd") return;
          if (d.cmd === "back") history.back();
          else if (d.cmd === "forward") history.forward();
          else if (d.cmd === "reload") location.reload();
        } catch (e) {}
      });

      // Links that ask for a new tab — `target=_blank`, or `window.open` —
      // become a new tab in the window rather than a navigation of this frame
      // (or a hand-off to the OS browser). Named/`_top`/`_parent` targets keep
      // the old behaviour of staying put.
      try {
        document.addEventListener(
          "click",
          function (event) {
            try {
              var a = event.target && event.target.closest ? event.target.closest("a[href]") : null;
              if (!a) return;
              var target = (a.getAttribute("target") || "").toLowerCase();
              if (target && target !== "_self" && target !== "_top" && target !== "_parent") {
                // `_blank` (and any named browsing context): open a tab.
                if (target === "_blank") {
                  var url = realUrlOf(a.href);
                  if (url) {
                    event.preventDefault();
                    post({ type: "shiny:new-tab", url: url });
                    return;
                  }
                }
                a.target = "_self";
              }
            } catch (e) {}
          },
          true
        );
        var origOpen = window.open;
        if (typeof origOpen === "function") {
          window.open = function (url) {
            try {
              var real = realUrlOf(url);
              if (real) {
                post({ type: "shiny:new-tab", url: real });
                return null; // popup blocked, as far as the page is concerned
              }
            } catch (e) {}
            return origOpen.apply(window, arguments);
          };
        }
      } catch (e) {}

      // Right-click on a link: let the parent draw the window's own menu
      // (open in a new tab / current tab / copy the link) instead of a native
      // menu whose "open in new tab" cannot be honoured inside the frame.
      try {
        document.addEventListener(
          "contextmenu",
          function (event) {
            try {
              var a = event.target && event.target.closest ? event.target.closest("a[href]") : null;
              if (!a) return;
              var url = realUrlOf(a.href);
              if (!url) return;
              event.preventDefault();
              post({ type: "shiny:link-menu", url: url, x: event.clientX || 0, y: event.clientY || 0 });
            } catch (e) {}
          },
          true
        );
      } catch (e) {}

      // Hovering a link: tell the parent, which asks the server for the
      // target's title/description and shows a preview card. Debounced on the
      // parent side; `leave` cancels it.
      try {
        var hoverLink = function (event) {
          try {
            var a = event.target && event.target.closest ? event.target.closest("a[href]") : null;
            if (!a) return;
            var url = realUrlOf(a.href);
            if (!url) return;
            var r = a.getBoundingClientRect ? a.getBoundingClientRect() : null;
            post({
              type: "shiny:hover-link",
              url: url,
              rect: r ? { x: r.left, y: r.top, w: r.width, h: r.height } : null,
            });
          } catch (e) {}
        };
        document.addEventListener("pointerover", hoverLink, true);
        document.addEventListener("focusin", hoverLink, true);
        document.addEventListener(
          "pointerout",
          function (event) {
            try {
              var a = event.target && event.target.closest ? event.target.closest("a[href]") : null;
              if (a) post({ type: "shiny:leave-link" });
            } catch (e) {}
          },
          true
        );
      } catch (e) {}
    }
  } catch (e) {}

  // ---- native URL rewriting for the page's own API surface ---------------
  try {
    window.__shinyProxyRewrite = rewrite;
  } catch (e) {}
})();
"##;

/// The document-start shim, with the proxy base and document URL substituted.
pub fn runtime_shim(proxy_base: &str, document_url: &str) -> String {
    SHIM_TEMPLATE
        .replace("__SHINY_PROXY__", &json_escape(proxy_base))
        .replace("__SHINY_DOC__", &json_escape(document_url))
}

/// Escape a string for embedding inside a double-quoted JS literal.
fn json_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Everything the proxy wants to put in front of a document, in one call.
#[derive(Debug, Default, Clone)]
pub struct Injections {
    /// Cosmetic-filter CSS (hide ad slots).
    pub cosmetic_css: String,
    /// Scriptlet JS the filter lists asked for.
    pub scriptlets: String,
    /// The proxy runtime shim.
    pub shim: String,
}

/// Build the full `<style>` + `<script>` block to insert right after `<head>`.
pub fn head_injections(document_url: &str, proxy_base: &str, parts: &Injections) -> String {
    let mut html = String::with_capacity(parts.cosmetic_css.len() + parts.shim.len() + 256);

    if !parts.cosmetic_css.trim().is_empty() {
        html.push_str("<style data-shiny-filter=\"cosmetic\">\n");
        html.push_str(&parts.cosmetic_css);
        html.push_str("\n</style>\n");
    }

    // The shim must run before any page script, so it goes first in <head>.
    html.push_str("<script data-shiny-filter=\"shim\">");
    html.push_str(&runtime_shim(proxy_base, document_url));
    html.push_str("</script>\n");

    if !parts.scriptlets.trim().is_empty() {
        html.push_str("<script data-shiny-filter=\"scriptlets\">\ntry{\n");
        html.push_str(&parts.scriptlets);
        html.push_str("\n}catch(e){}\n</script>\n");
    }

    html
}

/// Insert a block immediately after the opening `<head>` tag.
///
/// Falls back to before `<body>`, and finally to the very start of the
/// document — insertions must never be silently dropped, because a document
/// without the shim leaks every dynamic request around the proxy.
pub fn insert_after_head(html: &str, block: &str) -> String {
    if let Some(pos) = find_tag_end(html, "<head") {
        let mut out = String::with_capacity(html.len() + block.len());
        out.push_str(&html[..pos]);
        out.push_str(block);
        out.push_str(&html[pos..]);
        return out;
    }
    if let Some(pos) = find_tag_end(html, "<body") {
        let mut out = String::with_capacity(html.len() + block.len());
        out.push_str(&html[..pos]);
        out.push_str(block);
        out.push_str(&html[pos..]);
        return out;
    }
    // Last resort: after `<html …>` or at the very top.
    if let Some(pos) = find_tag_end(html, "<html") {
        let mut out = String::with_capacity(html.len() + block.len());
        out.push_str(&html[..pos]);
        out.push_str(block);
        out.push_str(&html[pos..]);
        return out;
    }
    format!("{block}{html}")
}

/// Byte offset just past the `>` that closes the first case-insensitive
/// occurrence of `needle` (which starts with `<`).
fn find_tag_end(html: &str, needle: &str) -> Option<usize> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find(needle)?;
    let rel = html[start..].find('>')?;
    Some(start + rel + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shim_embeds_proxy_and_doc() {
        let shim = runtime_shim("http://127.0.0.1:8899", "https://example.com/a?b=1");
        assert!(shim.contains(r#""http://127.0.0.1:8899""#));
        assert!(shim.contains("https://example.com/a?b=1"));
        assert!(!shim.contains("__SHINY_PROXY__"));
        assert!(!shim.contains("__SHINY_DOC__"));
    }

    #[test]
    fn shim_escapes_script_terminators() {
        let shim = runtime_shim("http://p", "https://x/</script><script>alert(1)</script>");
        assert!(!shim.contains("</script>"), "shim must not close its own tag");
        assert!(shim.contains("\\u003c"));
    }

    #[test]
    fn shim_carries_the_frame_navigation_bridge() {
        let shim = runtime_shim("http://127.0.0.1:8899", "https://example.com/a");
        // The framed document reports its URL and accepts back/forward/reload.
        assert!(shim.contains("shiny:location"), "no location report");
        assert!(shim.contains("shiny:cmd"), "no command channel");
        assert!(shim.contains("parent !== window"), "bridge not frame-guarded");
        // The window's link affordances ride the same channel.
        assert!(shim.contains("shiny:new-tab"), "no new-tab bridge");
        assert!(shim.contains("shiny:link-menu"), "no link-menu bridge");
        assert!(shim.contains("shiny:hover-link"), "no hover bridge");
        assert!(shim.contains("shiny:leave-link"), "no hover-leave bridge");
        // It must stay a bridge: the scripted-navigation gaps the nav e2e test
        // pins (location.assign / location.replace) are still unpatched by
        // design.
        assert!(!shim.contains("location.assign"));
        assert!(!shim.contains("location.replace"));
    }

    #[test]
    fn insert_goes_after_head() {
        let html = "<html><head><title>t</title></head><body>x</body></html>";
        let out = insert_after_head(html, "<!--X-->");
        assert!(out.contains("<head><!--X--><title>"), "got {out}");
    }

    #[test]
    fn insert_falls_back_to_body_then_start() {
        let out = insert_after_head("<body>x</body>", "<!--X-->");
        assert!(out.starts_with("<body><!--X-->"), "got {out}");

        let out = insert_after_head("bare text", "<!--X-->");
        assert_eq!(out, "<!--X-->bare text");
    }

    #[test]
    fn head_injections_include_css_and_shim() {
        let parts = Injections {
            cosmetic_css: ".ad{display:none !important;}\n".into(),
            scriptlets: String::new(),
            shim: String::new(),
        };
        let block = head_injections("https://example.com/", "http://127.0.0.1:8899", &parts);
        assert!(block.contains("data-shiny-filter=\"cosmetic\""));
        assert!(block.contains(".ad{display:none"));
        assert!(block.contains("data-shiny-filter=\"shim\""));
    }
}
