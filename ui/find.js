(function (query, direction, clear) {
  // Suche in der Seite. Die Treffer werden über die CSS Custom Highlight API
  // markiert – der Aufbau der Seite bleibt dabei unangetastet.
  // Rückgabe: [Nummer des aktuellen Treffers, Anzahl der Treffer].
  'use strict';
  var KEY = '__zunderFind';
  var state = window[KEY];
  var canHighlight = typeof CSS !== 'undefined' && CSS.highlights && typeof Highlight === 'function';

  function reset() {
    if (canHighlight) {
      CSS.highlights.delete('zunder-find');
      CSS.highlights.delete('zunder-find-current');
    }
    window[KEY] = null;
  }

  if (clear || !query) {
    reset();
    return [0, 0];
  }

  if (!canHighlight) {
    var found = window.find(query, false, direction < 0, true);
    return [found ? 1 : 0, found ? 1 : 0];
  }

  if (!state || state.query !== query) {
    reset();
    if (!window.__zunderFindStyle) {
      try {
        var sheet = new CSSStyleSheet();
        sheet.replaceSync(
          '::highlight(zunder-find){background-color:#ffe066;color:#111}' +
            '::highlight(zunder-find-current){background-color:#ff8a1f;color:#111}'
        );
        document.adoptedStyleSheets = document.adoptedStyleSheets.concat([sheet]);
        window.__zunderFindStyle = true;
      } catch (error) {
        /* ohne Stil bleiben die Treffer unmarkiert, die Suche läuft trotzdem */
      }
    }
    var needle = query.toLowerCase();
    var ranges = [];
    var walker = document.createTreeWalker(document.body || document.documentElement, NodeFilter.SHOW_TEXT, {
      acceptNode: function (node) {
        var parent = node.parentElement;
        if (!parent) return NodeFilter.FILTER_REJECT;
        var tag = parent.tagName;
        if (tag === 'SCRIPT' || tag === 'STYLE' || tag === 'NOSCRIPT' || tag === 'TEMPLATE') {
          return NodeFilter.FILTER_REJECT;
        }
        return NodeFilter.FILTER_ACCEPT;
      },
    });
    var node;
    while ((node = walker.nextNode()) && ranges.length < 2000) {
      var text = node.nodeValue;
      var lower = text.toLowerCase();
      // Nur wenn Klein- und Großschreibung gleich lang sind, stimmen die Positionen.
      if (lower.length !== text.length) continue;
      var from = 0;
      var at;
      while ((at = lower.indexOf(needle, from)) !== -1 && ranges.length < 2000) {
        var range = document.createRange();
        range.setStart(node, at);
        range.setEnd(node, at + needle.length);
        if (range.getClientRects().length > 0) ranges.push(range);
        from = at + needle.length;
      }
    }
    state = window[KEY] = { query: query, ranges: ranges, index: -1 };
    if (ranges.length) {
      var all = new Highlight();
      for (var i = 0; i < ranges.length; i++) all.add(ranges[i]);
      CSS.highlights.set('zunder-find', all);
      // Beim ersten Treffer im sichtbaren Bereich beginnen.
      for (var j = 0; j < ranges.length; j++) {
        if (ranges[j].getBoundingClientRect().bottom >= 0) {
          state.index = j - 1;
          break;
        }
      }
      if (direction < 0) state.index += 2;
    }
  }

  var count = state.ranges.length;
  if (!count) return [0, 0];
  state.index = (state.index + (direction < 0 ? -1 : 1) + count) % count;
  var current = state.ranges[state.index];
  CSS.highlights.set('zunder-find-current', new Highlight(current));
  var box = current.getBoundingClientRect();
  if (box.top < 60 || box.bottom > window.innerHeight - 60 || box.left < 0 || box.right > window.innerWidth) {
    var target = current.startContainer.parentElement;
    if (target && target.scrollIntoView) target.scrollIntoView({ block: 'center', inline: 'nearest' });
  }
  return [state.index + 1, count];
})
