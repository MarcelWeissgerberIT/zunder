// Später lesen – eine eingebaute App. Eine Leseliste: gespeicherte Seiten liegen
// als Karteikarten beim Browser (src/apps.rs), kein Netz. Seiten kommen über den
// Browser-Befehl „Später lesen" hinein (src/commands.rs legt einen Eintrag an).
(() => {
  'use strict';
  const Z = window.ZunderApp;
  const S = Z.strings;
  const LANG = Z.data.lang === 'de' ? 'de' : 'en';
  const $ = (id) => document.getElementById(id);
  const text = (key, values) =>
    String(S[key] || '').replace(/\{(\w+)\}/g, (whole, name) => (values && name in values ? values[name] : whole));

  let kept = false;
  let ready = false;
  const items = new Map(); // id (r-<id>) → { k:'r', url, title, when, read }
  const settings = { filter: 'all' }; // 'all' | 'unread'
  let query = '';
  let undo = null;

  function node(tag, className, content) {
    const element = document.createElement(tag);
    if (className) element.className = className;
    if (content !== undefined) element.textContent = content;
    return element;
  }

  function icon(name) {
    const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    const use = document.createElementNS('http://www.w3.org/2000/svg', 'use');
    use.setAttribute('href', '#i-' + name);
    svg.appendChild(use);
    return svg;
  }

  const hostOf = (url) => {
    try {
      return new URL(url).hostname.replace(/^www\./, '');
    } catch (error) {
      return url;
    }
  };
  const dateLabel = (ms) => (ms ? new Date(ms).toLocaleDateString(LANG, { day: 'numeric', month: 'short', year: 'numeric' }) : '');

  // ------------------------------------------------------------------------
  // Ablage
  // ------------------------------------------------------------------------

  function stored(state) {
    if (!kept || state === true || state === 'ok') return;
    toast(state === 'full' ? S.rd_full : S.rd_save_failed);
  }

  function saveNote() {
    Z.save({ v: 1, filter: settings.filter }).then(stored);
  }

  function putItem(id, item) {
    items.set(id, item);
    Z.records.put(id, item).then(stored);
  }

  function dropItem(id) {
    items.delete(id);
    Z.records.remove(id).then(stored);
  }

  function cleanItem(value) {
    if (!value || typeof value !== 'object' || typeof value.url !== 'string') return null;
    if (!/^https?:\/\//i.test(value.url)) return null;
    return {
      k: 'r',
      url: Z.clip(value.url, 2000),
      title: Z.clip(String(value.title == null ? '' : value.title).replace(/\s+/g, ' ').trim(), 300),
      when: Number(value.when) || 0,
      read: value.read === true,
    };
  }

  // ------------------------------------------------------------------------
  // Anzeige
  // ------------------------------------------------------------------------

  function ordered() {
    return [...items.entries()].sort((a, b) => Number(a[1].read) - Number(b[1].read) || b[1].when - a[1].when);
  }

  function matches(item) {
    if (settings.filter === 'unread' && item.read) return false;
    if (!query) return true;
    const q = query.toLowerCase();
    return (item.title || '').toLowerCase().includes(q) || item.url.toLowerCase().includes(q);
  }

  function render() {
    if (!ready) return;
    renderBar();
    const list = $('list');
    list.textContent = '';
    const all = ordered().filter(([, item]) => matches(item));
    const total = items.size;
    const unread = [...items.values()].filter((item) => !item.read).length;
    $('count').textContent = total ? text('rd_count', { n: total, u: unread }) : '';
    $('clear-read').hidden = unread >= total || !total;
    const empty = !all.length;
    $('empty').hidden = !empty;
    if (empty) {
      $('empty').textContent = query ? S.rd_no_results : settings.filter === 'unread' ? S.rd_empty_unread : S.rd_empty;
      return;
    }
    for (const [id, item] of all) list.appendChild(rowEl(id, item));
  }

  function renderBar() {
    const view = $('filter');
    view.textContent = '';
    for (const [key, label] of [['all', S.rd_all], ['unread', S.rd_unread]]) {
      const button = node('button', settings.filter === key ? 'on' : '', label);
      button.type = 'button';
      button.addEventListener('click', () => {
        settings.filter = key;
        saveNote();
        render();
      });
      view.appendChild(button);
    }
    $('private-note').hidden = kept;
    $('private-note').textContent = S.rd_private;
  }

  function rowEl(id, item) {
    const row = node('div', 'item' + (item.read ? ' read' : ''));
    const main = node('div', 'item-main');
    const link = document.createElement('a');
    link.className = 'item-title';
    link.href = item.url;
    link.target = '_blank';
    link.rel = 'noopener';
    link.textContent = item.title || hostOf(item.url);
    // Beim Öffnen gilt die Seite als gelesen.
    link.addEventListener('click', () => {
      if (!item.read) {
        item.read = true;
        putItem(id, item);
        setTimeout(render, 0);
      }
    });
    main.appendChild(link);
    const meta = node('div', 'item-meta');
    meta.appendChild(node('span', 'host', hostOf(item.url)));
    meta.appendChild(node('span', 'date', dateLabel(item.when)));
    if (!item.read) meta.appendChild(node('span', 'badge', S.rd_unread_badge));
    main.appendChild(meta);
    row.appendChild(main);

    const toggle = node('button', 'round');
    toggle.type = 'button';
    toggle.title = item.read ? S.rd_mark_unread : S.rd_mark_read;
    toggle.setAttribute('aria-label', toggle.title);
    toggle.appendChild(icon(item.read ? 'eye-off' : 'check'));
    toggle.addEventListener('click', () => {
      item.read = !item.read;
      putItem(id, item);
      render();
    });
    row.appendChild(toggle);

    const del = node('button', 'round');
    del.type = 'button';
    del.title = S.rd_delete;
    del.setAttribute('aria-label', S.rd_delete);
    del.appendChild(icon('trash'));
    del.addEventListener('click', () => {
      dropItem(id);
      render();
      toast(S.rd_deleted, () => {
        putItem(id, item);
        render();
      });
    });
    row.appendChild(del);
    return row;
  }

  // ------------------------------------------------------------------------
  // Suche, Filter, Gelesene entfernen
  // ------------------------------------------------------------------------

  $('search').addEventListener('input', () => {
    query = $('search').value.trim();
    render();
  });

  $('clear-read').addEventListener('click', () => {
    const gone = [...items.entries()].filter(([, item]) => item.read);
    for (const [id] of gone) dropItem(id);
    render();
    if (gone.length) {
      toast(text('rd_cleared', { n: gone.length }), () => {
        for (const [id, item] of gone) putItem(id, item);
        render();
      });
    }
  });

  // ---- Meldung mit „Rückgängig" ----------------------------------------------

  let toastTimer = 0;
  function toast(message, revert) {
    clearTimeout(toastTimer);
    undo = revert || null;
    $('toast-text').textContent = message;
    $('toast-undo').hidden = !revert;
    $('toast-undo').textContent = S.rd_undo;
    $('toast').hidden = false;
    toastTimer = setTimeout(() => {
      $('toast').hidden = true;
      undo = null;
    }, 6000);
  }
  $('toast-undo').addEventListener('click', () => {
    const revert = undo;
    undo = null;
    $('toast').hidden = true;
    if (revert) revert();
  });

  $('title').textContent = S.rd_title;
  $('search').placeholder = S.rd_search;
  $('search').setAttribute('aria-label', S.rd_search);
  $('clear-read').textContent = S.rd_clear_read;
  $('hint').textContent = S.rd_saved_hint;
  $('trouble-text').textContent = S.rd_load_failed;
  $('trouble-retry').textContent = S.rd_retry;
  document.title = S.app_read;

  // ------------------------------------------------------------------------
  // Laden
  // ------------------------------------------------------------------------

  function load() {
    return Promise.all([Z.load(), Z.records.all()]).then(([note, records]) => {
      const ok = note.ok && records.ok;
      $('trouble').hidden = ok;
      if (!ok) {
        ready = false;
        return;
      }
      kept = note.kept && records.kept;
      settings.filter = note.value.filter === 'unread' ? 'unread' : 'all';
      items.clear();
      for (const [id, record] of records.list) {
        if (id.startsWith('r-')) {
          const clean = cleanItem(record);
          if (clean) items.set(id, clean);
        }
      }
      $('hint').hidden = items.size > 0;
      ready = true;
      render();
    });
  }

  $('trouble-retry').addEventListener('click', load);
  // Kommt der Tab wieder nach vorn (nach „Später lesen" auf einer anderen Seite),
  // den Stand neu lesen, damit neue Einträge auftauchen.
  let awaySince = 0;
  document.addEventListener('visibilitychange', () => {
    if (document.hidden) awaySince = Date.now();
    else if (kept && awaySince && Date.now() - awaySince > 300) load();
  });
  window.addEventListener('focus', () => {
    if (kept && ready) load();
  });

  load();
})();
