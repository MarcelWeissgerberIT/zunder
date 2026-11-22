// Fehlerseite. Alle Angaben stehen in der Adresse und gelten als ungeprüft:
// Sie werden nur als Text angezeigt, und weitergeleitet wird ausschließlich
// auf http(s)-Adressen.
(() => {
  'use strict';
  const S = JSON.parse(document.getElementById('data').textContent).strings;
  const $ = (id) => document.getElementById(id);
  const params = new URLSearchParams(location.search);

  const failed = params.get('u') || '';
  const code = parseInt(params.get('c') || '0', 10);
  const domain = params.get('d') || '';
  const message = params.get('m') || '';
  const engine = params.get('e') || '';

  let target = null;
  try {
    const parsed = new URL(failed);
    if (parsed.protocol === 'http:' || parsed.protocol === 'https:') target = parsed;
  } catch (error) {
    /* unbrauchbare Adresse: nur der Text bleibt */
  }
  const host = target ? target.hostname : failed;

  let kind = 'generic';
  if (domain === 'NSURLErrorDomain') {
    if (code === -1009 || code === -1020) kind = 'offline';
    else if (code === -1003 || code === -1006) kind = 'dns';
    else if (code === -1001) kind = 'timeout';
    else if (code === -1004) kind = 'refused';
    else if (code <= -1200 && code >= -1206) kind = 'tls';
  }

  const tell = (message) => {
    try {
      window.webkit.messageHandlers.ipc.postMessage(message);
    } catch (error) {}
  };
  const goBack = (button) => {
    button.textContent = S.e_back;
    button.hidden = history.length <= 1;
    button.addEventListener('click', () => history.back());
  };
  const plausibleHost = (name) => /^[a-z0-9.-]{1,253}$/.test(name);
  const variant = params.get('v') || '';

  // Täuschend ähnliche Adresse: Diese Seite kennt der Mensch am Browser nicht –
  // aber eine, die fast genauso heißt. Der sichere Weg ist der Hauptknopf.
  if (variant === 'lookalike' && target) {
    const known = params.get('k') || '';
    const knownHost = params.get('h') || '';
    if (plausibleHost(known) && plausibleHost(knownHost)) {
      document.title = host;
      $('glyph').setAttribute('href', '#i-warning');
      $('title').textContent = S.e_like_title.replace('{known}', known);
      $('text').textContent = S.e_like_text.split('{host}').join(host).split('{known}').join(known);
      const go = $('retry');
      go.textContent = S.e_like_go.replace('{known}', known);
      go.addEventListener('click', () => location.replace('https://' + knownHost + '/'));
      const open = $('alt');
      open.textContent = S.e_like_open.replace('{host}', host);
      open.hidden = false;
      open.addEventListener('click', () => {
        // Gilt, bis der Browser beendet wird – oder dauerhaft, sobald die Seite
        // im Verlauf steht.
        tell('k:' + target.hostname);
        location.replace(target.href);
      });
      return;
    }
  }

  // Eine Website will den Tab zu einem Gerät im eigenen Netz schicken.
  if (variant === 'localnet' && target) {
    const from = params.get('f') || '';
    const replay = params.get('r') === '1';
    document.title = target.host;
    $('glyph').setAttribute('href', '#i-warning');
    $('title').textContent = S.e_local_title;
    let text = (plausibleHost(from) ? S.e_local_text : S.e_local_text_unknown)
      .split('{from}').join(from).split('{host}').join(target.host);
    if (!replay) text += ' ' + S.e_local_form;
    $('text').textContent = text;
    goBack($('retry'));
    if (replay) {
      const open = $('alt');
      open.textContent = S.e_local_open.replace('{host}', target.host);
      open.hidden = false;
      open.addEventListener('click', () => location.replace(target.href));
      if (plausibleHost(from) && from.includes('.')) {
        const always = $('more');
        always.textContent = S.e_local_always.replace('{from}', from);
        always.hidden = false;
        always.addEventListener('click', () => {
          tell('L:' + from);
          location.replace(target.href);
        });
      }
    }
    return;
  }

  // „Nur verschlüsselte Verbindungen“: Diese Adresse gibt es nur unverschlüsselt
  // (der verschlüsselte Versuch ist gescheitert oder gar nicht möglich). Statt
  // sie still zu laden, fragt die Seite nach. Fehlt bloß das Netz oder gibt es
  // den Namen nicht, ist das die eigentliche Nachricht – dann weiter unten.
  let plain = null;
  try {
    const parsed = new URL(params.get('p') || '');
    if (parsed.protocol === 'http:') plain = parsed;
  } catch (error) {
    /* keine Nachfrage */
  }
  if (plain && kind !== 'offline' && kind !== 'dns') {
    document.title = plain.hostname;
    $('glyph').setAttribute('href', '#i-warning');
    $('title').textContent = S.e_insecure_title;
    $('text').textContent = S.e_insecure_text.replace('{host}', plain.hostname);

    goBack($('retry'));

    const open = $('alt');
    open.textContent = S.e_insecure_open;
    open.hidden = false;
    open.addEventListener('click', () => {
      // Der Browser merkt sich die Zustimmung für diesen Rechner, bis er
      // beendet wird. Annehmen tut er sie nur von dieser Seite.
      tell('x:' + plain.hostname);
      location.replace(plain.href);
    });
    $('details').textContent = code ? S.e_details.replace('{code}', String(code)) : '';
    return;
  }

  // Die Adresse steht als ganze Seite auf einer Filterliste (Werbe- und
  // Betrugsseiten, die sich ungefragt öffnen): Der Blocker hat sie nicht geladen.
  if (domain === 'WebKitErrorDomain' && code === 104) {
    document.title = host || S.e_title;
    $('title').textContent = S.e_blocked_title;
    $('text').textContent = S.e_blocked_text.replace('{host}', host);
    $('glyph').setAttribute('href', '#i-shield');
    goBack($('retry'));
    $('details').textContent = S.e_details.replace('{code}', String(code));
    return;
  }

  const texts = {
    offline: S.e_offline,
    dns: S.e_dns,
    timeout: S.e_timeout,
    refused: S.e_refused,
    tls: S.e_tls,
    generic: S.e_generic,
  };

  document.title = host || S.e_title;
  $('title').textContent = kind === 'offline' ? S.e_offline : S.e_title;
  $('text').textContent = kind === 'offline' ? '' : texts[kind].replace('{host}', host);
  if (kind === 'tls') $('glyph').setAttribute('href', '#i-warning');

  $('retry').textContent = S.e_retry;
  $('retry').addEventListener('click', () => {
    if (target) location.replace(target.href);
  });
  $('retry').hidden = !target;

  const alt = $('alt');
  if (target && kind === 'dns' && /^https:\/\/[^\s]+\{\}/.test(engine)) {
    // Vielleicht war es gar keine Adresse, sondern ein Suchbegriff.
    alt.textContent = S.e_search;
    alt.hidden = false;
    alt.addEventListener('click', () => {
      location.replace(engine.replace('{}', encodeURIComponent(host)));
    });
  } else if (target && target.protocol === 'https:' && (kind === 'refused' || kind === 'timeout')) {
    alt.textContent = S.e_http;
    alt.hidden = false;
    alt.addEventListener('click', () => {
      const plain = new URL(target.href);
      plain.protocol = 'http:';
      location.replace(plain.href);
    });
  }

  const details = [S.e_details.replace('{code}', String(code))];
  if (message) details.push(message);
  $('details').textContent = details.join(' · ');
})();
