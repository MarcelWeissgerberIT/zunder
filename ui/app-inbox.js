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
  const settings = { view: 'list', me: '' }; // view: 'list'|'board'; me: eigener Atlas-Name (für „mir zugewiesen")
  const filter = { q: '', project: '', status: '', mine: false };
  let results = null; // null = Basis (Cache) zeigen; Array = Trefferliste (nicht gecacht)
  let filterTimer = 0;

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
    Z.save({ v: 1, view: settings.view, me: settings.me });
  }

  // ---- Suche und Filter ----------------------------------------------------
  // Alles über den schon erlaubten Pfad `/api/ingest` mit `q`/`project`/`status`
  // (serverseitig bestätigt). Ein aktiver Filter zeigt eine Trefferliste und
  // rührt den lokalen Cache (`items`) nicht an; „zurücksetzen" zeigt wieder die
  // gecachte Basis.
  const activeFilter = () => !!(filter.q || filter.project || filter.status || filter.mine);
  const baseItems = () => [...items.values()];
  const source = () => (results !== null ? results : baseItems());

  // Nur Unreserviertes + %XX durchlassen, damit die nativ geprüfte Query es
  // annimmt (encodeURIComponent lässt !'()* stehen).
  function enc(value) {
    return encodeURIComponent(value).replace(/[!'()*]/g, (c) => '%' + c.charCodeAt(0).toString(16).toUpperCase());
  }

  function fillSelect(sel, values, current) {
    sel.textContent = '';
    const add = (value, label) => {
      const option = document.createElement('option');
      option.value = value;
      option.textContent = label;
      sel.appendChild(option);
    };
    add('', S.ib_all);
    for (const value of values) add(value, value);
    sel.value = current && values.includes(current) ? current : '';
  }

  function populateFilters() {
    const base = baseItems();
    fillSelect($('f-project'), [...new Set(base.map((item) => item.project).filter(Boolean))].sort(), filter.project);
    fillSelect($('f-status'), [...new Set(base.map((item) => item.status).filter(Boolean))].sort(), filter.status);
  }

  async function applyFilter() {
    filter.q = $('q').value.trim();
    filter.project = $('f-project').value;
    filter.status = $('f-status').value;
    $('f-clear').hidden = !activeFilter();
    if (!activeFilter()) {
      results = null;
      banner('');
      render();
      return;
    }
    loading = true;
    render();
    let path;
    let query;
    if (filter.mine) {
      // „Mir zugewiesen" über die AQL-Suche. Ob der Token das darf, entscheidet
      // der Server (sonst 401 → „kein Zugriff"). Projekt/Status werden in die
      // AQL gehängt; die Volltextsuche gilt nur im normalen Modus.
      const safe = (value) => String(value).replace(/"/g, '');
      let aql = 'assignee = "' + safe(settings.me) + '"';
      if (filter.project) aql += ' AND project = "' + safe(filter.project) + '"';
      if (filter.status) aql += ' AND status = "' + safe(filter.status) + '"';
      path = '/api/query';
      query = 'aql=' + enc(aql);
    } else {
      path = '/api/ingest';
      const parts = ['kind=feature,ticket,bug'];
      if (filter.q) parts.push('q=' + enc(filter.q));
      if (filter.project) parts.push('project=' + enc(filter.project));
      if (filter.status) parts.push('status=' + enc(filter.status));
      query = parts.join('&');
    }
    const answer = await Z.api({ action: 'get', path, query });
    loading = false;
    if (answer.state === 'ok' && answer.status >= 200 && answer.status < 300) {
      let fresh = [];
      try {
        const parsed = JSON.parse(answer.body || '{}');
        fresh = Array.isArray(parsed) ? parsed : parsed.items || parsed.rows || [];
      } catch (error) {
        banner(S.ib_error, { label: S.ib_retry, run: applyFilter });
        return;
      }
      results = fresh.map(cacheClean).filter(Boolean);
      banner('');
      render();
    } else if (answer.state === 'unreachable') {
      banner(S.ib_unreachable, { label: S.ib_retry, run: applyFilter });
      render();
    } else if (answer.status === 401 || answer.status === 403) {
      // Im „mir zugewiesen"-Modus heißt 401/403: der Token darf die Suche nicht.
      if (filter.mine) banner(S.ib_no_access);
      else banner(answer.status === 401 ? S.ib_unauthorized : S.ib_forbidden, answer.status === 401 ? { label: S.ib_settings, run: openSetup } : null);
      render();
    } else {
      banner(S.ib_error, { label: S.ib_retry, run: applyFilter });
      render();
    }
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
    const all = source();
    $('count').textContent = all.length ? text('ib_count', { n: all.length }) : '';
    $('filters').hidden = !(base && hasToken);
    $('f-clear').hidden = !activeFilter();
    $('f-mine').classList.toggle('on', filter.mine);
    $('f-mine').setAttribute('aria-pressed', String(filter.mine));
    const empty = !all.length;
    $('empty').hidden = !empty;
    $('list').hidden = empty || settings.view !== 'list';
    $('view-board').hidden = empty || settings.view !== 'board';
    if (empty) {
      $('empty').textContent = activeFilter() ? S.ib_no_results : loading ? S.ib_loading : S.ib_empty;
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
  // Neuen Vorgang anlegen (POST /api/ingest über Z.api action:'create')
  // ------------------------------------------------------------------------

  const KINDS = ['feature', 'ticket', 'bug', 'task'];

  function openCreate() {
    const kind = $('cr-kind');
    kind.textContent = '';
    for (const value of KINDS) {
      const option = document.createElement('option');
      option.value = value;
      option.textContent = value;
      kind.appendChild(option);
    }
    kind.value = 'ticket';
    // Projekt aus den bekannten Projekten; Standard „(erstes Projekt)" = der
    // Server wählt (das Projekt bestimmt auch den Tenant).
    const projects = [...new Set(baseItems().map((item) => item.project).filter(Boolean))].sort();
    const sel = $('cr-project');
    sel.textContent = '';
    const first = document.createElement('option');
    first.value = '';
    first.textContent = S.ib_first_project;
    sel.appendChild(first);
    for (const value of projects) {
      const option = document.createElement('option');
      option.value = value;
      option.textContent = value;
      sel.appendChild(option);
    }
    sel.value = '';
    $('cr-title').value = '';
    $('cr-body').value = '';
    $('cr-tags').value = '';
    $('cr-status').textContent = '';
    $('create').hidden = false;
    setTimeout(() => $('cr-title').focus(), 0);
  }

  function closeCreate() {
    $('create').hidden = true;
  }

  $('create-form').addEventListener('submit', async (event) => {
    event.preventDefault();
    const title = $('cr-title').value.trim();
    if (!title) {
      $('cr-status').textContent = S.ib_title_required;
      return;
    }
    const record = { title, kind: $('cr-kind').value };
    const project = $('cr-project').value;
    if (project) record.project = project;
    const body = $('cr-body').value.trim();
    if (body) record.body = body;
    const tags = $('cr-tags').value.split(',').map((tag) => tag.trim()).filter(Boolean);
    if (tags.length) record.tags = tags;
    if (settings.me) record.actor = settings.me;
    $('cr-status').textContent = S.ib_loading;
    const answer = await Z.api({ action: 'create', record });
    if (answer.state === 'ok' && answer.status >= 200 && answer.status < 300) {
      closeCreate();
      toast(S.ib_created);
      loadInbox();
    } else if (answer.status === 401 || answer.status === 403) {
      $('cr-status').textContent = S.ib_no_access;
    } else {
      $('cr-status').textContent = S.ib_create_failed;
    }
  });
  $('cr-close').addEventListener('click', closeCreate);
  $('cr-cancel').addEventListener('click', closeCreate);
  $('create').addEventListener('click', (event) => {
    if (event.target === $('create')) closeCreate();
  });

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
        populateFilters();
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
    $('me').value = settings.me;
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
    settings.me = $('me').value.trim();
    saveNote();
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
  $('new-btn').addEventListener('click', openCreate);
  $('refresh').addEventListener('click', loadInbox);
  $('q').addEventListener('input', () => {
    clearTimeout(filterTimer);
    filterTimer = setTimeout(applyFilter, 350);
  });
  $('f-project').addEventListener('change', applyFilter);
  $('f-status').addEventListener('change', applyFilter);
  $('f-clear').addEventListener('click', () => {
    $('q').value = '';
    $('f-project').value = '';
    $('f-status').value = '';
    filter.mine = false;
    applyFilter();
  });
  $('f-mine').addEventListener('click', () => {
    if (!filter.mine && !settings.me) {
      toast(S.ib_need_name);
      openSetup();
      return;
    }
    filter.mine = !filter.mine;
    applyFilter();
  });
  document.addEventListener('keydown', (event) => {
    if (event.key !== 'Escape') return;
    if (!$('create').hidden) closeCreate();
    else if (!$('preview').hidden) closePreview();
    else if (!$('setup').hidden) closeSetup();
  });

  // ------------------------------------------------------------------------
  // Start
  // ------------------------------------------------------------------------

  $('title').textContent = S.ib_title;
  $('refresh').textContent = S.ib_refresh;
  $('settings-btn').textContent = S.ib_settings;
  $('q').placeholder = S.ib_search;
  $('q').setAttribute('aria-label', S.ib_search);
  $('f-project').setAttribute('aria-label', S.ib_project);
  $('f-status').setAttribute('aria-label', S.ib_status);
  $('f-clear').textContent = S.ib_filter_clear;
  $('f-mine').textContent = S.ib_mine;
  $('me-label').textContent = S.ib_me_label;
  $('me-hint').textContent = S.ib_me_hint;
  $('new-btn').textContent = S.ib_new;
  $('cr-heading').textContent = S.ib_new_title;
  $('cr-title-label').textContent = S.ib_f_title;
  $('cr-kind-label').textContent = S.ib_kind;
  $('cr-project-label').textContent = S.ib_project;
  $('cr-body-label').textContent = S.ib_f_body;
  $('cr-tags-label').textContent = S.ib_f_tags;
  $('cr-cancel').textContent = S.ib_cancel;
  $('cr-save').textContent = S.ib_create;
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
    if (note.ok && note.value) {
      if (note.value.view === 'board') settings.view = 'board';
      if (typeof note.value.me === 'string') settings.me = note.value.me;
    }
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
