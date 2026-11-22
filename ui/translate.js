// Übersetzen an Ort und Stelle: sammelt die sichtbaren Textstellen der Seite,
// setzt Übersetzungen ein und stellt das Original wieder her. Läuft in Zunders
// abgeschotteter Welt – die Skripte der Seite sehen davon nur, dass sich Text
// ändert.
//
// Zwei Regeln halten die Seite heil:
// 1. Geändert wird ausschließlich der Text vorhandener Textknoten, nie HTML und
//    nie der Aufbau. Seiten, die ihren Inhalt selbst verwalten, kommen sonst aus
//    dem Tritt, sobald ein Knoten fehlt oder woanders hängt.
// 2. Was eingesetzt wird, ist Text – was auch immer das Modell geantwortet hat.
(function (mode, payload) {
  'use strict';
  const KEY = '__zunderTranslate';
  // Was nie übersetzt wird: Code, Eingaben, Eingebettetes, Lesehilfen über Schriftzeichen.
  const SKIP = new Set([
    'SCRIPT', 'STYLE', 'NOSCRIPT', 'TEXTAREA', 'INPUT', 'SELECT', 'OPTION', 'CODE', 'PRE', 'KBD', 'SAMP', 'VAR',
    'SVG', 'MATH', 'CANVAS', 'VIDEO', 'AUDIO', 'IFRAME', 'OBJECT', 'EMBED', 'TEMPLATE', 'RT', 'RP',
  ]);
  // Schriften ohne Wortabstand: Dort zählt schon ein einzelnes Zeichen als Wort.
  const DENSE = /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}\p{Script=Thai}]/u;

  // Auskünfte über die Seite holt das Skript über die Funktionen der Vorlagen,
  // nicht über die Eigenschaften der einzelnen Elemente. Ein Formular mit einem
  // Feld namens „tagName“ oder „parentElement“ gäbe sonst dieses Feld zurück
  // statt der Auskunft – und das Skript liefe ins Leere oder im Kreis.
  const reader = (proto, name) => Function.prototype.call.bind(Object.getOwnPropertyDescriptor(proto, name).get);
  const nameOf = reader(Element.prototype, 'localName');
  const parentOf = reader(Node.prototype, 'parentElement');
  const kindOf = reader(Node.prototype, 'nodeType');
  const textOf = reader(Node.prototype, 'nodeValue');
  const setText = Function.prototype.call.bind(Object.getOwnPropertyDescriptor(Node.prototype, 'nodeValue').set);
  const classesOf = reader(Element.prototype, 'classList');
  const editable = reader(HTMLElement.prototype, 'isContentEditable');
  const attribute = Function.prototype.call.bind(Element.prototype.getAttribute);
  const bodyOf = reader(Document.prototype, 'body');
  const topOf = reader(Document.prototype, 'documentElement');
  const walkerFor = Function.prototype.call.bind(Document.prototype.createTreeWalker);
  const findIn = Function.prototype.call.bind(Document.prototype.querySelector);
  const listen = Function.prototype.call.bind(EventTarget.prototype.addEventListener);

  // Stellt das Original wieder her – aber nur dort, wo noch steht, was dieses
  // Skript zuletzt hingeschrieben hat. Was die Seite inzwischen selbst
  // geändert hat, bleibt ihres.
  const restoreAll = () => {
    const state = globalThis[KEY];
    if (!state) return;
    state.nodes.forEach((node, index) => {
      if (textOf(node) === state.current[index]) setText(node, state.original[index]);
    });
    globalThis[KEY] = null;
  };

  if (mode === 'collect') {
    // Ein früherer Auftrag in dieser Seite ist damit erledigt: erst sein Original zurück.
    restoreAll();
    const limit = Math.max(1000, Math.min(Number(payload && payload.limit) || 60000, 60000));
    const root = bodyOf(document) || topOf(document);
    const displays = new WeakMap();
    const displayOf = (element) => {
      let display = displays.get(element);
      if (display === undefined) {
        display = getComputedStyle(element).display;
        displays.set(element, display);
      }
      return display;
    };
    // Läuft das Element im Satz mit (Link, Fettdruck) – oder bildet es einen eigenen Kasten?
    const flows = (element) => {
      const display = displayOf(element);
      return display === 'inline' || display === 'contents';
    };
    // Der Kasten, in dem eine Textstelle steht: Zwei Stellen im selben Kasten,
    // zwischen denen nichts umbricht, gehören zum selben Satz.
    const boxOf = (node) => {
      let element = parentOf(node);
      // Tiefer als das ist keine Seite verschachtelt – eine Schleife, die nicht endet, gibt es so nicht.
      for (let depth = 0; depth < 2000 && element && element !== root && flows(element); depth += 1) {
        element = parentOf(element);
      }
      return element;
    };
    const unseen = (style) => style.display === 'none' || style.visibility === 'hidden' || style.visibility === 'collapse';
    const untouchable = (element) =>
      SKIP.has(String(nameOf(element)).toUpperCase()) ||
      (element instanceof HTMLElement && editable(element)) ||
      attribute(element, 'translate') === 'no' ||
      classesOf(element).contains('notranslate');
    // Ein richtiges Wort: zwei Buchstaben am Stück oder ein Schriftzeichen.
    const wordy = (text) => /\p{L}{2}/u.test(text) || DENSE.test(text);

    // Alle Textstellen mit wenigstens einem Buchstaben, in der Reihenfolge der
    // Seite. `cont` heißt: setzt den Satz der Stelle davor fort.
    const found = [];
    let capped = false;
    const walk = (start, except) => {
      // Seit der letzten gesammelten Stelle kam etwas, das den Satz beendet.
      let broken = true;
      let lastBox = null;
      const walker = walkerFor(document, start, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT, {
        acceptNode(node) {
          if (kindOf(node) === Node.TEXT_NODE) return NodeFilter.FILTER_ACCEPT;
          if (node === except) {
            broken = true;
            return NodeFilter.FILTER_REJECT;
          }
          const style = getComputedStyle(node);
          const display = style.display;
          // Unsichtbares zählt nicht – und unterbricht auch keinen Satz.
          if (unseen(style)) return NodeFilter.FILTER_REJECT;
          // Ein Umbruch oder ein eigener Kasten beendet den Satz; ein Stück Code
          // mitten im Satz tut das nicht.
          const tag = String(nameOf(node)).toUpperCase();
          if (tag === 'BR' || tag === 'HR' || !(display === 'inline' || display === 'contents')) broken = true;
          if (untouchable(node)) return NodeFilter.FILTER_REJECT;
          displays.set(node, display);
          // Das Element selbst ist keine Textstelle – seine Kinder kommen dran.
          return NodeFilter.FILTER_SKIP;
        },
      });
      for (let node = walker.nextNode(); node; node = walker.nextNode()) {
        const text = String(textOf(node)).trim();
        // Zahlen, Pfeile und Satzzeichen bleiben, wie sie sind.
        if (!text || !/\p{L}/u.test(text)) continue;
        // Solche Brocken sind kein Fließtext, sondern eingebettete Daten.
        if (text.length > 6000) {
          broken = true;
          continue;
        }
        if (found.length >= 4000) {
          capped = true;
          return;
        }
        const box = boxOf(node);
        found.push({ node, text, cont: !broken && lastBox !== null && box === lastBox, wordy: wordy(text) });
        broken = false;
        lastBox = box;
      }
    };

    // Der eigentliche Inhalt zuerst: Ist die Seite länger, als übersetzt wird,
    // soll nicht das Menü den Platz verbrauchen. Er zählt nur, wenn kein
    // Vorfahr ihn vom Übersetzen ausnimmt.
    let main = findIn(document, 'main, [role="main"]') || findIn(document, 'article');
    const within = Function.prototype.call.bind(Node.prototype.contains);
    if (main && (main === root || !within(root, main) || untouchable(main) || unseen(getComputedStyle(main)))) main = null;
    let above = main && parentOf(main);
    for (let depth = 0; depth < 2000 && above && above !== root; depth += 1) {
      if (untouchable(above) || unseen(getComputedStyle(above))) {
        main = null;
        break;
      }
      above = parentOf(above);
    }
    if (main) walk(main, null);
    walk(root, main);

    // Sätze bilden. Einer ohne ein richtiges Wort (ein „x“ zum Schließen, die
    // Buchstaben eines Registers) bleibt, wie er ist – ein einzelner Buchstabe
    // mitten in einem Satz („A lighthouse is …“) gehört dagegen dazu.
    const nodes = [];
    const texts = [];
    const cont = [];
    let total = 0;
    let truncated = capped;
    for (let from = 0; from < found.length && !truncated; ) {
      let to = from + 1;
      while (to < found.length && found[to].cont) to += 1;
      const group = found.slice(from, to);
      from = to;
      if (!group.some((piece) => piece.wordy)) continue;
      for (let slot = 0; slot < group.length; slot += 1) {
        const piece = group[slot];
        if (total + piece.text.length > limit || nodes.length >= 1500) {
          truncated = true;
          break;
        }
        nodes.push(piece.node);
        texts.push(piece.text.replace(/\s+/g, ' '));
        cont.push(slot > 0 ? 1 : 0);
        total += piece.text.length;
      }
    }
    const original = nodes.map((node) => textOf(node));
    // `job`: Nur der Auftrag mit dieser Nummer darf in diese Seite schreiben.
    // `current`: was an jeder Stelle zuletzt hingeschrieben wurde.
    globalThis[KEY] = { job: payload && payload.job, nodes, cont, original, current: original.slice() };
    // Verlässt die Seite den Tab – auch nur in den Zwischenspeicher fürs
    // Zurückgehen –, kommt ihr Original zurück. Wer später zu ihr zurückkehrt,
    // findet sie, wie sie war, und kein Auftrag von damals schreibt mehr in sie.
    if (!globalThis[KEY + 'Leaving']) {
      globalThis[KEY + 'Leaving'] = true;
      listen(window, 'pagehide', restoreAll);
    }
    return JSON.stringify({
      lang: String(attribute(topOf(document), 'lang') || '').toLowerCase().slice(0, 12),
      texts,
      cont,
      truncated,
    });
  }

  const state = globalThis[KEY];
  if (!state) return 'none';
  // Ein Auftrag schreibt nur in die Seite, die er selbst gelesen hat. Steht in
  // diesem Dokument ein anderer (eine Seite, die aus dem Verlauf zurückkam),
  // passiert nichts.
  if (!payload || payload.job !== state.job) return 'other';

  if (mode === 'apply') {
    const closing = /^[,.;:!?)\]}%»”’…、。，！？：；）」』]/u;
    const wordEnd = /[\p{L}\p{N}]$/u;
    const wordStart = /^[\p{L}\p{N}]/u;
    let changed = 0;
    for (const pair of Array.isArray(payload.pairs) ? payload.pairs : []) {
      const index = pair[0];
      const text = pair[1];
      const node = state.nodes[index];
      if (!node || typeof text !== 'string' || !text || text.length > 20000) continue;
      // Die Seite hat diese Stelle inzwischen selbst geändert: Dann bleibt sie, wie sie ist.
      if (textOf(node) !== state.current[index]) continue;
      const raw = state.original[index];
      let lead = raw.match(/^\s*/)[0];
      const trail = raw.match(/\s*$/)[0];
      if (lead && closing.test(text) && !closing.test(raw.trim())) {
        // Die Übersetzung beginnt mit einem Satzzeichen: Davor gehört kein Abstand.
        lead = '';
      } else if (!lead && state.cont[index] && index > 0) {
        // Quelle ohne Wortabstände (Chinesisch, Japanisch, Thai): Zwischen zwei
        // Stücken desselben Satzes fehlte sonst in der Übersetzung der Abstand.
        const rawBefore = state.original[index - 1];
        const before = String(textOf(state.nodes[index - 1]));
        const dense = DENSE.test(rawBefore.trim().slice(-2)) || DENSE.test(raw.trim().slice(0, 2));
        if (dense && !/\s$/.test(rawBefore) && wordEnd.test(before) && wordStart.test(text)) lead = ' ';
      }
      const value = lead + text + trail;
      setText(node, value);
      state.current[index] = value;
      changed += 1;
    }
    return String(changed);
  }

  if (mode === 'restore') {
    restoreAll();
    return 'ok';
  }
  return 'unknown';
})
