// Sicherheitscheck – eine eingebaute App.
//
// Man gibt die Adresse einer Website ein; der Browser (nicht diese Seite) ruft
// sie ab und prüft gefahrlos ihre Schutzschichten (src/security.rs). Gedacht,
// damit man die eigenen Seiten härten kann. Die Bewertung ist dieselbe wie
// hinter der Security-Ampel, um ein paar Befunde aus dem eigenen Abruf ergänzt.
// Geprüfte Adressen merkt sich der Browser als Karteikarten.
(() => {
  'use strict';
  const Z = window.ZunderApp;
  const S = Z.strings;
  const $ = (id) => document.getElementById(id);
  const text = (key, values) =>
    String(S[key] || '').replace(/\{(\w+)\}/g, (whole, name) => (values && name in values ? values[name] : whole));

  let kept = false;
  let recent = []; // [{ id, url, when }]
  let busyUrl = ''; // die Adresse, die gerade geprüft wird

  const newId = () => 's-' + Date.now().toString(36) + Math.random().toString(36).slice(2, 8);

  function node(tag, className, content) {
    const element = document.createElement(tag);
    if (className) element.className = className;
    if (content !== undefined) element.textContent = content;
    return element;
  }

  function icon(name, cls) {
    const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    if (cls) svg.setAttribute('class', cls);
    const use = document.createElementNS('http://www.w3.org/2000/svg', 'use');
    use.setAttribute('href', '#i-' + name);
    svg.appendChild(use);
    return svg;
  }

  // ------------------------------------------------------------------------
  // Feste Texte
  // ------------------------------------------------------------------------

  $('title').textContent = S.app_security;
  const input = $('url-input');
  input.placeholder = S.se_placeholder;
  input.setAttribute('aria-label', S.se_placeholder);
  $('check').textContent = S.se_check;
  $('own').textContent = S.se_own;
  $('intro-title').textContent = S.se_intro_title;
  $('intro-text').textContent = S.se_intro_text;
  $('busy-text').textContent = S.se_checking;
  $('rescan').textContent = S.se_again;
  $('note').textContent = S.se_note;
  document.title = S.app_security;

  // ------------------------------------------------------------------------
  // Geprüfte Adressen merken
  // ------------------------------------------------------------------------

  function remember(url) {
    if (!kept) return;
    // Dieselbe Adresse rückt nach vorn statt sich zu doppeln.
    const existing = recent.find((entry) => entry.url === url);
    if (existing) {
      existing.when = Date.now();
      Z.records.put(existing.id, { url, when: existing.when });
    } else {
      const entry = { id: newId(), url, when: Date.now() };
      recent.unshift(entry);
      Z.records.put(entry.id, { url, when: entry.when });
    }
    recent.sort((a, b) => b.when - a.when);
    // Höchstens zwölf; ältere werden vergessen.
    for (const gone of recent.splice(12)) Z.records.remove(gone.id);
    renderRecent();
  }

  function forget(entry) {
    recent = recent.filter((other) => other.id !== entry.id);
    Z.records.remove(entry.id);
    renderRecent();
  }

  function shortUrl(url) {
    return url.replace(/^https?:\/\//, '').replace(/\/$/, '');
  }

  function renderRecent() {
    const box = $('recent');
    box.textContent = '';
    if (!recent.length) {
      $('private-note').hidden = kept;
      $('private-note').textContent = S.se_private;
      return;
    }
    $('private-note').hidden = true;
    box.appendChild(node('span', 'hint', S.se_recent + ':'));
    for (const entry of recent.slice(0, 12)) {
      const chip = node('span', 'chip');
      const open = node('button', 'open', shortUrl(entry.url));
      open.type = 'button';
      open.addEventListener('click', () => {
        input.value = entry.url;
        start(entry.url);
      });
      const drop = node('button', 'drop');
      drop.type = 'button';
      drop.title = S.se_remove;
      drop.setAttribute('aria-label', S.se_remove);
      drop.appendChild(icon('close'));
      drop.addEventListener('click', () => forget(entry));
      chip.append(open, drop);
      box.appendChild(chip);
    }
  }

  // ------------------------------------------------------------------------
  // Prüfen
  // ------------------------------------------------------------------------

  function show(which) {
    for (const id of ['intro', 'busy', 'trouble', 'result']) $(id).hidden = id !== which;
  }

  function trouble(message) {
    show('trouble');
    $('trouble-text').textContent = message;
  }

  function start(raw) {
    const url = (raw === undefined ? input.value : raw).trim();
    if (!url || busyUrl) return;
    busyUrl = url;
    $('check').disabled = true;
    show('busy');
    Z.scan(url).then((answer) => {
      // Nur die Antwort auf die zuletzt gestartete Prüfung zählt.
      if (busyUrl !== url) return;
      busyUrl = '';
      $('check').disabled = false;
      if (answer.state === 'ok') {
        remember(url);
        renderResult(answer);
      } else if (answer.state === 'bad_url') {
        trouble(S.se_bad_url);
      } else if (answer.state === 'unreachable') {
        trouble(S.se_unreachable);
      } else if (answer.state === 'refused' && answer.why === 'stealth') {
        trouble(S.se_stealth);
      } else if (answer.state === 'busy') {
        trouble(S.se_busy);
      } else {
        trouble(S.se_failed);
      }
    });
  }

  $('search-form').addEventListener('submit', (event) => {
    event.preventDefault();
    start();
  });
  $('rescan').addEventListener('click', () => start($('rescan').dataset.url || input.value));

  // ------------------------------------------------------------------------
  // Ergebnis zeigen
  // ------------------------------------------------------------------------

  // Symbol je Stufe eines Befunds: 0 ernst, 1 Schwäche, 2 Hinweis, 3 in Ordnung.
  const PIP = ['warning', 'shield', 'info', 'check'];

  function renderResult(answer) {
    show('result');
    const level = Number(answer.level) || 0;
    $('verdict-dot').className = 'l' + level;
    const word = answer.partial && level === 1 ? S.sc_title_partial : S['sc_title_' + level] || '';
    $('verdict-word').textContent = word;
    const count = Number(answer.count) || 0;
    $('verdict-count').textContent = count > 0 ? text('se_count', { n: count }) : answer.partial ? S.se_partial : S.se_solid;
    $('rescan').dataset.url = answer.final || '';

    // Was wirklich geprüft wurde – samt Weiterleitung und Status.
    const checked = $('checked');
    checked.textContent = text('se_checked', { url: shortUrl(answer.final || ''), code: answer.status || '?' });
    if (answer.redirected) checked.append('  ·  ' + text('se_redirected', { from: shortUrl(input.value.trim()) }));

    // Nach Stufe gruppiert: erst das Ernste, zuletzt das, was in Ordnung ist.
    const groups = $('groups');
    groups.textContent = '';
    const items = Array.isArray(answer.items) ? answer.items : [];
    for (let level = 0; level <= 3; level++) {
      const part = items.filter((item) => Number(item.l) === level);
      if (!part.length) continue;
      const group = node('div', 'group');
      group.appendChild(node('h3', '', S['se_g_' + level] || ''));
      for (const item of part) {
        const finding = node('div', 'finding l' + level);
        const pip = node('span', 'pip');
        pip.appendChild(icon(PIP[level] || 'info'));
        finding.append(pip, node('b', '', String(item.t || '')), node('p', '', String(item.d || '')));
        group.appendChild(finding);
      }
      groups.appendChild(group);
    }
  }

  // ------------------------------------------------------------------------
  // Los
  // ------------------------------------------------------------------------

  Z.records.all().then((cards) => {
    kept = cards.kept;
    if (cards.ok) {
      for (const [id, record] of cards.list) {
        if (id.startsWith('s-') && record && typeof record.url === 'string') {
          recent.push({ id, url: record.url.slice(0, 2000), when: Number(record.when) || 0 });
        }
      }
      recent.sort((a, b) => b.when - a.when);
    }
    renderRecent();
    input.focus();
  });
})();
