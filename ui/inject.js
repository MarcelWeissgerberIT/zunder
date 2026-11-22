// Läuft in jeder Seite mit (nur im Hauptrahmen) und meldet dem Browser zwei
// Dinge: Adresswechsel ohne Neuladen und dass die Seite die Tastatur bekommen
// hat. (Das Icon der Seite holt Zunders abgeschottetes Skript – logins.js.) Mehr kann dieser Kanal nicht – der Browser wertet die
// Meldungen nur als Hinweis und prüft alles selbst nach.
(function () {
  'use strict';
  if (window.top !== window) return;
  var bridge = window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.ipc;
  if (!bridge) return;

  function post(message) {
    try {
      bridge.postMessage(message);
    } catch (error) {
      /* Seite wird gerade verlassen */
    }
  }

  function addressChanged() {
    setTimeout(function () {
      post('u');
    }, 0);
  }

  ['pushState', 'replaceState'].forEach(function (name) {
    var original = history[name];
    if (typeof original !== 'function') return;
    history[name] = function () {
      var result = original.apply(this, arguments);
      addressChanged();
      return result;
    };
  });
  // Bekommt die Seite die Tastatur, erfährt es der Browser: Stehen zwei Tabs
  // nebeneinander, folgt die Symbolleiste dem, in dem gearbeitet wird.
  window.addEventListener('focus', function (event) {
    if (event.target === window) post('a');
  });

  window.addEventListener('popstate', addressChanged, true);
  window.addEventListener('hashchange', addressChanged, true);
})();
