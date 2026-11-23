// Gemeinsamer Teil der eingebauten Apps: der Draht zum Browser.
//
// Eine App kann den Browser um genau das bitten, was für sie eingetragen ist
// (src/apps.rs): ihren Zettel lesen und schreiben, ihre Karteikarten, den
// Standort – nach Zustimmung, gerundet. Die Bitte geht über den Kanal der
// Seiten hinaus; die Antwort kommt als Aufruf von `window.__zunderApp` zurück.
(() => {
  'use strict';
  const DATA = JSON.parse(document.getElementById('data').textContent);
  const waiting = new Map();
  // Die Nummern beginnen irgendwo: Eine Antwort, die noch dem Dokument davor
  // galt (neu geladen, während der Browser fragte), trifft so keine Anfrage hier.
  let next = 1 + Math.floor(Math.random() * 0x3fffffff);

  // Eine Anfrage kann mehrere Antworten bekommen: erst den Zwischenstand
  // („der Browser fragt gerade“, „macOS ortet“), dann das Ergebnis.
  window.__zunderApp = (answer) => {
    const entry = answer && typeof answer === 'object' ? waiting.get(answer.n) : null;
    if (!entry) return;
    if (entry.done(answer)) {
      waiting.delete(answer.n);
      clearTimeout(entry.timer);
      entry.resolve(answer);
    } else if (entry.progress) {
      entry.progress(answer);
    }
  };

  function call(op, extra, options) {
    const settings = options || {};
    return new Promise((resolve) => {
      const n = next++;
      const timer = setTimeout(() => {
        waiting.delete(n);
        resolve({ state: 'timeout' });
      }, settings.timeout || 8000);
      waiting.set(n, { resolve, timer, progress: settings.progress, done: settings.done || (() => true) });
      try {
        window.webkit.messageHandlers.ipc.postMessage('A:' + JSON.stringify(Object.assign({ n, op }, extra || {})));
      } catch (error) {
        clearTimeout(timer);
        waiting.delete(n);
        resolve({ state: 'failed' });
      }
    });
  }

  // Ein Objekt aus seinem JSON-Text – alles andere ist ein leeres.
  function objectOf(text) {
    try {
      const parsed = JSON.parse(text || '{}');
      if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) return parsed;
    } catch (error) {
      /* Unlesbares ist leer */
    }
    return {};
  }

  window.ZunderApp = {
    data: DATA,
    strings: DATA.strings,

    // Text so kürzen, dass er Text bleibt: nach Zeichen, nie mitten in einem
    // Zeichenpaar (ein halbes Emoji nähme der Browser nicht an) und ohne
    // Steuerzeichen.
    clip(text, max) {
      const out = [];
      for (const char of String(text == null ? '' : text)) {
        const code = char.charCodeAt(0);
        if (code < 32 || code === 127) continue;
        if (char.length === 1 && code >= 0xd800 && code <= 0xdfff) continue;
        if (out.length >= max) break;
        out.push(char);
      }
      return out.join('');
    },

    // Der Zettel der App: { ok, kept, value }. `ok` ist falsch, wenn der Browser
    // nicht geantwortet hat; `kept`, wenn er sich hier nichts merkt (privates
    // Fenster).
    load() {
      return call('load').then((answer) => ({
        ok: answer.state === 'ok',
        kept: answer.state === 'ok' && answer.kept === true,
        value: objectOf(answer.data),
      }));
    },

    save(value) {
      return call('save', { data: JSON.stringify(value) }).then((answer) => answer.state === 'ok');
    },

    // Der Standort: { state: 'ok', lat, lon } – oder warum nicht ('denied',
    // 'refused', 'failed', 'timeout'). `progress` erfährt die Zwischenstände
    // 'asking' (der Browser fragt) und 'locating' (macOS ortet).
    location(progress) {
      return call('location', null, {
        timeout: 180000,
        progress,
        done: (answer) => answer.state !== 'asking' && answer.state !== 'locating',
      });
    },

    forgetLocation() {
      return call('forget');
    },

    // Sichert eine Datei, die die App selbst erzeugt hat, bei den Downloads –
    // unter diesem Namen (mit einer Endung, die für die App eingetragen ist).
    // Erst erfährt der Browser den Namen, dann geht die Datei auf den Weg.
    async saveFile(name, blob) {
      const answer = await call('filename', { name });
      if (answer.state !== 'ok') return false;
      const link = document.createElement('a');
      link.href = URL.createObjectURL(blob);
      link.download = name;
      document.body.appendChild(link);
      link.click();
      link.remove();
      setTimeout(() => URL.revokeObjectURL(link.href), 60000);
      return true;
    },

    // Die Karteikarten der App (wo für sie welche eingetragen sind): viele
    // kleine Einträge, jeder ein Objekt unter seiner Kennung aus Buchstaben,
    // Ziffern, „-“ und „_“.
    records: {
      // Alle Karten: { ok, kept, list: [[Kennung, Objekt], …] } – in mehreren
      // Zügen geholt. `ok` ist falsch, wenn die Liste nicht vollständig ist.
      async all() {
        const list = [];
        let after = '';
        for (let turn = 0; turn < 1000; turn++) {
          const answer = await call('list', { after });
          if (answer.state !== 'ok' || !Array.isArray(answer.items)) return { ok: false, kept: false, list };
          for (const item of answer.items) {
            if (Array.isArray(item) && typeof item[0] === 'string' && typeof item[1] === 'string') list.push([item[0], objectOf(item[1])]);
          }
          if (typeof answer.next !== 'string' || !answer.next) return { ok: true, kept: answer.kept === true, list };
          after = answer.next;
        }
        return { ok: false, kept: false, list };
      },

      // Legt eine Karte ab oder ersetzt sie; wirft eine weg. Die Antwort sagt,
      // was daraus wurde: 'ok', 'private' (dieses Fenster merkt sich nichts),
      // 'full', 'invalid', 'failed', 'timeout'.
      put(id, value) {
        return call('put', { id, data: JSON.stringify(value) }).then((answer) => answer.state);
      },

      remove(id) {
        return call('remove', { id }).then((answer) => answer.state);
      },
    },
  };
})();
