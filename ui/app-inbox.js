// Posteingang – eine eingebaute App. Zunder als Client für Atlas.
//
// Die Seite geht nicht ins Netz (src/apps.rs: connect leer). Den Abruf macht der
// Browser (`src/inbox.rs`, `Z.api`): Der Zugangstoken bleibt im Schlüsselbund,
// die Seite sieht ihn nie; die Serveradresse ist eine Einstellung des Browsers.
// Offene Vorgänge liegen zusätzlich als Karteikarten lokal vor – so ist der
// Posteingang sofort da und auch ohne Netz lesbar.
(() => {
  'use strict';
  const Z = window.ZunderApp;
  const S = Z.strings;
  const LANG = Z.data.lang === 'de' ? 'de' : 'en';
  const $ = (id) => document.getElementById(id);
  const text = (key, values) =>
    String(S[key] || '').replace(/\{(\w+)\}/g, (whole, name) => (values && name in values ? values[name] : whole));

  let base = '';
  let hasToken = false;
  let loading = false;
  const items = new Map(); // id (r-<ref>) → { ref, title, kind, status, project, slug, href, summary }
  const settings = { view: 'list' }; // 'list' | 'board' (Spalten nach Status)

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
  // Ablage (Karteikarten als Zwischenspeicher)
  // ------------------------------------------------------------------------

  const idOf = (ref) => 'r-' + String(ref);

  function cacheClean(value) {
    if (!value || typeof value !== 'object') return null;
    const ref = Number(value.ref);
    if (!Number.isFinite(ref)) return null;
    return {
      ref,
      title: Z.clip(value.title, 300),
      kind: Z.clip(value.kind, 40),
      status: Z.clip(value.status, 40),
      project: Z.clip(value.project, 80),
      slug: Z.clip(value.slug, 160),
      href: Z.clip(value.href, 300),
      summary: Z.clip(value.summary, 400),
      displayRef: Number(value.displayRef) || undefined,
    };
  }

  // ------------------------------------------------------------------------
  // Anzeige
  // ------------------------------------------------------------------------

  function linkFor(item) {
    // Deep-Link auf den Record. Fällt die Serveradresse weg, kein Link.
    if (!base) return '';
    if (item.ref) return base + '/r/' + encodeURIComponent(item.ref);
    if (item.href) return base + item.href;
    return '';
  }

  function saveNote() {
    Z.save({ v: 1, view: settings.view });
  }

  // Der Umschalter Liste | Board in der Kopfzeile.
  function buildViewToggle() {
    const view = $('view');
    view.textContent = '';
    for (const [key, label] of [['list', S.ib_view_list], ['board', S.ib_view_board]]) {
      const button = node('button', settings.view === key ? 'on' : '', label);
      button.type = 'button';
      button.addEventListener('click', () => {
        if (settings.view === key) return;
        settings.view = key;
        saveNote();
        render();
      });
      view.appendChild(button);
    }
  }

  function render() {
    buildViewToggle();
    const all = [...items.values()];
    $('count').textContent = all.length ? text('ib_count', { n: all.length }) : '';
    const empty = !all.length;
    $('empty').hidden = !empty;
    $('list').hidden = empty || settings.view !== 'list';
    $('view-board').hidden = empty || settings.view !== 'board';
    if (empty) {
      $('empty').textContent = loading ? S.ib_loading : S.ib_empty;
      return;
    }
    if (settings.view === 'board') renderBoard(all);
    else renderList(all);
  }

  // Liste: nach Projekt gruppiert, Gruppen alphabetisch, innerhalb Ref absteigend.
  function renderList(all) {
    const list = $('list');
    list.textContent = '';
    const groups = new Map();
    for (const item of all) {
      const key = item.project || '–';
      if (!groups.has(key)) groups.set(key, []);
      groups.get(key).push(item);
    }
    for (const key of [...groups.keys()].sort((a, b) => a.localeCompare(b))) {
      const group = node('div', 'group');
      group.appendChild(node('h2', '', key));
      const rows = groups.get(key).sort((a, b) => b.ref - a.ref);
      for (const item of rows) group.appendChild(itemEl(item));
      list.appendChild(group);
    }
  }

  // Board: Spalten nach Status. Read-only – Klick öffnet den Vorgang im Tab, kein
  // Ziehen (ein Statuswechsel wäre ein Schreibvorgang mit Atlas-Übergangsregeln).
  const STATUS_ORDER = ['todo', 'backlog', 'unplanned', 'open', 'in progress', 'in review', 'review', 'blocked', 'done', 'closed'];
  function statusRank(status) {
    const at = STATUS_ORDER.indexOf(String(status || '').toLowerCase());
    return at === -1 ? STATUS_ORDER.length : at;
  }
  function renderBoard(all) {
    const wrap = $('view-board');
    wrap.textContent = '';
    const cols = new Map();
    for (const item of all) {
      const key = item.status || '–';
      if (!cols.has(key)) cols.set(key, []);
      cols.get(key).push(item);
    }
    const keys = [...cols.keys()].sort((a, b) => statusRank(a) - statusRank(b) || a.localeCompare(b));
    for (const key of keys) {
      const column = node('div', 'column');
      const head = node('div', 'col-head');
      head.appendChild(node('span', 'name', key));
      head.appendChild(node('span', 'col-count', String(cols.get(key).length)));
      column.appendChild(head);
      const box = node('div', 'col-cards');
      for (const item of cols.get(key).sort((a, b) => b.ref - a.ref)) box.appendChild(itemEl(item));
      column.appendChild(box);
      wrap.appendChild(column);
    }
  }

  function itemEl(item) {
    const element = node('div', 'item');
    element.setAttribute('role', 'button');
    element.tabIndex = 0;
    element.title = S.ib_open_hint;
    element.addEventListener('click', () => openPreview(item));
    element.addEventListener('keydown', (event) => {
      if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        openPreview(item);
      }
    });

    const body = node('div', 'body');
    const head = node('div', 'head');
    if (item.kind) head.appendChild(node('span', 'chip kind', item.kind));
    head.appendChild(node('span', 'item-title', item.title || '–'));
    body.appendChild(head);
    if (item.summary) body.appendChild(node('p', 'summary', item.summary));

    const meta = node('div', 'meta');
    if (item.status) meta.appendChild(node('span', 'chip', item.status));
    if (item.ref) meta.appendChild(node('span', 'ref', '#' + (item.displayRef || item.ref)));
    if (meta.children.length) body.appendChild(meta);

    element.appendChild(body);
    const go = node('span', 'go');
    go.appendChild(icon('eye'));
    element.appendChild(go);
    return element;
  }

  // ------------------------------------------------------------------------
  // Schnellvorschau eines Vorgangs (read-only). Klick auf eine Karte holt den
  // Record (`GET /api/ingest?ref=…`) und zeigt ihn hier – ohne in die Web-App zu
  // wechseln; „Im Tab öffnen" führt bei Bedarf dorthin.
  // ------------------------------------------------------------------------

  function fillMeta(box, rec) {
    box.textContent = '';
    if (rec.kind) box.appendChild(node('span', 'chip kind', rec.kind));
    if (rec.status) box.appendChild(node('span', 'chip', rec.status));
    if (rec.project) box.appendChild(node('span', 'chip', rec.project));
    if (rec.displayRef || rec.ref) box.appendChild(node('span', 'chip', '#' + (rec.displayRef || rec.ref)));
    if (typeof rec.assignee === 'string' && rec.assignee) box.appendChild(node('span', 'chip', S.ib_assignee + ': ' + rec.assignee));
    if (typeof rec.due === 'string' && rec.due) box.appendChild(node('span', 'chip', S.ib_due + ': ' + rec.due));
  }

  function showTags(tags) {
    const box = $('pv-tags');
    box.textContent = '';
    if (Array.isArray(tags)) for (const tag of tags.slice(0, 24)) box.appendChild(node('span', 'chip', '#' + tag));
    box.hidden = !box.children.length;
  }

  function previewLink(ref, href) {
    return linkFor({ ref, href });
  }

  async function openPreview(item) {
    $('pv-title').textContent = item.title || '–';
    fillMeta($('pv-meta'), item);
    showTags(null);
    $('pv-body').textContent = S.ib_loading;
    const href = previewLink(item.ref, item.href);
    $('pv-open').hidden = !href;
    if (href) $('pv-open').href = href;
    $('preview').hidden = false;
    setTimeout(() => $('pv-close').focus(), 0);
    if (!item.ref) {
      $('pv-body').textContent = S.ib_preview_error;
      return;
    }
    const answer = await Z.api({ action: 'get', path: '/api/ingest', query: 'ref=' + encodeURIComponent(item.ref) });
    if ($('preview').hidden) return; // inzwischen geschlossen
    let rec = null;
    if (answer.state === 'ok' && answer.status >= 200 && answer.status < 300) {
      try {
        const parsed = JSON.parse(answer.body || '{}');
        rec = parsed.item || parsed.record || (parsed.items && parsed.items[0]) || parsed;
      } catch (error) {
        rec = null;
      }
    }
    if (rec && typeof rec === 'object') {
      $('pv-title').textContent = rec.title || item.title || '–';
      fillMeta($('pv-meta'), Object.assign({}, item, rec));
      showTags(rec.tags);
      const lead = typeof rec.summary === 'string' && rec.summary ? rec.summary + '\n\n' : '';
      const body = typeof rec.body === 'string' ? rec.body : '';
      $('pv-body').textContent = (lead + body).trim() || S.ib_no_body;
      const freshHref = previewLink(rec.ref || item.ref, rec.href || item.href);
      $('pv-open').hidden = !freshHref;
      if (freshHref) $('pv-open').href = freshHref;
    } else {
      $('pv-body').textContent = S.ib_preview_error;
    }
  }

  function closePreview() {
    $('preview').hidden = true;
  }

  // ------------------------------------------------------------------------
  // Banner und Meldung
  // ------------------------------------------------------------------------

  function banner(message, action) {
    $('banner-text').textContent = message || '';
    $('banner').hidden = !message;
    const button = $('banner-action');
    if (action) {
      button.textContent = action.label;
      button.hidden = false;
      button.onclick = action.run;
    } else {
      button.hidden = true;
      button.onclick = null;
    }
  }

  let toastTimer = 0;
  function toast(message) {
    clearTimeout(toastTimer);
    $('toast-text').textContent = message;
    $('toast').hidden = false;
    toastTimer = setTimeout(() => ($('toast').hidden = true), 4000);
  }

  // ------------------------------------------------------------------------
  // Abruf
  // ------------------------------------------------------------------------

  async function reconcile(fresh) {
    const seen = new Set();
    for (const raw of fresh) {
      const item = cacheClean(raw);
      if (!item) continue;
      const id = idOf(item.ref);
      if (raw.displayRef) item.displayRef = Number(raw.displayRef) || undefined;
      seen.add(id);
      items.set(id, item);
      Z.records.put(id, item);
    }
    for (const id of [...items.keys()]) {
      if (!seen.has(id)) {
        items.delete(id);
        Z.records.remove(id);
      }
    }
  }

  async function loadInbox() {
    if (!base || !hasToken) {
      needSetup();
      return;
    }
    loading = true;
    banner('');
    render();
    const answer = await Z.api({ action: 'get', path: '/api/ingest', query: 'kind=feature,ticket,bug' });
    loading = false;

    if (answer.state === 'ok') {
      if (answer.status >= 200 && answer.status < 300) {
        let fresh = [];
        try {
          const parsed = JSON.parse(answer.body || '{}');
          fresh = Array.isArray(parsed) ? parsed : parsed.items || parsed.rows || parsed.records || [];
        } catch (error) {
          banner(S.ib_error, { label: S.ib_retry, run: loadInbox });
          return;
        }
        await reconcile(fresh);
        banner('');
        render();
      } else if (answer.status === 401) {
        banner(S.ib_unauthorized, { label: S.ib_settings, run: openSetup });
        render();
      } else if (answer.status === 403) {
        banner(S.ib_forbidden);
        render();
      } else {
        banner(text('ib_bad_status', { status: answer.status }), { label: S.ib_retry, run: loadInbox });
        render();
      }
    } else if (answer.state === 'no_base' || answer.state === 'no_token') {
      needSetup();
    } else if (answer.state === 'refused' && answer.why === 'stealth') {
      banner(S.ib_stealth);
      render();
    } else if (answer.state === 'unreachable') {
      banner(S.ib_unreachable, { label: S.ib_retry, run: loadInbox });
      render();
    } else if (answer.state === 'busy') {
      setTimeout(loadInbox, 1200);
    } else {
      banner(S.ib_error, { label: S.ib_retry, run: loadInbox });
      render();
    }
  }

  function needSetup() {
    banner(S.ib_setup_hint, { label: S.ib_settings, run: openSetup });
    render();
  }

  // ------------------------------------------------------------------------
  // Einrichtung
  // ------------------------------------------------------------------------

  function openSetup() {
    $('server').value = base;
    $('token').value = '';
    $('signout').hidden = !hasToken;
    $('status-line').textContent = hasToken ? S.ib_connected : S.ib_no_token;
    $('status-line').classList.toggle('good', hasToken);
    $('setup').hidden = false;
    setTimeout(() => $(base ? 'token' : 'server').focus(), 0);
  }

  function closeSetup() {
    $('setup').hidden = true;
  }

  $('setup-form').addEventListener('submit', async (event) => {
    event.preventDefault();
    const server = $('server').value.trim();
    if (!/^https?:\/\/.+/i.test(server)) {
      $('status-line').textContent = S.ib_server_missing;
      $('status-line').classList.remove('good');
      return;
    }
    const payload = { action: 'set', base: server };
    const token = $('token').value;
    if (token) payload.token = token;
    const answer = await Z.api(payload);
    if (answer.state === 'ok') {
      base = typeof answer.base === 'string' ? answer.base : server;
      hasToken = answer.hasToken === true;
      toast(S.ib_saved);
      closeSetup();
      loadInbox();
    } else if (answer.state === 'private') {
      toast(S.ib_private);
    } else {
      $('status-line').textContent = S.ib_error;
      $('status-line').classList.remove('good');
    }
  });

  $('signout').addEventListener('click', async () => {
    const answer = await Z.api({ action: 'clear' });
    if (answer.state === 'ok') {
      hasToken = false;
      $('signout').hidden = true;
      $('status-line').textContent = S.ib_no_token;
      $('status-line').classList.remove('good');
    } else if (answer.state === 'private') {
      toast(S.ib_private);
    }
  });

  $('setup-close').addEventListener('click', closeSetup);
  $('cancel').addEventListener('click', closeSetup);
  $('setup').addEventListener('click', (event) => {
    if (event.target === $('setup')) closeSetup();
  });
  $('pv-close').addEventListener('click', closePreview);
  $('preview').addEventListener('click', (event) => {
    if (event.target === $('preview')) closePreview();
  });
  $('settings-btn').addEventListener('click', openSetup);
  $('refresh').addEventListener('click', loadInbox);
  document.addEventListener('keydown', (event) => {
    if (event.key !== 'Escape') return;
    if (!$('preview').hidden) closePreview();
    else if (!$('setup').hidden) closeSetup();
  });

  // ------------------------------------------------------------------------
  // Start
  // ------------------------------------------------------------------------

  $('title').textContent = S.ib_title;
  $('refresh').textContent = S.ib_refresh;
  $('settings-btn').textContent = S.ib_settings;
  $('setup-title').textContent = S.ib_setup_title;
  $('setup-hint').textContent = S.ib_setup_hint;
  $('server-label').textContent = S.ib_server_label;
  $('server-hint').textContent = S.ib_server_hint;
  $('token-label').textContent = S.ib_token_label;
  $('token-hint').textContent = S.ib_token_hint;
  $('save').textContent = S.ib_save;
  $('pv-open').textContent = S.ib_open_tab;
  $('pv-close').title = S.ib_close;
  $('cancel').textContent = S.ib_cancel;
  $('signout').textContent = S.ib_signout;
  document.title = S.app_inbox || S.ib_title;

  async function start() {
    // Erst den Zwischenspeicher zeigen (sofort da, auch ohne Netz).
    const cached = await Z.records.all();
    for (const [id, value] of cached.list) {
      const item = cacheClean(value);
      if (item) items.set(id, item);
    }
    // Gemerkte Ansicht (Liste/Board) vom Zettel der App holen.
    const note = await Z.load();
    if (note.ok && note.value && note.value.view === 'board') settings.view = 'board';
    render();
    // Dann den Stand holen (Adresse + ob ein Token hinterlegt ist) und abrufen.
    const status = await Z.api({ action: 'status' });
    if (status.state === 'ok') {
      base = typeof status.base === 'string' ? status.base : '';
      hasToken = status.hasToken === true;
    }
    loadInbox();
  }

  start();
})();
