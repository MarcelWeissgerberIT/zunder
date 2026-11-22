// Zunders Skript in jeder Seite. Es läuft in einer eigenen, von der Seite
// abgeschotteten Welt und nur im Hauptrahmen: Die Skripte der Seite sehen weder
// diese Funktionen noch den Nachrichtenkanal zum Browser.
//
// Aufgabe: Anmeldeformulare erkennen, dem Browser melden, was dort passiert
// (Feld angeklickt, Formular abgeschickt), und auf seine Anweisung ausfüllen.
// Von sich aus füllt das Skript nie etwas aus, und gesicherte Passwörter kennt
// es nicht – es bekommt genau das eine, das gerade ausgefüllt werden soll.
(function () {
  'use strict';
  if (window.top !== window) return;
  if (location.protocol !== 'http:' && location.protocol !== 'https:') return;
  var bridge = window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.zunder;
  // Schon eingerichtet? Gefragt wird nach einer eigenen Eigenschaft dieser
  // Welt – ein Element der Seite, das zufällig oder absichtlich „__zv“ heißt,
  // zählt nicht.
  if (!bridge || Object.prototype.hasOwnProperty.call(globalThis, '__zv')) return;

  function post(message) {
    try {
      bridge.postMessage(JSON.stringify(message));
    } catch (error) {
      /* Seite wird gerade verlassen */
    }
  }

  // ------------------------------------------------------------------------
  // Felder erkennen
  // ------------------------------------------------------------------------

  // Felder, die einmal ein Passwortfeld waren – „Passwort anzeigen“ macht aus
  // ihnen vorübergehend ein gewöhnliches Textfeld.
  var wasPassword = new WeakSet();

  function isInput(el) {
    return el instanceof HTMLInputElement;
  }

  function isPassword(el) {
    if (!isInput(el)) return false;
    if (el.type === 'password') {
      wasPassword.add(el);
      return true;
    }
    return wasPassword.has(el);
  }

  function isTextual(el) {
    if (!isInput(el) || isPassword(el)) return false;
    var type = (el.getAttribute('type') || 'text').toLowerCase();
    return type === 'text' || type === 'email' || type === 'tel';
  }

  function visible(el) {
    if (!el || !el.isConnected) return false;
    var box = el.getBoundingClientRect();
    if (box.width < 8 || box.height < 8) return false;
    var style = getComputedStyle(el);
    return style.visibility !== 'hidden' && style.display !== 'none' && parseFloat(style.opacity) > 0.05;
  }

  function editable(el) {
    return visible(el) && !el.disabled && !el.readOnly;
  }

  function words(el) {
    return (
      (el.getAttribute('autocomplete') || '') +
      ' ' +
      (el.name || '') +
      ' ' +
      (el.id || '') +
      ' ' +
      (el.getAttribute('placeholder') || '') +
      ' ' +
      (el.getAttribute('aria-label') || '')
    ).toLowerCase();
  }

  function autocomplete(el) {
    return (el.getAttribute('autocomplete') || '').toLowerCase();
  }

  function looksLikeUser(el) {
    if (!isTextual(el)) return false;
    var ac = autocomplete(el);
    if (/(^|\s)(username|email)(\s|$)/.test(ac)) return true;
    if (/one-time-code|cc-|search|postal|street|address-|tel|given-name|family-name/.test(ac)) return false;
    if (el.type === 'email') return true;
    return /user|login|e-?mail|kennung|benutzer|konto|account|identifier|anmeldename|member/.test(words(el));
  }

  function isNewPassword(el) {
    var ac = autocomplete(el);
    if (ac.indexOf('new-password') !== -1) return true;
    if (ac.indexOf('current-password') !== -1) return false;
    return /(^|[^a-z])(new|neu|neues)([^a-z]|$)|new[-_]?pass|neues[-_]?pass|pass(word|wort)?[-_]?(new|neu)|confirm|repeat|wiederhol|bestaetig|bestätig|again|retype|regist|sign-?up|create/.test(
      words(el)
    );
  }

  // Der Bereich, in dem ein Feld mit seinen Nachbarn zusammengehört: sein
  // Formular – oder, wo es keines gibt, das ganze Dokument.
  function scopeOf(el) {
    if (el && el.form) return el.form;
    var root = el && el.getRootNode ? el.getRootNode() : document;
    return root && root.querySelectorAll ? root : document;
  }

  // Das Element, das ein Ereignis wirklich betrifft – auch innerhalb von Schattenbäumen.
  function origin(event) {
    var path = event.composedPath ? event.composedPath() : null;
    return path && path.length ? path[0] : event.target;
  }

  function inputsIn(scope) {
    return Array.prototype.slice.call(scope.querySelectorAll('input'));
  }

  function passwordsIn(scope) {
    return inputsIn(scope).filter(function (el) {
      return isPassword(el) && visible(el);
    });
  }

  // Ordnet die Passwortfelder eines Bereichs: das bisherige und die neuen.
  function sortPasswords(scope) {
    var fields = passwordsIn(scope);
    var result = { current: null, fresh: [] };
    if (!fields.length) return result;
    if (fields.length === 1) {
      if (isNewPassword(fields[0])) result.fresh = fields;
      else result.current = fields[0];
    } else if (fields.length === 2) {
      if (autocomplete(fields[0]).indexOf('current-password') !== -1 || (!isNewPassword(fields[0]) && isNewPassword(fields[1]) && !/confirm|repeat|wiederhol|again|retype|verify/.test(words(fields[1])))) {
        result.current = fields[0];
        result.fresh = [fields[1]];
      } else {
        // Registrierung: Passwort und Wiederholung.
        result.fresh = fields;
      }
    } else {
      result.current = fields[0];
      result.fresh = fields.slice(1);
    }
    return result;
  }

  // Das Namensfeld zu einem Passwortfeld: das nächste Textfeld davor. Beim
  // Lesen zählen auch versteckte Felder – bei Anmeldungen in zwei Schritten
  // steht der Name dort für Passwortmanager bereit.
  function userFieldFor(password, forFilling) {
    var inputs = inputsIn(scopeOf(password));
    var index = inputs.indexOf(password);
    var fallback = null;
    for (var i = index - 1; i >= 0; i--) {
      var el = inputs[i];
      if (!isTextual(el)) continue;
      if (forFilling) {
        if (!editable(el)) continue;
      } else if (!visible(el) && !(el.value && looksLikeUser(el))) {
        continue;
      }
      if (looksLikeUser(el)) return el;
      if (!fallback && (forFilling || visible(el))) fallback = el;
    }
    return fallback;
  }

  // Ein Namensfeld ohne Passwortfeld daneben (erster Schritt einer Anmeldung in zwei Schritten).
  function loneUserField(scope) {
    if (passwordsIn(scope).length) return null;
    var fields = inputsIn(scope).filter(function (el) {
      return isTextual(el) && editable(el);
    });
    if (!fields.length || fields.length > 2) return null;
    for (var i = 0; i < fields.length; i++) {
      if (looksLikeUser(fields[i])) return fields[i];
    }
    return null;
  }

  // Was ist dieses Feld für eine Anmeldung? 'password', 'new', 'username' oder nichts.
  function roleOf(el) {
    if (!isInput(el) || !editable(el)) return null;
    var scope = scopeOf(el);
    if (isPassword(el)) {
      var sorted = sortPasswords(scope);
      return sorted.fresh.indexOf(el) !== -1 ? 'new' : 'password';
    }
    if (!isTextual(el)) return null;
    var passwords = passwordsIn(scope);
    for (var i = 0; i < passwords.length; i++) {
      if (userFieldFor(passwords[i], true) === el) return sortPasswords(scope).current ? 'username' : null;
    }
    return loneUserField(scope) === el ? 'username' : null;
  }

  // ------------------------------------------------------------------------
  // Überblick für den Browser: Gibt es hier etwas zum Anmelden?
  // ------------------------------------------------------------------------

  var lastForms = '';
  var scanTimer = null;

  function scan() {
    scanTimer = null;
    var login = false;
    var signup = false;
    var fields = document.querySelectorAll('input[type=password]');
    for (var i = 0; i < fields.length && i < 40; i++) {
      if (!visible(fields[i])) continue;
      isPassword(fields[i]);
      if (roleOf(fields[i]) === 'new') signup = true;
      else login = true;
    }
    var user = false;
    if (!login && !signup) {
      var all = document.forms;
      for (var f = 0; f < all.length && f < 20 && !user; f++) user = !!loneUserField(all[f]);
    }
    var summary = (login ? 'l' : '') + (signup ? 's' : '') + (user ? 'u' : '');
    if (summary !== lastForms) {
      lastForms = summary;
      post({ t: 'forms', login: login, signup: signup, user: user });
    }
    checkGone();
  }

  function scanSoon() {
    if (scanTimer === null) scanTimer = setTimeout(scan, 350);
  }

  // ------------------------------------------------------------------------
  // Feld angeklickt: Der Browser zeigt darunter an, was er dazu hat
  // ------------------------------------------------------------------------

  var focused = null; // zuletzt angeklicktes Anmeldefeld
  var shown = false;
  var filling = false;
  var gesture = 0; // wann der Mensch zuletzt selbst geklickt oder getippt hat

  // Vorschläge erscheinen nur, wenn kurz zuvor ein Mensch geklickt oder eine
  // Taste gedrückt hat. Eine Seite kann ein Feld auch von sich aus aktivieren –
  // das allein soll keine Sprechblase heraufbeschwören. Ob ein Ereignis echt
  // ist, sagt der Browser; nachstellen lässt sich das von der Seite aus nicht.
  function noteGesture(event) {
    if (event.isTrusted) gesture = Date.now();
  }

  window.addEventListener('mousedown', noteGesture, true);
  window.addEventListener('keydown', noteGesture, true);
  window.addEventListener('touchstart', noteGesture, true);

  function announce(el) {
    if (filling) return;
    if (Date.now() - gesture > 1500) {
      // Trotzdem merken, welches Feld gemeint ist – für den Schlüssel-Knopf.
      if (roleOf(el)) focused = el;
      return;
    }
    var role = roleOf(el);
    if (!role) {
      hide();
      return;
    }
    focused = el;
    shown = true;
    var box = el.getBoundingClientRect();
    post({ t: 'focus', kind: role, x: box.left, y: box.top, w: box.width, h: box.height, empty: !el.value });
  }

  function hide() {
    if (!shown) return;
    shown = false;
    post({ t: 'blur' });
  }

  window.addEventListener(
    'focusin',
    function (event) {
      var el = origin(event);
      if (isInput(el)) announce(el);
      else hide();
    },
    true
  );

  window.addEventListener(
    'mousedown',
    function (event) {
      var el = origin(event);
      // Ein Klick in das schon aktive Feld zeigt den Vorschlag erneut.
      if (isInput(el) && el === (el.getRootNode ? el.getRootNode().activeElement : document.activeElement)) announce(el);
      else if (!isInput(el)) hide();
    },
    true
  );

  // Wer selbst tippt, braucht keinen Vorschlag mehr – und die Blase stünde dem
  // nächsten Feld im Weg.
  window.addEventListener(
    'input',
    function (event) {
      if (!filling && isInput(origin(event))) hide();
    },
    true
  );

  // Rollt die Seite oder ändert sich ihre Größe (auch nur, weil oben ein
  // Hinweis erscheint oder verschwindet), wandert die Blase mit dem Feld mit –
  // und verschwindet erst, wenn das Feld nicht mehr zu sehen oder nicht mehr
  // aktiv ist.
  var moveTimer = null;

  function follow() {
    moveTimer = null;
    if (!shown) return;
    var root = focused && focused.getRootNode ? focused.getRootNode() : document;
    if (!focused || root.activeElement !== focused || !visible(focused)) {
      hide();
      return;
    }
    var box = focused.getBoundingClientRect();
    if (box.bottom < 0 || box.top > window.innerHeight) {
      hide();
      return;
    }
    post({ t: 'move', x: box.left, y: box.top, w: box.width, h: box.height });
  }

  function followSoon() {
    if (shown && moveTimer === null) moveTimer = setTimeout(follow, 60);
  }

  window.addEventListener('scroll', followSoon, true);
  window.addEventListener('resize', followSoon, true);

  // ------------------------------------------------------------------------
  // Abgeschickt: Zugangsdaten melden – der Browser fragt dann, ob er sie sichern soll
  // ------------------------------------------------------------------------

  var lastSent = '';
  var lastSentAt = 0;
  var watched = null; // Passwortfeld der zuletzt gemeldeten Anmeldung
  var watchedUntil = 0;
  var watchTimer = null;

  function collect(scope) {
    var sorted = sortPasswords(scope);
    var field = null;
    var kind = 'login';
    for (var i = 0; i < sorted.fresh.length; i++) {
      if (sorted.fresh[i].value) {
        field = sorted.fresh[i];
        kind = 'new';
        break;
      }
    }
    if (!field && sorted.current && sorted.current.value) field = sorted.current;
    if (!field) return null;
    var first = sorted.current || sorted.fresh[0] || field;
    var user = userFieldFor(first, false);
    // Beim Ändern eines Passworts: das bisherige – daran erkennt der Browser, um welches Konto es geht.
    var old = kind === 'new' && sorted.current && sorted.current.value ? String(sorted.current.value) : '';
    return { field: field, kind: kind, u: user ? String(user.value || '').trim() : '', p: String(field.value), o: old };
  }

  // `sure`: wirklich abgeschickt (Formular, Eingabetaste ohne Formular) – nicht
  // bloß ein Klick, der auch etwas anderes bedeuten kann.
  function report(scope, sure) {
    var data = collect(scope);
    if (!data) {
      // Erster Schritt einer Anmeldung in zwei Schritten: nur der Name.
      var lone = loneUserField(scope);
      if (lone && lone.value) post({ t: 'user', u: String(lone.value).trim().slice(0, 320) });
      return;
    }
    if (data.p.length > 1000 || data.u.length > 320 || data.o.length > 1000) return;
    var signature = (sure ? '1' : '0') + data.kind + '\n' + data.u + '\n' + data.p;
    var now = Date.now();
    if (signature === lastSent && now - lastSentAt < 3000) return;
    lastSent = signature;
    lastSentAt = now;
    post({ t: 'submit', kind: data.kind, u: data.u, p: data.p, o: data.o, sure: !!sure });
    watched = data.field;
    watchedUntil = now + 20000;
    if (watchTimer === null) watchTimer = setInterval(checkGone, 500);
  }

  // ------------------------------------------------------------------------
  // Seiten-Icon
  // ------------------------------------------------------------------------
  //
  // Das Icon der Seite holt nicht Zunders Oberfläche aus dem Netz – das
  // verriete der Website jedes Mal, wenn jemand den Verlauf öffnet oder ihre
  // Adresse tippt. Stattdessen zeichnet dieses Skript es einmal, in der Seite
  // selbst, auf 32 × 32 Punkte und reicht das frisch erzeugte Bild weiter.
  // Icons von fremden Rechnern, die das nicht erlauben, gibt es dann nicht –
  // an ihrer Stelle steht der Anfangsbuchstabe.
  var iconTried = '';

  function bestIcon() {
    var best = null;
    var bestScore = -1;
    var links = document.querySelectorAll('link[rel][href]');
    for (var i = 0; i < links.length && i < 200; i++) {
      var rel = (links[i].getAttribute('rel') || '').toLowerCase().split(/\s+/);
      var isIcon = rel.indexOf('icon') !== -1;
      var isTouch = rel.indexOf('apple-touch-icon') !== -1;
      if (!isIcon && !isTouch) continue;
      var href = String(links[i].href || '');
      if (!/^https?:/.test(href) || href.length > 2000) continue;
      var score = isIcon ? 2 : 1;
      var size = /(\d+)x\d+/.exec(links[i].getAttribute('sizes') || '');
      if (size) {
        var pixels = parseInt(size[1], 10);
        score += pixels >= 32 && pixels <= 128 ? 2 : pixels > 128 ? 1 : 0;
      }
      if (score > bestScore) {
        bestScore = score;
        best = href;
      }
    }
    return best || location.origin + '/favicon.ico';
  }

  function sendIcon() {
    var url = bestIcon();
    if (!url || url === iconTried) return;
    iconTried = url;
    var image = new Image();
    image.crossOrigin = 'anonymous';
    image.referrerPolicy = 'no-referrer';
    image.onload = function () {
      try {
        var canvas = document.createElement('canvas');
        canvas.width = 32;
        canvas.height = 32;
        canvas.getContext('2d').drawImage(image, 0, 0, 32, 32);
        var data = canvas.toDataURL('image/png');
        if (data.length <= 12000) post({ t: 'icon', d: data });
      } catch (error) {
        /* fremdes Bild ohne Erlaubnis: kein Icon */
      }
    };
    image.src = url;
  }

  // Was jemand in ein Passwortfeld tippt, geht kurz nach der letzten Taste an
  // den Browser. Er vergleicht es mit den gesicherten Passwörtern ANDERER
  // Websites und warnt, noch bevor abgeschickt wird – das sicherste Zeichen
  // einer nachgebauten Seite. Was Zunder selbst ausfüllt, zählt nicht.
  var typedTimer = null;
  var typedLast = '';
  window.addEventListener(
    'input',
    function (event) {
      var el = origin(event);
      if (!event.isTrusted || !isPassword(el)) return;
      clearTimeout(typedTimer);
      typedTimer = setTimeout(function () {
        var value = String(el.value || '');
        if (value.length < 8 || value.length > 1000 || value === typedLast) return;
        typedLast = value;
        post({ t: 'typed', p: value });
      }, 400);
    },
    true
  );

  // Verschwindet das Passwortfeld nach dem Abschicken, ohne dass die Seite neu
  // lädt, war die Anmeldung vermutlich erfolgreich – der Browser darf fragen.
  function checkGone() {
    if (!watched) return;
    if (!visible(watched)) {
      post({ t: 'gone' });
    } else if (Date.now() < watchedUntil) {
      return;
    }
    watched = null;
    clearInterval(watchTimer);
    watchTimer = null;
  }

  window.addEventListener(
    'submit',
    function (event) {
      if (event.target instanceof HTMLFormElement) report(event.target, true);
    },
    true
  );

  window.addEventListener(
    'keydown',
    function (event) {
      var el = origin(event);
      if (event.key === 'Escape') hide();
      // In einem Formular folgt auf die Eingabetaste das Abschicken selbst – nur ohne Formular zählt sie allein.
      if (event.key === 'Enter' && isInput(el)) report(scopeOf(el), !el.form);
    },
    true
  );

  window.addEventListener(
    'click',
    function (event) {
      var el = origin(event);
      if (!(el instanceof Element)) return;
      // Klicks in Felder schicken nichts ab.
      if (el.closest('input:not([type=submit]):not([type=button]):not([type=image]), textarea, select, label')) return;
      var form = el.closest('form');
      if (form) {
        report(form, false);
        return;
      }
      // Ohne Formular: nur Knöpfe zählen.
      if (el.closest('button, [role=button], input, a')) report(scopeOf(el), false);
    },
    true
  );

  // ------------------------------------------------------------------------
  // Ausfüllen – nur auf Anweisung des Browsers
  // ------------------------------------------------------------------------

  var nativeValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value');

  function setValue(el, value) {
    try {
      el.focus({ preventScroll: true });
    } catch (error) {
      /* Feld nimmt den Fokus nicht an */
    }
    nativeValue.set.call(el, value);
    el.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertReplacementText', data: value }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
    if (el.value !== value) {
      // Die Seite hat den Wert zurückgesetzt – wie getippt einfügen.
      try {
        el.select();
        document.execCommand('insertText', false, value);
      } catch (error) {
        /* bleibt beim ersten Versuch */
      }
    }
    return el.value === value;
  }

  function firstEditable(list) {
    for (var i = 0; i < list.length; i++) if (editable(list[i])) return list[i];
    return null;
  }

  // Wohin ausgefüllt wird: in den Bereich des zuletzt angeklickten Feldes –
  // sonst in das erste Anmeldeformular der Seite.
  function target(mode) {
    var anchor = focused && editable(focused) && roleOf(focused) ? focused : null;
    if (!anchor) {
      var all = Array.prototype.slice.call(document.querySelectorAll('input[type=password]')).filter(editable);
      if (mode === 'new') {
        anchor = firstEditable(all.filter(function (el) { return roleOf(el) === 'new'; }));
      } else {
        anchor = firstEditable(all.filter(function (el) { return roleOf(el) === 'password'; })) || firstEditable(all);
      }
    }
    if (!anchor) {
      // Kein Passwortfeld: vielleicht der erste Schritt mit dem Namen allein.
      var forms = [document].concat(Array.prototype.slice.call(document.forms));
      for (var f = 0; f < forms.length && !anchor; f++) anchor = loneUserField(forms[f]);
      return anchor ? { user: anchor, passwords: [] } : null;
    }
    var scope = scopeOf(anchor);
    var sorted = sortPasswords(scope);
    if (!isPassword(anchor)) {
      // Angeklickt ist das Namensfeld.
      var current = sorted.current && editable(sorted.current) ? [sorted.current] : [];
      return { user: anchor, passwords: mode === 'new' ? [] : current };
    }
    var passwords;
    if (mode === 'new') {
      // Alle Felder für das neue Passwort (samt Wiederholung) – oder genau das angeklickte.
      passwords = sorted.fresh.indexOf(anchor) !== -1 ? sorted.fresh.filter(editable) : [anchor];
    } else if (sorted.fresh.indexOf(anchor) !== -1 || !sorted.current || !editable(sorted.current) || inputsIn(scope).indexOf(anchor) === -1) {
      passwords = [anchor];
    } else {
      passwords = [sorted.current];
    }
    var first = passwords[0] || anchor;
    return { user: mode === 'new' ? null : userFieldFor(first, true), passwords: passwords };
  }

  // request: { origin, u, p, mode: 'login' | 'new' }
  function fill(request) {
    // Hier fällt die Entscheidung, ob das Passwort in dieses Dokument darf: nur
    // im Hauptrahmen und nur, wenn seine Herkunft die ist, für die der Browser
    // es herausgibt. `self.origin` ist die wirkliche Herkunft – ein Dokument,
    // das die Website eigens abgeschottet hat, zählt damit nicht zu ihr.
    if (window.top !== window || self.origin !== request.origin) return 'origin';
    var where = target(request.mode);
    if (!where) return 'nofield';
    var done = 0;
    filling = true;
    try {
      if (where.user && request.u && request.mode !== 'new') {
        if (where.user.value === request.u || setValue(where.user, request.u)) done += 1;
      }
      for (var i = 0; i < where.passwords.length; i++) {
        if (setValue(where.passwords[i], request.p)) done += 1;
      }
    } finally {
      filling = false;
    }
    shown = false;
    return done ? (where.passwords.length ? 'ok' : 'user') : 'nofield';
  }

  // Für ein neu vorgeschlagenes Passwort: Welcher Name steht gerade im Formular?
  function currentUser() {
    var where = target('new');
    var first = where && where.passwords[0];
    if (!first) return '';
    var sorted = sortPasswords(scopeOf(first));
    var user = userFieldFor(sorted.current || sorted.fresh[0] || first, false);
    return user ? String(user.value || '').trim().slice(0, 320) : '';
  }

  // Der Browser zeigt seine Vorschläge am Schlüssel-Knopf statt am Feld: Auch
  // dann soll ein Klick in die Seite sie wieder schließen.
  function expectHide() {
    shown = true;
  }

  Object.defineProperty(globalThis, '__zv', {
    value: Object.freeze({ fill: fill, user: currentUser, shown: expectHide }),
    writable: false,
    configurable: false,
    enumerable: false
  });

  // ------------------------------------------------------------------------
  // Lohnt sich der Lesemodus? Nur ein Hinweis – danach richtet sich, ob die
  // Adressleiste den Knopf dafür zeigt.
  // ------------------------------------------------------------------------

  var lastReadable = null;
  var readableChecks = 0;

  function checkReadable() {
    if (readableChecks++ > 5) return;
    var blocks = document.querySelectorAll('p');
    var long = 0;
    var total = 0;
    for (var i = 0; i < blocks.length && i < 3000; i++) {
      var length = (blocks[i].textContent || '').trim().length;
      if (length >= 80) {
        long += 1;
        total += length;
      }
    }
    var on = long >= 4 && total >= 1500;
    if (on !== lastReadable) {
      lastReadable = on;
      post({ t: 'readable', on: on });
    }
  }

  // ------------------------------------------------------------------------
  // In welcher Sprache die Seite nach eigener Angabe steht. Auch nur ein
  // Hinweis – danach bietet die Adressleiste das Übersetzen an.
  // ------------------------------------------------------------------------

  var lastLang = null;

  function checkLang() {
    var root = document.documentElement;
    var lang = String((root && root.getAttribute('lang')) || '').toLowerCase().slice(0, 12);
    if (lang !== lastLang) {
      lastLang = lang;
      post({ t: 'lang', v: lang });
    }
  }

  // ------------------------------------------------------------------------
  // Start
  // ------------------------------------------------------------------------

  new MutationObserver(scanSoon).observe(document, { childList: true, subtree: true });
  document.addEventListener('DOMContentLoaded', function () {
    scan();
    checkReadable();
    checkLang();
  });
  window.addEventListener('load', function () {
    sendIcon();
    scan();
    checkReadable();
    checkLang();
    // Seiten, die ihren Text erst nach dem Laden einsetzen.
    setTimeout(checkReadable, 2500);
  });
  window.addEventListener('pageshow', function () {
    lastForms = '';
    lastReadable = null;
    readableChecks = 0;
    lastLang = null;
    scanSoon();
    checkReadable();
    checkLang();
  });
})();
