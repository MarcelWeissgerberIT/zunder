// Gedächtnis – eine eingebaute App. Seiten, die der Mensch gemerkt hat, liegen
// als Karteikarten beim Browser (src/apps.rs), kein Netz. Hinein kommen sie über
// den Befehl „Seite ins Gedächtnis legen" (src/commands.rs → assistant.rs legt
// Text an); der Assistent kann daraus antworten.
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
  const items = new Map(); // id (m-<id>) → { k:'m', url, title, text, when }
  const settings = { sort: 'new' }; // 'new' | 'old' | 'title'
  let query = '';

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
  const snippet = (value) => {
    const clean = (value || '').replace(/\s+/g, ' ').trim();
    return clean ? clean.slice(0, 220) : '';
  };

  // ------------------------------------------------------------------------
  // Ablage
  // ------------------------------------------------------------------------

  function stored(state) {
    if (!kept || state === true || state === 'ok') return;
    toast(state === 'full' ? S.gd_full : S.gd_save_failed);
  }

  function saveNote() {
    Z.save({ v: 1, sort: settings.sort }).then(stored);
  }

  function dropItem(id) {
    items.delete(id);
    Z.records.remove(id).then(stored);
  }

  function cleanItem(value) {
    if (!value || typeof value !== 'object' || typeof value.url !== 'string') return null;
    if (!/^https?:\/\//i.test(value.url)) return null;
    return {
      k: 'm',
      url: Z.clip(value.url, 2000),
      title: Z.clip(String(value.title == null ? '' : value.title).replace(/\s+/g, ' ').trim(), 300),
      text: Z.clip(String(value.text == null ? '' : value.text), 20000),
      when: Number(value.when) || 0,
    };
  }

  // ------------------------------------------------------------------------
  // Anzeige
  // ------------------------------------------------------------------------

  function ordered() {
    const all = [...items.entries()];
    if (settings.sort === 'title') {
      return all.sort((a, b) => (a[1].title || a[1].url).localeCompare(b[1].title || b[1].url, LANG));
    }
    const direction = settings.sort === 'old' ? 1 : -1;
    return all.sort((a, b) => direction * (a[1].when - b[1].when));
  }

  function matches(item) {
    if (!query) return true;
    const q = query.toLowerCase();
    return (
      (item.title || '').toLowerCase().includes(q) ||
      item.url.toLowerCase().includes(q) ||
      (item.text || '').toLowerCase().includes(q)
    );
  }

  function render() {
    if (!ready) return;
    renderBar();
    const list = $('list');
    list.textContent = '';
    const all = ordered().filter(([, item]) => matches(item));
    const total = items.size;
    $('count').textContent = total ? text('gd_count', { n: total }) : '';
    $('clear').hidden = !total;
    const empty = !all.length;
    $('empty').hidden = !empty;
    if (empty) {
      $('empty').textContent = query ? S.gd_no_results : S.gd_empty;
      return;
    }
    for (const [id, item] of all) list.appendChild(rowEl(id, item));
  }

  function renderBar() {
    const view = $('sort');
    view.textContent = '';
    for (const [key, label] of [['new', S.gd_sort_new], ['old', S.gd_sort_old], ['title', S.gd_sort_title]]) {
      const button = node('button', settings.sort === key ? 'on' : '', label);
      button.type = 'button';
      button.addEventListener('click', () => {
        settings.sort = key;
        saveNote();
        render();
      });
      view.appendChild(button);
    }
    $('private-note').hidden = kept;
    $('private-note').textContent = S.gd_private;
  }

  function rowEl(id, item) {
    const row = node('div', 'item');
    const main = node('div', 'item-main');
    const link = document.createElement('a');
    link.className = 'item-title';
    link.href = item.url;
    link.target = '_blank';
    link.rel = 'noopener';
    link.title = S.gd_open;
    link.textContent = item.title || hostOf(item.url);
    main.appendChild(link);
    const snip = snippet(item.text);
    main.appendChild(node('div', 'item-snippet', snip || S.gd_snippet_none));
    const meta = node('div', 'item-meta');
    meta.appendChild(node('span', 'host', hostOf(item.url)));
    meta.appendChild(node('span', 'date', dateLabel(item.when)));
    main.appendChild(meta);
    row.appendChild(main);

    const del = node('button', 'round');
    del.type = 'button';
    del.title = S.gd_delete;
    del.setAttribute('aria-label', S.gd_delete);
    del.appendChild(icon('trash'));
    // Gemerkte Seiten tragen viel Text; ein Zurückholen über den App-Kanal wäre
    // begrenzt. Das Entfernen gilt darum sofort – eine Seite ist schnell wieder
    // gemerkt.
    del.addEventListener('click', () => {
      dropItem(id);
      render();
      toast(S.gd_deleted);
    });
    row.appendChild(del);
    return row;
  }

  // ------------------------------------------------------------------------
  // Suche, Alles entfernen
  // ------------------------------------------------------------------------

  $('search').addEventListener('input', () => {
    query = $('search').value.trim();
    render();
  });

  // Alles entfernen braucht zwei Klicks – der erste fragt nach.
  let clearArmed = false;
  let clearTimer = 0;
  function resetClear() {
    clearArmed = false;
    clearTimeout(clearTimer);
    $('clear').classList.remove('danger');
    $('clear').textContent = S.gd_clear;
  }
  $('clear').addEventListener('click', () => {
    if (!clearArmed) {
      clearArmed = true;
      $('clear').classList.add('danger');
      $('clear').textContent = S.gd_clear_confirm;
      clearTimer = setTimeout(resetClear, 3500);
      return;
    }
    const count = items.size;
    for (const [id] of [...items.entries()]) dropItem(id);
    resetClear();
    render();
    if (count) toast(text('gd_cleared', { n: count }));
  });

  // ---- Kurze Meldung ---------------------------------------------------------

  let toastTimer = 0;
  function toast(message) {
    clearTimeout(toastTimer);
    $('toast-text').textContent = message;
    $('toast-undo').hidden = true;
    $('toast').hidden = false;
    toastTimer = setTimeout(() => {
      $('toast').hidden = true;
    }, 3200);
  }

  $('title').textContent = S.gd_title;
  $('search').placeholder = S.gd_search;
  $('search').setAttribute('aria-label', S.gd_search);
  $('clear').textContent = S.gd_clear;
  $('hint').textContent = S.gd_hint;
  $('trouble-text').textContent = S.gd_load_failed;
  $('trouble-retry').textContent = S.gd_retry;
  document.title = S.app_memory;

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
      settings.sort = ['new', 'old', 'title'].includes(note.value.sort) ? note.value.sort : 'new';
      items.clear();
      for (const [id, record] of records.list) {
        if (id.startsWith('m-')) {
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
  // Kommt der Tab wieder nach vorn (nach „ins Gedächtnis legen" auf einer anderen
  // Seite), den Stand neu lesen, damit neue Einträge auftauchen.
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
