// Läuft in Zunders abgeschotteter Skriptwelt in jeder Seite mit (nur im
// Hauptrahmen) und hilft PDF-Rahmen, die WebKit leer lässt.
//
// Web-Apps zeigen ein PDF oft in einem Rahmen mit einer blob:-Adresse – die
// Datei liegt nur im Speicher der Seite. Verbietet die Seite per
// „frame-ancestors 'none'“, dass andere sie einbetten, wendet WebKit das auch
// auf diesen Rahmen an: Er bleibt leer (andere Browser tun das nicht). Dieses
// Skript holt die Datei dann aus der Seite, gibt sie dem Browser und lässt den
// Rahmen sie unter einer eigenen Adresse laden. Regeln dazu: src/pdfs.rs.
//
// Die Skripte der Seite sehen weder dieses Skript noch seinen Schlüssel.
(function () {
  'use strict';
  if (window.top !== window) return;
  var KEY = '%KEY%';
  if (KEY.length !== 32) return;
  // Eine Seite ohne eigene Herkunft (Schutzregel „sandbox“) kommt an ihre
  // blob:-Adressen selbst nicht heran – dort gibt es nichts zu helfen.
  if (self.origin === 'null') return;

  var PUT = 'zunder-pdf://put/';
  // Die Adresse nennt bewusst nichts aus der Seite – auch keinen Dateinamen:
  // An ihr soll keine Sperrliste hängen bleiben, die nach Wörtern in Adressen sucht.
  var NAME = 'Dokument.pdf';
  var LIMIT = 64 * 1024 * 1024;
  // So oft hilft das Skript demselben Rahmen in kurzer Zeit höchstens – eine
  // Seite, die ihren Rahmen bei jedem Laden neu belegt, dreht sonst endlos.
  // Nach einer ruhigen halben Minute beginnt die Geduld von vorn.
  var PATIENCE = 6;
  var CALM = 30000;
  // So lange wartet die Reihe darauf, dass ein Rahmen sein PDF geladen hat.
  var WAIT = 4000;
  // Nimmt der Browser gerade nichts mehr an (zu viel in kurzer Zeit), kommen
  // noch drei Versuche – zusammen reichen sie über seine Sperrfrist.
  var RETRY = [4000, 9000, 17000];

  // Je Rahmen: für welche Adresse was schon geschehen ist.
  var known = new WeakMap();
  // Geholfen wird einem Rahmen nach dem anderen: Der Browser behält nur wenige
  // Dateien je Tab – viele auf einmal würden einander verdrängen, bevor ihr
  // Rahmen sie abgeholt hat.
  var queue = [];
  var active = null;
  var timer = 0;

  // Belegt die Seite einen Rahmen neu (dieselbe Adresse noch einmal), will sie
  // ihre Datei wieder sehen. Das unterscheidet sich von einem Rahmen, der von
  // selbst woandershin geht – etwa über einen Link im PDF.
  var watcher = new MutationObserver(function (records) {
    for (var i = 0; i < records.length; i++) {
      var entry = known.get(records[i].target);
      if (entry) entry.touched = true;
    }
  });

  function blobAddress(frame) {
    var src = frame.getAttribute('src') || '';
    return /^blob:/i.test(src) && src.length < 2048 ? src : '';
  }

  // Hat WebKit das Laden abgebrochen, steht im Rahmen ein leeres Dokument
  // fremder Herkunft. Ein geladenes blob:-Dokument gehört dagegen zur Seite
  // selbst und lässt sich von ihr aus ansehen.
  function emptied(frame) {
    try {
      return frame.contentWindow !== null && frame.contentDocument === null;
    } catch (error) {
      return false;
    }
  }

  function isPdf(bytes) {
    var end = Math.min(bytes.length, 1024) - 5;
    for (var i = 0; i <= end; i++) {
      // %PDF-
      if (bytes[i] === 37 && bytes[i + 1] === 80 && bytes[i + 2] === 68 && bytes[i + 3] === 70 && bytes[i + 4] === 45) {
        return true;
      }
    }
    return false;
  }

  // Gehört der Rahmen noch der Aufgabe – oder hat die Seite ihn inzwischen
  // anders belegt oder entfernt? Dann bleibt er, wie sie ihn haben will.
  function still(job) {
    return known.get(job.frame) === job.entry && blobAddress(job.frame) === job.src && job.frame.isConnected && !!job.frame.contentWindow;
  }

  function pump() {
    while (!active && queue.length) {
      var job = queue.shift();
      if (!still(job)) {
        job.entry.phase = 'left';
        continue;
      }
      active = job;
      job.entry.phase = 'busy';
      rescue(job);
    }
  }

  function finish(job) {
    if (active !== job) return;
    clearTimeout(timer);
    active = null;
    pump();
  }

  function rescue(job) {
    var frame = job.frame;
    var entry = job.entry;
    var hash = job.src.indexOf('#');
    var address = hash < 0 ? job.src : job.src.slice(0, hash);
    // Anzeigewünsche hinter der Adresse (#view=FitH …) wandern mit.
    var tail = hash < 0 ? '' : job.src.slice(hash);
    if (!/^#[\w=&.,+-]{0,200}$/.test(tail)) tail = '';

    fetch(address)
      .then(function (response) {
        var length = Number(response.headers.get('content-length') || 0);
        if (!response.ok || length > LIMIT) throw new Error('size');
        return response.arrayBuffer();
      })
      .then(function (buffer) {
        var bytes = new Uint8Array(buffer);
        if (bytes.length < 16 || bytes.length > LIMIT || !isPdf(bytes)) throw new Error('kind');
        var body = new Uint8Array(KEY.length + bytes.length);
        for (var i = 0; i < KEY.length; i++) body[i] = KEY.charCodeAt(i);
        body.set(bytes, KEY.length);
        return fetch(PUT, { method: 'POST', body: body, credentials: 'omit', cache: 'no-store', referrerPolicy: 'no-referrer' });
      })
      .then(function (response) {
        if (response.status === 429) {
          var later = new Error('busy');
          later.busy = true;
          throw later;
        }
        if (!response.ok) throw new Error('put');
        return response.text();
      })
      .then(function (token) {
        if (!/^[0-9a-f]{32}$/.test(token)) throw new Error('token');
        if (!still(job)) {
          entry.phase = 'left';
          finish(job);
          return;
        }
        entry.phase = 'moving';
        entry.touched = false;
        entry.win = frame.contentWindow;
        watcher.observe(frame, { attributes: true, attributeFilter: ['src'] });
        // Kommt kein „load“ (die Seite hat den Rahmen gerade entfernt), geht
        // die Reihe trotzdem weiter.
        timer = setTimeout(function () {
          finish(job);
        }, WAIT);
        frame.contentWindow.location.replace('zunder-pdf://' + token + '/' + NAME + tail);
      })
      .catch(function (error) {
        if (error && error.busy && job.attempt < RETRY.length) {
          entry.phase = 'waiting';
          setTimeout(function () {
            if (known.get(frame) !== entry || entry.phase !== 'waiting') return;
            entry.phase = 'queued';
            queue.push(job);
            pump();
          }, RETRY[job.attempt]);
          job.attempt += 1;
        } else {
          entry.phase = 'failed';
        }
        finish(job);
      });
  }

  // „load“ steigt nicht auf – beim Einfangen kommt es trotzdem hier vorbei.
  document.addEventListener(
    'load',
    function (event) {
      var frame = event.target;
      if (!frame || frame.localName !== 'iframe') return;
      var src = blobAddress(frame);
      if (!src || frame.hasAttribute('sandbox')) return;
      var entry = known.get(frame);
      if (!entry || entry.src !== src) {
        entry = { src: src, phase: 'new', tries: 0, last: 0, win: null, touched: false };
        known.set(frame, entry);
      }
      if (entry.phase === 'moving') {
        // Das war Zunders eigener Abruf: Das PDF steht.
        entry.phase = 'shown';
        entry.touched = false;
        if (active && active.entry === entry) finish(active);
        return;
      }
      // „fine“: Der Rahmen hat seine Datei selbst geladen – er bleibt für immer
      // in Ruhe, auch wenn die Seite ihn später woandershin schickt.
      if (entry.phase === 'fine' || entry.phase === 'busy' || entry.phase === 'queued' || entry.phase === 'waiting') return;
      if (!emptied(frame)) {
        if (entry.phase === 'new') entry.phase = 'fine';
        return;
      }
      if (entry.phase === 'shown' || entry.phase === 'away') {
        // Der Rahmen hat nach Zunders PDF noch einmal geladen. Hat die Seite
        // ihn neu belegt (dieselbe Adresse noch einmal gesetzt oder den Rahmen
        // neu eingehängt – dann hat er ein neues Fenster), will sie ihre Datei
        // wieder sehen. Sonst ist er von selbst woandershin gegangen, etwa
        // über einen Link im PDF – und bleibt dort.
        if (!entry.touched && frame.contentWindow === entry.win) {
          entry.phase = 'away';
          return;
        }
      }
      var now = Date.now();
      if (now - entry.last > CALM) entry.tries = 0;
      if (entry.tries >= PATIENCE) return;
      entry.tries += 1;
      entry.last = now;
      entry.touched = false;
      entry.phase = 'queued';
      queue.push({ frame: frame, src: src, entry: entry, attempt: 0 });
      pump();
    },
    true
  );
})();
