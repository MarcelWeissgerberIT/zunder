// Notizen – eine eingebaute App. Rein lokal: jede Notiz ist eine Karteikarte
// beim Browser (src/apps.rs), kein Netz. Liste + Suche, Editor, Anheften,
// Markdown-Export.
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
  const notes = new Map(); // id → { k:'n', title, body, pinned, when, edited }
  const settings = { sort: 'edited' }; // 'edited' | 'created' | 'title'
  let query = '';
  let editing = '';
  let undo = null;

  const newId = () => 'n-' + Date.now().toString(36) + Math.random().toString(36).slice(2, 7);

  function cleanText(value, max) {
    const out = [];
    for (const ch of String(value == null ? '' : value)) {
      const code = ch.charCodeAt(0);
      if (ch.length === 1 && code >= 0xd800 && code <= 0xdfff) continue;
      if (code === 10 || code === 9 || (code >= 32 && code !== 127)) {
        out.push(ch);
        if (out.length >= max) break;
      }
    }
    return out.join('');
  }

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

  // ------------------------------------------------------------------------
  // Ablage
  // ------------------------------------------------------------------------

  function stored(state) {
    if (!kept || state === true || state === 'ok') return;
    toast(state === 'full' ? S.nt_full : S.nt_save_failed);
  }

  function saveNote() {
    Z.save({ v: 1, sort: settings.sort }).then(stored);
  }

  // Eine Notiz muss <= ~1500 Bytes sein (RECORD_MAX); zu lange wird der Text
  // per Byte-Maß gekürzt, damit das Sichern nie scheitert.
  function fit(note) {
    const budget = 1400;
    const size = (value) => new TextEncoder().encode(JSON.stringify(value)).length;
    if (size(note) <= budget) return note;
    let body = note.body;
    while (body && size({ ...note, body }) > budget) {
      body = body.slice(0, Math.max(0, body.length - Math.max(4, Math.ceil(body.length / 16))));
    }
    note.body = body.trimEnd();
    return note;
  }

  function putNote(id, note) {
    fit(note);
    notes.set(id, note);
    Z.records.put(id, note).then(stored);
  }

  function dropNote(id) {
    notes.delete(id);
    Z.records.remove(id).then(stored);
  }

  function cleanNote(value) {
    if (!value || typeof value !== 'object') return null;
    return {
      k: 'n',
      title: cleanText(value.title, 200).replace(/[\n\t]/g, ' ').trim(),
      body: cleanText(value.body, 1200),
      pinned: value.pinned === true,
      when: Number(value.when) || 0,
      edited: Number(value.edited) || Number(value.when) || 0,
    };
  }

  // ------------------------------------------------------------------------
  // Anzeige
  // ------------------------------------------------------------------------

  const snippet = (note) => (note.body || '').replace(/\s+/g, ' ').trim();
  const dateLabel = (ms) => (ms ? new Date(ms).toLocaleDateString(LANG, { day: 'numeric', month: 'short', year: 'numeric' }) : '');

  function ordered() {
    const list = [...notes.entries()];
    const by = {
      edited: (a, b) => b[1].edited - a[1].edited,
      created: (a, b) => b[1].when - a[1].when,
      title: (a, b) => (a[1].title || '').localeCompare(b[1].title || ''),
    }[settings.sort] || ((a, b) => b[1].edited - a[1].edited);
    return list.sort((a, b) => Number(b[1].pinned) - Number(a[1].pinned) || by(a, b));
  }

  function matches(note) {
    if (!query) return true;
    const q = query.toLowerCase();
    return (note.title || '').toLowerCase().includes(q) || (note.body || '').toLowerCase().includes(q);
  }

  function render() {
    if (!ready) return;
    renderBar();
    const list = $('list');
    list.textContent = '';
    const all = ordered().filter(([, note]) => matches(note));
    $('count').textContent = all.length ? text('nt_count', { n: all.length }) : '';
    const empty = !all.length;
    $('empty').hidden = !empty;
    if (empty) {
      $('empty').textContent = query ? S.nt_no_results : S.nt_empty;
      return;
    }
    for (const [id, note] of all) list.appendChild(cardEl(id, note));
  }

  function renderBar() {
    const sort = $('sort');
    sort.textContent = '';
    for (const [key, label] of [['edited', S.nt_sort_edited], ['created', S.nt_sort_created], ['title', S.nt_sort_title]]) {
      const option = document.createElement('option');
      option.value = key;
      option.textContent = label;
      sort.appendChild(option);
    }
    sort.value = settings.sort;
    $('private-note').hidden = kept;
    $('private-note').textContent = S.nt_private;
  }

  function cardEl(id, note) {
    const card = node('div', 'note' + (note.pinned ? ' pinned' : ''));
    card.setAttribute('role', 'button');
    card.tabIndex = 0;
    const head = node('div', 'note-head');
    head.appendChild(node('span', 'note-title', note.title || S.nt_untitled));
    if (note.pinned) {
      const star = node('span', 'pin-mark');
      star.appendChild(icon('star-filled'));
      head.appendChild(star);
    }
    card.appendChild(head);
    const text = snippet(note);
    if (text) card.appendChild(node('p', 'note-body', text));
    card.appendChild(node('span', 'note-date', dateLabel(note.edited)));
    card.addEventListener('click', () => openEditor(id));
    card.addEventListener('keydown', (event) => {
      if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        openEditor(id);
      }
    });
    return card;
  }

  // ------------------------------------------------------------------------
  // Editor
  // ------------------------------------------------------------------------

  function openEditor(id) {
    const note = notes.get(id);
    if (!note) return;
    editing = id;
    $('ed-title').value = note.title;
    $('ed-title').placeholder = S.nt_title_ph;
    $('ed-title').maxLength = 200;
    $('ed-body').value = note.body;
    $('ed-body').placeholder = S.nt_body_ph;
    $('ed-body').maxLength = 1200;
    $('ed-pin').textContent = note.pinned ? S.nt_unpin : S.nt_pin;
    $('ed-pin').classList.toggle('on', note.pinned);
    $('ed-delete').textContent = S.nt_delete;
    $('ed-cancel').textContent = S.nt_cancel;
    $('ed-save').textContent = S.nt_save;
    $('editor').hidden = false;
    setTimeout(() => $(note.title ? 'ed-body' : 'ed-title').focus(), 0);
  }

  function closeEditor() {
    editing = '';
    $('editor').hidden = true;
  }

  $('editor').querySelector('form').addEventListener('submit', (event) => {
    event.preventDefault();
    const note = notes.get(editing);
    if (note) {
      note.title = cleanText($('ed-title').value, 200).replace(/[\n\t]/g, ' ').trim();
      note.body = cleanText($('ed-body').value, 1200);
      note.pinned = $('ed-pin').classList.contains('on');
      note.edited = Date.now();
      putNote(editing, note);
    }
    closeEditor();
    render();
  });
  $('ed-pin').addEventListener('click', () => {
    const on = !$('ed-pin').classList.contains('on');
    $('ed-pin').classList.toggle('on', on);
    $('ed-pin').textContent = on ? S.nt_unpin : S.nt_pin;
  });
  $('ed-cancel').addEventListener('click', closeEditor);
  $('ed-close').addEventListener('click', closeEditor);
  $('ed-delete').addEventListener('click', () => {
    const id = editing;
    const note = notes.get(id);
    closeEditor();
    if (note) {
      dropNote(id);
      render();
      toast(S.nt_deleted, () => {
        putNote(id, note);
        render();
      });
    }
  });
  $('editor').addEventListener('click', (event) => {
    if (event.target === $('editor')) closeEditor();
  });

  // ------------------------------------------------------------------------
  // Neu, Suche, Sortierung, Export
  // ------------------------------------------------------------------------

  function addNote() {
    const id = newId();
    const now = Date.now();
    putNote(id, { k: 'n', title: '', body: '', pinned: false, when: now, edited: now });
    render();
    openEditor(id);
  }

  $('new-btn').addEventListener('click', addNote);
  $('search').addEventListener('input', () => {
    query = $('search').value.trim();
    render();
  });
  $('sort').addEventListener('change', () => {
    settings.sort = $('sort').value;
    saveNote();
    render();
  });

  $('export').addEventListener('click', () => {
    const lines = ['# ' + S.nt_title, ''];
    for (const [, note] of ordered()) {
      lines.push('## ' + (note.title || S.nt_untitled));
      if (note.pinned) lines.push('> ' + S.nt_pinned);
      if (note.body) lines.push('', note.body);
      lines.push('');
    }
    const name = S.nt_file + '-' + new Date().toISOString().slice(0, 10) + '.md';
    Z.saveFile(name, new Blob([lines.join('\n')], { type: 'text/markdown;charset=utf-8' })).then((ok) => toast(ok ? S.nt_exported : S.nt_save_failed));
  });

  // ---- Meldung mit „Rückgängig" ----------------------------------------------

  let toastTimer = 0;
  function toast(message, revert) {
    clearTimeout(toastTimer);
    undo = revert || null;
    $('toast-text').textContent = message;
    $('toast-undo').hidden = !revert;
    $('toast-undo').textContent = S.nt_undo;
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

  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && !$('editor').hidden) closeEditor();
  });

  $('title').textContent = S.nt_title;
  $('new-btn').textContent = S.nt_new;
  $('search').placeholder = S.nt_search;
  $('search').setAttribute('aria-label', S.nt_search);
  $('sort').setAttribute('aria-label', S.nt_sort);
  $('export').textContent = S.nt_export;
  $('trouble-text').textContent = S.nt_load_failed;
  $('trouble-retry').textContent = S.nt_retry;
  document.title = S.app_notes;

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
      settings.sort = ['edited', 'created', 'title'].includes(note.value.sort) ? note.value.sort : 'edited';
      notes.clear();
      for (const [id, record] of records.list) {
        if (id.startsWith('n-')) {
          const clean = cleanNote(record);
          if (clean) notes.set(id, clean);
        }
      }
      ready = true;
      render();
    });
  }

  $('trouble-retry').addEventListener('click', load);
  let awaySince = 0;
  document.addEventListener('visibilitychange', () => {
    if (document.hidden) awaySince = Date.now();
    else if (kept && awaySince && Date.now() - awaySince > 1500 && !editing) load();
  });

  load();
})();
