// Oberfläche des Browsers: Tab-Leiste, Adressleiste, Vorschläge, Menü,
// Seitenleisten. Der Zustand selbst lebt im Rust-Teil – diese Datei zeigt
// ihn an und meldet Eingaben zurück.
//
// Alles, was von Webseiten stammt (Titel, Adressen, Icon-Adressen), wird nur
// als Text eingesetzt, nie als HTML.
(() => {
  'use strict';

  const DATA = JSON.parse(document.getElementById('data').textContent);
  const S = DATA.strings;
  const SVG_NS = 'http://www.w3.org/2000/svg';
  const $ = (id) => document.getElementById(id);
  const post = (message) => window.ipc.postMessage(JSON.stringify(message));
  const command = (id) => post({ c: 'cmd', id });

  function icon(name, extra) {
    const svg = document.createElementNS(SVG_NS, 'svg');
    svg.setAttribute('class', 'ic' + (extra ? ' ' + extra : ''));
    const use = document.createElementNS(SVG_NS, 'use');
    use.setAttribute('href', '#i-' + name);
    svg.appendChild(use);
    return svg;
  }

  function el(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text != null) node.textContent = text;
    return node;
  }

  function setUse(id, name) {
    const use = $(id);
    const href = '#i-' + name;
    if (use.getAttribute('href') !== href) use.setAttribute('href', href);
  }

  // Bild für ein Seiten-Icon; fällt auf ein neutrales Symbol zurück.
  function favicon(src, fallback) {
    const box = el('span', 'fav');
    const fallbackIcon = () => {
      box.textContent = '';
      box.appendChild(icon(fallback || 'globe', 'sm'));
    };
    // Ein Icon ist immer ein kleines Bild, das als Text mitkommt – nie eine
    // Adresse: Diese Oberfläche lädt nichts aus dem Netz.
    if (src && src.startsWith('data:image/png;base64,')) {
      const img = document.createElement('img');
      img.decoding = 'async';
      img.addEventListener('error', fallbackIcon, { once: true });
      img.src = src;
      box.appendChild(img);
    } else {
      fallbackIcon();
    }
    return box;
  }

  // Buchstaben-Zeichen für angeheftete Tabs ohne eigenes Seiten-Icon: der erste
  // Buchstabe des Titels, eingefärbt nach der Website – so ist jeder angeheftete
  // Tab (nur Symbol, kein Titel) auf einen Blick zu unterscheiden.
  // Angeheftete Tabs zeigen nur das Favicon. Ist es sehr hell (ein weißes
  // Logo), verschwände es auf dem weißen aktiven Tab – dann legen wir eine
  // dunkle Kachel darunter. Dunkle oder bunte Favicons bleiben ohne Kachel.
  function tintPinnedFav(box) {
    const img = box.querySelector('img');
    if (!img) return;
    const check = () => {
      try {
        const canvas = document.createElement('canvas');
        canvas.width = 16;
        canvas.height = 16;
        const ctx = canvas.getContext('2d', { willReadFrequently: true });
        if (!ctx) return;
        ctx.drawImage(img, 0, 0, 16, 16);
        const data = ctx.getImageData(0, 0, 16, 16).data;
        let opaque = 0;
        let light = 0;
        for (let i = 0; i < data.length; i += 4) {
          if (data[i + 3] < 40) continue;
          opaque += 1;
          if (0.299 * data[i] + 0.587 * data[i + 1] + 0.114 * data[i + 2] > 205) light += 1;
        }
        // Fast nur helle Punkte: braucht die Kachel.
        box.classList.toggle('light-fav', opaque > 4 && light / opaque > 0.7);
      } catch (error) {
        /* lieber keine Kachel als ein Fehler */
      }
    };
    if (img.complete && img.naturalWidth) check();
    else img.addEventListener('load', check, { once: true });
  }

  function monogram(tab) {
    const box = el('span', 'fav mono');
    let host = '';
    try {
      host = new URL(tab.url).hostname.replace(/^www\./, '');
    } catch (error) {
      /* keine brauchbare Adresse */
    }
    const label = String(tab.title || host || '').trim();
    const match = label.match(/[\p{L}\p{N}]/u);
    box.textContent = match ? match[0].toUpperCase() : '?';
    const seed = host || label || '?';
    let hash = 0;
    for (const ch of seed) hash = (hash * 31 + ch.charCodeAt(0)) >>> 0;
    box.style.setProperty('--mono', 'hsl(' + (hash % 360) + ' 52% 45%)');
    return box;
  }

  function prettyUrl(url) {
    let rest = url.replace(/^https?:\/\//i, '').replace(/^www\./i, '');
    if (rest.endsWith('/') && rest.indexOf('/') === rest.length - 1) rest = rest.slice(0, -1);
    return rest;
  }

  let state = null;
  const ui = {
    editing: false, // in der Adressleiste wird getippt
    typed: '', // der tatsächlich getippte Text (ohne Vervollständigung)
    seq: 0,
    items: [],
    selected: 0,
    arrowed: false,
    menu: null, // 'main' | 'tab'
    tabSearch: false,
    palette: false,
    lastPanel: null,
    panel: null,
    panelQuery: '',
    find: false,
    overlay: false,
    noticeId: null,
    noticeTimer: null,
    remoteTimer: null,
    pollTimer: null,
  };

  const urlInput = $('url');
  const tabsBox = $('tabs');

  // ------------------------------------------------------------------------
  // Tabs
  // ------------------------------------------------------------------------

  const tabNodes = new Map();

  function makeTab(tab) {
    const node = el('div', 'tab');
    node.dataset.id = String(tab.id);
    node.appendChild(el('span', 'fav'));
    node.appendChild(el('span', 'title'));
    const close = el('button', 'x');
    close.type = 'button';
    close.title = S.tip_close_tab;
    close.appendChild(icon('close'));
    node.appendChild(close);
    return node;
  }

  const groupNodes = new Map();

  function groupOf(id) {
    return (state.groups || []).find((group) => group.id === id) || null;
  }

  function containerOf(id) {
    return (state.containers || []).find((container) => container.id === id) || null;
  }

  // Der Kopf einer Tab-Gruppe: Farbe und Name; ein Klick klappt sie zu oder auf.
  function groupChip(group, count) {
    let node = groupNodes.get(group.id);
    if (!node) {
      node = el('div', 'gchip');
      node.dataset.group = String(group.id);
      node.appendChild(el('span', 'n'));
      node.appendChild(el('span', 'k'));
      groupNodes.set(group.id, node);
    }
    node.className = 'gchip gc' + group.color + (group.collapsed ? ' closed' : '');
    node.querySelector('.n').textContent = group.name;
    node.querySelector('.k').textContent = group.collapsed ? String(count) : '';
    node.title = group.name;
    return node;
  }

  function updateTab(node, tab) {
    node.classList.toggle('active', tab.id === state.active);
    node.classList.toggle('pinned', !!tab.pinned);
    const group = tab.group ? groupOf(tab.group) : null;
    node.className = node.className.replace(/ ?gc[0-7]/g, '') + (group ? ' gc' + group.color : '');
    node.classList.toggle('grouped', !!group);
    node.hidden = !!(group && group.collapsed);
    // Container: ein farbiger Streifen am Tab zeigt, in welchem er läuft.
    const container = tab.container ? containerOf(tab.container) : null;
    node.className = node.className.replace(/ ?cc[0-7]/g, '') + (container ? ' cc' + container.color : '');
    node.classList.toggle('contained', !!container);
    // Steht der Tab neben dem aktiven im Fenster, sieht man ihm das an.
    node.classList.toggle('paired', !!tab.paired && tab.id !== state.active);
    const title = node.querySelector('.title');
    if (title.textContent !== tab.title) title.textContent = tab.title;
    node.title = container ? tab.title + ' · ' + container.name : tab.title;
    const hasFav = !state.private && typeof tab.favicon === 'string' && tab.favicon.startsWith('data:image/png;base64,');
    // Angeheftete Tabs zeigen nur ein Symbol; ohne eigenes Seiten-Icon bekommen
    // sie ein Buchstaben-Zeichen (hängt an Titel + Adresse, damit es mitwandert).
    const key = tab.loading
      ? 'loading'
      : state.private
        ? 'private'
        : hasFav
          ? (tab.pinned ? 'pin:' : '') + tab.favicon
          : tab.pinned
            ? 'mono:' + (tab.title || '') + '|' + tab.url
            : 'none';
    if (node.dataset.fav !== key) {
      node.dataset.fav = key;
      let fav;
      if (tab.loading) {
        fav = el('span', 'fav');
        fav.appendChild(el('span', 'spinner'));
      } else if (hasFav) {
        fav = favicon(tab.favicon, 'globe');
        // Nur angeheftete Tabs zeigen das Favicon allein – nur sie brauchen
        // bei einem hellen Logo die Kachel.
        if (tab.pinned) tintPinnedFav(fav);
      } else if (tab.pinned) {
        fav = monogram(tab);
      } else {
        fav = favicon(state.private ? null : tab.favicon, 'globe');
      }
      node.replaceChild(fav, node.querySelector('.fav'));
    }
  }

  function renderTabs() {
    const seen = new Set();
    const seenGroups = new Set();
    // Die Leiste in ihrer Reihenfolge: vor dem ersten Tab jeder Gruppe ihr Kopf.
    const order = [];
    state.tabs.forEach((tab) => {
      let node = tabNodes.get(tab.id);
      if (!node) {
        node = makeTab(tab);
        tabNodes.set(tab.id, node);
      }
      updateTab(node, tab);
      const group = tab.group ? groupOf(tab.group) : null;
      if (group && !seenGroups.has(group.id)) {
        seenGroups.add(group.id);
        const count = state.tabs.filter((other) => other.group === group.id).length;
        order.push(groupChip(group, count));
      }
      order.push(node);
      seen.add(tab.id);
    });
    order.forEach((node, index) => {
      if (tabsBox.children[index] !== node) tabsBox.insertBefore(node, tabsBox.children[index] || null);
    });
    for (const [id, node] of tabNodes) {
      if (!seen.has(id)) {
        node.remove();
        tabNodes.delete(id);
      }
    }
    for (const [id, node] of groupNodes) {
      if (!seenGroups.has(id)) {
        node.remove();
        groupNodes.delete(id);
      }
    }
    measureTabs();
    // Bei sehr vielen Tabs passt nicht alles in die Leiste: Der aktive bleibt sichtbar.
    if (shownActive !== state.active) {
      shownActive = state.active;
      keepActiveVisible();
    }
    $('alltabs').hidden = state.tabs.length < 6;
  }

  let shownActive = null;

  function keepActiveVisible() {
    const current = state && tabNodes.get(state.active);
    if (current) current.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  }

  // Die Tab-Leiste lässt sich mit dem Rad bzw. zwei Fingern verschieben, wenn sie überläuft.
  tabsBox.addEventListener(
    'wheel',
    (event) => {
      if (tabsBox.scrollWidth <= tabsBox.clientWidth) return;
      tabsBox.scrollLeft += Math.abs(event.deltaX) > Math.abs(event.deltaY) ? event.deltaX : event.deltaY;
      event.preventDefault();
    },
    { passive: false }
  );

  function measureTabs() {
    for (const node of tabNodes.values()) {
      node.classList.toggle('narrow', node.offsetWidth < 66);
    }
  }

  // Ziehen zum Umsortieren, Klick zum Aktivieren.
  let drag = null;

  tabsBox.addEventListener('mousedown', (event) => {
    const node = event.target.closest('.tab');
    if (!node || event.button !== 0) return;
    if (event.target.closest('.x')) return;
    const id = Number(node.dataset.id);
    if (id !== state.active) post({ c: 'activateTab', id });
    const nodes = shownTabs();
    drag = {
      id,
      node,
      startX: event.clientX,
      index: nodes.indexOf(node),
      target: nodes.indexOf(node),
      width: node.getBoundingClientRect().width + 4,
      centers: nodes.map((n) => {
        const r = n.getBoundingClientRect();
        return r.left + r.width / 2;
      }),
      moved: false,
    };
  });

  window.addEventListener('mousemove', (event) => {
    if (!drag) return;
    const dx = event.clientX - drag.startX;
    if (!drag.moved && Math.abs(dx) < 6) return;
    if (!drag.moved) {
      drag.moved = true;
      drag.node.classList.add('dragging');
      shownTabs().forEach((n) => n !== drag.node && n.classList.add('shifting'));
    }
    drag.node.style.transform = 'translateX(' + dx + 'px)';
    const center = drag.centers[drag.index] + dx;
    let target = drag.index;
    drag.centers.forEach((c, i) => {
      if (i < drag.index && center < c) target = Math.min(target, i);
      if (i > drag.index && center > c) target = Math.max(target, i);
    });
    drag.target = target;
    shownTabs().forEach((n, i) => {
      if (n === drag.node) return;
      let shift = 0;
      if (target > drag.index && i > drag.index && i <= target) shift = -drag.width;
      if (target < drag.index && i >= target && i < drag.index) shift = drag.width;
      n.style.transform = shift ? 'translateX(' + shift + 'px)' : '';
    });
  });

  window.addEventListener('mouseup', () => {
    if (!drag) return;
    const done = drag;
    drag = null;
    Array.from(tabsBox.children).forEach((n) => {
      n.style.transform = '';
      n.classList.remove('dragging', 'shifting');
    });
    if (done.moved && done.target !== done.index) {
      // Wohin in der ganzen Liste: an die Stelle des Tabs, der dort gerade steht.
      const at = shownTabs()[done.target];
      const index = at ? state.tabs.findIndex((tab) => tab.id === Number(at.dataset.id)) : -1;
      if (index >= 0) post({ c: 'moveTab', id: done.id, index });
    }
  });

  // Die Tabs, die gerade in der Leiste zu sehen sind (ohne Gruppenköpfe, ohne Zugeklapptes).
  function shownTabs() {
    return Array.from(tabsBox.children).filter((node) => node.classList.contains('tab') && !node.hidden);
  }

  // Gruppenkopf: Klick klappt zu oder auf, Rechtsklick öffnet das Menü der Gruppe.
  tabsBox.addEventListener('click', (event) => {
    const chip = event.target.closest('.gchip');
    if (chip) post({ c: 'groupToggle', id: Number(chip.dataset.group) });
  });

  tabsBox.addEventListener('click', (event) => {
    const close = event.target.closest('.x');
    if (!close) return;
    const node = close.closest('.tab');
    if (node) post({ c: 'closeTab', id: Number(node.dataset.id) });
  });

  tabsBox.addEventListener('auxclick', (event) => {
    const node = event.target.closest('.tab');
    if (node && event.button === 1) post({ c: 'closeTab', id: Number(node.dataset.id) });
  });

  tabsBox.addEventListener('contextmenu', (event) => {
    const chip = event.target.closest('.gchip');
    if (chip) {
      event.preventDefault();
      openGroupMenu(Number(chip.dataset.group), event.clientX);
      return;
    }
    const node = event.target.closest('.tab');
    if (!node) return;
    event.preventDefault();
    openTabMenu(Number(node.dataset.id), event.clientX);
  });

  $('newtab').addEventListener('click', () => post({ c: 'newTab' }));
  $('alltabs').addEventListener('click', () => (ui.tabSearch ? closeTabSearch() : openTabSearch()));

  // Freie Flächen des Rahmens verschieben das Fenster.
  function isDragArea(target) {
    return (
      target.id === 'tabstrip' ||
      target.id === 'tabs' ||
      target.id === 'dragfill' ||
      target.id === 'toolbar' ||
      target.id === 'private-badge' ||
      (target.parentElement && target.parentElement.id === 'private-badge')
    );
  }

  $('chrome').addEventListener('mousedown', (event) => {
    if (event.button === 0 && isDragArea(event.target)) {
      if (event.detail === 2) post({ c: 'zoomWindow' });
      else post({ c: 'drag' });
    }
  });

  // ------------------------------------------------------------------------
  // Adressleiste
  // ------------------------------------------------------------------------

  // Ob die Adressleiste wirklich den Tastaturfokus hat. Das Feld bleibt für das
  // Dokument „aktiv“, auch wenn der Fokus längst in der Seite liegt.
  function addressFocused() {
    return document.activeElement === urlInput && document.hasFocus();
  }

  // Solange niemand tippt, zeigt die Adressleiste die Adresse in drei Teilen:
  // Unterbereiche, der Teil, der sagt, wem die Seite gehört, und der Rest.
  function showParts() {
    const parts = state && state.nav.parts;
    const on = !!parts && !addressFocused() && !ui.editing && urlInput.value === state.nav.pretty;
    $('address').classList.toggle('parts', on);
    if (!on) return;
    const show = $('url-show');
    show.querySelector('.pre span').textContent = parts[0];
    show.querySelector('.dom').textContent = parts[1];
    show.querySelector('.rest').textContent = parts[2];
  }

  function showUrl() {
    if (!state) return;
    const focused = addressFocused();
    const value = focused ? state.nav.url : state.nav.pretty;
    if (urlInput.value !== value) urlInput.value = value;
    if (focused) urlInput.select();
    showParts();
  }

  function stopEditing() {
    ui.editing = false;
    ui.typed = '';
    ui.arrowed = false;
    closeSuggest();
  }

  // Die Adressleiste „hat den Fokus“, wenn das Feld aktiv ist UND der Rahmen
  // die Tastatur hat. Beides kann sich getrennt ändern, deshalb eigener Merker.
  let addressActive = false;
  let addressActivatedAt = 0;

  function addressGainedFocus() {
    if (addressActive) return;
    addressActive = true;
    addressActivatedAt = performance.now();
    if (state && !ui.editing) {
      urlInput.value = state.nav.url;
      urlInput.select();
    }
    showParts();
  }

  function addressLostFocus() {
    // Beim Klick auf einen Vorschlag kommt der Klick vor dem Schließen an.
    setTimeout(() => {
      if (addressFocused()) return;
      addressActive = false;
      closeSuggest();
      // Angefangene Eingaben bleiben stehen, bis Esc sie verwirft oder der Tab
      // eine neue Seite zeigt.
      if (!ui.editing) showUrl();
    }, 0);
  }

  urlInput.addEventListener('focus', () => {
    if (document.hasFocus()) addressGainedFocus();
  });
  urlInput.addEventListener('blur', addressLostFocus);
  window.addEventListener('blur', addressLostFocus);
  window.addEventListener('focus', () => {
    if (document.activeElement === urlInput) addressGainedFocus();
  });

  // Der Klick, der die Adressleiste aktiviert, markiert alles; erst der
  // nächste setzt die Einfügemarke.
  urlInput.addEventListener('mousedown', (event) => {
    const activating = !addressActive || performance.now() - addressActivatedAt < 300;
    if (activating && !ui.editing) {
      event.preventDefault();
      urlInput.focus();
      addressGainedFocus();
      urlInput.select();
    }
  });

  urlInput.addEventListener('input', (event) => {
    ui.editing = true;
    $('address').classList.remove('parts');
    ui.arrowed = false;
    ui.typed = urlInput.value;
    const deleting = typeof event.inputType === 'string' && event.inputType.startsWith('delete');
    requestSuggest(deleting);
  });

  function requestSuggest(plain) {
    const seq = ++ui.seq;
    const text = ui.typed;
    clearTimeout(ui.remoteTimer);
    if (!text.trim()) {
      closeSuggest();
      return;
    }
    post({ c: 'suggest', text, seq, plain: !!plain });
    ui.remoteTimer = setTimeout(() => post({ c: 'suggestRemote', text, seq }), 150);
  }

  function activateSuggestion(item, newTab) {
    if (!item) return;
    stopEditing();
    urlInput.blur();
    if (item.k === 'tab' && item.id != null && !newTab) post({ c: 'activateTab', id: item.id });
    else post({ c: 'open', url: item.url, newTab: !!newTab });
  }

  urlInput.addEventListener('keydown', (event) => {
    const open = !$('suggest').hidden;
    if (event.key === 'Enter') {
      event.preventDefault();
      const newTab = event.metaKey || event.altKey;
      if (open && ui.arrowed && ui.items[ui.selected]) {
        activateSuggestion(ui.items[ui.selected], newTab);
        return;
      }
      const text = urlInput.value;
      stopEditing();
      urlInput.blur();
      if (text.trim()) post({ c: 'navigate', text, newTab });
      else post({ c: 'focusContent' });
    } else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      if (!open || !ui.items.length) return;
      event.preventDefault();
      const step = event.key === 'ArrowDown' ? 1 : -1;
      ui.selected = (ui.selected + step + ui.items.length) % ui.items.length;
      ui.arrowed = true;
      const item = ui.items[ui.selected];
      urlInput.value = item.k === 'search' ? item.title : item.url;
      renderSuggest();
    } else if (event.key === 'Escape') {
      event.preventDefault();
      if (open) {
        urlInput.value = ui.typed;
        ui.arrowed = false;
        closeSuggest();
      } else {
        stopEditing();
        urlInput.blur();
        post({ c: 'focusContent' });
      }
    }
  });

  function suggestionIcon(item) {
    if (item.k === 'search') return favicon(null, 'search');
    if (item.k === 'tab') return favicon(item.icon, 'window');
    if (item.k === 'bookmark') return favicon(item.icon, 'star');
    if (item.k === 'history') return favicon(item.icon, 'history');
    return favicon(item.icon, 'globe');
  }

  function renderSuggest() {
    const box = $('suggest');
    if (!ui.items.length || !addressFocused()) {
      closeSuggest();
      return;
    }
    const anchor = $('address').getBoundingClientRect();
    box.style.left = anchor.left + 'px';
    box.style.top = anchor.bottom + 6 + 'px';
    box.style.width = anchor.width + 'px';
    box.textContent = '';
    ui.items.forEach((item, index) => {
      const row = el('div', 'sug' + (index === ui.selected ? ' sel' : ''));
      row.appendChild(suggestionIcon(item));
      row.appendChild(el('span', 't', item.title || prettyUrl(item.url)));
      const sub = item.sub || (item.k === 'search' ? '' : prettyUrl(item.url));
      if (sub) row.appendChild(el('span', 's', sub));
      row.addEventListener('mousedown', (event) => {
        event.preventDefault();
        activateSuggestion(item, event.metaKey);
      });
      box.appendChild(row);
    });
    box.hidden = false;
    syncOverlay();
  }

  function closeSuggest() {
    ui.items = [];
    ui.selected = 0;
    if (!$('suggest').hidden) {
      $('suggest').hidden = true;
      syncOverlay();
    }
  }

  function onSuggest(message) {
    if (message.seq !== ui.seq || !ui.editing || !addressFocused()) return;
    ui.items = message.items || [];
    ui.selected = 0;
    ui.arrowed = false;
    // Vervollständigung anhängen und markieren – weitertippen ersetzt sie einfach.
    const typed = ui.typed;
    const complete = message.complete;
    if (
      complete &&
      urlInput.value === typed &&
      urlInput.selectionStart === typed.length &&
      complete.length > typed.length &&
      complete.toLowerCase().startsWith(typed.toLowerCase())
    ) {
      urlInput.value = typed + complete.slice(typed.length);
      urlInput.setSelectionRange(typed.length, urlInput.value.length);
    }
    renderSuggest();
  }

  function onSuggestMore(message) {
    if (message.seq !== ui.seq || !ui.editing || $('suggest').hidden) return;
    const known = new Set(ui.items.map((i) => (i.k === 'search' ? 's:' + i.title.toLowerCase() : i.url)));
    for (const item of message.items || []) {
      if (ui.items.length >= 9) break;
      const key = 's:' + String(item.title).toLowerCase();
      if (!known.has(key)) {
        ui.items.push(item);
        known.add(key);
      }
    }
    renderSuggest();
  }

  // ------------------------------------------------------------------------
  // Symbolleiste
  // ------------------------------------------------------------------------

  $('back').addEventListener('click', () => command('back'));
  $('forward').addEventListener('click', () => command('forward'));
  $('reload').addEventListener('click', (event) => {
    if (state && state.nav.loading) command('stop');
    else command(event.shiftKey ? 'hard_reload' : 'reload');
  });
  $('star').addEventListener('click', () => command('bookmark_page'));
  $('reader').addEventListener('click', () => command('reader'));
  $('translate').addEventListener('click', () => command('translate_page'));
  $('mobile').addEventListener('click', () => command('mobile_view'));
  $('shield').addEventListener('click', () => (ui.menu === 'shield' ? closeMenu() : openShield()));
  // Klick auf den Mini-Knopf: in den laufenden Eintrag, sonst in die App. Steht
  // gerade die Deckel-Frage, zählt der Klick als „weiterlaufen“.
  $('timer').addEventListener('click', () => {
    const run = timerRun();
    if (timerAsk) {
      const capH = (ui.timer && ui.timer.cap) || TIMER_CAP_DEFAULT;
      if (run) run.cap = Math.floor(timerElapsed(run, Date.now()) / 3600000) + capH;
      timerAsk = null;
      renderTimer();
      post({ c: 'timerSnooze' });
    }
    post({ c: 'timerOpen', edit: !!run });
  });
  $('ampel').addEventListener('click', () => (ui.menu === 'ampel' ? closeMenu() : openAmpel()));
  $('sec').addEventListener('click', () => (ui.menu === 'check' ? closeMenu() : openCheck()));
  $('key').addEventListener('click', () => {
    closeEverything();
    post({ c: 'pwKey' });
  });
  $('downloads').addEventListener('click', () => togglePanel('downloads'));
  $('assistant').addEventListener('click', () => {
    closeEverything();
    command('assistant');
  });
  // TL;DR: öffnet den Assistenten und fasst die aktuelle Seite zusammen (⇧⌘J).
  $('summarize').addEventListener('click', () => {
    closeEverything();
    command('assistant_summary');
  });
  $('summarize').title = S.m_assistant_summary;
  $('menu').addEventListener('click', () => (ui.menu === 'main' ? closeMenu() : openMainMenu()));

  let progressValue = 0;

  function setProgress(loading, value) {
    const bar = $('progress-bar');
    const box = $('progress');
    if (loading) {
      progressValue = Math.max(progressValue, value || 0.1);
      box.classList.add('on');
      bar.style.width = Math.round(progressValue * 100) + '%';
    } else if (box.classList.contains('on')) {
      bar.style.width = '100%';
      progressValue = 0;
      box.classList.remove('on');
      setTimeout(() => {
        if (!box.classList.contains('on')) bar.style.width = '0';
      }, 600);
    }
  }

  const SECURITY_ICON = {
    secure: 'lock',
    insecure: 'warning',
    error: 'warning',
    none: 'search',
    internal: 'info',
    file: 'file',
  };

  function applyState(next) {
    const previous = state;
    state = next;
    const nav = next.nav;
    document.body.classList.toggle('private', !!next.private);
    document.body.classList.toggle('stealth', !!next.stealth);
    document.body.classList.toggle('tor', !!next.tor);
    document.body.classList.toggle('fullscreen', !!next.fullscreen);
    $('private-badge').hidden = !next.private;
    // Stealth-Fenster: eigenes Abzeichen – und ob es über Tor läuft.
    setUse('private-icon', next.stealth ? 'stealth' : 'private');
    $('private-badge').querySelector('span').textContent = next.stealth ? (next.tor ? S.stealth_tor : S.stealth) : S.private;
    $('private-badge').title = next.stealth ? (next.tor ? S.stealth_tip_tor : S.stealth_tip) : '';

    renderTabs();
    if (ui.overview) renderOverview();

    $('back').disabled = !nav.canBack;
    $('forward').disabled = !nav.canForward;
    setUse('reload-icon', nav.loading ? 'close' : 'reload');
    $('reload').title = nav.loading ? S.tip_stop : S.tip_reload;

    const tabChanged = !previous || previous.active !== next.active;
    const pageChanged = !!previous && previous.nav.url !== nav.url;
    if (tabChanged) {
      stopEditing();
      progressValue = 0;
      if (ui.find) closeFind(true);
    } else if (pageChanged && !addressFocused()) {
      // Der Tab zeigt eine andere Seite – eine liegen gebliebene Eingabe ist überholt.
      stopEditing();
    }
    if (!ui.editing) showUrl();

    const security = $('security');
    security.className = nav.security;
    setUse('security-icon', SECURITY_ICON[nav.security] || 'search');
    security.title = nav.security === 'secure' ? S.sec_secure : nav.security === 'insecure' ? S.sec_insecure : '';

    const star = $('star');
    star.hidden = !nav.canBookmark;
    star.classList.toggle('on', !!nav.bookmarked);
    setUse('star-icon', nav.bookmarked ? 'star-filled' : 'star');
    star.title = nav.bookmarked ? S.tip_bookmarked : S.tip_bookmark;

    setProgress(nav.loading, nav.progress);

    const downloads = $('downloads');
    downloads.hidden = !next.downloads.any;
    downloads.classList.toggle('busy', next.downloads.active > 0);

    $('assistant').classList.toggle('on', !!next.assistant);

    // Schlüssel: Für diese Website gibt es Gesichertes – oder eine offene Frage.
    const logins = next.logins || {};
    const key = $('key');
    key.hidden = !logins.show;
    key.classList.toggle('on', (logins.saved || 0) > 0);
    key.classList.toggle('pending', !!logins.pending);

    $('mobile').hidden = !next.mobile;
    renderAmpel(next.ampel);
    renderCheck(next.check);
    renderShield(next.shield);
    // Das Container-Fenster zeigt immer den aktuellen Stand – nur nicht,
    // während gerade ein Name getippt wird (das würde die Eingabe stören).
    if (ui.menu === 'containers') {
      const active = document.activeElement;
      const typing = active && active.classList && active.classList.contains('cnname');
      if (!typing) fillContainers();
    }
    ui.timer = next.timer || null;
    renderTimer();
    timerGuard();
    if (ui.menu === 'timer') fillTimer();
    renderSplitBar(next);
    const reader = $('reader');
    reader.hidden = !next.canRead;
    reader.classList.toggle('on', !!next.reader);
    // Übersetzen: angeboten, wenn die Seite eine andere Sprache angibt – und
    // solange sie übersetzt dasteht, führt derselbe Knopf zurück zum Original.
    const translate = $('translate');
    translate.hidden = !next.canTranslate;
    translate.classList.toggle('on', !!next.translated);
    translate.title = next.translated ? S.tip_translate_undo : S.tip_translate;

    if (ui.tabSearch) renderTabSearch();
    if (ui.palette) renderPalette();

    const pct = document.querySelector('#popup .pct');
    if (pct) pct.textContent = nav.zoom + ' %';
  }

  // ------------------------------------------------------------------------
  // Ebene über der Seite
  // ------------------------------------------------------------------------

  function syncOverlay() {
    const need =
      !$('suggest').hidden || !!ui.menu || !!ui.panel || ui.tabSearch || ui.palette || !!ui.overview || !!ui.splash;
    document.body.classList.toggle('sheet', !!ui.panel);
    if (need === ui.overlay) return;
    ui.overlay = need;
    document.body.classList.toggle('overlay', need);
    post({ c: 'overlay', open: need });
  }

  function closeEverything() {
    closeMenu();
    closeTabSearch();
    closePalette();
    closeOverview();
    closePanel();
    stopEditing();
    if (document.activeElement === urlInput) urlInput.blur();
    showUrl();
  }

  $('scrim').addEventListener('mousedown', () => {
    closeEverything();
    post({ c: 'focusContent' });
  });

  // Höhe des Rahmens melden – Suchleiste und Hinweise schieben die Seite nach unten.
  let reportedHeight = 0;

  function reportHeight() {
    const height = Math.round($('chrome').getBoundingClientRect().height);
    document.documentElement.style.setProperty('--chrome-bottom', height + 'px');
    if (height !== reportedHeight) {
      reportedHeight = height;
      post({ c: 'chromeHeight', h: height });
    }
  }

  new ResizeObserver(reportHeight).observe($('chrome'));
  new ResizeObserver(() => {
    measureTabs();
    keepActiveVisible();
  }).observe(tabsBox);

  // ------------------------------------------------------------------------
  // Menüs
  // ------------------------------------------------------------------------

  function menuItem(label, iconName, shortcut, action) {
    const row = el('div', 'mi');
    row.appendChild(iconName ? icon(iconName) : el('span', 'ic'));
    row.appendChild(el('span', 'l', label));
    if (shortcut) row.appendChild(el('span', 'k', shortcut));
    row.addEventListener('click', () => {
      closeMenu();
      action();
    });
    return row;
  }

  function openMainMenu() {
    const box = $('popup');
    box.textContent = '';
    const add = (id, label, iconName, shortcut) => box.appendChild(menuItem(label, iconName, shortcut, () => command(id)));
    const sep = () => box.appendChild(el('div', 'msep'));

    add('new_tab', S.m_new_tab, 'plus', '⌘T');
    add('new_window', S.m_new_window, 'window', '⌘N');
    add('new_private', S.m_new_private, 'private', '⇧⌘N');
    add('new_stealth', S.m_new_stealth, 'stealth', '⌥⌘N');
    box.appendChild(menuItem(S.m_command_palette, 'search', '⌘K', openPalette));
    box.appendChild(menuItem(S.m_tab_search, 'tabs', '⇧⌘A', openTabSearch));
    box.appendChild(menuItem(S.m_tab_overview, 'grid', '⇧⌘\\', openOverview));
    // Container gibt es nur in gewöhnlichen Fenstern – private und
    // Stealth-Fenster haben ohnehin eigene, getrennte Speicher.
    if (state && !state.private && !state.stealth) {
      box.appendChild(menuItem(S.m_containers, 'window', '', openContainers));
    }
    sep();
    box.appendChild(menuItem(S.p_history, 'history', '⌘Y', () => togglePanel('history')));
    box.appendChild(menuItem(S.p_bookmarks, 'bookmark', '⌥⌘B', () => togglePanel('bookmarks')));
    add('bookmark_bar', S.m_bookmark_bar, 'star', '⇧⌘B');
    box.appendChild(menuItem(S.p_downloads, 'download', '⌥⌘L', () => togglePanel('downloads')));
    box.appendChild(menuItem(S.p_passwords, 'key', '', () => togglePanel('passwords')));
    sep();
    add('reader', S.m_reader, 'reader', '⌥⌘R');
    add('translate_page', state && state.translated ? S.m_translate_undo : S.m_translate, 'translate', '⌥⌘T');
    if (ui.check) box.appendChild(menuItem(S.m_site_check, 'lock', '⌥⌘S', openCheck));
    add('assistant', S.m_assistant, 'sparkle', '⌘J');
    add('assistant_summary', S.m_assistant_summary, 'list', '⇧⌘J');
    add('compare', S.m_compare, 'compare', '⌥⌘J');
    if (state && state.split) add('unsplit', S.m_unsplit, 'window', '');
    sep();

    const zoom = el('div', 'mi plain');
    zoom.appendChild(icon('search'));
    zoom.appendChild(el('span', 'l', S.x_zoom));
    const controls = el('span', 'zoomctl');
    const minus = el('button', 'ibtn sm');
    minus.type = 'button';
    minus.appendChild(icon('minus', 'sm'));
    minus.addEventListener('click', () => command('zoom_out'));
    const pct = el('span', 'pct', (state ? state.nav.zoom : 100) + ' %');
    pct.addEventListener('click', () => command('zoom_reset'));
    const plus = el('button', 'ibtn sm');
    plus.type = 'button';
    plus.appendChild(icon('plus', 'sm'));
    plus.addEventListener('click', () => command('zoom_in'));
    controls.append(minus, pct, plus);
    zoom.appendChild(controls);
    box.appendChild(zoom);
    sep();

    add('find', S.m_find, 'search', '⌘F');
    add('print', S.m_print, 'print', '⌘P');
    add('mobile_view', S.m_mobile_view, 'phone', '⇧⌘M');
    add('view_source', S.m_view_source, 'file', '⌥⌘U');
    add('devtools', S.m_devtools, 'code', '⌥⌘I');
    sep();
    box.appendChild(menuItem(S.p_settings, 'settings', '⌘,', () => togglePanel('settings')));

    const anchor = $('menu').getBoundingClientRect();
    box.style.top = anchor.bottom + 8 + 'px';
    box.style.right = Math.max(8, window.innerWidth - anchor.right) + 'px';
    box.style.left = 'auto';
    box.hidden = false;
    ui.menu = 'main';
    closePanel();
    closeTabSearch();
    syncOverlay();
  }

  function openTabMenu(id, x) {
    const box = $('popup');
    box.textContent = '';
    const add = (label, iconName, action) =>
      box.appendChild(menuItem(label, iconName, null, () => post({ c: 'tabMenu', id, action })));
    add(S.c_reload, 'reload', 'reload');
    add(S.c_duplicate, 'copy', 'duplicate');
    // Compared Browsing: diesen Tab neben den aktuellen stellen – oder zurück.
    const tab = state ? state.tabs.find((entry) => entry.id === id) : null;
    if (tab && tab.paired) add(S.c_unsplit, 'compare', 'unsplit');
    else if (state && id !== state.active) add(S.c_split, 'compare', 'split');
    box.appendChild(el('div', 'msep'));
    // Anheften und Gruppen.
    if (tab && tab.pinned) add(S.c_unpin, 'bookmark', 'unpin');
    else add(S.c_pin, 'bookmark', 'pin');
    if (tab && !tab.pinned) {
      add(S.c_group_new, 'folder-plus', 'groupNew');
      for (const group of (state && state.groups) || []) {
        if (tab.group === group.id) continue;
        box.appendChild(
          menuItem(S.c_group_add.replace('{name}', group.name), 'folder', null, () =>
            post({ c: 'tabMenu', id, action: 'groupAdd', group: group.id })
          )
        );
      }
      if (tab.group) add(S.c_group_leave, 'minus', 'groupLeave');
    }
    box.appendChild(el('div', 'msep'));
    add(S.c_close, 'close', 'close');
    add(S.c_close_others, 'close', 'closeOthers');
    box.style.top = reportedHeight - 38 + 'px';
    box.style.left = Math.max(8, Math.min(x - 20, window.innerWidth - 270)) + 'px';
    box.style.right = 'auto';
    box.hidden = false;
    ui.menu = 'tab';
    syncOverlay();
  }

  // Die Tarn-Ampel im Stealth-Fenster: wie gut man auf DIESER Seite getarnt
  // ist – und was die Seite über das Gerät abgefragt hat.
  const AMPEL_KINDS = ['screen', 'canvas', 'webgl', 'device', 'agent', 'language', 'timezone', 'fonts', 'cookie_write',
    'cookie_read', 'storage', 'referrer', 'location', 'media', 'clipboard', 'notify', 'permissions', 'beacon'];

  function renderAmpel(ampel) {
    ui.ampel = ampel || null;
    const button = $('ampel');
    button.hidden = !ampel;
    if (!ampel) {
      if (ui.menu === 'ampel') closeMenu();
      return;
    }
    button.className = 'lvl' + ampel.level;
    button.title = S['am_tip_' + ampel.level];
    if (ui.menu === 'ampel') fillAmpel();
  }

  function ampelReason(ampel) {
    if (ampel.signedIn) return S.am_why_signed;
    if (!ampel.secure) return S.am_why_http;
    if (!ampel.tor) return S.am_why_ip;
    if (!ampel.blocker) return S.am_why_blocker;
    return S.am_why_ok;
  }

  function fillAmpel() {
    const ampel = ui.ampel;
    if (!ampel) return;
    const box = $('popup');
    const scroll = box.scrollTop;
    box.textContent = '';
    box.classList.add('ampelbox');

    const head = el('div', 'amh lvl' + ampel.level);
    const lights = el('div', 'lights');
    for (const name of ['r', 'y', 'g']) lights.appendChild(el('i', name));
    head.appendChild(lights);
    const words = el('div', 'txt');
    words.appendChild(el('div', 't', S['am_title_' + ampel.level]));
    words.appendChild(el('div', 'h', ampelReason(ampel)));
    head.appendChild(words);
    box.appendChild(head);

    // Woraus sich die Farbe ergibt.
    const fact = (level, label, value) => {
      const row = el('div', 'amf');
      row.appendChild(el('i', 'dot lvl' + level));
      row.appendChild(el('span', 'l', label));
      row.appendChild(el('span', 'v', value));
      box.appendChild(row);
    };
    fact(ampel.tor ? 2 : 1, S.am_f_ip, ampel.tor ? S.am_f_ip_on : S.am_f_ip_off);
    fact(ampel.secure ? 2 : 0, S.am_f_conn, ampel.secure ? S.am_f_conn_on : S.am_f_conn_off);
    fact(
      ampel.blocker ? 2 : 1,
      S.am_f_block,
      !ampel.blocker ? S.am_f_block_off : ampel.blocked ? S.am_f_block_n.replace('{n}', ampel.blocked.toLocaleString()) : S.am_f_block_on
    );
    fact(2, S.am_f_island, S.am_f_island_on);
    fact(2, S.am_f_print, S.am_f_print_on);
    fact(ampel.signedIn ? 0 : 2, S.am_f_login, ampel.signedIn ? S.am_f_login_yes : S.am_f_login_no);

    // Was die Seite selbst abgefragt hat.
    box.appendChild(el('div', 'amsub', S.am_asked_head.replace('{site}', ampel.site || '')));
    const asked = new Map((ampel.asked || []).map((entry) => [entry.k, entry.n]));
    const kinds = AMPEL_KINDS.filter((kind) => asked.has(kind));
    if (!kinds.length) box.appendChild(el('div', 'amnone', S.am_asked_none));
    for (const kind of kinds) {
      const row = el('div', 'ama');
      const text = el('div', 'txt');
      text.appendChild(el('span', 'l', S['am_k_' + kind] || kind));
      const note = S['am_n_' + kind];
      if (note) text.appendChild(el('span', 'n', note));
      row.appendChild(text);
      const count = asked.get(kind);
      row.appendChild(el('span', 'c', count >= 9999 ? '9999+ ×' : count.toLocaleString() + ' ×'));
      box.appendChild(row);
    }
    const embedded = ampel.embedded || [];
    if (embedded.length) {
      box.appendChild(el('div', 'amsub', S.am_embedded_head));
      for (const frame of embedded) {
        const row = el('div', 'ama');
        const text = el('div', 'txt');
        text.appendChild(el('span', 'l', frame.host));
        text.appendChild(el('span', 'n', frame.kinds.map((kind) => S['am_k_' + kind] || kind).join(', ')));
        row.appendChild(text);
        box.appendChild(row);
      }
    }
    box.appendChild(el('div', 'amnote', S.am_note));
    box.scrollTop = scroll;
  }

  function openAmpel() {
    fillAmpel();
    const box = $('popup');
    const place = $('ampel').getBoundingClientRect();
    box.style.top = Math.round(place.bottom + 8) + 'px';
    box.style.left = Math.max(8, Math.min(place.right - 360, window.innerWidth - 368)) + 'px';
    box.style.right = 'auto';
    box.hidden = false;
    ui.menu = 'ampel';
    closePanel();
    closeTabSearch();
    syncOverlay();
  }

  // ------------------------------------------------------------------------
  // Security-Ampel: der Seitencheck. Links in der Adresszeile steht, wie die
  // gezeigte Seite dasteht; ein Klick zeigt, wo sie angreifbar wäre. Die Texte
  // kommen fertig vom Browser – hier werden sie nur als Text eingesetzt.
  // ------------------------------------------------------------------------

  function renderCheck(check) {
    ui.check = check || null;
    const button = $('sec');
    button.hidden = !check;
    // Die Ampel sagt mehr als das Schloss – sie steht dann an seiner Stelle.
    $('security').hidden = !!check;
    if (!check) {
      if (ui.menu === 'check') closeMenu();
      return;
    }
    button.className = 'lvl' + check.level;
    button.title = S['sc_tip_' + check.level];
    $('sec-n').textContent = check.count ? String(check.count) : '';
    if (ui.menu === 'check') fillCheck();
  }

  function fillCheck() {
    const check = ui.check;
    if (!check) return;
    const box = $('popup');
    const scroll = box.scrollTop;
    box.textContent = '';
    box.classList.add('ampelbox');

    const head = el('div', 'amh lvl' + check.level);
    const lights = el('div', 'lights');
    for (const name of ['r', 'y', 'g']) lights.appendChild(el('i', name));
    head.appendChild(lights);
    const words = el('div', 'txt');
    const unsure = check.partial && !check.count;
    words.appendChild(el('div', 't', unsure ? S.sc_title_partial : S['sc_title_' + check.level]));
    const sub = check.count
      ? S.sc_sub.replace('{site}', check.site).replace('{a}', check.serious).replace('{b}', check.count - check.serious)
      : unsure
        ? check.site
        : S.sc_sub_ok.replace('{site}', check.site);
    words.appendChild(el('div', 'h', sub));
    head.appendChild(words);
    box.appendChild(head);

    const items = check.items || [];
    const section = (title, list, compact) => {
      if (!list.length) return;
      box.appendChild(el('div', 'amsub', title));
      for (const item of list) {
        const row = el('div', 'scf' + (compact ? ' ok' : ''));
        row.appendChild(el('i', 'dot lvl' + (item.l > 2 ? 2 : item.l === 2 ? 'x' : item.l)));
        const text = el('div', 'txt');
        text.appendChild(el('span', 'l', item.t));
        if (item.d && !compact) text.appendChild(el('span', 'n', item.d));
        row.appendChild(text);
        box.appendChild(row);
      }
    };
    section(S.sc_head_open, items.filter((item) => item.l <= 1), false);
    section(S.sc_head_hints, items.filter((item) => item.l === 2), false);
    section(S.sc_head_good, items.filter((item) => item.l === 3), true);
    box.appendChild(el('div', 'amnote', S.sc_note));
    box.scrollTop = scroll;
  }

  function openCheck() {
    if (!ui.check) return;
    // Die Seite kann sich seit dem Laden geändert haben: noch einmal ansehen.
    post({ c: 'checkRefresh' });
    fillCheck();
    const box = $('popup');
    const place = $('sec').getBoundingClientRect();
    box.style.top = Math.round(place.bottom + 8) + 'px';
    box.style.left = Math.max(8, Math.min(place.left - 6, window.innerWidth - 368)) + 'px';
    box.style.right = 'auto';
    box.scrollTop = 0;
    box.hidden = false;
    ui.menu = 'check';
    closePanel();
    closeTabSearch();
    syncOverlay();
  }

  // Das Schild in der Adresszeile: wie viel auf dieser Seite gesperrt wurde –
  // und der Schalter, falls eine Website mit dem Blocker nicht zurechtkommt.
  function renderShield(shield) {
    const info = shield || {};
    ui.shield = info;
    const button = $('shield');
    button.hidden = !(info.on && info.site);
    button.classList.toggle('off', !!info.off);
    setBlocked(info.count || 0);
    if (ui.menu === 'shield') {
      if (button.hidden) closeMenu();
      else fillShield();
    }
  }

  function blockedText(count) {
    const info = ui.shield || {};
    if (info.off) return S.sh_off_text;
    return count ? S.sh_count.replace('{n}', count.toLocaleString()) : S.sh_count_none;
  }

  function setBlocked(count) {
    ui.blocked = count;
    const info = ui.shield || {};
    $('shield-n').textContent = !info.off && count ? (count > 999 ? '999+' : String(count)) : '';
    $('shield').title = info.off ? S.sh_tip_off : count ? S.sh_tip.replace('{n}', count.toLocaleString()) : S.sh_tip_none;
    const line = document.getElementById('shield-count');
    if (line) line.textContent = blockedText(count);
  }

  function fillShield() {
    const info = ui.shield || {};
    const box = $('popup');
    box.textContent = '';
    const head = el('div', 'shd');
    head.appendChild(icon('shield'));
    const text = el('div', 'txt');
    text.appendChild(el('div', 't', info.off ? S.sh_off_title : S.sh_on_title));
    const count = el('div', 'h', blockedText(ui.blocked || 0));
    count.id = 'shield-count';
    text.appendChild(count);
    head.appendChild(text);
    box.appendChild(head);

    // Der Bericht: welche Domains auf dieser Seite wie oft gesperrt wurden.
    const hosts = !info.off && Array.isArray(info.hosts) ? info.hosts : [];
    if (hosts.length) {
      box.appendChild(el('div', 'shl-t', S.sh_report));
      const list = el('div', 'shl');
      for (const entry of hosts) {
        const line = el('div', 'shlr');
        line.appendChild(el('span', 'h', entry.host || ''));
        line.appendChild(el('span', 'n', (entry.n || 0).toLocaleString()));
        list.appendChild(line);
      }
      box.appendChild(list);
    }

    const row = el('div', 'shr');
    row.appendChild(el('span', 'l', S.sh_site.replace('{site}', info.site || '')));
    const toggle = el('button', 'switch' + (info.off ? '' : ' on'));
    toggle.type = 'button';
    toggle.addEventListener('click', () => {
      closeMenu();
      post({ c: 'blockerSite', off: !info.off, site: info.site || '' });
    });
    row.appendChild(toggle);
    box.appendChild(row);
    box.appendChild(el('div', 'shn', info.off ? S.sh_note_off : S.sh_note_on));
  }

  function openShield() {
    fillShield();
    const box = $('popup');
    const place = $('shield').getBoundingClientRect();
    box.style.top = Math.round(place.bottom + 8) + 'px';
    box.style.left = Math.max(8, Math.min(place.right - 320, window.innerWidth - 328)) + 'px';
    box.style.right = 'auto';
    box.hidden = false;
    ui.menu = 'shield';
    closePanel();
    closeTabSearch();
    syncOverlay();
  }

  // Wie viele gemeldete Adressen gerade gesperrt sind – als Zusatz zum Hinweis.
  function guardNote(info) {
    if (!info) return '';
    if (info.guardRules) return ' ' + S.s_guard_state.replace('{n}', info.guardRules.toLocaleString());
    return info.guardBusy ? ' ' + S.s_guard_loading : '';
  }

  // Einstellungen: Was der Werbeblocker gerade an Listen hat.
  function filterText(info) {
    if (!info || !info.rules) return info && info.busy ? S.s_filters_building : S.s_filters_none;
    const stamp = String(info.stamp || '');
    let date = '';
    if (/^[0-9]{8}/.test(stamp)) {
      date = new Date(+stamp.slice(0, 4), +stamp.slice(4, 6) - 1, +stamp.slice(6, 8)).toLocaleDateString();
    }
    let text = S.s_filters_state
      .replace('{n}', info.rules.toLocaleString())
      .replace('{lists}', (info.titles || []).join(', '))
      .replace('{date}', date);
    if (info.busy) text += ' · ' + S.s_filters_busy;
    return text;
  }

  function fillFilterLine() {
    const line = document.getElementById('filters-line');
    if (line) line.textContent = filterText(ui.filters);
    const button = document.getElementById('filters-refresh');
    if (button) button.disabled = !!(ui.filters && ui.filters.busy);
  }

  function filterRow(info) {
    ui.filters = info;
    const row = el('div', 'set');
    const text = el('div', 'txt');
    text.appendChild(el('div', '', S.s_filters));
    const line = el('div', 'h', filterText(info));
    line.id = 'filters-line';
    text.appendChild(line);
    row.appendChild(text);
    if (info && info.canRefresh) {
      const button = el('button', 'tbtn', S.s_filters_refresh);
      button.type = 'button';
      button.id = 'filters-refresh';
      button.disabled = !!info.busy;
      button.addEventListener('click', () => post({ c: 'filtersRefresh' }));
      row.appendChild(button);
    }
    return row;
  }

  // Menü einer Tab-Gruppe: Name, Farbe, auflösen, schließen.
  function openGroupMenu(id, x) {
    const group = groupOf(id);
    if (!group) return;
    const box = $('popup');
    box.textContent = '';
    const line = el('div', 'gname');
    const field = document.createElement('input');
    field.type = 'text';
    field.value = group.name;
    field.maxLength = 40;
    field.spellcheck = false;
    const rename = () => {
      const name = field.value.trim();
      if (name && name !== group.name) post({ c: 'groupName', id, name });
    };
    field.addEventListener('keydown', (event) => {
      if (event.key === 'Enter') {
        rename();
        closeMenu();
      }
      event.stopPropagation();
    });
    field.addEventListener('blur', rename);
    line.appendChild(field);
    box.appendChild(line);
    const colors = el('div', 'gcolors');
    for (let color = 0; color < 8; color++) {
      const dot = el('button', 'gc' + color + (color === group.color ? ' on' : ''));
      dot.type = 'button';
      dot.addEventListener('click', () => {
        post({ c: 'groupColor', id, color });
        closeMenu();
      });
      colors.appendChild(dot);
    }
    box.appendChild(colors);
    box.appendChild(el('div', 'msep'));
    box.appendChild(menuItem(group.collapsed ? S.tg_open : S.tg_close, 'chevron-down', null, () => post({ c: 'groupToggle', id })));
    box.appendChild(menuItem(S.tg_dissolve, 'minus', null, () => post({ c: 'groupDissolve', id })));
    box.appendChild(menuItem(S.tg_close_tabs, 'close', null, () => post({ c: 'groupClose', id })));
    box.style.top = reportedHeight - 38 + 'px';
    box.style.left = Math.max(8, Math.min(x - 20, window.innerWidth - 270)) + 'px';
    box.style.right = 'auto';
    box.hidden = false;
    ui.menu = 'group';
    closePanel();
    closeTabSearch();
    syncOverlay();
    setTimeout(() => field.select(), 30);
  }

  // Container verwalten: anlegen, umbenennen, umfärben, löschen – und einen
  // neuen Tab in einem Container öffnen. Jeder Container hat seinen eigenen,
  // dauerhaften Datenspeicher.
  function openContainers() {
    const box = $('popup');
    box.textContent = '';
    box.appendChild(el('div', 'cnh', S.cn_title));
    box.appendChild(el('div', 'cnhint', S.cn_hint));
    const list = el('div', 'cnlist');
    list.id = 'cn-list';
    box.appendChild(list);
    fillContainers();

    const add = el('div', 'cnadd');
    const field = document.createElement('input');
    field.type = 'text';
    field.className = 'cnname';
    field.placeholder = S.cn_add_name;
    field.maxLength = 40;
    field.spellcheck = false;
    const create = () => {
      const value = field.value.trim();
      if (value) {
        post({ c: 'containerNew', name: value });
        field.value = '';
      }
    };
    field.addEventListener('keydown', (event) => {
      if (event.key === 'Enter') create();
      event.stopPropagation();
    });
    add.appendChild(field);
    const addBtn = el('button', 'cnbtn');
    addBtn.type = 'button';
    addBtn.title = S.cn_add;
    addBtn.appendChild(icon('plus', 'sm'));
    addBtn.addEventListener('click', create);
    add.appendChild(addBtn);
    box.appendChild(add);

    const anchor = $('menu').getBoundingClientRect();
    box.style.top = anchor.bottom + 8 + 'px';
    box.style.right = Math.max(8, window.innerWidth - anchor.right) + 'px';
    box.style.left = 'auto';
    box.hidden = false;
    ui.menu = 'containers';
    closePanel();
    closeTabSearch();
    syncOverlay();
  }

  // Baut die Liste der Container neu auf – auch bei jeder Zustandsmeldung,
  // solange das Fenster offen ist und gerade kein Name bearbeitet wird.
  function fillContainers() {
    const list = document.getElementById('cn-list');
    if (!list) return;
    list.textContent = '';
    const containers = (state && state.containers) || [];
    if (!containers.length) {
      list.appendChild(el('div', 'cnempty', S.cn_empty));
      return;
    }
    for (const container of containers) {
      let color = container.color;
      const row = el('div', 'cnr cc' + color);
      const dot = el('button', 'cndot');
      dot.type = 'button';
      dot.title = S.cn_color;
      dot.addEventListener('click', (event) => {
        event.stopPropagation();
        color = (color + 1) % 8;
        row.className = 'cnr cc' + color;
        post({ c: 'containerColor', id: container.id, color });
      });
      row.appendChild(dot);
      const name = document.createElement('input');
      name.type = 'text';
      name.className = 'cnname';
      name.value = container.name;
      name.maxLength = 40;
      name.spellcheck = false;
      name.title = S.cn_rename;
      const rename = () => {
        const value = name.value.trim();
        if (value && value !== container.name) post({ c: 'containerRename', id: container.id, name: value });
      };
      name.addEventListener('keydown', (event) => {
        if (event.key === 'Enter') {
          rename();
          name.blur();
        }
        event.stopPropagation();
      });
      name.addEventListener('blur', rename);
      row.appendChild(name);
      const open = el('button', 'cnbtn');
      open.type = 'button';
      open.title = S.cn_new_tab;
      open.appendChild(icon('plus', 'sm'));
      open.addEventListener('click', () => {
        closeMenu();
        post({ c: 'newContainerTab', id: container.id });
      });
      row.appendChild(open);
      const del = el('button', 'cnbtn danger');
      del.type = 'button';
      del.title = S.cn_delete;
      del.appendChild(icon('trash', 'sm'));
      del.addEventListener('click', () => confirmContainerDelete(row, container.id));
      row.appendChild(del);
      list.appendChild(row);
    }
  }

  function confirmContainerDelete(row, id) {
    if (row.querySelector('.cnconfirm')) return;
    const bar = el('div', 'cnconfirm');
    bar.appendChild(el('span', 'q', S.cn_delete_ask));
    const yes = el('button', 'cnbtn danger', S.cn_delete_yes);
    yes.type = 'button';
    yes.addEventListener('click', () => {
      closeMenu();
      post({ c: 'containerDelete', id });
    });
    const no = el('button', 'cnbtn', S.cn_delete_no);
    no.type = 'button';
    no.addEventListener('click', () => bar.remove());
    bar.append(yes, no);
    row.appendChild(bar);
  }

  // ------------------------------------------------------------------------
  // Timer der Zeiterfassung – aus der Leiste steuerbar. Der Zustand kommt im
  // Zustandstakt (`state.timer.run`), geteilt mit der Zeit-App; Stoppen rechnet
  // den Eintrag hier aus (die Uhr-Mathe der App) und bucht ihn über den Browser.
  // ------------------------------------------------------------------------

  const TIMER_STEP_DEFAULT = 5;
  const TIMER_MAX_BREAKS = 12;
  const TIMER_CAP_DEFAULT = 10; // Stunden, bis die Uhr nachfragt – überschreibbar in den Einstellungen
  const TIMER_GRACE_MS = 5 * 60 * 1000; // so lange wartet die Nachfrage, dann stoppt und bucht sie
  let timerTick = 0;
  // Die Uhr hat ihre Schwelle erreicht und fragt: { deadline } – reagiert niemand
  // bis dahin, wird gestoppt und gebucht.
  let timerAsk = null;
  const pad2 = (n) => (n < 10 ? '0' : '') + n;

  function timerRun() {
    return ui.timer && ui.timer.run && typeof ui.timer.run === 'object' ? ui.timer.run : null;
  }

  // Nach wie vielen Millisekunden gearbeiteter Zeit die Uhr nachfragt. Die
  // Schwelle steht am Lauf (nach „Weiterlaufen“ höher), sonst gilt die Einstellung.
  function timerCapMs(run) {
    const hours = (run && run.cap > 0 ? run.cap : ui.timer && ui.timer.cap) || TIMER_CAP_DEFAULT;
    return hours * 3600000;
  }

  // Wacht über den Deckel: Ist er erreicht, wird gefragt; bleibt das ohne Antwort,
  // stoppt und bucht die Uhr von selbst. Läuft nichts (oder privat/getarnt),
  // verschwindet eine offene Frage.
  function timerGuard() {
    const dark = !state || state.private || state.stealth;
    const run = timerRun();
    if (dark || !run || run.pa > 0 || timerElapsed(run, Date.now()) < timerCapMs(run)) {
      if (timerAsk) {
        timerAsk = null;
        renderTimer();
      }
      return;
    }
    if (!timerAsk) {
      timerAsk = { deadline: Date.now() + TIMER_GRACE_MS };
      renderTimer();
      openTimer();
      return;
    }
    if (Date.now() >= timerAsk.deadline) {
      const step = (ui.timer && ui.timer.step) || TIMER_STEP_DEFAULT;
      const entries = timerStopEntries(run, step);
      timerAsk = null;
      post({ c: 'timerBook', entry: entries });
      closeMenu();
    }
  }

  function timerElapsed(run, now) {
    let ms = now - run.t0;
    for (const pair of run.b || []) ms -= Math.max(0, pair[1] - pair[0]);
    if (run.pa > 0) ms -= Math.max(0, now - run.pa);
    return Math.max(0, ms);
  }

  function timerShort(ms) {
    const total = Math.floor(ms / 60000);
    return Math.floor(total / 60) + ':' + pad2(total % 60);
  }

  function timerClock(ms) {
    const total = Math.floor(ms / 1000);
    return Math.floor(total / 3600) + ':' + pad2(Math.floor((total % 3600) / 60)) + ':' + pad2(total % 60);
  }

  function renderTimer() {
    const button = $('timer');
    if (!button) return;
    const hidden = !state || state.private || state.stealth;
    button.hidden = !!hidden;
    const run = timerRun();
    button.classList.toggle('running', !!run);
    button.classList.toggle('paused', !!(run && run.pa > 0));
    button.classList.toggle('asking', !!timerAsk);
    const label = $('timer-time');
    if (run) {
      label.textContent = timerShort(timerElapsed(run, Date.now()));
      button.title = S.ti_open_entry + ' · ' + timerClock(timerElapsed(run, Date.now()));
    } else {
      label.textContent = '';
      button.title = S.ti_open_idle;
    }
    clearInterval(timerTick);
    if (run && !(run.pa > 0) && !hidden) {
      timerTick = setInterval(() => {
        const current = timerRun();
        if (!current || current.pa > 0) {
          clearInterval(timerTick);
          return;
        }
        $('timer-time').textContent = timerShort(timerElapsed(current, Date.now()));
        if (ui.menu === 'timer') fillTimer();
        timerGuard();
      }, 1000);
    }
  }

  function timerTidy(start, end, breaks) {
    const list = breaks
      .map(([from, to]) => [Math.max(start, from), Math.min(end, to)])
      .filter(([from, to]) => to > from)
      .sort((a, b) => a[0] - b[0]);
    const merged = [];
    for (const [from, to] of list) {
      const last = merged[merged.length - 1];
      if (last && from <= last[1]) last[1] = Math.max(last[1], to);
      else merged.push([from, to]);
    }
    let s = start;
    let e = end;
    while (merged.length && merged[0][0] <= s) s = merged.shift()[1];
    while (merged.length && merged[merged.length - 1][1] >= e) e = merged.pop()[0];
    return { s, e, b: merged.slice(0, TIMER_MAX_BREAKS) };
  }

  // Aus der laufenden Uhr die Einträge rechnen – dieselbe Mathe wie die
  // Zeit-App: Minute des Tages, auf den Schritt gerundet, Pausen geordnet.
  // Läuft sie über Mitternacht, bekommt jeder Tag seinen Teil (ein Eintrag je
  // Tag). Zurück kommt eine Liste.
  function timerStopEntries(run, step) {
    const until = run.pa > 0 ? run.pa : Date.now();
    const keyOf = (date) => date.getFullYear() + '-' + pad2(date.getMonth() + 1) + '-' + pad2(date.getDate());
    const dateOf = (key) => {
      const parts = key.split('-').map(Number);
      return new Date(parts[0], parts[1] - 1, parts[2]);
    };
    const shiftDay = (key) => {
      const date = dateOf(key);
      date.setDate(date.getDate() + 1);
      return keyOf(date);
    };
    const minuteIn = (day, ms) => {
      const date = new Date(ms);
      const key = keyOf(date);
      if (key !== day) return key < day ? 0 : 1440;
      return date.getHours() * 60 + date.getMinutes();
    };
    const round = (minute) => Math.round(minute / step) * step;
    const entries = [];
    const lastDay = keyOf(new Date(until));
    let day = keyOf(new Date(run.t0));
    for (let guard = 0; guard < 8; guard++) {
      const last = day === lastDay;
      let begin = round(minuteIn(day, run.t0));
      let end = Math.min(1440, round(minuteIn(day, until)));
      const breaks = (run.b || []).map((pair) => [round(minuteIn(day, pair[0])), round(minuteIn(day, pair[1]))]);
      // Eine eben gestartete Uhr ergibt trotzdem einen Eintrag – einen Schritt lang.
      if (last && end <= begin && !breaks.length && !entries.length) {
        begin = Math.floor(minuteIn(day, run.t0) / step) * step;
        end = Math.min(1440, begin + step);
      }
      const shaped = timerTidy(begin, end, breaks);
      if (shaped.e > shaped.s) {
        const entry = { k: 'e', d: day, s: shaped.s, e: shaped.e, p: run.p || '', n: run.n || '' };
        if (shaped.b.length) entry.b = shaped.b;
        entries.push(entry);
      }
      if (last) break;
      day = shiftDay(day);
    }
    return entries;
  }

  function timerButton(cls, iconName, label, handler) {
    const button = el('button', 'tibtn' + (cls ? ' ' + cls : ''));
    button.type = 'button';
    button.appendChild(icon(iconName, 'sm'));
    button.appendChild(el('span', '', label));
    button.addEventListener('click', handler);
    return button;
  }

  // Die Blase gibt es nur noch für die Deckel-Frage: Weiterlaufen oder jetzt
  // stoppen und buchen. Steuern lässt sich die Uhr sonst in der App.
  function fillTimer() {
    const box = $('popup');
    box.textContent = '';
    const run = timerRun();
    if (!run || !timerAsk) {
      closeMenu();
      return;
    }
    const head = el('div', 'tih');
    head.appendChild(icon('timer'));
    const text = el('div', 'txt');
    text.appendChild(el('div', 't', S.ti_cap_title));
    const line = el('div', 'h');
    const left = Math.max(0, timerAsk.deadline - Date.now());
    line.textContent =
      timerClock(timerElapsed(run, Date.now())) + ' · ' + S.ti_cap_in + ' ' + Math.floor(left / 60000) + ':' + pad2(Math.floor((left % 60000) / 1000));
    text.appendChild(line);
    head.appendChild(text);
    box.appendChild(head);

    const actions = el('div', 'tiactions');
    actions.appendChild(
      timerButton('primary', 'play', S.ti_cap_keep, () => {
        const capH = (ui.timer && ui.timer.cap) || TIMER_CAP_DEFAULT;
        run.cap = Math.floor(timerElapsed(run, Date.now()) / 3600000) + capH;
        timerAsk = null;
        post({ c: 'timerSnooze' });
        renderTimer();
        closeMenu();
      })
    );
    actions.appendChild(
      timerButton('', 'stop', S.ti_stop, () => {
        const step = (ui.timer && ui.timer.step) || TIMER_STEP_DEFAULT;
        const entries = timerStopEntries(run, step);
        timerAsk = null;
        post({ c: 'timerBook', entry: entries });
        closeMenu();
      })
    );
    box.appendChild(actions);
    box.appendChild(el('div', 'tin', S.ti_cap_note));
  }

  function openTimer() {
    fillTimer();
    const box = $('popup');
    const place = $('timer').getBoundingClientRect();
    box.style.top = Math.round(place.bottom + 8) + 'px';
    box.style.left = Math.max(8, Math.min(place.left - 30, window.innerWidth - 300)) + 'px';
    box.style.right = 'auto';
    box.hidden = false;
    ui.menu = 'timer';
    closePanel();
    closeTabSearch();
    syncOverlay();
  }

  function closeMenu() {
    if (!ui.menu) return;
    ui.menu = null;
    $('popup').hidden = true;
    $('popup').classList.remove('ampelbox');
    syncOverlay();
  }

  // ------------------------------------------------------------------------
  // Seitenleisten
  // ------------------------------------------------------------------------

  const PANEL_TITLE = {
    history: S.p_history,
    bookmarks: S.p_bookmarks,
    downloads: S.p_downloads,
    settings: S.p_settings,
    passwords: S.p_passwords,
    sitedata: S.p_sitedata,
    rights: S.p_rights,
  };

  function togglePanel(name) {
    if (ui.panel === name) closePanel();
    else openPanel(name);
  }

  function openPanel(name, options) {
    closeMenu();
    closeTabSearch();
    if (ui.panel === 'passwords' && name !== 'passwords') lockPasswords();
    ui.panel = name;
    ui.panelQuery = '';
    ui.lastPanel = null;
    ui.panelOptions = options || {};
    $('panel-title').textContent = PANEL_TITLE[name] || '';
    const searchable =
      name === 'history' || name === 'bookmarks' || name === 'passwords' || name === 'sitedata' || name === 'rights';
    $('panel-search').hidden = !searchable;
    const query = $('panel-query');
    query.value = '';
    query.placeholder =
      name === 'history'
        ? S.p_search_history
        : name === 'passwords'
          ? S.p_search_passwords
          : name === 'sitedata' || name === 'rights'
            ? S.p_search_sitedata
            : S.p_search_bookmarks;
    const extra = $('panel-extra');
    extra.hidden = name !== 'downloads';
    extra.textContent = S.p_clear_downloads;
    $('panel-body').textContent = '';
    $('panel').hidden = false;
    syncOverlay();
    post({ c: 'panel', name, q: '' });
    if (searchable) setTimeout(() => query.focus(), 0);
    clearInterval(ui.pollTimer);
    if (name === 'downloads') {
      ui.pollTimer = setInterval(() => {
        if (ui.panel === 'downloads' && state && state.downloads.active > 0) post({ c: 'panel', name, q: '' });
      }, 1000);
    }
  }

  function closePanel() {
    if (!ui.panel) return;
    if (ui.panel === 'passwords') lockPasswords();
    ui.panel = null;
    ui.lastPanel = null;
    clearInterval(ui.pollTimer);
    $('panel').hidden = true;
    syncOverlay();
  }

  $('panel-close').addEventListener('click', () => {
    closePanel();
    post({ c: 'focusContent' });
  });
  $('panel-extra').addEventListener('click', () => post({ c: 'downloadsClear' }));
  $('panel-query').addEventListener('input', () => {
    ui.panelQuery = $('panel-query').value;
    post({ c: 'panel', name: ui.panel, q: ui.panelQuery });
  });

  function dayLabel(timestamp) {
    const date = new Date(timestamp * 1000);
    const today = new Date();
    const days = Math.round(
      (new Date(today.getFullYear(), today.getMonth(), today.getDate()) -
        new Date(date.getFullYear(), date.getMonth(), date.getDate())) /
        86400000
    );
    if (days === 0) return S.day_today;
    if (days === 1) return S.day_yesterday;
    return date.toLocaleDateString(undefined, { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' });
  }

  function timeLabel(timestamp) {
    return new Date(timestamp * 1000).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });
  }

  function sizeLabel(bytes) {
    if (!bytes) return '';
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    let value = bytes;
    let unit = 0;
    while (value >= 1000 && unit < units.length - 1) {
      value /= 1000;
      unit += 1;
    }
    return (unit === 0 ? value : value.toFixed(value < 10 ? 1 : 0)).toString().replace('.', ',') + ' ' + units[unit];
  }

  function actionButton(iconName, title, handler) {
    const button = el('button', 'ibtn sm act');
    button.type = 'button';
    button.title = title;
    button.appendChild(icon(iconName, 'sm'));
    button.addEventListener('click', (event) => {
      event.stopPropagation();
      handler();
    });
    return button;
  }

  function pageRow(item, fallbackIcon, onDelete, showTime) {
    const row = el('div', 'row');
    row.appendChild(favicon(item.icon, fallbackIcon));
    const text = el('div', 'txt');
    text.appendChild(el('div', 't', item.title || prettyUrl(item.url)));
    text.appendChild(el('div', 's', prettyUrl(item.url)));
    row.appendChild(text);
    if (showTime) row.appendChild(el('span', 'time', timeLabel(item.ts)));
    row.appendChild(actionButton('trash', S.delete, onDelete));
    row.addEventListener('click', (event) => {
      closePanel();
      post({ c: 'open', url: item.url, newTab: event.metaKey });
    });
    return row;
  }

  function renderPanel(message) {
    if (message.name !== ui.panel) return;
    if ((message.q || '') !== (ui.panelQuery || '')) return;
    ui.lastPanel = message;
    const body = $('panel-body');
    const scroll = body.scrollTop;
    body.textContent = '';
    const data = message.data || {};
    const items = data.items || [];

    if (message.name === 'sitedata') {
      // Was Websites auf diesem Mac abgelegt haben – einzeln oder ganz zu löschen.
      body.appendChild(el('div', 'setnote', S.sd_note));
      if (!items.length) body.appendChild(el('div', 'empty', ui.panelQuery ? S.p_no_results : S.sd_empty));
      const KIND = { cookies: S.sd_cookies, storage: S.sd_storage, cache: S.sd_cache };
      for (const item of items) {
        const row = el('div', 'row');
        row.appendChild(icon('globe'));
        const text = el('div', 'txt');
        text.appendChild(el('div', 't', item.site));
        text.appendChild(el('div', 's', (item.kinds || []).map((kind) => KIND[kind] || kind).join(' · ')));
        row.appendChild(text);
        row.appendChild(
          actionButton('trash', S.sd_delete, () => post({ c: 'siteDataDelete', site: item.site, q: ui.panelQuery || '' }))
        );
        body.appendChild(row);
      }
      if (items.length && !ui.panelQuery) {
        const tools = el('div', 'setbtns');
        tools.appendChild(textButton(S.sd_delete_all.replace('{n}', String(data.total || items.length)), 'danger', () => post({ c: 'siteDataDelete', site: '', q: '' })));
        body.appendChild(tools);
      }
    } else if (message.name === 'rights') {
      // Welche Website was darf – jede Entscheidung einzeln zurückzunehmen.
      body.appendChild(el('div', 'setnote', S.rt_note));
      if (!items.length) body.appendChild(el('div', 'empty', ui.panelQuery ? S.p_no_results : S.rt_empty));
      const RIGHT = {
        camera: ['camera', S.rt_camera_yes, S.rt_camera_no],
        mic: ['microphone', S.rt_mic_yes, S.rt_mic_no],
        localnet: ['globe', S.rt_localnet, S.rt_localnet],
        blocker: ['shield', S.rt_blocker, S.rt_blocker],
      };
      for (const item of items) {
        const row = el('div', 'row rights');
        row.appendChild(icon('globe'));
        const text = el('div', 'txt');
        text.appendChild(el('div', 't', item.site));
        const chips = el('div', 'rchips');
        for (const right of item.rights || []) {
          const known = RIGHT[right.kind];
          if (!known) continue;
          const chip = el('span', 'chip' + (right.allowed ? '' : ' no'));
          chip.appendChild(icon(known[0], 'sm'));
          chip.appendChild(el('span', '', right.allowed ? known[1] : known[2]));
          const remove = el('button', '');
          remove.type = 'button';
          remove.title = S.rt_remove;
          remove.textContent = '×';
          remove.addEventListener('click', () =>
            post({ c: 'rightRemove', site: item.site, kind: right.kind, q: ui.panelQuery || '' })
          );
          chip.appendChild(remove);
          chips.appendChild(chip);
        }
        text.appendChild(chips);
        row.appendChild(text);
        body.appendChild(row);
      }
    } else if (message.name === 'history') {
      if (!items.length) body.appendChild(el('div', 'empty', ui.panelQuery ? S.p_no_results : S.p_empty_history));
      let lastDay = null;
      for (const item of items) {
        const day = dayLabel(item.ts);
        if (day !== lastDay) {
          body.appendChild(el('div', 'day', day));
          lastDay = day;
        }
        body.appendChild(pageRow(item, 'globe', () => post({ c: 'historyDelete', id: item.id, q: ui.panelQuery }), true));
      }
    } else if (message.name === 'bookmarks') {
      renderBookmarks(body, data, items);
      if (!ui.panelQuery) {
        // Ordnen, von einem anderen Browser herüberholen – oder als Datei mitnehmen.
        const tools = el('div', 'setbtns');
        tools.appendChild(
          textButton(S.bm_new_folder, '', () => {
            bm.adding = true;
            rerenderPanel();
            $('panel-body').scrollTop = 0;
          })
        );
        tools.appendChild(textButton(S.bm_import, '', () => post({ c: 'bookmarksImport' })));
        if (items.length) tools.appendChild(textButton(S.bm_export, '', () => post({ c: 'bookmarksExport' })));
        for (const source of data.sources || []) {
          tools.appendChild(
            textButton(S.bm_from.replace('{name}', source.name), '', () => post({ c: 'bookmarksFrom', browser: source.id }))
          );
        }
        body.appendChild(tools);
      }
    } else if (message.name === 'passwords') {
      renderPasswords(body, data);
    } else if (message.name === 'downloads') {
      if (!items.length) body.appendChild(el('div', 'empty', S.p_empty_downloads));
      for (const item of items) {
        const row = el('div', 'row' + (item.state === 'done' && !item.exists ? ' gone' : ''));
        const fav = el('span', 'fav');
        if (item.state === 'active') fav.appendChild(el('span', 'spinner'));
        else fav.appendChild(icon(item.state === 'failed' ? 'warning' : 'file', 'sm'));
        row.appendChild(fav);
        const text = el('div', 'txt');
        text.appendChild(el('div', 't', item.name));
        let status = sizeLabel(item.bytes);
        if (item.state === 'active') status = S.d_active + (status ? ' ' + status : '');
        if (item.state === 'failed') status = S.d_failed;
        const sub = el('div', 's' + (item.state === 'failed' ? ' bad' : ''), status || prettyUrl(item.url));
        text.appendChild(sub);
        row.appendChild(text);
        if (item.exists) {
          row.appendChild(actionButton('folder', S.d_show, () => post({ c: 'downloadReveal', id: item.id })));
          if (item.state === 'done') row.addEventListener('click', () => post({ c: 'downloadOpen', id: item.id }));
        }
        body.appendChild(row);
      }
    } else if (message.name === 'settings') {
      renderSettings(body, data);
    }
    body.scrollTop = scroll;
  }

  // ------------------------------------------------------------------------
  // Lesezeichen: Ordner, Bearbeiten, Sortieren durch Ziehen
  // ------------------------------------------------------------------------

  const bm = { editing: null, renaming: null, adding: false, closed: new Set(), drag: null };

  function rerenderPanel() {
    if (ui.lastPanel) renderPanel(ui.lastPanel);
  }

  function renderBookmarks(body, data, items) {
    const folders = data.folders || [];
    const searching = !!ui.panelQuery;
    const send = (message) => post(Object.assign({ q: ui.panelQuery || '' }, message));
    if (!items.length && !folders.length) {
      body.appendChild(el('div', 'empty', searching ? S.p_no_results : S.p_empty_bookmarks));
    }

    // Ziel beim Ziehen: vor ein anderes Lesezeichen oder in einen Ordner.
    const dropTarget = (node, folder, before) => {
      node.addEventListener('dragover', (event) => {
        if (bm.drag == null || bm.drag === before) return;
        event.preventDefault();
        event.dataTransfer.dropEffect = 'move';
        node.classList.add('drop');
      });
      node.addEventListener('dragleave', () => node.classList.remove('drop'));
      node.addEventListener('drop', (event) => {
        event.preventDefault();
        node.classList.remove('drop');
        if (bm.drag == null) return;
        send({ c: 'bookmarkMove', id: bm.drag, folder, before: before || 0 });
        bm.drag = null;
      });
    };

    const editRow = (item) => {
      const box = el('div', 'bmedit');
      const title = field(S.bm_title, 'text', item.title, '');
      const address = field(S.bm_address, 'text', item.url, '');
      const where = el('label', 'pwfield');
      where.appendChild(el('span', '', S.bm_folder));
      const select = document.createElement('select');
      for (const name of [''].concat(folders)) {
        const option = document.createElement('option');
        option.value = name;
        option.textContent = name || S.bm_no_folder;
        option.selected = name === (item.folder || '');
        select.appendChild(option);
      }
      where.appendChild(select);
      const save = () => {
        bm.editing = null;
        send({ c: 'bookmarkEdit', id: item.id, title: title.input.value, url: address.input.value, folder: select.value });
      };
      const buttons = el('div', 'btns');
      buttons.appendChild(textButton(S.bm_save, 'primary', save));
      buttons.appendChild(
        textButton(S.cancel, '', () => {
          bm.editing = null;
          rerenderPanel();
        })
      );
      for (const input of [title.input, address.input]) {
        input.addEventListener('keydown', (event) => {
          if (event.key === 'Enter') save();
          if (event.key === 'Escape') {
            event.stopPropagation();
            bm.editing = null;
            rerenderPanel();
          }
        });
      }
      box.append(title.box, address.box, where, buttons);
      setTimeout(() => title.input.focus(), 0);
      return box;
    };

    const bookmarkRow = (item) => {
      if (bm.editing === item.id) return editRow(item);
      const row = pageRow(item, 'star', () => send({ c: 'bookmarkDelete', id: item.id }), false);
      row.insertBefore(
        actionButton('edit', S.bm_edit, () => {
          bm.editing = item.id;
          rerenderPanel();
        }),
        row.lastChild
      );
      if (!searching) {
        row.draggable = true;
        row.addEventListener('dragstart', (event) => {
          bm.drag = item.id;
          event.dataTransfer.effectAllowed = 'move';
          event.dataTransfer.setData('text/plain', item.url);
        });
        row.addEventListener('dragend', () => {
          bm.drag = null;
        });
        dropTarget(row, item.folder || '', item.id);
      }
      return row;
    };

    if (searching) {
      for (const item of items) body.appendChild(bookmarkRow(item));
      return;
    }

    // Neuer Ordner: ein Eingabefeld ganz oben.
    if (bm.adding) {
      const add = field(S.bm_new_folder, 'text', '', S.bm_folder_name);
      add.box.classList.add('bmedit');
      add.input.addEventListener('keydown', (event) => {
        if (event.key === 'Enter' && add.input.value.trim()) {
          bm.adding = false;
          send({ c: 'folderAdd', name: add.input.value });
        }
        if (event.key === 'Escape') {
          event.stopPropagation();
          bm.adding = false;
          rerenderPanel();
        }
      });
      body.appendChild(add.box);
      setTimeout(() => add.input.focus(), 0);
    }

    const loose = items.filter((item) => !item.folder);
    if (folders.length && loose.length) {
      const head = el('div', 'bmhead plain');
      head.appendChild(el('span', 'n', S.bm_no_folder));
      dropTarget(head, '', 0);
      body.appendChild(head);
    }
    for (const item of loose) body.appendChild(bookmarkRow(item));

    for (const name of folders) {
      const inside = items.filter((item) => item.folder === name);
      const closed = bm.closed.has(name);
      const head = el('div', 'bmhead' + (closed ? ' closed' : ''));
      head.appendChild(icon('chevron-down', 'sm'));
      head.appendChild(icon('folder', 'sm'));
      if (bm.renaming === name) {
        const input = document.createElement('input');
        input.type = 'text';
        input.value = name;
        input.spellcheck = false;
        input.addEventListener('click', (event) => event.stopPropagation());
        input.addEventListener('keydown', (event) => {
          if (event.key === 'Enter') {
            bm.renaming = null;
            send({ c: 'folderRename', old: name, name: input.value });
          }
          if (event.key === 'Escape') {
            event.stopPropagation();
            bm.renaming = null;
            rerenderPanel();
          }
        });
        head.appendChild(input);
        setTimeout(() => {
          input.focus();
          input.select();
        }, 0);
      } else {
        head.appendChild(el('span', 'n', name));
        head.appendChild(el('span', 'c', String(inside.length)));
        head.appendChild(
          actionButton('edit', S.bm_rename, () => {
            bm.renaming = name;
            rerenderPanel();
          })
        );
        head.appendChild(actionButton('trash', S.bm_folder_delete, () => send({ c: 'folderDelete', name })));
      }
      head.addEventListener('click', () => {
        if (bm.renaming === name) return;
        if (closed) bm.closed.delete(name);
        else bm.closed.add(name);
        rerenderPanel();
      });
      dropTarget(head, name, 0);
      body.appendChild(head);
      if (closed) continue;
      const group = el('div', 'bmgroup');
      if (!inside.length) group.appendChild(el('div', 'bmempty', S.bm_folder_empty));
      for (const item of inside) group.appendChild(bookmarkRow(item));
      dropTarget(group, name, 0);
      body.appendChild(group);
    }
  }

  // Zwei Tabs nebeneinander: Jede Hälfte zeigt ihre eigene Adresse – der Teil,
  // der sagt, wem die Seite gehört, steht auch hier kräftig da. Ein Klick macht
  // die Hälfte zur aktiven.
  function renderSplitBar(next) {
    const bar = $('splitbar');
    const panes = next.panes || [];
    const shown = panes.length === 2;
    if (bar.hidden === shown) {
      bar.hidden = !shown;
      setTimeout(reportHeight, 0);
    }
    bar.textContent = '';
    if (!shown) return;
    bar.style.paddingRight = (next.assistantWidth || 0) + 'px';
    for (const pane of panes) {
      const box = el('div', 'pane' + (pane.id === next.active ? ' on' : ''));
      const state = el('span', pane.security);
      state.appendChild(icon(SECURITY_ICON[pane.security] || 'search', 'sm'));
      box.appendChild(state);
      const text = el('span', 'txt');
      if (pane.parts) {
        text.appendChild(document.createTextNode(pane.parts[0]));
        text.appendChild(el('span', 'dom', pane.parts[1]));
        text.appendChild(document.createTextNode(pane.parts[2]));
      } else {
        text.textContent = pane.pretty || '';
      }
      box.appendChild(text);
      box.title = pane.pretty || '';
      box.addEventListener('click', () => post({ c: 'activateTab', id: pane.id }));
      bar.appendChild(box);
    }
  }

  // Die Lesezeichenleiste unter der Adresszeile: Ordner zuerst, dann die
  // Lesezeichen ohne Ordner.
  function barButton(label, node, title) {
    const button = el('button', 'bmi');
    button.type = 'button';
    button.title = title || label;
    button.appendChild(node);
    button.appendChild(el('span', '', label));
    return button;
  }

  function openBarFolder(folder, anchor) {
    const box = $('popup');
    box.textContent = '';
    if (!(folder.items || []).length) box.appendChild(el('div', 'mi plain', S.bm_folder_empty));
    for (const item of folder.items || []) {
      const row = el('div', 'mi');
      row.appendChild(favicon(item.icon, 'globe'));
      row.appendChild(el('span', 'l', item.title || prettyUrl(item.url)));
      row.title = item.url;
      row.addEventListener('click', (event) => {
        closeMenu();
        post({ c: 'open', url: item.url, newTab: event.metaKey });
      });
      box.appendChild(row);
    }
    const place = anchor.getBoundingClientRect();
    box.style.top = Math.round(place.bottom + 4) + 'px';
    box.style.left = Math.max(8, Math.min(place.left, window.innerWidth - 290)) + 'px';
    box.style.right = 'auto';
    box.hidden = false;
    ui.menu = 'bar';
    closePanel();
    closeTabSearch();
    syncOverlay();
  }

  function renderBar() {
    const bar = $('bmbar');
    const data = ui.bar || {};
    bar.textContent = '';
    bar.hidden = !data.on;
    if (data.on) {
      for (const folder of data.folders || []) {
        const button = barButton(folder.name, icon('folder', 'sm'));
        button.addEventListener('click', (event) => {
          event.stopPropagation();
          openBarFolder(folder, button);
        });
        bar.appendChild(button);
      }
      for (const item of data.items || []) {
        const button = barButton(item.title || prettyUrl(item.url), favicon(item.icon, 'globe'), item.url);
        button.addEventListener('click', (event) => post({ c: 'open', url: item.url, newTab: event.metaKey }));
        bar.appendChild(button);
      }
      if (!bar.childNodes.length) bar.appendChild(el('span', 'hint', S.bm_bar_empty));
    }
    reportHeight();
  }

  function switchRow(label, hint, key, value) {
    const row = el('div', 'set');
    const text = el('div', 'txt');
    text.appendChild(el('div', '', label));
    if (hint) text.appendChild(el('div', 'h', hint));
    row.appendChild(text);
    const toggle = el('button', 'switch' + (value ? ' on' : ''));
    toggle.type = 'button';
    toggle.addEventListener('click', () => post({ c: 'setting', key, value: !value }));
    row.appendChild(toggle);
    return row;
  }

  function selectRow(label, hint, key, entries, selected) {
    const row = el('div', 'set');
    const text = el('div', 'txt');
    text.appendChild(el('div', '', label));
    if (hint) text.appendChild(el('div', 'h', hint));
    row.appendChild(text);
    const select = document.createElement('select');
    for (const entry of entries) {
      const option = document.createElement('option');
      option.value = entry.id;
      option.textContent = entry.name;
      option.selected = entry.id === selected;
      select.appendChild(option);
    }
    select.addEventListener('change', () => post({ c: 'setting', key, value: select.value }));
    row.appendChild(select);
    return row;
  }

  function renderSettings(body, data) {
    body.appendChild(el('div', 'sethead', S.s_head_general));
    body.appendChild(selectRow(S.s_search_engine, null, 'engine', data.engines || [], data.engine));
    body.appendChild(switchRow(S.s_suggestions, S.s_suggestions_hint, 'suggest', data.suggest));
    body.appendChild(switchRow(S.s_saver, S.s_saver_hint, 'saver', data.saver));
    body.appendChild(switchRow(S.s_restore, null, 'restore', data.restore));
    body.appendChild(switchRow(S.s_splash, S.s_splash_hint, 'splash', data.splash));

    // Sicherheit: wenige Schalter, die wirklich etwas bewirken.
    body.appendChild(el('div', 'sethead', S.s_head_security));
    body.appendChild(switchRow(S.s_https, S.s_https_hint, 'https_only', data.httpsOnly));
    body.appendChild(switchRow(S.s_blocker, S.s_blocker_hint, 'blocker', data.blocker));
    if (data.blocker) {
      body.appendChild(filterRow(data.filters || {}));
      body.appendChild(switchRow(S.s_filter_update, S.s_filter_update_hint, 'filter_update', data.filterUpdate));
      if ((data.blockerOff || []).length) {
        // Websites, für die der Blocker abgeschaltet wurde – einzeln wieder einschaltbar.
        const sites = el('div', 'sites');
        sites.appendChild(el('div', 'lbl', S.s_blocker_off_sites));
        for (const site of data.blockerOff) {
          const chip = el('span', 'chip', site);
          const remove = el('button', '');
          remove.type = 'button';
          remove.title = S.s_blocker_off_remove;
          remove.textContent = '×';
          remove.addEventListener('click', () => post({ c: 'blockerSiteRemove', site }));
          chip.appendChild(remove);
          sites.appendChild(chip);
        }
        body.appendChild(sites);
      }
    }
    body.appendChild(switchRow(S.s_guard, S.s_guard_hint + guardNote(data.filters), 'guard', data.guard));
    body.appendChild(switchRow(S.s_tracking, S.s_tracking_hint, 'strip_tracking', data.stripTracking));
    body.appendChild(switchRow(S.s_lookalike, S.s_lookalike_hint, 'lookalike', data.lookalike));
    body.appendChild(switchRow(S.s_local_net, S.s_local_net_hint, 'local_net', data.localNet));
    if ((data.localSites || []).length) {
      // Websites, denen der Zugriff ins eigene Netz erlaubt wurde – einzeln widerrufbar.
      const sites = el('div', 'sites');
      sites.appendChild(el('div', 'lbl', S.s_local_sites));
      for (const site of data.localSites) {
        const chip = el('span', 'chip', site);
        const remove = el('button', '');
        remove.type = 'button';
        remove.title = S.s_local_remove;
        remove.textContent = '×';
        remove.addEventListener('click', () => post({ c: 'localSiteRemove', site }));
        chip.appendChild(remove);
        sites.appendChild(chip);
      }
      body.appendChild(sites);
    }
    body.appendChild(switchRow(S.s_hardened, S.s_hardened_hint, 'hardened', data.hardened));
    body.appendChild(switchRow(S.s_devtools, S.s_devtools_hint, 'devtools', data.devtools));
    body.appendChild(switchRow(S.s_clear_on_quit, S.s_clear_on_quit_hint, 'clear_on_quit', data.clearOnQuit));
    {
      // Cookies und andere Website-Daten je Website ansehen und löschen.
      const row = el('div', 'set');
      const text = el('div', 'txt');
      text.appendChild(el('div', '', S.s_sitedata));
      text.appendChild(el('div', 'h', S.s_sitedata_hint));
      row.appendChild(text);
      row.appendChild(textButton(S.s_sitedata_open, '', () => openPanel('sitedata')));
      body.appendChild(row);
    }
    {
      // Was Websites dürfen: Kamera, Mikrofon, eigenes Netz, Werbeblocker aus.
      const count = data.rightsCount || 0;
      const state = !count ? S.s_rights_none : count === 1 ? S.s_rights_one : S.s_rights_count.replace('{n}', String(count));
      const row = el('div', 'set');
      const text = el('div', 'txt');
      text.appendChild(el('div', '', S.s_rights));
      text.appendChild(el('div', 'h', S.s_rights_hint + ' ' + state));
      row.appendChild(text);
      row.appendChild(textButton(S.s_sitedata_open, '', () => openPanel('rights')));
      body.appendChild(row);
    }
    body.appendChild(el('div', 'setnote', S.s_always));

    body.appendChild(el('div', 'sethead', S.s_head_start));
    body.appendChild(switchRow(S.s_start_bookmarks, S.s_start_bookmarks_hint, 'start_bookmarks', data.startBookmarks));
    if ((data.wallpapers || []).length) {
      const entries = [{ id: '', name: S.wp_none }].concat(data.wallpapers);
      body.appendChild(selectRow(S.s_wallpaper, S.s_wallpaper_hint, 'wallpaper', entries, data.wallpaper || ''));
    }

    const buttons = el('div', 'setbtns');
    const makeDefault = el('button', 'tbtn', S.s_default_browser);
    makeDefault.type = 'button';
    makeDefault.addEventListener('click', () => post({ c: 'defaultBrowser' }));
    const clear = el('button', 'tbtn', S.s_clear);
    clear.type = 'button';
    buttons.append(makeDefault, clear);
    body.appendChild(buttons);

    const confirm = el('div', 'confirm');
    confirm.hidden = !ui.panelOptions.confirmClear;
    confirm.appendChild(el('div', '', S.s_clear_confirm));
    const confirmButtons = el('div', 'btns');
    const yes = el('button', 'tbtn danger', S.delete);
    yes.type = 'button';
    yes.addEventListener('click', () => {
      ui.panelOptions.confirmClear = false;
      confirm.hidden = true;
      post({ c: 'clearData' });
    });
    const no = el('button', 'tbtn', S.cancel);
    no.type = 'button';
    no.addEventListener('click', () => {
      ui.panelOptions.confirmClear = false;
      confirm.hidden = true;
    });
    confirmButtons.append(yes, no);
    confirm.appendChild(confirmButtons);
    body.appendChild(confirm);
    clear.addEventListener('click', () => {
      ui.panelOptions.confirmClear = true;
      confirm.hidden = false;
    });

    body.appendChild(el('div', 'about', S.s_version.replace('{v}', DATA.version)));
  }

  // ------------------------------------------------------------------------
  // Passwörter
  // ------------------------------------------------------------------------
  //
  // Die Liste zeigt Website und Benutzername. Ein Passwort kommt erst hierher,
  // wenn es ausdrücklich angezeigt werden soll (nach Touch ID), und verschwindet
  // nach kurzer Zeit oder mit dem Schließen der Seitenleiste wieder.

  const pw = { open: null, editing: null, confirm: null, adding: false, secrets: new Map(), timers: new Map() };

  function hideSecret(id) {
    clearTimeout(pw.timers.get(id));
    pw.timers.delete(id);
    pw.secrets.delete(id);
  }

  function lockPasswords() {
    for (const id of Array.from(pw.secrets.keys())) hideSecret(id);
    pw.open = null;
    pw.editing = null;
    pw.confirm = null;
    pw.adding = false;
    post({ c: 'pwLock' });
  }

  function rerenderPanel() {
    if (ui.lastPanel) renderPanel(ui.lastPanel);
  }

  function onSecret(message) {
    if (ui.panel !== 'passwords' || message.password == null) return;
    pw.secrets.set(message.id, String(message.password));
    clearTimeout(pw.timers.get(message.id));
    pw.timers.set(
      message.id,
      setTimeout(() => {
        hideSecret(message.id);
        rerenderPanel();
      }, 45000)
    );
    pw.open = message.id;
    rerenderPanel();
  }

  function textButton(label, className, handler) {
    const button = el('button', 'tbtn' + (className ? ' ' + className : ''), label);
    button.type = 'button';
    button.addEventListener('click', (event) => {
      event.stopPropagation();
      handler();
    });
    return button;
  }

  function field(label, type, value, placeholder) {
    const box = el('label', 'pwfield');
    box.appendChild(el('span', '', label));
    const input = document.createElement('input');
    input.type = type;
    input.value = value || '';
    if (placeholder) input.placeholder = placeholder;
    input.spellcheck = false;
    input.autocomplete = type === 'password' ? 'new-password' : 'off';
    input.setAttribute('autocorrect', 'off');
    input.setAttribute('autocapitalize', 'off');
    box.appendChild(input);
    return { box, input };
  }

  function detailLine(label, value, code) {
    const line = el('div', 'pwline');
    line.appendChild(el('span', 'k', label));
    line.appendChild(el('span', 'v' + (code ? ' code' : ''), value));
    return line;
  }

  function passwordRow(item) {
    const open = pw.open === item.id;
    const wrap = el('div', 'pwitem' + (open ? ' open' : ''));
    const row = el('div', 'row');
    row.appendChild(favicon(item.icon, 'key'));
    const text = el('div', 'txt');
    text.appendChild(el('div', 't', item.host));
    text.appendChild(el('div', 's', item.username || S.pw_no_user));
    row.appendChild(text);
    row.addEventListener('click', () => {
      pw.open = open ? null : item.id;
      pw.editing = null;
      pw.confirm = null;
      rerenderPanel();
    });
    wrap.appendChild(row);
    if (!open) return wrap;

    const detail = el('div', 'pwdetail');
    if (pw.editing === item.id) {
      const user = field(S.pw_user, 'text', item.username);
      const pass = field(S.pw_password, 'password', '', S.pw_keep_hint);
      detail.append(user.box, pass.box);
      const buttons = el('div', 'btns');
      buttons.append(
        textButton(S.pw_save, 'primary', () => {
          pw.editing = null;
          hideSecret(item.id);
          post({ c: 'pwEdit', id: item.id, username: user.input.value, password: pass.input.value, q: ui.panelQuery });
        }),
        textButton(S.cancel, '', () => {
          pw.editing = null;
          rerenderPanel();
        })
      );
      detail.appendChild(buttons);
      wrap.appendChild(detail);
      return wrap;
    }

    const userLine = detailLine(S.pw_user, item.username || S.pw_no_user, false);
    if (item.username) {
      userLine.appendChild(actionButton('copy', S.pw_copy_user, () => post({ c: 'pwCopy', id: item.id, what: 'username' })));
    }
    detail.appendChild(userLine);

    const shown = pw.secrets.has(item.id);
    const passLine = detailLine(S.pw_password, shown ? pw.secrets.get(item.id) : '••••••••••', shown);
    passLine.appendChild(
      actionButton(shown ? 'eye-off' : 'eye', shown ? S.pw_hide : S.pw_show, () => {
        if (shown) {
          hideSecret(item.id);
          rerenderPanel();
        } else {
          post({ c: 'pwReveal', id: item.id });
        }
      })
    );
    passLine.appendChild(actionButton('copy', S.pw_copy_pass, () => post({ c: 'pwCopy', id: item.id, what: 'password' })));
    detail.appendChild(passLine);

    const siteLine = detailLine(S.pw_site, item.origin, false);
    siteLine.appendChild(
      actionButton('external', S.open, () => {
        closePanel();
        post({ c: 'open', url: item.origin, newTab: true });
      })
    );
    detail.appendChild(siteLine);

    if (pw.confirm === item.id) {
      const confirm = el('div', 'confirm');
      confirm.appendChild(el('div', '', S.pw_delete_q));
      const buttons = el('div', 'btns');
      buttons.append(
        textButton(S.delete, 'danger', () => {
          pw.confirm = null;
          pw.open = null;
          hideSecret(item.id);
          post({ c: 'pwDelete', id: item.id, q: ui.panelQuery });
        }),
        textButton(S.cancel, '', () => {
          pw.confirm = null;
          rerenderPanel();
        })
      );
      confirm.appendChild(buttons);
      detail.appendChild(confirm);
    } else {
      const buttons = el('div', 'btns');
      buttons.append(
        textButton(S.pw_edit, '', () => {
          pw.editing = item.id;
          rerenderPanel();
        }),
        textButton(S.delete, '', () => {
          pw.confirm = item.id;
          rerenderPanel();
        })
      );
      detail.appendChild(buttons);
    }
    wrap.appendChild(detail);
    return wrap;
  }

  // Schalter, der nach dem Umlegen die Liste neu holt (die Antwort kann auf sich
  // warten lassen: Abschalten der Abfrage verlangt selbst Touch ID).
  function passwordSwitch(label, hint, key, value) {
    const row = switchRow(label, hint, key, value);
    row.querySelector('.switch').addEventListener('click', () => {
      post({ c: 'panel', name: 'passwords', q: ui.panelQuery });
    });
    return row;
  }

  function renderPasswords(body, data) {
    const items = data.items || [];
    if (!ui.panelQuery) {
      const tools = el('div', 'setbtns top');
      tools.appendChild(
        textButton(S.pw_add, '', () => {
          pw.adding = !pw.adding;
          rerenderPanel();
        })
      );
      tools.appendChild(textButton(S.pw_import, '', () => post({ c: 'pwImport' })));
      if (data.total) tools.appendChild(textButton(S.pw_export, '', () => post({ c: 'pwExport' })));
      body.appendChild(tools);
    }
    if (pw.adding && !ui.panelQuery) {
      const form = el('div', 'pwdetail add');
      const site = field(S.pw_site, 'text', '', 'example.com');
      const user = field(S.pw_user, 'text', '');
      const pass = field(S.pw_password, 'password', '');
      form.append(site.box, user.box, pass.box);
      const buttons = el('div', 'btns');
      buttons.append(
        textButton(S.pw_save, 'primary', () => {
          pw.adding = false;
          post({ c: 'pwAdd', url: site.input.value, username: user.input.value, password: pass.input.value, q: ui.panelQuery });
        }),
        textButton(S.cancel, '', () => {
          pw.adding = false;
          rerenderPanel();
        })
      );
      form.appendChild(buttons);
      body.appendChild(form);
    }

    if (!items.length) body.appendChild(el('div', 'empty', ui.panelQuery ? S.p_no_results : S.p_empty_passwords));
    for (const item of items) body.appendChild(passwordRow(item));
    if (ui.panelQuery) return;
    if (data.total > items.length) {
      body.appendChild(el('div', 'setnote', S.pw_more.replace('{n}', data.total - items.length)));
    }

    body.appendChild(el('div', 'sethead', S.p_settings));
    body.appendChild(passwordSwitch(S.pw_s_offer, S.pw_s_offer_hint, 'pw_offer', data.offer));
    body.appendChild(
      passwordSwitch(S.pw_s_auth, data.touch ? S.pw_s_auth_hint : S.pw_s_auth_hint_none, 'pw_auth', data.auth)
    );
    if ((data.never || []).length) {
      body.appendChild(el('div', 'sethead', S.pw_never_head));
      for (const site of data.never) {
        const row = el('div', 'set');
        row.appendChild(el('div', 'txt', site));
        row.appendChild(textButton(S.pw_allow, '', () => post({ c: 'pwAllow', site, q: ui.panelQuery })));
        body.appendChild(row);
      }
    }
    body.appendChild(el('div', 'setnote', S.pw_note));
  }

  // ------------------------------------------------------------------------
  // Tabs durchsuchen
  // ------------------------------------------------------------------------

  const tsInput = $('ts-input');
  let tsItems = [];
  let tsSelected = 0;

  function openTabSearch() {
    closeMenu();
    closePanel();
    ui.tabSearch = true;
    tsSelected = 0;
    tsInput.value = '';
    $('tabsearch').hidden = false;
    renderTabSearch();
    syncOverlay();
    post({ c: 'focusChrome' });
    setTimeout(() => tsInput.focus(), 0);
  }

  function closeTabSearch() {
    if (!ui.tabSearch) return;
    ui.tabSearch = false;
    $('tabsearch').hidden = true;
    syncOverlay();
  }

  function pickTab(tab) {
    closeTabSearch();
    post({ c: 'activateTab', id: tab.id });
    post({ c: 'focusContent' });
  }

  function renderTabSearch() {
    if (!state) return;
    const needle = tsInput.value.trim().toLowerCase();
    tsItems = state.tabs.filter(
      (tab) => !needle || tab.title.toLowerCase().includes(needle) || (tab.url || '').toLowerCase().includes(needle)
    );
    tsSelected = Math.max(0, Math.min(tsSelected, tsItems.length - 1));
    const list = $('ts-list');
    list.textContent = '';
    if (!tsItems.length) list.appendChild(el('div', 'empty', S.ts_none));
    tsItems.forEach((tab, index) => {
      const row = el('div', 'row' + (index === tsSelected ? ' sel' : '') + (tab.id === state.active ? ' current' : ''));
      row.appendChild(favicon(state.private ? null : tab.favicon, 'globe'));
      const text = el('div', 'txt');
      text.appendChild(el('div', 't', tab.title));
      if (tab.url) text.appendChild(el('div', 's', tab.url));
      row.appendChild(text);
      row.appendChild(actionButton('close', S.tip_close_tab, () => post({ c: 'closeTab', id: tab.id })));
      row.addEventListener('click', () => pickTab(tab));
      list.appendChild(row);
    });
    const selected = list.children[tsSelected];
    if (selected && selected.scrollIntoView) selected.scrollIntoView({ block: 'nearest' });
  }

  tsInput.placeholder = S.ts_hint;
  tsInput.addEventListener('input', () => {
    tsSelected = 0;
    renderTabSearch();
  });
  tsInput.addEventListener('keydown', (event) => {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      if (!tsItems.length) return;
      tsSelected = (tsSelected + (event.key === 'ArrowDown' ? 1 : -1) + tsItems.length) % tsItems.length;
      renderTabSearch();
    } else if (event.key === 'Enter') {
      event.preventDefault();
      if (tsItems[tsSelected]) pickTab(tsItems[tsSelected]);
    } else if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      closeTabSearch();
      post({ c: 'focusContent' });
    }
  });

  // ------------------------------------------------------------------------
  // Befehls-Palette (⌘K)
  // ------------------------------------------------------------------------

  const plInput = $('pl-input');
  let plItems = [];
  let plSelected = 0;

  // Kuratierte Aktionen; Labels/Icons/Kürzel wie im Hauptmenü. Alles läuft über
  // command(id) – derselbe Weg wie die Menüeinträge, kein eigener Befehlspfad.
  function paletteCommands() {
    const translated = state && state.translated;
    const rows = [
      ['new_tab', S.m_new_tab, 'plus', '⌘T'],
      ['new_window', S.m_new_window, 'window', '⌘N'],
      ['new_private', S.m_new_private, 'private', '⇧⌘N'],
      ['new_stealth', S.m_new_stealth, 'stealth', '⌥⌘N'],
      ['reopen_tab', S.m_reopen_tab, 'tabs', '⇧⌘T'],
      ['duplicate_tab', S.m_duplicate_tab, 'copy', ''],
      ['tab_search', S.m_tab_search, 'tabs', '⇧⌘A'],
      ['tab_overview', S.m_tab_overview, 'grid', '⇧⌘\\'],
      ['reader', S.m_reader, 'reader', '⌥⌘R'],
      ['translate_page', translated ? S.m_translate_undo : S.m_translate, 'translate', '⌥⌘T'],
      ['bookmark_page', S.m_bookmark_page, 'star', '⌘D'],
      ['read_later', S.m_read_later, 'bookmark', ''],
      ['remember_page', S.m_remember_page, 'sparkle', ''],
      ['copy_clean_url', S.m_copy_clean_url, 'copy', ''],
      ['find', S.m_find, 'search', '⌘F'],
      ['print', S.m_print, 'print', '⌘P'],
      ['mobile_view', S.m_mobile_view, 'phone', '⇧⌘M'],
      ['assistant', S.m_assistant, 'sparkle', '⌘J'],
      ['assistant_summary', S.m_assistant_summary, 'list', '⇧⌘J'],
      ['site_check', S.m_site_check, 'lock', '⌥⌘S'],
      ['history', S.p_history, 'history', '⌘Y'],
      ['bookmarks', S.p_bookmarks, 'bookmark', '⌥⌘B'],
      ['downloads', S.p_downloads, 'download', '⌥⌘L'],
      ['passwords', S.p_passwords, 'key', ''],
      ['settings', S.p_settings, 'settings', '⌘,'],
      ['app_weather', S.m_app_weather, 'weather', ''],
      ['app_time', S.m_app_time, 'timer', '⌥⌘Z'],
      ['app_security', S.m_app_security, 'shield', ''],
      ['app_request', S.m_app_request, 'send', ''],
      ['app_tasks', S.m_app_tasks, 'board', '⌥⌘K'],
      ['app_inbox', S.m_app_inbox, 'list', ''],
      ['app_notes', S.m_app_notes, 'edit', ''],
      ['app_read', S.m_app_read, 'bookmark', ''],
      ['app_memory', S.m_app_memory, 'sparkle', ''],
    ];
    return rows.map(([id, label, ic, hint]) => ({ id, label: label || id, icon: ic, hint }));
  }

  function openPalette() {
    closeMenu();
    closePanel();
    closeTabSearch();
    ui.palette = true;
    plSelected = 0;
    plInput.value = '';
    $('palette').hidden = false;
    renderPalette();
    syncOverlay();
    post({ c: 'focusChrome' });
    setTimeout(() => plInput.focus(), 0);
  }

  function closePalette() {
    if (!ui.palette) return;
    ui.palette = false;
    $('palette').hidden = true;
    syncOverlay();
  }

  function runPaletteItem(item) {
    closePalette();
    command(item.id);
    post({ c: 'focusContent' });
  }

  function renderPalette() {
    const needle = plInput.value.trim().toLowerCase();
    const terms = needle ? needle.split(/\s+/) : [];
    plItems = paletteCommands().filter((cmd) => {
      if (!terms.length) return true;
      const hay = cmd.label.toLowerCase();
      return terms.every((term) => hay.includes(term));
    });
    plSelected = Math.max(0, Math.min(plSelected, plItems.length - 1));
    const list = $('pl-list');
    list.textContent = '';
    if (!plItems.length) {
      list.appendChild(el('div', 'empty', S.pl_none));
      return;
    }
    plItems.forEach((cmd, index) => {
      const row = el('div', 'row' + (index === plSelected ? ' sel' : ''));
      row.appendChild(icon(cmd.icon || 'globe'));
      const text = el('div', 'txt');
      text.appendChild(el('div', 't', cmd.label));
      row.appendChild(text);
      if (cmd.hint) row.appendChild(el('div', 'plkey', cmd.hint));
      row.addEventListener('click', () => runPaletteItem(cmd));
      list.appendChild(row);
    });
    const selected = list.children[plSelected];
    if (selected && selected.scrollIntoView) selected.scrollIntoView({ block: 'nearest' });
  }

  plInput.placeholder = S.pl_hint;
  plInput.addEventListener('input', () => {
    plSelected = 0;
    renderPalette();
  });
  plInput.addEventListener('keydown', (event) => {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      if (!plItems.length) return;
      plSelected = (plSelected + (event.key === 'ArrowDown' ? 1 : -1) + plItems.length) % plItems.length;
      renderPalette();
    } else if (event.key === 'Enter') {
      event.preventDefault();
      if (plItems[plSelected]) runPaletteItem(plItems[plSelected]);
    } else if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      closePalette();
      post({ c: 'focusContent' });
    }
  });

  // ------------------------------------------------------------------------
  // Suche in der Seite
  // ------------------------------------------------------------------------

  const findInput = $('find-input');

  function openFind() {
    closeMenu();
    closePanel();
    ui.find = true;
    $('findbar').hidden = false;
    reportHeight();
    findInput.focus();
    findInput.select();
    if (findInput.value) runFind(false);
  }

  function closeFind(silent) {
    if (!ui.find) return;
    ui.find = false;
    $('findbar').hidden = true;
    $('find-count').textContent = '';
    reportHeight();
    if (!silent) post({ c: 'findClose' });
  }

  function runFind(back) {
    if (!findInput.value) {
      $('find-count').textContent = '';
      post({ c: 'find', text: '', back: false });
      return;
    }
    post({ c: 'find', text: findInput.value, back: !!back });
  }

  findInput.placeholder = S.find_hint;
  findInput.addEventListener('input', () => runFind(false));
  findInput.addEventListener('keydown', (event) => {
    if (event.key === 'Enter') {
      event.preventDefault();
      runFind(event.shiftKey);
    } else if (event.key === 'Escape') {
      event.preventDefault();
      closeFind(false);
    }
  });
  $('find-next').addEventListener('click', () => runFind(false));
  $('find-prev').addEventListener('click', () => runFind(true));
  $('find-close').addEventListener('click', () => closeFind(false));
  $('find-next').title = S.find_next;
  $('find-prev').title = S.find_prev;

  function onFindResult(message) {
    if (!ui.find) return;
    const result = Array.isArray(message.result) ? message.result : [0, 0];
    const count = Number(result[1]) || 0;
    const index = Number(result[0]) || 0;
    $('find-count').textContent = !findInput.value ? '' : count ? index + ' / ' + count : S.find_none;
  }

  // ------------------------------------------------------------------------
  // Hinweise
  // ------------------------------------------------------------------------

  // Eine Warnung räumt ihren Platz nicht für einen gewöhnlichen Hinweis – und
  // in ihren ersten Sekunden für gar nichts. Sonst könnte eine Seite sie mit
  // einem selbst ausgelösten Hinweis (Download, anderes Programm) überdecken,
  // kaum dass sie dasteht. Was währenddessen kommt, wartet.
  const NOTICE_HOLD = 6000;
  const waitingNotices = [];

  function warningStands() {
    return ui.noticeId != null && ui.noticeWarn && !$('notice').hidden;
  }

  function nextNotice() {
    clearTimeout(ui.noticeHoldTimer);
    if (!waitingNotices.length) return;
    // Erst was stehen bleiben muss, dann Warnungen, dann der Rest.
    let index = waitingNotices.findIndex((message) => message.keep);
    if (index < 0) index = waitingNotices.findIndex((message) => message.warn);
    const message = waitingNotices[Math.max(index, 0)];
    const held = warningStands() && (!message.warn || performance.now() - ui.noticeAt < NOTICE_HOLD);
    if (held) return;
    waitingNotices.splice(Math.max(index, 0), 1);
    displayNotice(message);
  }

  function hideNotice() {
    clearTimeout(ui.noticeTimer);
    if (ui.noticeId != null) post({ c: 'noticeGone', id: ui.noticeId });
    ui.noticeId = null;
    ui.noticeWarn = false;
    $('notice').hidden = true;
    reportHeight();
    // Was gewartet hat, kommt jetzt dran.
    if (waitingNotices.length) setTimeout(nextNotice, 0);
  }

  function showNotice(message) {
    const age = performance.now() - (ui.noticeAt || 0);
    if (warningStands() && (!message.warn || age < NOTICE_HOLD)) {
      waitingNotices.push(message);
      // Mehr als vier warten nicht: Der älteste gewöhnliche fällt weg, sonst der neue.
      if (waitingNotices.length > 4) {
        let drop = waitingNotices.findIndex((waiting) => !waiting.warn);
        if (drop < 0) drop = waitingNotices.findIndex((waiting) => !waiting.keep);
        if (drop < 0) drop = waitingNotices.length - 1;
        post({ c: 'noticeGone', id: waitingNotices.splice(drop, 1)[0].id, why: 'dropped' });
      }
      // Eine wartende Warnung kommt dran, sobald die erste lange genug stand.
      if (message.warn && age < NOTICE_HOLD) {
        clearTimeout(ui.noticeHoldTimer);
        ui.noticeHoldTimer = setTimeout(nextNotice, NOTICE_HOLD - age + 30);
      }
      return;
    }
    displayNotice(message);
  }

  function displayNotice(message) {
    // „replaced“: Ein neuer Hinweis verdrängt den alten – anders als beim
    // Schließen oder Ablaufen hat ihn dann womöglich niemand gelesen.
    if (ui.noticeId != null) post({ c: 'noticeGone', id: ui.noticeId, why: 'replaced' });
    clearTimeout(ui.noticeTimer);
    ui.noticeId = message.id;
    ui.noticeWarn = !!message.warn;
    ui.noticeAt = performance.now();
    $('notice').classList.toggle('warn', !!message.warn);
    setUse('notice-icon', message.warn ? 'warning' : 'info');
    $('notice-text').textContent = message.text;
    $('notice-text').title = message.text;
    const action = $('notice-action');
    action.hidden = !message.action;
    action.textContent = message.action || '';
    $('notice').hidden = false;
    reportHeight();
    if (message.timeout) ui.noticeTimer = setTimeout(hideNotice, message.timeout);
  }

  $('notice-action').addEventListener('click', () => {
    // Ein Hinweis, der eben erst erschienen ist (oder einen anderen ersetzt
    // hat), nimmt noch keinen Klick an – sonst träfe ein Klick, der dem alten
    // galt, den neuen.
    if (performance.now() - (ui.noticeAt || 0) < 600) return;
    const id = ui.noticeId;
    ui.noticeId = null;
    post({ c: 'noticeAction', id });
    hideNotice();
  });
  $('notice-close').addEventListener('click', hideNotice);

  // ------------------------------------------------------------------------
  // Nachrichten vom Rust-Teil
  // ------------------------------------------------------------------------

  // ------------------------------------------------------------------------
  // Übersicht aller Tabs: Karten mit Vorschaubild, durchsuchbar
  // ------------------------------------------------------------------------

  const thumbs = new Map(); // Tab → Stand des Vorschaubilds
  const tgInput = $('tg-input');
  let tgItems = [];

  function openOverview() {
    closeMenu();
    closePanel();
    closeTabSearch();
    ui.overview = true;
    tgInput.value = '';
    $('tabgrid').hidden = false;
    renderOverview();
    syncOverlay();
    post({ c: 'focusChrome' });
    // Der Browser macht jetzt frische Bilder der geladenen Tabs.
    post({ c: 'overview' });
    setTimeout(() => tgInput.focus(), 0);
  }

  function closeOverview() {
    if (!ui.overview) return;
    ui.overview = false;
    $('tabgrid').hidden = true;
    syncOverlay();
  }

  function renderOverview() {
    if (!state) return;
    const needle = tgInput.value.trim().toLowerCase();
    tgItems = state.tabs.filter(
      (tab) => !needle || tab.title.toLowerCase().includes(needle) || (tab.url || '').toLowerCase().includes(needle)
    );
    const box = $('tg-cards');
    const scroll = box.scrollTop;
    box.textContent = '';
    if (!tgItems.length) box.appendChild(el('div', 'empty', S.ts_none));
    tgItems.forEach((tab, index) => {
      const card = el('div', 'card' + (tab.id === state.active ? ' current' : '') + (needle && index === 0 ? ' sel' : ''));
      const shot = el('div', 'shot');
      if (thumbs.has(tab.id)) {
        const img = document.createElement('img');
        img.className = 'thumb';
        img.decoding = 'async';
        img.addEventListener('error', () => img.remove(), { once: true });
        img.src = 'thumb/' + tab.id + '?v=' + thumbs.get(tab.id);
        shot.appendChild(img);
      } else {
        shot.appendChild(favicon(state.private ? null : tab.favicon, 'globe'));
      }
      card.appendChild(shot);
      const meta = el('div', 'meta');
      meta.appendChild(favicon(state.private ? null : tab.favicon, 'globe'));
      const text = el('div', 'txt');
      text.appendChild(el('div', 't', tab.title));
      text.appendChild(el('div', 's', tab.url || ''));
      meta.appendChild(text);
      card.appendChild(meta);
      const close = el('button', 'ibtn sm x');
      close.type = 'button';
      close.title = S.to_close;
      close.appendChild(icon('close', 'sm'));
      close.addEventListener('click', (event) => {
        event.stopPropagation();
        post({ c: 'closeTab', id: tab.id });
      });
      card.appendChild(close);
      card.addEventListener('click', () => {
        closeOverview();
        post({ c: 'activateTab', id: tab.id });
        post({ c: 'focusContent' });
      });
      box.appendChild(card);
    });
    box.scrollTop = scroll;
  }

  tgInput.addEventListener('input', renderOverview);
  tgInput.addEventListener('keydown', (event) => {
    if (event.key === 'Enter' && tgItems.length) {
      const tab = tgItems[0];
      closeOverview();
      post({ c: 'activateTab', id: tab.id });
      post({ c: 'focusContent' });
    } else if (event.key === 'Escape') {
      event.stopPropagation();
      closeOverview();
      post({ c: 'focusContent' });
    }
  });
  $('tg-close').addEventListener('click', () => {
    closeOverview();
    post({ c: 'focusContent' });
  });
  $('overview').addEventListener('click', () => (ui.overview ? closeOverview() : openOverview()));
  $('tabgrid').addEventListener('mousedown', (event) => {
    // Ein Klick ins Leere schließt die Übersicht.
    if (event.target === $('tabgrid') || event.target === $('tg-cards')) {
      closeOverview();
      post({ c: 'focusContent' });
    }
  });

  function show(what) {
    if (what === 'find') openFind();
    else if (what === 'findNext' || what === 'findPrev') {
      // Bei offener Suchleiste bleibt der Fokus, wo er ist (meist in der Seite).
      if (ui.find) runFind(what === 'findPrev');
      else {
        post({ c: 'focusChrome' });
        openFind();
      }
    } else if (what === 'clearData') openPanel('settings', { confirmClear: true });
    else if (what === 'siteCheck') ui.menu === 'check' ? closeMenu() : openCheck();
    else if (what === 'tab_search') ui.tabSearch ? closeTabSearch() : openTabSearch();
    else if (what === 'tab_overview') ui.overview ? closeOverview() : openOverview();
    else if (what === 'palette') ui.palette ? closePalette() : openPalette();
    else if (PANEL_TITLE[what]) togglePanel(what);
  }

  window.__zunder = (message) => {
    switch (message.t) {
      case 'state':
        applyState(message);
        break;
      case 'progress':
        if (state && state.nav.loading) setProgress(true, message.p);
        break;
      case 'suggest':
        onSuggest(message);
        break;
      case 'suggestMore':
        onSuggestMore(message);
        break;
      case 'focusAddress':
        closeMenu();
        closePanel();
        urlInput.focus();
        // Kurz warten, bis der Rahmen die Tastatur wirklich hat.
        setTimeout(() => {
          if (document.activeElement !== urlInput) return;
          addressActive = false;
          addressGainedFocus();
          if (!ui.editing) urlInput.select();
        }, 30);
        break;
      case 'show':
        show(message.what);
        break;
      case 'panel':
        renderPanel(message);
        break;
      case 'findResult':
        onFindResult(message);
        break;
      case 'bmbar':
        ui.bar = message;
        renderBar();
        break;
      case 'blocked':
        setBlocked(message.n);
        if (ui.ampel) {
          ui.ampel.blocked = message.n;
          if (ui.menu === 'ampel') fillAmpel();
        }
        break;
      case 'ampel':
        renderAmpel(message.ampel);
        break;
      case 'filters':
        ui.filters = message.info;
        fillFilterLine();
        break;
      case 'thumb':
        thumbs.set(message.id, message.v);
        if (ui.overview) renderOverview();
        break;
      case 'notice':
        showNotice(message);
        break;
      case 'noticeText': {
        // Ein Hinweis, der noch dasteht, bekommt neuen Text (Fortschritt).
        if (ui.noticeId === message.id && !$('notice').hidden) {
          $('notice-text').textContent = message.text;
          $('notice-text').title = message.text;
        }
        const waiting = waitingNotices.find((entry) => entry.id === message.id);
        if (waiting) waiting.text = message.text;
        break;
      }
      case 'noticeHide': {
        if (ui.noticeId === message.id) hideNotice();
        const index = waitingNotices.findIndex((entry) => entry.id === message.id);
        if (index >= 0) waitingNotices.splice(index, 1);
        break;
      }
      case 'pwSecret':
        onSecret(message);
        break;
      case 'committed':
        if (ui.find) $('find-count').textContent = '';
        break;
    }
  };

  // ------------------------------------------------------------------------
  // Start
  // ------------------------------------------------------------------------

  urlInput.placeholder = DATA.stealth ? S.address_hint_stealth : DATA.private ? S.address_hint_private : S.address_hint;
  $('back').title = S.tip_back;
  $('forward').title = S.tip_forward;
  $('newtab').title = S.tip_new_tab;
  $('menu').title = S.tip_menu;
  $('downloads').title = S.tip_downloads;
  $('assistant').title = S.tip_assistant;
  $('key').title = S.tip_key;
  $('reader').title = S.tip_reader;
  $('mobile').title = S.tip_mobile;
  $('alltabs').title = S.tip_tab_search;
  $('overview').title = S.tip_tab_overview;
  tgInput.placeholder = S.to_search;
  $('private-badge').querySelector('span').textContent = S.private;
  document.body.classList.toggle('private', !!DATA.private);
  document.body.classList.toggle('stealth', !!DATA.stealth);

  window.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && (ui.menu || ui.panel || ui.tabSearch || ui.palette || ui.overview) && !addressFocused()) {
      closeEverything();
      post({ c: 'focusContent' });
    }
  });

  // Das eingebaute Kontextmenü gehört in den Rahmen nur bei Textfeldern.
  window.addEventListener('contextmenu', (event) => {
    if (!(event.target instanceof HTMLInputElement)) event.preventDefault();
  });

  // Dateien, die auf den Rahmen fallen, sollen ihn nicht ersetzen.
  window.addEventListener('dragover', (event) => event.preventDefault());
  window.addEventListener('drop', (event) => event.preventDefault());

  reportHeight();
  // ------------------------------------------------------------------------
  // Ladebild beim Start: fünf Sekunden Animation. Der Browser dahinter ist
  // längst bei der Arbeit – die Seiten laden schon, und ein Klick oder ein
  // Tastendruck gibt ihn sofort frei.
  // ------------------------------------------------------------------------

  (function splash() {
    const node = $('splash');
    if (!document.body.classList.contains('boot')) {
      node.remove();
      return;
    }
    const TOTAL = 5000;
    const OUT = 450;
    const started = performance.now();
    // Solange das Ladebild steht, deckt der Rahmen das ganze Fenster ab – so
    // hat ihn der Browser auch angelegt.
    ui.splash = true;
    ui.overlay = true;
    $('splash-line').textContent = S.splash_line;
    $('splash-skip').textContent = S.splash_skip;

    const calm = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

    // Funken: feste Folge statt Zufall, damit jeder Start gleich aussieht.
    const embers = $('splash-embers');
    let seed = 7;
    const next = () => {
      seed = (seed * 16807) % 2147483647;
      return seed / 2147483647;
    };
    for (let i = 0; i < 22; i++) {
      const ember = document.createElement('i');
      ember.style.setProperty('--x', (4 + next() * 92).toFixed(1) + '%');
      ember.style.setProperty('--s', (2 + next() * 3.5).toFixed(1) + 'px');
      ember.style.setProperty('--d', (3.2 + next() * 3.4).toFixed(2) + 's');
      ember.style.setProperty('--w', (next() * 2.6).toFixed(2) + 's');
      ember.style.setProperty('--dx', ((next() - 0.5) * 120).toFixed(0) + 'px');
      embers.appendChild(ember);
    }

    // Dahinter die Glut-Szene: erst ihr Standbild, dann – sobald es läuft – das
    // Video. Fehlen die Dateien, bleibt der Verlauf aus dem Stil.
    const still = $('splash-still');
    still.addEventListener('load', () => still.classList.add('on'), { once: true });
    still.addEventListener('error', () => still.remove(), { once: true });
    still.src = 'wp/glut.jpg';
    const video = $('splash-video');
    if (calm) {
      video.remove();
    } else {
      video.addEventListener('playing', () => video.classList.add('on'), { once: true });
      video.addEventListener('error', () => video.remove(), { once: true });
      video.src = 'wp/glut.mp4';
      const playing = video.play();
      if (playing && playing.catch) playing.catch(() => {});
    }

    let leaving = false;
    function leave() {
      if (leaving) return;
      leaving = true;
      clearTimeout(timer);
      node.classList.add('out');
      setTimeout(() => {
        try {
          video.pause();
        } catch (error) {}
        node.remove();
        document.body.classList.remove('boot');
        ui.splash = false;
        syncOverlay();
        // Die Tastatur geht dorthin, wo sie ohne Ladebild wäre.
        if (!addressFocused()) post({ c: 'focusContent' });
      }, OUT);
    }
    const timer = setTimeout(leave, Math.max(0, TOTAL - OUT - (performance.now() - started)));
    node.addEventListener('mousedown', leave);
    // Wer sofort lostippt, landet in der Adressleiste – das Ladebild geht dabei weg.
    window.addEventListener('keydown', leave, { capture: true, once: true });

    // Sobald das Ladebild wirklich zu sehen ist, darf darunter die Seite erscheinen.
    requestAnimationFrame(() => requestAnimationFrame(() => post({ c: 'splashShown' })));
  })();

  post({ c: 'ready' });
})();
