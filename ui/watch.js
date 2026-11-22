// Beobachter im Stealth-Fenster: zählt mit, was eine Seite über das Gerät
// abfragt (Bildschirm, Zeichenfläche, Prozessor, Zeitzone, Cookies …), und
// meldet die Zahlen dem Browser. Die Tarn-Ampel in der Adresszeile zeigt sie.
//
// Das Skript läuft in der Welt der Seite – anders ließe sich nicht sehen, was
// ihre Skripte aufrufen. Deshalb gilt: Es ändert an keiner Antwort etwas (das
// Verwischen selbst macht WebKit), es reicht jeden Aufruf unverändert durch,
// und was es meldet, ist eine Auskunft, kein Beweis: Eine Seite, die es darauf
// anlegt, kann an ihm vorbei fragen oder falsche Zahlen melden. Mehr als die
// Anzeige beeinflusst sie damit nicht.
(() => {
  'use strict';
  let post;
  try {
    const handler = window.webkit.messageHandlers.ipc;
    post = handler.postMessage.bind(handler);
  } catch (error) {
    return;
  }
  const later = window.setTimeout.bind(window);
  const apply = Reflect.apply;
  const describe = Object.getOwnPropertyDescriptor;
  const define = Object.defineProperty;
  const text = JSON.stringify;

  const counts = Object.create(null);
  let dirty = false;
  let waiting = false;
  let sent = 0;
  function flush() {
    waiting = false;
    if (!dirty || sent >= 300) return;
    dirty = false;
    sent += 1;
    try {
      post('q:' + text(counts));
    } catch (error) {
      /* der Browser hört nicht zu – dann eben nicht */
    }
  }
  function hit(key) {
    const before = counts[key] || 0;
    if (before < 9999) counts[key] = before + 1;
    dirty = true;
    if (!waiting) {
      waiting = true;
      // Die ersten Meldungen kommen zügig, danach in Ruhe.
      later(flush, sent < 3 ? 350 : 1500);
    }
  }

  // Die Hüllen sollen aussehen wie das, was sie umhüllen: gleicher Name,
  // gleiche Länge, und als Text das Original. Sonst verriete sich das
  // Stealth-Fenster gerade durch den Beobachter.
  const originals = new WeakMap();
  function cloak(wrapper, original) {
    originals.set(wrapper, original);
    try {
      define(wrapper, 'length', { value: original.length, configurable: true });
      define(wrapper, 'name', { value: original.name, configurable: true });
    } catch (error) {
      /* unwichtig */
    }
    return wrapper;
  }
  try {
    const found = describe(Function.prototype, 'toString');
    const original = found.value;
    const holder = {
      toString() {
        return apply(original, originals.get(this) || this, []);
      },
    };
    define(Function.prototype, 'toString', {
      value: cloak(holder.toString, original),
      writable: found.writable,
      enumerable: found.enumerable,
      configurable: found.configurable,
    });
  } catch (error) {
    /* dann bleiben die Hüllen eben erkennbar */
  }

  function getter(owner, name, key) {
    try {
      const found = describe(owner, name);
      if (!found || typeof found.get !== 'function' || !found.configurable) return;
      const original = found.get;
      const holder = {
        get [name]() {
          hit(key);
          return apply(original, this, []);
        },
      };
      define(owner, name, {
        get: cloak(describe(holder, name).get, original),
        set: found.set,
        enumerable: found.enumerable,
        configurable: true,
      });
    } catch (error) {
      /* diese Eigenschaft bleibt unbeobachtet */
    }
  }

  function setter(owner, name, key) {
    try {
      const found = describe(owner, name);
      if (!found || typeof found.set !== 'function' || !found.configurable) return;
      const original = found.set;
      const holder = {
        set [name](value) {
          hit(key);
          apply(original, this, [value]);
        },
      };
      define(owner, name, {
        get: found.get,
        set: cloak(describe(holder, name).set, original),
        enumerable: found.enumerable,
        configurable: true,
      });
    } catch (error) {
      /* diese Eigenschaft bleibt unbeobachtet */
    }
  }

  function method(owner, name, key, when) {
    try {
      const found = describe(owner, name);
      if (!found || typeof found.value !== 'function' || !found.configurable) return;
      const original = found.value;
      const holder = {
        [name](...args) {
          if (!when || when(args)) hit(key);
          return apply(original, this, args);
        },
      };
      define(owner, name, {
        value: cloak(holder[name], original),
        writable: found.writable,
        enumerable: found.enumerable,
        configurable: true,
      });
    } catch (error) {
      /* diese Funktion bleibt unbeobachtet */
    }
  }

  const proto = (name) => {
    try {
      const found = window[name];
      return found && found.prototype ? found.prototype : null;
    } catch (error) {
      return null;
    }
  };

  const screenProto = proto('Screen');
  if (screenProto) {
    for (const name of ['width', 'height', 'availWidth', 'availHeight', 'colorDepth', 'pixelDepth']) getter(screenProto, name, 'screen');
  }

  const canvasProto = proto('HTMLCanvasElement');
  if (canvasProto) {
    method(canvasProto, 'toDataURL', 'canvas');
    method(canvasProto, 'toBlob', 'canvas');
    method(canvasProto, 'getContext', 'webgl', (args) => /webgl/i.test(String(args[0])));
  }
  const contextProto = proto('CanvasRenderingContext2D');
  if (contextProto) method(contextProto, 'getImageData', 'canvas');

  const navigatorProto = proto('Navigator');
  if (navigatorProto) {
    for (const name of ['hardwareConcurrency', 'platform', 'vendor', 'maxTouchPoints', 'plugins', 'mimeTypes', 'deviceMemory', 'pdfViewerEnabled', 'webdriver', 'cookieEnabled']) {
      getter(navigatorProto, name, 'device');
    }
    getter(navigatorProto, 'userAgent', 'agent');
    getter(navigatorProto, 'appVersion', 'agent');
    getter(navigatorProto, 'language', 'language');
    getter(navigatorProto, 'languages', 'language');
    method(navigatorProto, 'sendBeacon', 'beacon');
  }

  try {
    method(Intl.DateTimeFormat.prototype, 'resolvedOptions', 'timezone');
  } catch (error) {
    /* ohne Intl keine Zeitzone auf diesem Weg */
  }
  method(Date.prototype, 'getTimezoneOffset', 'timezone');

  const fontsProto = proto('FontFaceSet');
  if (fontsProto) method(fontsProto, 'check', 'fonts');

  const storageProto = proto('Storage');
  if (storageProto) method(storageProto, 'setItem', 'storage');
  const databaseProto = proto('IDBFactory');
  if (databaseProto) method(databaseProto, 'open', 'storage');

  const documentProto = proto('Document');
  if (documentProto) {
    getter(documentProto, 'cookie', 'cookie_read');
    setter(documentProto, 'cookie', 'cookie_write');
    getter(documentProto, 'referrer', 'referrer');
  }

  const placeProto = proto('Geolocation');
  if (placeProto) {
    method(placeProto, 'getCurrentPosition', 'location');
    method(placeProto, 'watchPosition', 'location');
  }
  const mediaProto = proto('MediaDevices');
  if (mediaProto) {
    method(mediaProto, 'enumerateDevices', 'media');
    method(mediaProto, 'getUserMedia', 'media');
  }
  const clipboardProto = proto('Clipboard');
  if (clipboardProto) {
    method(clipboardProto, 'readText', 'clipboard');
    method(clipboardProto, 'read', 'clipboard');
  }
  const permissionsProto = proto('Permissions');
  if (permissionsProto) method(permissionsProto, 'query', 'permissions');
  try {
    if (typeof window.Notification === 'function') method(window.Notification, 'requestPermission', 'notify');
  } catch (error) {
    /* keine Mitteilungen in dieser Umgebung */
  }
})();
