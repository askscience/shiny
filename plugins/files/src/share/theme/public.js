/* PEAK'D! — the page a share link opens (see custom.css for the design notes).
 *
 * Gokapi renders its stock page; this script builds `.pk` on top and moves
 * Gokapi's working nodes into it, keeping their ids and handlers:
 *   download page   #buttondiv (with #downloadbutton, #wasmspinner), #errordiv
 *   password page   the <form> (#passwordFile, #pw_hidden)
 * #filename stays where Gokapi's scripts write it and is mirrored here.
 *
 * Sender text (custom/s/<id>.json, written by Shiny) is only ever inserted
 * with textContent. No request leaves this origin. */
(function () {
  'use strict';

  var root = document.documentElement;
  var reduced = window.matchMedia && matchMedia('(prefers-reduced-motion: reduce)').matches;
  var lang = (navigator.language || 'en').slice(0, 2).toLowerCase();

  var STR = {
    en: {
      transfer: 'File transfer',
      from: 'From',
      forYou: ['A file', 'for you.'],
      wants: 'wants to share a file with you.',
      file: 'File',
      size: 'Size',
      status: 'Status',
      e2e: 'End-to-end encrypted',
      direct: 'Direct link',
      download: 'Download',
      loading: 'Loading',
      pwKicker: 'Password',
      pwTitle: ['Protected.'],
      pwSub: 'Enter the password to open the file.',
      pwPlaceholder: 'Password',
      pwWrong: 'Wrong password',
      linkKicker: 'Link',
      expiredTitle: ['Link', 'expired.'],
      expiredSub: 'It reached its time limit or its download limit.',
      noKeyTitle: ['Key', 'missing.'],
      noKeySub: 'This file is encrypted and the link is incomplete. Ask the sender for the full link.',
      badKeyTitle: ['Wrong', 'key.'],
      badKeySub: 'This file is encrypted with a different key. Ask the sender for the right link.',
      marquee: ['Shared securely', 'No account needed', 'PEAK’D!'],
      sharedVia: 'Shared via PEAK’D!',
    },
    it: {
      transfer: 'Trasferimento file',
      from: 'Da',
      forYou: ['Un file', 'per te.'],
      wants: 'vuole condividere un file con te.',
      file: 'File',
      size: 'Dimensione',
      status: 'Stato',
      e2e: 'Cifrato end-to-end',
      direct: 'Link diretto',
      download: 'Scarica',
      loading: 'Caricamento',
      pwKicker: 'Password',
      pwTitle: ['Protetto.'],
      pwSub: 'Inserisci la password per aprire il file.',
      pwPlaceholder: 'Password',
      pwWrong: 'Password errata',
      linkKicker: 'Link',
      expiredTitle: ['Link', 'scaduto.'],
      expiredSub: 'Ha raggiunto il limite di tempo o di download.',
      noKeyTitle: ['Chiave', 'mancante.'],
      noKeySub: 'Il file è cifrato e il link è incompleto. Chiedi a chi te l’ha inviato il link completo.',
      badKeyTitle: ['Chiave', 'errata.'],
      badKeySub: 'Il file è cifrato con un’altra chiave. Chiedi a chi te l’ha inviato il link giusto.',
      marquee: ['Condiviso in modo sicuro', 'Nessun account richiesto', 'PEAK’D!'],
      sharedVia: 'Condiviso con PEAK’D!',
    },
  };
  var S = STR[lang] || STR.en;

  var LOCK = '<svg viewBox="0 0 16 16" aria-hidden="true"><path fill="currentColor" d="M8 1a3 3 0 0 0-3 3v3H4a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1V8a1 1 0 0 0-1-1h-1V4a3 3 0 0 0-3-3zm-2 6V4a2 2 0 1 1 4 0v3H6z"/></svg>';
  var LINK = '<svg viewBox="0 0 16 16" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" d="M6.5 9.5l3-3M7 4.5l1-1a2.8 2.8 0 0 1 4 4l-1 1M9 11.5l-1 1a2.8 2.8 0 0 1-4-4l1-1"/></svg>';
  var GLYPHS = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789';

  function el(tag, cls, text) {
    var n = document.createElement(tag);
    if (cls) n.className = cls;
    if (text != null) n.textContent = text;
    return n;
  }

  /* Next painted frame, or a timer when frames are paused (background tab,
     headless renderer): whatever comes first, once. */
  function nextFrame(fn) {
    var done = false;
    var go = function () { if (!done) { done = true; fn(); } };
    requestAnimationFrame(function () { requestAnimationFrame(go); });
    setTimeout(go, 150);
  }

  function onReady(fn) {
    if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', fn);
    else fn();
  }

  /* Stable hue from a name, so each sender tints the glow differently. */
  function hue(text) {
    var h = 0;
    for (var i = 0; i < text.length; i++) h = (h * 31 + text.charCodeAt(i)) % 360;
    return h;
  }

  /* Kinetic title: each line masks its letters, which rise in sequence. */
  function kinetic(target, lines, baseDelay) {
    target.textContent = '';
    target.setAttribute('aria-label', lines.join(' '));
    var i = 0;
    lines.forEach(function (line) {
      var row = el('span', 'pk-line');
      row.setAttribute('aria-hidden', 'true');
      Array.from(line).forEach(function (ch) {
        var c = el('span', ch === ' ' ? 'pk-ch sp' : 'pk-ch', ch === ' ' ? ' ' : ch);
        c.style.setProperty('--i', i++);
        row.appendChild(c);
      });
      target.appendChild(row);
    });
    target.style.setProperty('--pk-d', baseDelay + 'ms');
  }

  /* Shrink a one-line title until it fits its column. */
  function fit(title) {
    if (!title || getComputedStyle(title).whiteSpace !== 'nowrap') return;
    title.style.fontSize = '';
    var avail = title.parentElement.clientWidth;
    var need = title.scrollWidth;
    if (need > avail && need > 0) {
      var size = parseFloat(getComputedStyle(title).fontSize);
      title.style.fontSize = Math.max(40, Math.floor(size * avail / need)) + 'px';
    }
  }

  /* Letters cycle through glyphs and settle left to right (decodeText). */
  function decode(node, text, duration) {
    if (reduced) { node.textContent = text; return; }
    // Frames can pause mid-way (hidden tab): never leave glyph noise behind.
    setTimeout(function () { node.textContent = text; }, duration + 400);
    var t0 = performance.now();
    (function tick(now) {
      var k = Math.min(1, (now - t0) / duration);
      var fixed = Math.floor(k * text.length);
      node.textContent = Array.from(text).map(function (ch, i) {
        return i < fixed || ch === ' ' ? ch : GLYPHS[Math.floor(Math.random() * GLYPHS.length)];
      }).join('');
      if (k < 1) requestAnimationFrame(tick); else node.textContent = text;
    })(t0);
  }

  /* A number rolls up to its value, keeping unit and decimals (rollNumber). */
  function roll(node, duration) {
    var raw = node.textContent.trim();
    var m = raw.match(/^([\d.,]+)(.*)$/);
    if (reduced || !m) return;
    var sep = m[1].indexOf(',') >= 0 && m[1].indexOf('.') < 0 ? ',' : '.';
    var target = Number(m[1].replace(sep === ',' ? /\./g : /,/g, '').replace(',', '.'));
    if (!isFinite(target)) return;
    var dec = (m[1].split(sep)[1] || '').length;
    setTimeout(function () { node.textContent = raw; }, duration + 400);
    var t0 = performance.now();
    (function tick(now) {
      var k = Math.min(1, (now - t0) / duration);
      var v = target * (1 - Math.pow(1 - k, 4));
      node.textContent = k < 1 ? v.toFixed(dec).replace('.', sep) + m[2] : raw;
      if (k < 1) requestAnimationFrame(tick);
    })(t0);
  }

  /* Translate Gokapi's button labels in place, without touching its spinner
     (its wasm loader looks that node up by id). */
  function relabel(button) {
    var walker = document.createTreeWalker(button, NodeFilter.SHOW_TEXT, null);
    var node;
    while ((node = walker.nextNode())) {
      var t = node.nodeValue.trim();
      var next = /^download( file)?$/i.test(t) ? S.download : /^loading/i.test(t) ? S.loading : null;
      // Write only real changes: this runs from a MutationObserver on the
      // same node, and rewriting an equal value would re-trigger it forever.
      if (next !== null && t !== next) node.nodeValue = next;
    }
  }

  /* Watch `node` and run `fn` on changes, never on fn's own writes. */
  function watch(node, fn) {
    var obs = new MutationObserver(function () {
      obs.disconnect();
      try { fn(); } finally { obs.observe(node, { childList: true, characterData: true, subtree: true }); }
    });
    obs.observe(node, { childList: true, characterData: true, subtree: true });
  }

  function logo(cls) {
    var l = el('span', 'pk-logo' + (cls ? ' ' + cls : ''));
    l.setAttribute('aria-hidden', 'true');
    ['r', 'c', 'b'].forEach(function (k) { l.appendChild(el('i', k)); });
    return l;
  }

  function fact(label, valueNode, i) {
    var f = el('div', 'pk-fact');
    f.style.setProperty('--i', i);
    f.appendChild(el('p', 'pk-label', label));
    f.appendChild(valueNode);
    return f;
  }

  function chip(svg, text) {
    var c = el('span', 'pk-chip');
    c.innerHTML = svg; // static icon markup only
    c.appendChild(document.createTextNode(text));
    return c;
  }

  function magnet(target, area) {
    if (reduced || !target || !window.matchMedia || !matchMedia('(pointer: fine)').matches) return;
    area.addEventListener('pointermove', function (e) {
      var r = target.getBoundingClientRect();
      var dx = e.clientX - (r.left + r.width / 2);
      var dy = e.clientY - (r.top + r.height / 2);
      if (Math.hypot(dx, dy) > r.width * 1.4) { reset(); return; }
      target.classList.add('is-magnet');
      target.style.setProperty('--mx', dx * 0.22 + 'px');
      target.style.setProperty('--my', dy * 0.22 + 'px');
    });
    area.addEventListener('pointerleave', reset);
    function reset() { target.style.setProperty('--mx', '0px'); target.style.setProperty('--my', '0px'); }
  }

  function pointerFx(cursor, watermark) {
    if (reduced || !window.matchMedia || !matchMedia('(pointer: fine)').matches) return;
    var x = innerWidth / 2, y = innerHeight / 2, cx = x, cy = y;
    addEventListener('pointermove', function (e) { x = e.clientX; y = e.clientY; cursor.classList.add('is-live'); });
    (function loop() {
      cx += (x - cx) * 0.08; cy += (y - cy) * 0.08;
      cursor.style.transform = 'translate(' + cx + 'px,' + cy + 'px)';
      var px = (cx / innerWidth - 0.5) * -24, py = (cy / innerHeight - 0.5) * -18;
      watermark.style.transform = 'translate(' + px + 'px,' + py + 'px)';
      requestAnimationFrame(loop);
    })();
  }

  function detect() {
    if (document.querySelector('.file-card')) return 'download';
    if (document.getElementById('passwordFile')) return 'password';
    return 'error';
  }

  function build() {
    var page = detect();
    var id = new URLSearchParams(location.search).get('id') || '';
    var e2e = !!document.querySelector('.badge.bg-info') || /#.{50,}/.test(location.href);

    var pk = el('div', 'pk');
    var glow = el('div', 'pk-glow');
    var cursor = el('div', 'pk-cursor');
    var watermark = el('div', 'pk-watermark');
    [glow, cursor, watermark].forEach(function (n) { n.setAttribute('aria-hidden', 'true'); pk.appendChild(n); });

    /* top bar */
    var top = el('header', 'pk-top');
    var brand = el('span', 'pk-brand');
    brand.appendChild(logo());
    brand.appendChild(el('span', 'pk-brand-word', 'PEAK’D!'));
    var metaTop = el('div', 'pk-meta-top');
    var date = new Date().toLocaleDateString(lang === 'it' ? 'it-IT' : undefined, { day: '2-digit', month: 'short', year: 'numeric' });
    metaTop.appendChild(el('span', 'pk-hide-sm', S.transfer + ' — '));
    metaTop.appendChild(el('span', '', date));
    top.appendChild(brand);
    top.appendChild(metaTop);
    pk.appendChild(top);

    /* hero */
    var hero = el('main', 'pk-hero');
    var kicker = el('p', 'pk-kicker');
    var title = el('h1', 'pk-title');
    var sub = el('p', 'pk-sub');
    hero.appendChild(kicker);
    hero.appendChild(title);
    hero.appendChild(sub);
    pk.appendChild(hero);

    var fileRow = null;
    var filenameValue = null;
    var sizeValue = null;

    if (page === 'download') {
      kicker.textContent = S.transfer;
      kinetic(title, S.forYou, 150);
      sub.style.display = 'none';

      fileRow = el('section', 'pk-file');
      var facts = el('div', 'pk-facts');
      filenameValue = el('div', 'pk-value');
      var gokapiName = document.getElementById('filename');
      filenameValue.textContent = gokapiName ? gokapiName.textContent.trim() : '';
      if (gokapiName) {
        // End-to-end pages write the real name into #filename after decrypting.
        watch(gokapiName, function () { decode(filenameValue, gokapiName.textContent.trim(), 500); });
      }
      sizeValue = el('div', 'pk-value mono');
      var sizeSrc = document.querySelector('.file-card .list-group-item .fw-bold');
      sizeValue.textContent = sizeSrc ? sizeSrc.textContent.trim() : '';
      var statusValue = el('div', 'pk-value');
      statusValue.appendChild(e2e ? chip(LOCK, S.e2e) : chip(LINK, S.direct));
      facts.appendChild(fact(S.file, filenameValue, 0));
      facts.appendChild(fact(S.size, sizeValue, 1));
      facts.appendChild(fact(S.status, statusValue, 2));

      var cta = el('div', 'pk-cta');
      var buttondiv = document.getElementById('buttondiv');
      var errordiv = document.getElementById('errordiv');
      if (buttondiv) cta.appendChild(buttondiv);
      if (errordiv) cta.appendChild(errordiv);
      var button = cta.querySelector('button');
      if (button) {
        relabel(button);
        watch(button, function () { relabel(button); });
      }
      fileRow.appendChild(facts);
      fileRow.appendChild(cta);
      pk.appendChild(fileRow);
      magnet(button, pk);
    } else if (page === 'password') {
      kicker.textContent = S.pwKicker;
      kinetic(title, S.pwTitle, 150);
      sub.textContent = S.pwSub;
      var form = document.getElementById('form') || document.querySelector('main form');
      if (form) {
        form.classList.add('pk-form');
        var input = document.getElementById('passwordFile');
        if (input) input.placeholder = S.pwPlaceholder;
        var submit = form.querySelector('button[type="submit"]');
        if (submit) submit.setAttribute('aria-label', S.pwKicker);
        Array.from(form.querySelectorAll('span')).forEach(function (s) {
          if (/incorrect/i.test(s.textContent)) { s.className = 'pk-bad'; s.textContent = S.pwWrong; s.removeAttribute('style'); }
        });
        hero.appendChild(form);
        magnet(submit, form);
      }
    } else {
      var src = document.querySelector('main .card-title');
      var srcText = src ? src.textContent.trim().toLowerCase() : '';
      kicker.textContent = S.linkKicker;
      var t = S.expiredTitle, s = S.expiredSub;
      if (srcText.indexOf('missing decryption') >= 0) { t = S.noKeyTitle; s = S.noKeySub; }
      else if (srcText.indexOf('invalid decryption') >= 0) { t = S.badKeyTitle; s = S.badKeySub; }
      else if (src && srcText.indexOf('not found') < 0) { t = [src.textContent.trim()]; s = ''; }
      kinetic(title, t, 150);
      sub.textContent = s;
    }

    /* marquee */
    var marquee = el('div', 'pk-marquee');
    marquee.setAttribute('aria-hidden', 'true');
    var track = el('div', 'pk-marquee-track');
    var words = S.marquee.slice();
    if (e2e) words.splice(1, 0, S.e2e);
    for (var k = 0; k < 6; k++) words.forEach(function (w) { track.appendChild(el('span', '', w)); });
    marquee.appendChild(track);
    pk.appendChild(marquee);

    /* foot: keep Gokapi's credit */
    var foot = el('footer', 'pk-foot');
    foot.appendChild(el('span', '', S.sharedVia));
    var credit = el('a', '', 'Powered by Gokapi');
    credit.href = 'https://github.com/Forceu/Gokapi';
    credit.target = '_blank';
    credit.rel = 'noopener noreferrer';
    foot.appendChild(credit);
    pk.appendChild(foot);

    var grain = el('div', 'pk-grain');
    grain.setAttribute('aria-hidden', 'true');
    var curtain = el('div', 'pk-curtain');
    curtain.setAttribute('aria-hidden', 'true');
    curtain.appendChild(logo());

    document.body.appendChild(pk);
    document.body.appendChild(grain);
    document.body.appendChild(curtain);
    root.classList.add('pk-built');
    document.title = 'PEAK’D!' + (filenameValue && filenameValue.textContent ? ' — ' + filenameValue.textContent : '');

    var revealed = false;
    function reveal() {
      if (revealed) return;
      revealed = true;
      fit(title);
      nextFrame(function () {
        root.classList.add('pk-ready');
        if (filenameValue) setTimeout(function () { decode(filenameValue, filenameValue.textContent, 700); }, 750);
        if (sizeValue) setTimeout(function () { roll(sizeValue, 1100); }, 850);
        setTimeout(function () { curtain.remove(); }, 1400);
      });
    }

    function applySender(meta) {
      var from = typeof meta.from === 'string' ? meta.from.trim().slice(0, 64) : '';
      var message = typeof meta.message === 'string' ? meta.message.trim().slice(0, 500) : '';
      if (from) {
        var h = hue(from);
        root.style.setProperty('--pk-glow-a', 'hsl(' + h + ' 85% 58%)');
        root.style.setProperty('--pk-glow-b', 'hsl(' + ((h + 48) % 360) + ' 90% 60%)');
        if (page === 'download') {
          kicker.textContent = S.from;
          kinetic(title, [from], 150);
          sub.style.display = '';
          sub.textContent = S.wants;
        }
      }
      if (message && page === 'download') {
        var msg = el('p', 'pk-message', message);
        hero.appendChild(msg);
      }
    }

    // Fetch the sender card before the curtain lifts, so the title animates
    // once with the right words; never wait more than a moment for it.
    if ((page === 'download' || page === 'password') && /^[A-Za-z0-9_-]{1,64}$/.test(id) && window.fetch) {
      var timer = setTimeout(reveal, 700);
      fetch('./custom/s/' + id + '.json', { cache: 'no-store', credentials: 'omit' })
        .then(function (r) { return r.ok ? r.json() : null; })
        .then(function (meta) { if (!revealed && meta && typeof meta === 'object') applySender(meta); })
        .catch(function () {})
        .then(function () { clearTimeout(timer); reveal(); });
    } else {
      reveal();
    }

    pointerFx(cursor, watermark);
    var resizeTimer;
    addEventListener('resize', function () { clearTimeout(resizeTimer); resizeTimer = setTimeout(function () { fit(title); }, 120); });
    if (document.fonts && document.fonts.ready) document.fonts.ready.then(function () { fit(title); });
  }

  onReady(function () {
    try { build(); } catch (e) {
      // Never leave a blank page: show Gokapi's own layout instead.
      root.classList.remove('pk-built');
      var stock = document.querySelector('body > div.d-flex');
      if (stock) stock.style.visibility = 'visible';
      if (window.console) console.error(e);
    }
  });
})();
