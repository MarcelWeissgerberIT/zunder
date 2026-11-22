// Lesemodus. Läuft auf Anweisung des Browsers in Zunders abgeschotteter Welt
// der Seite: sucht den eigentlichen Text, baut daraus eine aufgeräumte Fassung
// und legt sie über die Seite. Ein zweiter Aufruf nimmt sie wieder weg.
//
// Übernommen werden nur Text, Überschriften, Listen, Tabellen, Bilder und
// Links – als neu erzeugte Elemente, nie als HTML-Text der Seite. Skripte,
// Formulare, Rahmen und Stile der Seite bleiben draußen. Alles geschieht im
// Dokument der Seite selbst; Sonderrechte hat der Lesemodus keine.
//
// Ergebnis: 'on' (gezeigt), 'off' (beendet) oder 'none' (kein Artikel gefunden).
(function (options) {
  'use strict';
  var HOST_ID = 'zunder-reader';
  var bridge = window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.zunder;

  function tell(message) {
    try {
      if (bridge) bridge.postMessage(JSON.stringify(message));
    } catch (error) {
      /* Seite wird gerade verlassen */
    }
  }

  // Läuft der Lesemodus schon? Gefragt wird nach einer eigenen Eigenschaft
  // dieser Welt – ein Element der Seite mit diesem Namen zählt nicht.
  var running = Object.prototype.hasOwnProperty.call(globalThis, '__zunderReader') ? globalThis.__zunderReader : null;
  if (running && typeof running.close === 'function') {
    running.close(false);
    return 'off';
  }
  if (!document.body) return 'none';

  // ------------------------------------------------------------------------
  // Den Artikel finden
  // ------------------------------------------------------------------------

  // Was nicht zum Artikel gehört: Navigation, Randspalten, Formulare, Kommentare.
  var OUTSIDE =
    'nav, aside, footer, form, [role=navigation], [role=complementary], [role=banner], [role=contentinfo], ' +
    '[class*="comment" i], [id*="comment" i], #disqus_thread';

  function pick() {
    var direct = new Map(); // Punkte aus Absätzen unmittelbar darin (und eine Ebene tiefer)
    var within = new Map(); // Punkte aller Absätze irgendwo darin
    var best = null;
    var bestScore = 0;
    var blocks = document.querySelectorAll('p, pre, blockquote');
    for (var i = 0; i < blocks.length && i < 6000; i++) {
      var block = blocks[i];
      var text = block.textContent || '';
      var length = text.trim().length;
      if (length < 60 || block.closest(OUTSIDE)) continue;
      var score = 1 + Math.min(3, length / 100) + (text.split(/[,;]/).length - 1) * 0.3;
      var parent = block.parentElement;
      for (var level = 0; parent && level < 2; level++) {
        var total = (direct.get(parent) || 0) + score / (level + 1);
        direct.set(parent, total);
        if (total > bestScore) {
          bestScore = total;
          best = parent;
        }
        parent = parent.parentElement;
      }
      for (var up = block.parentElement; up; up = up.parentElement) within.set(up, (within.get(up) || 0) + score);
    }
    if (!best) best = document.querySelector('article, [itemprop="articleBody"], main, [role="main"]');
    if (!best) return null;
    // Verteilt sich der Text auf mehrere Abschnitte, gehört der gemeinsame Behälter dazu:
    // so weit hinauf, wie dabei noch spürbar Text hinzukommt.
    while (best.parentElement && best.parentElement !== document.documentElement) {
      var outer = best.parentElement;
      if ((within.get(outer) || 0) < (within.get(best) || 0) * 1.35) break;
      best = outer;
    }
    // Umschließt ein Artikel-Element den Fund, ohne viel Fremdes mitzubringen, dann dieses.
    var wrap = best.closest('article, [itemprop="articleBody"]');
    if (wrap && (wrap.textContent || '').length < (best.textContent || '').length * 2.5) best = wrap;
    return best;
  }

  var root = pick();
  if (!root || (root.textContent || '').trim().length < 400) return 'none';
  var rootLength = (root.textContent || '').length;

  // ------------------------------------------------------------------------
  // Aufgeräumte Fassung bauen
  // ------------------------------------------------------------------------

  var KEEP = {
    P: 1, H1: 1, H2: 1, H3: 1, H4: 1, H5: 1, H6: 1, UL: 1, OL: 1, LI: 1, BLOCKQUOTE: 1, PRE: 1,
    FIGURE: 1, FIGCAPTION: 1, TABLE: 1, THEAD: 1, TBODY: 1, TFOOT: 1, TR: 1, TH: 1, TD: 1, DL: 1, DT: 1, DD: 1,
    A: 1, EM: 1, I: 1, STRONG: 1, B: 1, CODE: 1, SUP: 1, SUB: 1, MARK: 1, Q: 1, CITE: 1, U: 1, S: 1,
    SMALL: 1, ABBR: 1, KBD: 1, TIME: 1, CAPTION: 1
  };
  var DROP = {
    SCRIPT: 1, STYLE: 1, NOSCRIPT: 1, IFRAME: 1, FORM: 1, BUTTON: 1, INPUT: 1, SELECT: 1, TEXTAREA: 1,
    NAV: 1, ASIDE: 1, FOOTER: 1, CANVAS: 1, VIDEO: 1, AUDIO: 1, OBJECT: 1, EMBED: 1, TEMPLATE: 1,
    DIALOG: 1, MENU: 1, LINK: 1, META: 1, SVG: 1, MATH: 1, LABEL: 1
  };
  var CONTAINER = { DIV: 1, SECTION: 1, ARTICLE: 1, MAIN: 1, HEADER: 1, SPAN: 1, CENTER: 1, FONT: 1 };
  var BLOCK = /^(P|H[1-6]|UL|OL|LI|BLOCKQUOTE|PRE|FIGURE|TABLE|DIV|SECTION|ARTICLE|DL|HR)$/;
  // Immer fremd – und: nur fremd, solange es nicht selbst den Großteil des Textes trägt.
  var ALWAYS_JUNK = /(^|[\s_-])(comments?|disqus|editsection|noprint|navbox|catlinks|outbrain|taboola|jump)([\s_-]|$)/i;
  var JUNK = /(^|[\s_-])(share|sharing|social|related|promo|advert\w*|ads?|sponsor\w*|newsletter|subscribe|sidebar|breadcrumbs?|toolbar|toc|cookie|consent|paywall|popup|modal|recommend\w*|print|skip|metadata)([\s_-]|$)/i;

  function junk(el) {
    if (el.hidden || el.getAttribute('aria-hidden') === 'true') return true;
    var marks = (typeof el.className === 'string' ? el.className : '') + ' ' + (el.id || '');
    if (marks.length > 1) {
      if (ALWAYS_JUNK.test(marks)) return true;
      if (JUNK.test(marks) && (el.textContent || '').length < rootLength * 0.4) return true;
    }
    var tag = el.tagName;
    if (CONTAINER[tag] || BLOCK.test(tag)) {
      var style = getComputedStyle(el);
      if (style.display === 'none' || style.visibility === 'hidden') return true;
    }
    return false;
  }

  function image(el) {
    var source = el.currentSrc || el.src || '';
    var lazy = el.getAttribute('data-src') || el.getAttribute('data-lazy-src') || el.getAttribute('data-original');
    if (lazy && (!source || source.indexOf('data:') === 0)) {
      try {
        source = new URL(lazy, document.baseURI).href;
      } catch (error) {
        source = '';
      }
    }
    if (!/^(https?:|data:image\/)/i.test(source)) return null;
    // Zählpixel und Symbole sind keine Abbildungen.
    if (el.naturalWidth && (el.naturalWidth < 60 || el.naturalHeight < 40)) return null;
    var copy = document.createElement('img');
    copy.src = source;
    copy.alt = el.alt || '';
    copy.loading = 'lazy';
    copy.decoding = 'async';
    return copy;
  }

  var skip = null; // die Überschrift, die schon oben steht

  function hasBlockChild(el) {
    for (var child = el.firstElementChild; child; child = child.nextElementSibling) {
      if (BLOCK.test(child.tagName)) return true;
    }
    return false;
  }

  function clean(node, out, depth) {
    if (depth > 70) return;
    for (var child = node.firstChild; child; child = child.nextSibling) {
      if (child.nodeType === 3) {
        var value = child.nodeValue;
        if (value.trim() || (out.lastChild && out.lastChild.nodeType !== 3)) out.appendChild(document.createTextNode(value));
        continue;
      }
      if (child.nodeType !== 1 || child === skip) continue;
      var tag = String(child.tagName).toUpperCase();
      if (DROP[tag] || junk(child)) continue;
      if (tag === 'IMG') {
        var picture = image(child);
        if (picture) out.appendChild(picture);
      } else if (tag === 'PICTURE') {
        var inner = child.querySelector('img');
        var shown = inner ? image(inner) : null;
        if (shown) out.appendChild(shown);
      } else if (tag === 'BR' || tag === 'HR') {
        out.appendChild(document.createElement(tag));
      } else if (KEEP[tag]) {
        var copy = document.createElement(tag);
        if (tag === 'A') {
          var href = child.href || '';
          if (/^(https?:|mailto:)/i.test(href)) {
            copy.href = href;
            copy.rel = 'noopener';
          }
        } else if (tag === 'TD' || tag === 'TH') {
          if (child.colSpan > 1) copy.colSpan = Math.min(child.colSpan, 50);
          if (child.rowSpan > 1) copy.rowSpan = Math.min(child.rowSpan, 200);
        }
        if (tag === 'PRE') copy.textContent = child.textContent;
        else clean(child, copy, depth + 1);
        if (copy.firstChild) out.appendChild(copy);
      } else if (CONTAINER[tag] && tag !== 'SPAN' && tag !== 'FONT' && !hasBlockChild(child)) {
        // Ein Behälter, der selbst nur Text trägt: wird zum Absatz.
        var paragraph = document.createElement('p');
        clean(child, paragraph, depth + 1);
        if ((paragraph.textContent || '').trim() || paragraph.querySelector('img')) out.appendChild(paragraph);
      } else {
        // Alles andere: Inhalt übernehmen, Hülle weglassen.
        clean(child, out, depth + 1);
      }
    }
  }

  // Überschrift: die der Seite – sonst der Titel, den sie für das Teilen angibt,
  // sonst der Fenstertitel ohne den angehängten Namen der Website.
  // Überschrift: die der Seite – sonst der Titel, den sie für das Teilen angibt
  // oder im Fenster zeigt, jeweils ohne den angehängten Namen der Website.
  function withoutSiteName(text) {
    var parts = text.split(/\s+[|·–—-]\s+/);
    if (parts.length < 2) return text;
    var named = document.querySelector('meta[property="og:site_name"]');
    var site = ((named && named.getAttribute('content')) || '').toLowerCase().replace(/\s+/g, '');
    var host = location.hostname.replace(/^www\./, '').split('.')[0].toLowerCase();
    function isSite(part) {
      var plain = part.toLowerCase().replace(/\s+/g, '');
      return !!plain && (plain === site || plain.indexOf(host) !== -1 || host.indexOf(plain) !== -1);
    }
    if (isSite(parts[parts.length - 1])) parts.pop();
    else if (isSite(parts[0])) parts.shift();
    else return text;
    return parts.join(' – ');
  }

  var heading = root.querySelector('h1') || document.querySelector('h1');
  var shared = document.querySelector('meta[property="og:title"], meta[name="twitter:title"]');
  var title = ((heading && heading.textContent) || '').replace(/\s+/g, ' ').trim();
  if (!title) {
    title = ((shared && shared.getAttribute('content')) || document.title || '').replace(/\s+/g, ' ').trim();
    title = withoutSiteName(title);
  }
  if (heading && root.contains(heading)) skip = heading;

  var article = document.createElement('article');
  clean(root, article, 0);
  var words = (article.textContent || '').trim().split(/\s+/).length;
  if (words < 80) return 'none';

  // ------------------------------------------------------------------------
  // Anzeigen
  // ------------------------------------------------------------------------

  var SVG_NS = 'http://www.w3.org/2000/svg';

  function shape(spec, parent) {
    var node = document.createElementNS(SVG_NS, spec.tag);
    for (var name in spec.attrs) node.setAttribute(name, spec.attrs[name]);
    (spec.children || []).forEach(function (child) {
      shape(child, node);
    });
    parent.appendChild(node);
  }

  function button(iconSpec, label, action) {
    var node = document.createElement('button');
    node.type = 'button';
    node.title = label;
    node.setAttribute('aria-label', label);
    var svg = document.createElementNS(SVG_NS, 'svg');
    svg.setAttribute('viewBox', '0 0 1024 1024');
    (iconSpec || []).forEach(function (spec) {
      shape(spec, svg);
    });
    node.appendChild(svg);
    node.addEventListener('click', action);
    return node;
  }

  var size = Math.min(30, Math.max(14, Number(options.size) || 19));
  var host = document.createElement('div');
  host.id = HOST_ID;
  host.style.cssText = 'all: initial; position: fixed; inset: 0; z-index: 2147483647; display: block;';
  var shadow = host.attachShadow({ mode: 'closed' });

  var css = [
    ':host { all: initial; }',
    '.page { position: fixed; inset: 0; overflow-y: auto; overscroll-behavior: contain; outline: none;',
    '  background: #fbf8f3; color: #1e1d1b; --muted: #6d6964; --line: rgba(0,0,0,.12); --accent: #c2410c; --hover: rgba(0,0,0,.07);',
    '  font: var(--size) / 1.62 ui-serif, "New York", "Iowan Old Style", Charter, Georgia, serif; -webkit-font-smoothing: antialiased; }',
    '@media (prefers-color-scheme: dark) { .page { background: #1c1b19; color: #ece8e2; --muted: #a8a29b; --line: rgba(255,255,255,.14); --accent: #ff8a3d; --hover: rgba(255,255,255,.1); } }',
    '.bar { position: sticky; top: 0; z-index: 1; display: flex; justify-content: flex-end; gap: 4px; padding: 10px 14px 0; pointer-events: none; }',
    '.bar button { pointer-events: auto; width: 34px; height: 34px; padding: 7px; border: 0; border-radius: 9px; background: none; color: inherit; cursor: default; }',
    '.bar button:hover { background: var(--hover); }',
    '.bar svg { width: 20px; height: 20px; display: block; }',
    '.sheet { max-width: 40em; margin: 0 auto; padding: 8px 28px 96px; }',
    '.site { font: 13px/1.3 -apple-system, system-ui, sans-serif; color: var(--muted); letter-spacing: .01em; }',
    'h1.title { font: 700 1.9em/1.18 -apple-system, system-ui, sans-serif; letter-spacing: -.02em; margin: .35em 0 .7em; }',
    'article { overflow-wrap: break-word; }',
    'article h1, article h2, article h3, article h4, article h5, article h6 { font-family: -apple-system, system-ui, sans-serif; line-height: 1.25; margin: 1.6em 0 .5em; letter-spacing: -.01em; }',
    'article h1, article h2 { font-size: 1.35em; } article h3 { font-size: 1.15em; } article h4, article h5, article h6 { font-size: 1em; }',
    'article p, article ul, article ol, article blockquote, article pre, article figure, article table, article dl { margin: 0 0 1.05em; }',
    'article a { color: var(--accent); text-decoration: underline; text-decoration-thickness: 1px; text-underline-offset: 2px; }',
    'article img { max-width: 100%; height: auto; border-radius: 6px; display: block; margin: .3em auto; }',
    'article figure { margin-left: 0; margin-right: 0; } article figcaption { font: .78em/1.4 -apple-system, system-ui, sans-serif; color: var(--muted); margin-top: .5em; text-align: center; }',
    'article blockquote { margin-left: 0; margin-right: 0; padding-left: 1em; border-left: 3px solid var(--line); color: var(--muted); }',
    'article pre, article code, article kbd { font: .82em/1.5 ui-monospace, "SF Mono", Menlo, monospace; }',
    'article pre { padding: .9em 1em; border-radius: 8px; background: var(--hover); overflow-x: auto; white-space: pre; }',
    'article ul, article ol { padding-left: 1.4em; } article li { margin: .25em 0; }',
    'article table { border-collapse: collapse; font: .85em/1.45 -apple-system, system-ui, sans-serif; display: block; overflow-x: auto; }',
    'article th, article td { border: 1px solid var(--line); padding: .4em .7em; text-align: left; vertical-align: top; }',
    'article hr { border: 0; border-top: 1px solid var(--line); margin: 2em 0; }',
    '::selection { background: rgba(232, 89, 12, .25); }'
  ].join('\n');
  // Als fertiges Stilblatt eingehängt – so greifen die Regeln auch auf Seiten,
  // die eingebettete Stile per Richtlinie verbieten.
  try {
    var sheetObject = new CSSStyleSheet();
    sheetObject.replaceSync(css);
    shadow.adoptedStyleSheets = [sheetObject];
  } catch (error) {
    var style = document.createElement('style');
    style.textContent = css;
    shadow.appendChild(style);
  }

  var page = document.createElement('div');
  page.className = 'page';
  page.tabIndex = -1;
  page.style.setProperty('--size', size + 'px');

  function resize(step) {
    size = Math.min(30, Math.max(14, size + step));
    page.style.setProperty('--size', size + 'px');
    tell({ t: 'reader', size: size });
  }

  var icons = options.icons || {};
  var strings = options.strings || {};
  var bar = document.createElement('div');
  bar.className = 'bar';
  bar.appendChild(button(icons.minus, strings.smaller || '', function () { resize(-1); }));
  bar.appendChild(button(icons.plus, strings.larger || '', function () { resize(1); }));
  bar.appendChild(button(icons.close, strings.close || '', function () { close(true); }));
  page.appendChild(bar);

  var sheet = document.createElement('div');
  sheet.className = 'sheet';
  var site = document.createElement('div');
  site.className = 'site';
  var minutes = Math.max(1, Math.round(words / 220));
  site.textContent = location.hostname.replace(/^www\./, '') + ' · ' + String(strings.minutes || '{n} min').replace('{n}', minutes);
  sheet.appendChild(site);
  if (title) {
    var top = document.createElement('h1');
    top.className = 'title';
    top.textContent = title;
    sheet.appendChild(top);
  }
  sheet.appendChild(article);
  page.appendChild(sheet);
  shadow.appendChild(page);

  function onKey(event) {
    if (event.key === 'Escape') {
      event.stopPropagation();
      close(true);
    }
  }

  function close(report) {
    window.removeEventListener('keydown', onKey, true);
    host.remove();
    globalThis.__zunderReader = null;
    if (report) tell({ t: 'reader', on: false });
  }

  window.addEventListener('keydown', onKey, true);
  document.documentElement.appendChild(host);
  globalThis.__zunderReader = { close: close, resize: resize };
  try {
    page.focus({ preventScroll: true });
  } catch (error) {
    /* ohne Fokus geht es auch */
  }
  return 'on';
})
