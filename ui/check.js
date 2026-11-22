// Seitencheck: liest aus der Seite, was sich über ihre Angriffsflächen sagen
// lässt – woher ihre Skripte kommen, wohin ihre Formulare senden, was sie
// einbettet. Läuft in Zunders abgeschotteter Welt, ändert nichts und ruft
// nichts ab; die Skripte der Seite bemerken davon nichts.
//
// Zurück geht eine Aufstellung als Text. Ausgewertet wird sie im Browser
// (`src/sitecheck.rs`), nicht hier.
(function () {
  'use strict';
  // Auskünfte über die Seite holt das Skript über die Funktionen der Vorlagen,
  // nicht über die Eigenschaften der einzelnen Elemente: Ein Formular mit einem
  // Feld namens „getAttribute“ gäbe sonst dieses Feld zurück – und die Seite
  // könnte so verbergen, wohin ihr Formular sendet.
  const all = Function.prototype.call.bind(Document.prototype.querySelectorAll);
  const one = Function.prototype.call.bind(Document.prototype.querySelector);
  const attribute = Function.prototype.call.bind(Element.prototype.getAttribute);
  const has = Function.prototype.call.bind(Element.prototype.hasAttribute);
  const inside = Function.prototype.call.bind(Element.prototype.querySelector);
  const base = Function.prototype.call.bind(Object.getOwnPropertyDescriptor(Node.prototype, 'baseURI').get)(document);
  const cut = (text, max) => String(text == null ? '' : text).slice(0, max);
  const absolute = (value) => {
    try {
      return new URL(value, base).href;
    } catch (error) {
      return '';
    }
  };
  const secure = location.protocol === 'https:';

  // Skripte mit Adresse – und ob eine Prüfsumme über sie wacht.
  const scripts = [];
  for (const script of all(document, 'script[src]')) {
    if (scripts.length >= 300) break;
    scripts.push({ u: cut(absolute(attribute(script, 'src')), 400), i: !!attribute(script, 'integrity') });
  }

  // Wohin die Formulare senden. Ohne Angabe: an die Seite selbst. Auch der
  // Knopf eines Formulars kann ein eigenes Ziel nennen (`formaction`).
  const forms = [];
  for (const form of all(document, 'form')) {
    if (forms.length >= 60) break;
    const action = attribute(form, 'action');
    forms.push({ a: cut(action ? absolute(action) : location.href, 400) });
    const button = inside(form, '[formaction]');
    if (button && forms.length < 60) forms.push({ a: cut(absolute(attribute(button, 'formaction')), 400) });
  }
  const passwords = Math.min(all(document, 'input[type="password" i]').length, 999);

  // Eingebettete Rahmen – und ob sie eingeschränkt laufen.
  const frames = [];
  for (const frame of all(document, 'iframe[src]')) {
    if (frames.length >= 120) break;
    frames.push({ u: cut(absolute(attribute(frame, 'src')), 400), s: has(frame, 'sandbox') });
  }

  // Gemischte Inhalte: Unverschlüsseltes in einer verschlüsselten Seite.
  let active = 0;
  let passive = 0;
  if (secure) {
    const plain = (value) => /^http:/i.test(absolute(value));
    for (const node of all(document, 'script[src], iframe[src]')) {
      if (plain(attribute(node, 'src'))) active += 1;
    }
    for (const node of all(document, 'link[rel~="stylesheet" i][href]')) {
      if (plain(attribute(node, 'href'))) active += 1;
    }
    for (const node of all(document, 'img[src], video[src], audio[src], source[src]')) {
      if (plain(attribute(node, 'src'))) passive += 1;
    }
  }

  // Eine Schutzregel, die im Dokument selbst steht.
  const meta = one(document, 'meta[http-equiv="Content-Security-Policy" i]');

  return JSON.stringify({
    scripts,
    forms,
    passwords,
    frames,
    mixed: { active: Math.min(active, 999), passive: Math.min(passive, 999) },
    csp: cut(meta && attribute(meta, 'content'), 4000),
  });
})()
