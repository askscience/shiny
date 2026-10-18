/*
 * youtube-ads.js — the shell's YouTube ad scriptlet.
 *
 * Injected at document creation into every frame of every view (see
 * `Shell::scriptSources` in shim/peakd.cpp), which is what covers the YouTube
 * plugin's player too: that one is a youtube.com/embed iframe inside the app
 * view, not a shell page view. Self-gating — on any other host the function
 * returns immediately and touches nothing.
 *
 * Why a script and not a filter rule: YouTube's pre-roll and mid-roll ads come
 * from the same googlevideo.com/videoplayback shape as the video itself, so no
 * network rule can block them without blocking the video. What does work — and
 * what uBlock Origin does for the same site — is to stop the player from being
 * told there is an ad at all: the ad keys are renamed out of the player
 * response (`"adPlacements"` → `"no_ads"`, uBO's trusted-replace-xhr-response
 * rules in uAssets' filters/quick-fixes.txt), with a skip-the-ad fallback for
 * the cases this cannot see.
 *
 * Network blocking of the ad *requests* around the player is separate and
 * already works: the Qt request interceptor and the engine's list (see
 * crates/peakd/src/filter.rs, plugins/browser/src/filter.rs).
 *
 * Rules for this file: ES5-compatible, no module syntax (it is injected raw),
 * nothing on `window` except the guard below, and every block in a try/catch —
 * a throw here would be a broken page, which is never an acceptable outcome
 * for an ad filter.
 */
