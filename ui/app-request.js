// Request-Inspektor – eine eingebaute App.
//
// Man stellt eine HTTP-Anfrage zusammen (Methode, Adresse, Kopfzeilen, Rumpf);
// der Browser (nicht diese Seite) schickt sie und zeigt die rohe Antwort
// (src/inspect.rs). Wie „curl“, nur im Browser – für die eigenen Schnittstellen.
// Weitergeleitet wird nicht. Zuletzt benutzte Anfragen (Methode und Adresse)
// merkt sich der Browser als Karteikarten.
(() => {
  'use strict';
  const Z = window.ZunderApp;
  const S = Z.strings;
  const $ = (id) => document.getElementById(id);
  const text = (key, values) =>
    String(S[key] || '').replace(/\{(\w+)\}/g, (whole, name) => (values && name in values ? values[name] : whole));

  const METHODS = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'HEAD', 'OPTIONS'];
  let kept = false;
  let recent = []; // [{ id, m, u, when }]
  let busy = false;
  let shownRaw = ''; // der zuletzt gezeigte Rumpf, roh (für „formatieren“/„kopieren“)
  let formatted = false;

  const newId = () => 'r-' + Date.now().toString(36) + Math.random().toString(36).slice(2, 8);

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
  // Feste Texte und Aufbau
  // ------------------------------------------------------------------------

  $('title').textContent = S.app_request;
  const method = $('method');
  for (const name of METHODS) method.appendChild(new Option(name, name));
  $('url').placeholder = S.ri_url;
  $('url').setAttribute('aria-label', S.ri_url);
  $('send').textContent = S.ri_send;
  $('headers-title').textContent = S.ri_headers;
  $('add').textContent = '+ ' + S.ri_add_header;
  $('body-label').textContent = S.ri_body_hint;
  $('busy-text').textContent = S.ri_sending;
  $('intro-title').textContent = S.ri_intro_title;
  $('intro-text').textContent = S.ri_intro_text;
  $('rh-title').textContent = S.ri_resp_headers;
  $('rb-title').textContent = S.ri_resp_body;
  $('copy').textContent = S.ri_copy;
  $('note').textContent = S.ri_note;
  document.title = S.app_request;

  // ------------------------------------------------------------------------
  // Kopfzeilen-Zeilen
  // ------------------------------------------------------------------------

  function headerRow(name, value) {
    const row = node('div', 'hrow');
    const k = document.createElement('input');
    k.className = 'k';
    k.placeholder = S.ri_header_name;
    k.setAttribute('aria-label', S.ri_header_name);
    k.value = name || '';
    const v = document.createElement('input');
    v.className = 'v';
    v.placeholder = S.ri_header_value;
    v.setAttribute('aria-label', S.ri_header_value);
    v.value = value || '';
    const del = node('button', 'del');
    del.type = 'button';
    del.setAttribute('aria-label', S.se_remove || 'x');
    del.appendChild(icon('close'));
    del.addEventListener('click', () => {
      row.remove();
      if (!$('headers').children.length) addHeaderRow();
    });
    row.append(k, v, del);
    return row;
  }

  function addHeaderRow(name, value) {
    $('headers').appendChild(headerRow(name, value));
  }

  $('add').addEventListener('click', () => addHeaderRow());
  addHeaderRow();

  function gatherHeaders() {
    const out = [];
    for (const row of $('headers').querySelectorAll('.hrow')) {
      const inputs = row.querySelectorAll('input');
      const name = inputs[0].value.trim();
      const value = inputs[1].value.trim();
      if (name) out.push([name.slice(0, 100), value.slice(0, 2000)]);
    }
    return out.slice(0, 50);
  }

  // ------------------------------------------------------------------------
  // Senden
  // ------------------------------------------------------------------------

  function show(which) {
    for (const id of ['intro', 'busy', 'trouble', 'result']) $(id).hidden = id !== which;
  }

  function trouble(message) {
    show('trouble');
    $('trouble-text').textContent = message;
  }

  function send() {
    if (busy) return;
    const url = $('url').value.trim();
    if (!url) return;
    const req = { method: method.value, url, headers: gatherHeaders(), body: $('body').value };
    busy = true;
    $('send').disabled = true;
    show('busy');
    Z.request(req).then((answer) => {
      busy = false;
      $('send').disabled = false;
      if (answer.state === 'ok') {
        remember(req.method, req.url);
        renderResponse(answer);
      } else if (answer.state === 'too_big') {
        trouble(S.ri_too_big);
      } else if (answer.state === 'bad') {
        trouble(S.ri_bad);
      } else if (answer.state === 'unreachable') {
        trouble(S.ri_unreachable);
      } else if (answer.state === 'refused' && answer.why === 'stealth') {
        trouble(S.ri_stealth);
      } else if (answer.state === 'busy') {
        trouble(S.ri_busy);
      } else {
        trouble(S.ri_failed);
      }
    });
  }

  $('form').addEventListener('submit', (event) => {
    event.preventDefault();
    send();
  });
  // ⌘↵ sendet auch aus dem Rumpf.
  $('body').addEventListener('keydown', (event) => {
    if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
      event.preventDefault();
      send();
    }
  });

  // ------------------------------------------------------------------------
  // Antwort zeigen
  // ------------------------------------------------------------------------

  function statusClass(code) {
    if (code >= 200 && code < 300) return 'ok';
    if (code >= 300 && code < 400) return 'warn';
    if (code >= 400) return 'bad';
    return '';
  }

  function renderResponse(answer) {
    show('result');
    const code = Number(answer.status) || 0;
    $('status-dot').className = statusClass(code);
    $('status').textContent = code + (answer.statusText ? ' ' + answer.statusText : '');
    const parts = [text('ri_time', { n: answer.timeMs || 0 }), text('ri_bytes', { n: answer.bytes || 0 })];
    if (answer.truncated) parts.push(S.ri_truncated);
    $('meta').textContent = parts.join('  ·  ');

    // Eine Weiterleitung folgt der Inspektor nicht – darauf weisen wir hin.
    $('redirect').hidden = !(code >= 300 && code < 400);
    $('redirect').textContent = S.ri_redirect;

    const headers = $('resp-headers');
    headers.textContent = '';
    for (const pair of Array.isArray(answer.headers) ? answer.headers : []) {
      const row = node('div', 'rh');
      row.append(node('span', 'n', String(pair[0] || '')), node('span', 'v', String(pair[1] || '')));
      headers.appendChild(row);
    }

    // Der Rumpf: Text, Binär-Hinweis oder „kein Rumpf“.
    const format = $('format');
    formatted = false;
    shownRaw = '';
    if (answer.binary) {
      $('resp-body').textContent = text('ri_binary', { n: answer.bytes || 0 });
      format.hidden = true;
      $('copy').hidden = true;
    } else if (!answer.body) {
      $('resp-body').textContent = S.ri_empty_body;
      format.hidden = true;
      $('copy').hidden = true;
    } else {
      shownRaw = String(answer.body);
      $('resp-body').textContent = shownRaw;
      $('copy').hidden = false;
      $('copy').textContent = S.ri_copy;
      // „JSON formatieren“ nur, wenn der Rumpf wirklich JSON ist.
      let pretty = null;
      try {
        pretty = JSON.stringify(JSON.parse(shownRaw), null, 2);
      } catch (error) {
        pretty = null;
      }
      format.hidden = pretty === null;
      format.textContent = S.ri_format;
      format.dataset.pretty = pretty === null ? '' : pretty;
    }
  }

  $('format').addEventListener('click', () => {
    formatted = !formatted;
    const pretty = $('format').dataset.pretty;
    $('resp-body').textContent = formatted && pretty ? pretty : shownRaw;
    $('format').textContent = formatted ? S.ri_raw : S.ri_format;
  });

  $('copy').addEventListener('click', () => {
    const value = $('resp-body').textContent;
    const done = () => {
      $('copy').textContent = S.ri_copied;
      setTimeout(() => ($('copy').textContent = S.ri_copy), 1500);
    };
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(value).then(done, () => fallbackCopy(value, done));
    } else {
      fallbackCopy(value, done);
    }
  });

  function fallbackCopy(value, done) {
    const area = document.createElement('textarea');
    area.value = value;
    area.style.position = 'fixed';
    area.style.opacity = '0';
    document.body.appendChild(area);
    area.select();
    try {
      if (document.execCommand('copy')) done();
    } catch (error) {
      /* dann eben nicht */
    }
    area.remove();
  }

  // ------------------------------------------------------------------------
  // Zuletzt benutzte Anfragen (Methode und Adresse – keine Kopfzeilen, kein Rumpf)
  // ------------------------------------------------------------------------

  function shortUrl(url) {
    return url.replace(/^https?:\/\//, '');
  }

  function remember(m, u) {
    if (!kept) return;
    const same = recent.find((entry) => entry.m === m && entry.u === u);
    if (same) {
      same.when = Date.now();
      Z.records.put(same.id, { m, u, when: same.when });
    } else {
      const entry = { id: newId(), m, u, when: Date.now() };
      recent.unshift(entry);
      Z.records.put(entry.id, { m, u, when: entry.when });
    }
    recent.sort((a, b) => b.when - a.when);
    for (const gone of recent.splice(12)) Z.records.remove(gone.id);
    renderRecent();
  }

  function forget(entry) {
    recent = recent.filter((other) => other.id !== entry.id);
    Z.records.remove(entry.id);
    renderRecent();
  }

  function renderRecent() {
    const box = $('recent');
    box.textContent = '';
    if (!recent.length) {
      $('private-note').hidden = kept;
      $('private-note').textContent = S.ri_private;
      return;
    }
    $('private-note').hidden = true;
    box.appendChild(node('span', 'head', S.ri_recent));
    for (const entry of recent.slice(0, 12)) {
      const chip = node('span', 'chip');
      const open = node('button', 'open');
      open.type = 'button';
      open.append(node('span', 'm', entry.m + ' '), document.createTextNode(shortUrl(entry.u)));
      open.addEventListener('click', () => {
        method.value = METHODS.includes(entry.m) ? entry.m : 'GET';
        $('url').value = entry.u;
        $('url').focus();
      });
      const del = node('button', 'del');
      del.type = 'button';
      del.setAttribute('aria-label', S.se_remove || 'x');
      del.appendChild(icon('close'));
      del.addEventListener('click', () => forget(entry));
      chip.append(open, del);
      box.appendChild(chip);
    }
  }

  // ------------------------------------------------------------------------
  // Los
  // ------------------------------------------------------------------------

  Z.records.all().then((cards) => {
    kept = cards.kept;
    if (cards.ok) {
      for (const [id, record] of cards.list) {
        if (id.startsWith('r-') && record && typeof record.u === 'string' && typeof record.m === 'string') {
          recent.push({ id, m: record.m.slice(0, 10), u: record.u.slice(0, 2000), when: Number(record.when) || 0 });
        }
      }
      recent.sort((a, b) => b.when - a.when);
    }
    renderRecent();
    $('url').focus();
  });
})();