(function () {
  'use strict';

  if (window.__peakdYoutubeAds) return;
  var host = String(location.hostname || '').replace(/^www\./, '');
  if (host !== 'youtube.com' && !/\.youtube\.com$/.test(host)) return;
  window.__peakdYoutubeAds = true;

  // ---------------------------------------------------------------------------
  // 1. Player responses.
  //
  // The player asks /youtubei/v1/player (and get_watch) for the video, and that
  // answer carries the ad breaks. The keys are renamed rather than deleted so
  // the JSON stays well formed for whatever parses it afterwards — a missing
  // key is much more likely to be noticed than an unknown one.
  // ---------------------------------------------------------------------------

  var AD_KEYS = /"(?:adPlacements|adSlots|playerAds)":/g;

  function clean(text) {
    if (typeof text !== 'string' || text.indexOf('"ad') === -1) return text;
    return text.replace(AD_KEYS, '"no_ads":');
  }

  // Deliberately narrow: rewriting every response on the site would be both
  // slow and risky.
  function isPlayerUrl(url) {
    return /\/youtubei\/v1\/(?:player|get_watch)/.test(url) ||
      /\/playlist\?list=/.test(url);
  }

  try {
    var nativeFetch = window.fetch;
    if (typeof nativeFetch === 'function') {
      window.fetch = function (input) {
        var url = typeof input === 'string' ? input : (input && input.url) || '';
        var promise = nativeFetch.apply(this, arguments);
        if (!isPlayerUrl(url)) return promise;
        return promise.then(function (response) {
          return response.clone().text().then(function (text) {
            var cleaned = clean(text);
            if (cleaned === text) return response;
            return new Response(cleaned, {
              status: response.status,
              statusText: response.statusText,
              headers: response.headers
            });
          }).catch(function () {
            return response;
          });
        });
      };
    }
  } catch (err) {
    // An unpatched fetch is not worth breaking a page for.
  }

  try {
    var xhrOpen = XMLHttpRequest.prototype.open;
    var xhrSend = XMLHttpRequest.prototype.send;

    // The response is only rewritten for the player, so the request is tagged
    // where its URL is still known.
    XMLHttpRequest.prototype.open = function (method, url) {
      this.__peakdPlayerUrl = isPlayerUrl(String(url == null ? '' : url));
      return xhrOpen.apply(this, arguments);
    };

    XMLHttpRequest.prototype.send = function () {
      if (this.__peakdPlayerUrl) {
        this.addEventListener('readystatechange', function () {
          var xhr = this;
          if (xhr.readyState !== 4) return;
          try {
            var type = xhr.responseType;
            if (type === 'json') {
              // Round-trip the object so the rename applies to it too, then
              // expose the result back on the instance.
              var value = xhr.response;
              if (!value) return;
              var before = JSON.stringify(value);
              var after = clean(before);
              if (after === before) return;
              define(xhr, 'response', JSON.parse(after));
            } else if (type === '' || type === 'text') {
              var original = xhr.responseText;
              var replaced = clean(original);
              if (replaced === original) return;
              define(xhr, 'responseText', replaced);
              if (type === '') define(xhr, 'response', replaced);
            }
          } catch (err) {
            // Leave the response exactly as the site produced it.
          }
        });
      }
      return xhrSend.apply(this, arguments);
    };
  } catch (err) {
    // Same as fetch: a browser without the patch is a browser without the fix.
  }

  // An own property on the instance shadows the prototype accessor (what uBO's
  // json-prune-xhr-response does); a fallback assignment covers the engines
  // where defineProperty refuses.
  function define(target, key, value) {
    try {
      Object.defineProperty(target, key, { value: value, configurable: true });
    } catch (err) {
      try { target[key] = value; } catch (ignored) { /* give up quietly */ }
    }
  }

  // ---------------------------------------------------------------------------
  // 2. The inlined player response.
  //
  // ytInitialPlayerResponse is written into the document itself, so it never
  // passes through fetch or XHR. It is replaced on every SPA navigation, hence
  // the event listener plus a short-lived interval.
  // ---------------------------------------------------------------------------

  var AD_FIELDS = ['adPlacements', 'adSlots', 'playerAds'];

  function pruneGlobals() {
    try {
      var response = window.ytInitialPlayerResponse;
      if (!response) return;
      for (var i = 0; i < AD_FIELDS.length; i++) delete response[AD_FIELDS[i]];
    } catch (err) {
      // A player response we cannot touch is not fatal.
    }
  }

  pruneGlobals();
  document.addEventListener('DOMContentLoaded', pruneGlobals, true);
  window.addEventListener('yt-navigate-finish', pruneGlobals, true);

  var ticks = 0;
  var timer = setInterval(function () {
    pruneGlobals();
    if (++ticks >= 30) clearInterval(timer);
  }, 1000);

  // ---------------------------------------------------------------------------
  // 3. Ads that start anyway.
  //
  // Covers what the response rewriting cannot see (a format it does not match,
  // a break inserted after the response). Skip when the player offers it,
  // otherwise run the ad to its end with the sound off.
  // ---------------------------------------------------------------------------

  var SKIP = '.ytp-ad-skip-button, .ytp-ad-skip-button-modern, .ytp-skip-ad-button';
  var CLOSE = '.ytp-ad-overlay-close-button';
  var AD_PLAYING = '.ad-showing, .ad-interrupting';
  var muted = null;

  function skipAds() {
    try {
      var close = document.querySelector(CLOSE);
      if (close) close.click();

      var player = document.querySelector('#movie_player');
      var adPlaying = !!player && player.matches(AD_PLAYING);
      var video = player ? player.querySelector('video') : null;

      if (!adPlaying) {
        // The ad is over: hand the element back with the mute state it had.
        if (muted !== null && video) video.muted = muted;
        muted = null;
        return;
      }

      var skip = document.querySelector(SKIP);
      if (skip) {
        skip.click();
        return;
      }

      if (video) {
        if (muted === null) muted = video.muted;
        video.muted = true;
        if (isFinite(video.duration) && video.duration > 0) {
          video.currentTime = video.duration;
        }
      }
    } catch (err) {
      // Never let the fallback throw at the player.
    }
  }

  // Two observers: one on the document to find (and re-find, after a SPA
  // navigation replaces it) the player element, and one on the player itself
  // for the ad classes only — that second one is what makes this cheap enough
  // to leave running.
  var playerElement = null;
  var playerObserver = null;

  function watchPlayer() {
    try {
      var player = document.querySelector('#movie_player');
      if (!player || player === playerElement) return;
      playerElement = player;
      if (playerObserver) playerObserver.disconnect();
      playerObserver = new MutationObserver(skipAds);
      playerObserver.observe(player, { attributes: true, attributeFilter: ['class'] });
    } catch (err) {
      // No player yet: the interval below keeps trying.
    }
  }

  try {
    new MutationObserver(function () {
      watchPlayer();
      skipAds();
    }).observe(document.documentElement || document, { childList: true, subtree: true });
  } catch (err) {
    // Without an observer the interval is the whole fallback.
  }

  watchPlayer();
  setInterval(skipAds, 1000);

  // ---------------------------------------------------------------------------
  // 4. The boxes the player reserves for ads.
  //
  // Purely cosmetic, so nothing here is load-bearing: a page that renames its
  // classes just loses the styling.
  // ---------------------------------------------------------------------------

  var CSS = '.video-ads,.ytp-ad-module,.ytp-ad-overlay-slot,.ytp-ad-text-overlay,' +
    '.ytp-ad-progress-list,.ytp-ad-player-overlay,#player-ads,' +
    'ytd-promoted-sparkles-web-renderer,ytd-display-ad-renderer,' +
    'ytd-in-feed-ad-layout-renderer,ytd-ad-slot-renderer,' +
    'ytd-banner-promo-renderer,.ytd-companion-slot-renderer{display:none !important;}';

  // Appends only to an element that exists. Never to the `document` node:
  // at document creation the parser has no root yet, and giving the document
  // an element child of its own stops the parser from ever building
  // html/head/body — the page then loads as a lone <style> and renders blank
  // (this is what `document.head || document.documentElement || document`
  // did on youtube.com).
  function hideAdBoxes() {
    try {
      var root = document.head || document.documentElement;
      if (!root) return;
      var style = document.createElement('style');
      style.textContent = CSS;
      root.appendChild(style);
    } catch (err) {
      // Styling only.
    }
  }

  hideAdBoxes();
  document.addEventListener('DOMContentLoaded', hideAdBoxes, true);
})();
