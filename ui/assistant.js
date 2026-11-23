// Seitenleiste des Assistenten: zeigt die Unterhaltung zur aktuellen Seite und
// meldet Fragen an den Rust-Teil. Dort wird der Seitentext geholt und das
// Modell angesprochen – diese Datei sieht nur die Antworten.
//
// Jeder Tab hat seine eigene Unterhaltung. Sie liegt nur im Arbeitsspeicher
// dieser Leiste und ist weg, sobald das Fenster zugeht.
//
// „Compared Browsing“: Eine Unterhaltung kann weitere Tabs dazunehmen. Dann
// gehen Vergleich und Fragen an alle diese Seiten zugleich; auf Wunsch stehen
// zwei davon nebeneinander im Fenster.
//
// Antworten eines Modells werden als Text eingesetzt, nie als HTML: Was eine
// Webseite dem Modell unterschiebt, kann hier nichts ausführen und nichts nachladen.
(() => {
  'use strict';

  const DATA = JSON.parse(document.getElementById('data').textContent);
  const S = DATA.strings;
  const SVG_NS = 'http://www.w3.org/2000/svg';
  const $ = (id) => document.getElementById(id);
  const post = (message) => window.ipc.postMessage(JSON.stringify(message));

  function icon(name, extra) {
    const svg = document.createElementNS(SVG_NS, 'svg');
    svg.setAttribute('class', 'ic' + (extra ? ' ' + extra : ''));
    const use = document.createElementNS(SVG_NS, 'use');
    use.setAttribute('href', '#i-' + name);
    svg.appendChild(use);
    return svg;
  }

  function setUse(id, name) {
    const use = $(id);
    if (use.getAttribute('href') !== '#i-' + name) use.setAttribute('href', '#i-' + name);
  }

  function el(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text != null) node.textContent = text;
    return node;
  }

  function button(className, text, handler) {
    const node = el('button', className, text);
    node.type = 'button';
    node.addEventListener('click', handler);
    return node;
  }

  // Aufträge zu einer einzelnen Seite.
  const ACTIONS = [
    { id: 'summary', label: S.ai_summary, hint: S.ai_summary_hint, icon: 'reader' },
    { id: 'points', label: S.ai_points, hint: S.ai_points_hint, icon: 'menu' },
    { id: 'explain', label: S.ai_explain, hint: S.ai_explain_hint, icon: 'info' },
    { id: 'translate', label: S.ai_translate, hint: S.ai_translate_hint, icon: 'globe' },
  ];

  let models = null; // was der Rust-Teil über die Modelle weiß
  let page = null; // die Seite, die gerade vor der Leiste liegt
  let view = 'chat'; // 'chat' | 'models' | 'pick'
  const talks = new Map(); // Tab -> { items, key, others }
  let running = null; // { tab, answer, job }
  let keyDraft = ''; // angefangene Eingabe im Schlüsselfeld
  let keyError = '';
  // Dasselbe für den „anderen Dienst“ – dazu Adresse und Modellname, solange sie getippt werden.
  const other = { keyDraft: '', keyError: '', urlDraft: null, modelDraft: null, custom: false };
  let tabList = []; // die anderen Tabs des Fensters, für die Auswahl zum Vergleichen
  const pick = { chosen: new Set(), split: true, focus: '' };

  const input = $('input');
  const chat = $('chat');

  // ------------------------------------------------------------------------
  // Unterhaltung
  // ------------------------------------------------------------------------

  // Zu welchem Tab die gezeigte Unterhaltung gehört. Stehen zwei Tabs
  // nebeneinander und läuft dort ein Vergleich, gehört sie dem Paar – egal,
  // in welcher Hälfte gerade gearbeitet wird.
  function talkKey() {
    if (!page) return null;
    if (page.pair) {
      const left = talks.get(page.pair[0]);
      if (left && left.others && left.others.some((other) => other.id === page.pair[1])) return page.pair[0];
    }
    return page.tab;
  }

  function talk() {
    const key = talkKey();
    if (key == null) return null;
    let current = talks.get(key);
    if (!current) {
      current = { items: [], key: page.key, others: null };
      talks.set(key, current);
    }
    return current;
  }

  function comparing(current) {
    return !!current && !!current.others && current.others.length > 0;
  }

  // Was das Modell von der bisherigen Unterhaltung erfährt: nur abgeschlossene
  // Antworten seit dem letzten Wechsel der Seite oder des Vergleichs.
  function history(current) {
    let start = 0;
    current.items.forEach((item, index) => {
      if (item.kind === 'page' || item.kind === 'mode') start = index + 1;
    });
    const pairs = [];
    for (const item of current.items.slice(start)) {
      if (item.kind === 'a' && item.state === 'done' && item.asked && item.text) {
        pairs.push([item.asked, item.text]);
      }
    }
    return pairs;
  }

  function ask(action, text) {
    if (!page) return;
    if (running) {
      // Der Rust-Teil bricht die alte Anfrage ab, sobald die neue eintrifft.
      settle(running.answer, 'stopped');
      running = null;
    }
    const key = talkKey();
    const current = talk();
    const pairs = history(current);
    // Vergleich und freie Fragen gehen an alle Seiten des Vergleichs; die
    // übrigen Aufträge gelten der Seite, die gerade vorn ist.
    const several = comparing(current) && (action === 'compare' || action === 'ask');
    const preset = ACTIONS.find((entry) => entry.id === action);
    let label = preset ? preset.label : text;
    if (action === 'compare') label = text ? S.ai_compare + ' – ' + text : S.ai_compare;
    current.items.push({ kind: 'q', text: label });
    const answer = {
      kind: 'a',
      text: '',
      state: 'run',
      status: models && models.probed ? S.ai_reading : S.ai_checking,
      action,
      question: text || '',
    };
    current.items.push(answer);
    running = { tab: key, answer, job: null };
    // Die Frage gilt genau der Seite, die diese Leiste gerade vor sich hat – der
    // Browser prüft das nach, statt einfach den Tab zu nehmen, der beim
    // Eintreffen vorn ist.
    const message = { c: 'aiAsk', action, text: text || '', history: pairs, base: key };
    if (key === page.tab) message.page = page.key;
    if (several) {
      message.others = current.others.map((other) => other.id);
    }
    post(message);
    view = 'chat';
    render(true);
  }

  function settle(answer, state, errorText) {
    answer.state = state;
    answer.status = '';
    if (state === 'error') answer.error = errorText || '';
  }

  function finish(state, errorText) {
    if (!running) return;
    const done = running;
    running = null;
    settle(done.answer, state, errorText);
    if (done.tab === talkKey() && view === 'chat') {
      const stick = nearEnd();
      paint(done.answer);
      if (stick) toEnd();
    }
    renderComposer();
  }

  function stop() {
    if (!running) return;
    post({ c: 'aiStop' });
    finish('stopped');
  }

  function mine(message) {
    return !!running && running.job != null && running.job === message.job;
  }

  // Der laufenden Antwort etwas hinzufügen – gezeichnet wird nur, wenn ihre
  // Unterhaltung gerade zu sehen ist.
  function update(change) {
    change(running.answer);
    if (running.tab === talkKey() && view === 'chat') {
      const stick = nearEnd();
      paint(running.answer);
      if (stick) toEnd();
    }
  }

  function nearEnd() {
    return chat.scrollHeight - chat.scrollTop - chat.clientHeight < 70;
  }

  function toEnd() {
    chat.scrollTop = chat.scrollHeight;
  }

  // ------------------------------------------------------------------------
  // Compared Browsing
  // ------------------------------------------------------------------------

  function openPicker() {
    if (!page || !page.readable) return;
    pick.chosen.clear();
    pick.focus = '';
    pick.split = true;
    tabList = [];
    view = 'pick';
    post({ c: 'aiTabs' });
    render();
  }

  function startCompare() {
    const current = talk();
    const others = tabList.filter((tab) => pick.chosen.has(tab.id)).map((tab) => ({ id: tab.id, title: tab.title }));
    if (!current || !others.length) return;
    current.others = others;
    current.items.push({ kind: 'mode', title: S.ai_compare_with + ' ' + others.map((other) => other.title).join(' · ') });
    if (pick.split && others.length === 1) post({ c: 'aiSplit', tab: others[0].id, base: talkKey() });
    view = 'chat';
    ask('compare', pick.focus.trim());
  }

  function endCompare() {
    const current = talk();
    if (!comparing(current)) return;
    if (running && running.tab === talkKey()) stop();
    const wasPair = !!page && !!page.pair;
    const base = talkKey();
    current.others = null;
    current.items.push({ kind: 'mode', title: S.ai_compare_end });
    if (wasPair) post({ c: 'aiSplit', tab: 0, base });
    render(true);
  }

  // ------------------------------------------------------------------------
  // Antworttext darstellen
  // ------------------------------------------------------------------------

  // Hervorhebungen und Code im Fließtext – aufgebaut aus Textknoten.
  function inline(node, text) {
    const parts = text.split(/(\*\*[^*\n]+\*\*|`[^`\n]+`)/g);
    for (const part of parts) {
      if (!part) continue;
      if (part.length > 4 && part.startsWith('**') && part.endsWith('**')) {
        node.appendChild(el('strong', '', part.slice(2, -2)));
      } else if (part.length > 2 && part.startsWith('`') && part.endsWith('`')) {
        node.appendChild(el('code', '', part.slice(1, -1)));
      } else {
        node.appendChild(document.createTextNode(part));
      }
    }
  }

  // Zellen einer Tabellenzeile „| a | b | c |“ – oder null, wenn es keine ist.
  function tableCells(line) {
    if (!line.startsWith('|') || line.length < 3) return null;
    const inner = line.endsWith('|') ? line.slice(1, -1) : line.slice(1);
    return inner.split('|').map((cell) => cell.trim());
  }

  // Absätze, Aufzählungen und Tabellen. Mehr Auszeichnung gibt es absichtlich
  // nicht: keine Links, keine Bilder, kein HTML.
  function renderText(box, text) {
    box.textContent = '';
    let list = null;
    let listKind = '';
    let paragraph = null;
    let table = null;
    for (const raw of text.replace(/\r/g, '').split('\n')) {
      const line = raw.trim();
      if (!line) {
        list = null;
        paragraph = null;
        table = null;
        continue;
      }
      const cells = tableCells(line);
      if (cells) {
        list = null;
        paragraph = null;
        // Die Trennzeile unter dem Kopf („|---|---|“) trägt nichts bei.
        if (cells.every((cell) => /^:?-{2,}:?$/.test(cell))) continue;
        if (!table) {
          const wrap = el('div', 'tablewrap');
          table = el('table');
          wrap.appendChild(table);
          box.appendChild(wrap);
        }
        const row = el('tr');
        const head = table.children.length === 0;
        cells.forEach((cell, column) => {
          const node = el(head || column === 0 ? 'th' : 'td');
          inline(node, cell);
          row.appendChild(node);
        });
        table.appendChild(row);
        continue;
      }
      table = null;
      const bullet = /^(?:[-*•–]|(\d{1,2})[.)])\s+(.*)$/.exec(line);
      if (bullet) {
        const kind = bullet[1] ? 'ol' : 'ul';
        if (!list || listKind !== kind) {
          list = el(kind);
          listKind = kind;
          if (kind === 'ol') list.start = Number(bullet[1]) || 1;
          box.appendChild(list);
        }
        const entry = el('li');
        inline(entry, bullet[2]);
        list.appendChild(entry);
        paragraph = null;
        continue;
      }
      list = null;
      const heading = /^#{1,4}\s+(.*)$/.exec(line);
      if (heading) {
        const head = el('p');
        head.appendChild(el('strong', '', heading[1]));
        box.appendChild(head);
        paragraph = null;
        continue;
      }
      // Eine Zeile, die mit einer Hervorhebung beginnt, eröffnet einen Abschnitt.
      if (paragraph && !line.startsWith('**')) {
        paragraph.appendChild(document.createElement('br'));
      } else {
        paragraph = el('p');
        box.appendChild(paragraph);
      }
      inline(paragraph, line);
    }
  }

  function copyText(text, trigger) {
    const area = document.createElement('textarea');
    area.value = text;
    area.style.position = 'fixed';
    area.style.opacity = '0';
    document.body.appendChild(area);
    area.select();
    let copied = false;
    try {
      copied = document.execCommand('copy');
    } catch (error) {}
    area.remove();
    if (copied) {
      trigger.textContent = '';
      trigger.appendChild(icon('check', 'sm'));
      trigger.title = S.ai_copied;
      setTimeout(() => {
        trigger.textContent = '';
        trigger.appendChild(icon('copy', 'sm'));
        trigger.title = S.ai_copy;
      }, 1400);
    }
  }

  function contextLabel(item) {
    if (item.ctx === 'compare') return S.ai_ctx_compare.replace('{n}', item.pages || 2);
    if (item.ctx === 'selection') return S.ai_ctx_selection;
    if (item.ctx === 'none') return S.ai_ctx_none;
    if (item.ctx === 'page') return S.ai_ctx_page;
    return '';
  }

  function paint(item) {
    if (!item.node) item.node = el('div', item.kind === 'a' ? 'answer' : '');
    const node = item.node;
    node.textContent = '';

    if (item.kind === 'q') {
      node.className = 'q';
      node.textContent = item.text;
      return node;
    }
    if (item.kind === 'page' || item.kind === 'mode') {
      node.className = 'divider';
      node.appendChild(el('span', '', item.title || S.ai_new_page));
      return node;
    }

    if (item.text) {
      const body = el('div', 'a');
      renderText(body, item.text);
      node.appendChild(body);
    }
    if (item.state === 'run') {
      if (item.status || !item.text) {
        const status = el('div', 'status');
        status.appendChild(el('span', 'spinner'));
        status.appendChild(el('span', '', item.status || S.ai_waiting));
        node.appendChild(status);
      }
      return node;
    }
    if (item.state === 'error') {
      const box = el('div', 'err');
      box.appendChild(icon('warning'));
      box.appendChild(el('div', '', item.error || S.ai_r_unknown));
      node.appendChild(box);
    }
    if (item.note) {
      const note = el('div', 'note');
      note.appendChild(icon('info'));
      note.appendChild(el('span', '', item.note));
      node.appendChild(note);
    }

    const meta = el('div', 'meta');
    const facts = [];
    if (item.state === 'stopped') facts.push(S.ai_stopped);
    if (item.engine) facts.push(item.engine);
    if (item.state === 'done') facts.push(contextLabel(item));
    meta.appendChild(el('span', '', facts.filter(Boolean).join(' · ')));
    meta.appendChild(el('span', 'gap'));
    if (item.state !== 'done') {
      meta.appendChild(button('tbtn', S.ai_retry, () => ask(item.action, item.question)));
    }
    if (item.text) {
      const copy = button('ibtn sm', null, () => copyText(item.text, copy));
      copy.title = S.ai_copy;
      copy.appendChild(icon('copy', 'sm'));
      meta.appendChild(copy);
    }
    node.appendChild(meta);
    return node;
  }

  // ------------------------------------------------------------------------
  // Ansicht
  // ------------------------------------------------------------------------

  function card(symbol, label, hint, handler) {
    const node = button('card', null, handler);
    node.appendChild(icon(symbol));
    const text = el('div');
    text.appendChild(el('b', '', label));
    text.appendChild(el('span', '', hint));
    node.appendChild(text);
    return node;
  }

  function renderIntro(current) {
    const intro = $('intro');
    intro.textContent = '';
    if (current && current.items.length) {
      intro.hidden = true;
      return;
    }
    intro.hidden = false;
    const canRead = !!page && page.readable;
    intro.appendChild(el('h2', '', canRead ? S.ai_intro : S.ai_intro_blank));
    intro.appendChild(el('p', '', canRead ? S.ai_intro_hint : S.ai_intro_blank_hint));
    if (!canRead) return;
    for (const action of ACTIONS) {
      intro.appendChild(card(action.icon, action.label, action.hint, () => ask(action.id, '')));
    }
    intro.appendChild(card('compare', S.ai_compare, S.ai_compare_hint, openPicker));
  }

  function renderComposer() {
    const current = talk();
    const canRead = !!page && page.readable;
    const several = comparing(current);
    const quick = $('quick');
    quick.textContent = '';
    quick.hidden = !(canRead && current && current.items.length);
    if (!quick.hidden) {
      if (several) quick.appendChild(button('chip strong', S.ai_compare, () => ask('compare', '')));
      for (const action of ACTIONS) quick.appendChild(button('chip', action.label, () => ask(action.id, '')));
      if (!several) quick.appendChild(button('chip', S.ai_compare, openPicker));
    }
    input.placeholder = several ? S.ai_input_compare : canRead ? S.ai_input : S.ai_input_blank;

    const send = $('send');
    const busy = !!running;
    send.classList.toggle('stop', busy);
    send.disabled = !busy && !input.value.trim();
    send.title = busy ? S.ai_stop : S.ai_send;
    setUse('send-icon', busy ? 'stop' : 'send');

    const engine = $('engine');
    let text = S.ai_checking;
    let symbol = 'info';
    let kind = '';
    if (models && models.probed) {
      if (!models.engine) {
        // Läuft Ollama, fehlt nur die Wahl – das soll man sehen.
        const ollama = models.ollama || {};
        text = ollama.running && (ollama.models || []).length && !models.private ? S.ai_none_pick : S.ai_none;
        symbol = 'warning';
        kind = 'none';
      } else if (models.engine.local) {
        text = models.engine.label + ' · ' + S.ai_local;
        symbol = 'lock';
      } else {
        const host = models.engine.host;
        text = models.engine.label + ' · ' + (host ? S.ai_cloud_host.replace('{host}', host) : S.ai_cloud);
        symbol = 'globe';
        kind = 'cloud';
      }
    }
    engine.className = kind;
    engine.title = S.ai_choose;
    $('engine-text').textContent = text;
    setUse('engine-icon', symbol);
  }

  function renderPageline(current) {
    const line = $('pageline');
    const several = comparing(current);
    line.hidden = view !== 'chat' || !(page && page.readable);
    line.classList.toggle('pair', several);
    setUse('pageline-icon', several ? 'compare' : 'globe');
    $('compare-end').hidden = !several;
    if (several) {
      const text = S.ai_compare_with + ': ' + current.others.map((other) => other.title).join(' · ');
      $('pagetitle').textContent = text;
      line.title = text;
    } else {
      $('pagetitle').textContent = page ? page.title || page.url : '';
      line.title = page ? page.url : '';
    }
  }

  function render(scrollToEnd) {
    const current = talk();
    $('title').textContent = view === 'models' ? S.ai_choose : view === 'pick' ? S.ai_pick_title : S.ai_title;
    $('chat').hidden = view !== 'chat';
    $('models').hidden = view !== 'models';
    $('pick').hidden = view !== 'pick';
    $('composer').hidden = view !== 'chat';
    $('clear').hidden = view !== 'chat' || !(current && current.items.length);
    renderPageline(current);

    if (view === 'models') {
      renderModels();
      return;
    }
    if (view === 'pick') {
      renderPick();
      return;
    }
    renderIntro(current);
    const log = $('log');
    log.textContent = '';
    if (current) for (const item of current.items) log.appendChild(paint(item));
    renderComposer();
    if (scrollToEnd) toEnd();
  }

  // ------------------------------------------------------------------------
  // Auswahl zum Vergleichen
  // ------------------------------------------------------------------------

  function renderPick() {
    $('pick-intro').textContent = S.ai_pick_intro;
    $('pick-hint').textContent = S.ai_pick_hint;
    const list = $('pick-list');
    list.textContent = '';
    const base = talkKey();
    const candidates = tabList.filter((tab) => tab.id !== base && tab.web);
    if (!candidates.length) {
      list.appendChild(el('div', 'none', S.ai_pick_none));
    }
    for (const tab of candidates) {
      const chosen = pick.chosen.has(tab.id);
      const row = el('div', 'tabrow' + (chosen ? ' on' : '') + (tab.ready ? '' : ' off'));
      row.appendChild(el('span', 'box'));
      const text = el('div', 'txt');
      text.appendChild(el('div', 't', tab.title));
      text.appendChild(el('div', 's', tab.ready ? tab.url : S.ai_pick_unloaded));
      row.appendChild(text);
      if (tab.ready) {
        row.addEventListener('click', () => {
          if (chosen) pick.chosen.delete(tab.id);
          else if (pick.chosen.size < 3) pick.chosen.add(tab.id);
          renderPick();
        });
      }
      list.appendChild(row);
    }

    const one = pick.chosen.size === 1;
    const split = $('pick-split');
    split.classList.toggle('off', !one);
    $('pick-split-label').textContent = S.ai_pick_split;
    $('pick-split-hint').textContent = S.ai_pick_split_hint;
    $('pick-split-switch').classList.toggle('on', one && pick.split);
    const focus = $('pick-focus');
    focus.placeholder = S.ai_pick_focus;
    if (focus.value !== pick.focus) focus.value = pick.focus;
    $('pick-go').disabled = pick.chosen.size === 0;
  }

  $('pick-split-switch').addEventListener('click', () => {
    if (pick.chosen.size !== 1) return;
    pick.split = !pick.split;
    renderPick();
  });
  $('pick-focus').addEventListener('input', () => {
    pick.focus = $('pick-focus').value;
  });
  $('pick-focus').addEventListener('keydown', (event) => {
    if (event.key === 'Enter' && pick.chosen.size) startCompare();
  });
  $('pick-go').addEventListener('click', startCompare);
  $('pick-cancel').addEventListener('click', () => {
    view = 'chat';
    render(true);
    input.focus();
  });
  $('compare-end').addEventListener('click', endCompare);

  // ------------------------------------------------------------------------
  // Modell wählen
  // ------------------------------------------------------------------------

  const REASON = {
    missing: S.ai_r_missing,
    unsupported: S.ai_r_unsupported,
    device_not_eligible: S.ai_r_device_not_eligible,
    not_enabled: S.ai_r_not_enabled,
    model_not_ready: S.ai_r_model_not_ready,
  };

  function option(id, name, sub, usable, stateText) {
    const active = !!models.engine && models.engine.id === id;
    const box = el('div', 'opt' + (active ? ' on' : '') + (usable ? ' can' : ''));
    const top = el('div', 'top');
    top.appendChild(el('span', 'radio'));
    top.appendChild(el('span', 'name', name));
    top.appendChild(el('span', 'state' + (usable ? ' ok' : ''), stateText));
    box.appendChild(top);
    box.appendChild(el('div', 'sub', sub));
    if (usable) {
      box.addEventListener('click', (event) => {
        if (event.target.closest('select, input, button')) return;
        post({ c: 'aiSetting', key: 'engine', value: id });
      });
    }
    return box;
  }

  function picker(label, entries, selected, key) {
    const line = el('div', 'line');
    line.appendChild(el('label', '', label));
    const select = document.createElement('select');
    for (const entry of entries) {
      const item = document.createElement('option');
      item.value = entry.id;
      item.textContent = entry.name;
      item.selected = entry.id === selected;
      select.appendChild(item);
    }
    select.addEventListener('change', () => post({ c: 'aiSetting', key, value: select.value }));
    line.appendChild(select);
    return line;
  }

  function renderModels() {
    const list = $('models-list');
    list.textContent = '';
    const note = $('models-note');
    note.hidden = !DATA.private;
    note.textContent = S.ai_private_note;
    $('disclaimer').textContent = S.ai_disclaimer;
    if (!models || !models.probed) {
      const status = el('div', 'status');
      status.appendChild(el('span', 'spinner'));
      status.appendChild(el('span', '', S.ai_checking));
      list.appendChild(status);
      return;
    }

    // Auf diesem Mac (Apple)
    const device = models.device;
    list.appendChild(
      option('device', S.ai_e_device, S.ai_e_device_sub, device.ok, device.ok ? S.ai_ok : REASON[device.reason] || S.ai_r_unknown)
    );

    // Ollama
    const ollama = models.ollama;
    const ollamaOk = ollama.running && ollama.models.length > 0;
    const ollamaBox = option(
      'ollama',
      S.ai_e_ollama,
      S.ai_e_ollama_sub,
      ollamaOk,
      ollamaOk ? S.ai_ok : ollama.running ? S.ai_r_ollama_empty : S.ai_r_ollama_off
    );
    if (ollamaOk) {
      const extra = el('div', 'extra');
      extra.appendChild(
        picker(S.ai_model, ollama.models.map((name) => ({ id: name, name })), ollama.model, 'ollamaModel')
      );
      ollamaBox.appendChild(extra);
    }
    list.appendChild(ollamaBox);

    // Claude
    const claude = models.claude;
    const claudeOk = claude.key && !models.private;
    const claudeBox = option(
      'claude',
      S.ai_e_claude,
      S.ai_e_claude_sub,
      claudeOk,
      models.private ? S.ai_r_private : claude.key ? S.ai_ok : S.ai_r_no_key
    );
    const extra = el('div', 'extra');
    extra.appendChild(picker(S.ai_model, claude.models, claude.model, 'claudeModel'));
    const keyLine = el('div', 'line');
    if (claude.key) {
      keyLine.appendChild(el('span', 'saved', S.ai_key_saved));
      keyLine.appendChild(
        button('tbtn', S.ai_key_remove, () => {
          keyError = '';
          post({ c: 'aiKeyDelete' });
        })
      );
    } else {
      const field = document.createElement('input');
      field.type = 'password';
      field.id = 'key-field';
      field.autocomplete = 'off';
      field.spellcheck = false;
      field.placeholder = S.ai_key;
      field.value = keyDraft;
      const save = button('tbtn primary', S.ai_key_save, () => {
        const value = field.value.trim();
        if (!value) return;
        keyError = '';
        post({ c: 'aiKey', key: value });
      });
      field.addEventListener('input', () => {
        keyDraft = field.value;
      });
      field.addEventListener('keydown', (event) => {
        if (event.key === 'Enter') save.click();
      });
      keyLine.appendChild(field);
      keyLine.appendChild(save);
    }
    extra.appendChild(keyLine);
    if (keyError) extra.appendChild(el('div', 'small bad', keyError));
    extra.appendChild(el('div', 'small', S.ai_key_hint));
    extra.appendChild(el('div', 'small', S.ai_fallback_hint));
    claudeBox.appendChild(extra);
    list.appendChild(claudeBox);

    list.appendChild(renderOther());
  }

  // Ein anderer Dienst mit der verbreiteten Chat-Schnittstelle: auf diesem Mac
  // (LM Studio, llama.cpp) oder im Netz (OpenAI, OpenRouter, Mistral, eigene Adresse).
  function renderOther() {
    const data = models.other || {};
    const blocked = models.private && !data.local;
    const usable = !!data.usable && !blocked;
    let stateText = S.ai_ok;
    if (blocked) stateText = S.ai_r_private;
    else if (!data.url) stateText = S.ai_r_other_url;
    else if (data.status === 'off') stateText = S.ai_r_other_off;
    else if (data.status === 'denied') stateText = S.ai_r_other_denied;
    else if (!data.local && !data.key) stateText = S.ai_r_no_key;
    else if (!data.model) stateText = S.ai_r_other_model;
    const box = option('other', S.ai_e_other, S.ai_e_other_sub, usable, stateText);
    const extra = el('div', 'extra');

    // Welcher Dienst?
    const custom = other.custom || data.service === 'custom';
    const entries = (data.services || []).map((service) => ({
      id: service.id,
      name: service.local ? service.name + ' (' + S.ai_on_this_mac + ')' : service.name,
    }));
    entries.unshift({ id: '', name: S.ai_service_pick });
    entries.push({ id: 'custom', name: S.ai_service_custom });
    const line = el('div', 'line');
    line.appendChild(el('label', '', S.ai_service));
    const select = document.createElement('select');
    for (const entry of entries) {
      const item = document.createElement('option');
      item.value = entry.id;
      item.textContent = entry.name;
      item.selected = entry.id === (custom ? 'custom' : data.service || '');
      select.appendChild(item);
    }
    select.addEventListener('change', () => {
      other.custom = select.value === 'custom';
      other.urlDraft = null;
      other.modelDraft = null;
      if (other.custom) refreshModels();
      else if (select.value) post({ c: 'aiSetting', key: 'otherService', value: select.value });
    });
    line.appendChild(select);
    extra.appendChild(line);

    if (custom) {
      const urlLine = el('div', 'line');
      const field = document.createElement('input');
      field.type = 'text';
      field.id = 'other-url';
      field.autocomplete = 'off';
      field.spellcheck = false;
      field.placeholder = S.ai_service_url;
      field.value = other.urlDraft !== null ? other.urlDraft : data.url || '';
      const apply = button('tbtn', S.ai_apply, () => {
        const value = field.value.trim();
        if (!value) return;
        other.urlDraft = null;
        post({ c: 'aiSetting', key: 'otherUrl', value });
      });
      field.addEventListener('input', () => {
        other.urlDraft = field.value;
        // Solange hier eine andere Adresse steht als die eingetragene, nimmt
        // das Schlüsselfeld darunter nichts an.
        const pending = field.value.trim() !== (data.url || '');
        const key = document.getElementById('other-key');
        if (key) {
          key.disabled = pending;
          if (key.nextElementSibling) key.nextElementSibling.disabled = pending;
        }
      });
      field.addEventListener('keydown', (event) => {
        if (event.key === 'Enter') apply.click();
      });
      urlLine.appendChild(field);
      urlLine.appendChild(apply);
      extra.appendChild(urlLine);
      if (data.status === 'badurl') extra.appendChild(el('div', 'small bad', S.ai_service_url_bad));
      extra.appendChild(el('div', 'small', S.ai_service_url_hint));
    }

    if (data.url) {
      // Schlüssel – auf diesem Mac braucht es keinen.
      if (!data.local) {
        const keyLine = el('div', 'line');
        if (data.key) {
          keyLine.appendChild(el('span', 'saved', S.ai_key_saved));
          keyLine.appendChild(
            button('tbtn', S.ai_key_remove, () => {
              other.keyError = '';
              post({ c: 'aiOtherKeyDelete' });
            })
          );
        } else {
          const field = document.createElement('input');
          field.type = 'password';
          field.id = 'other-key';
          field.autocomplete = 'off';
          field.spellcheck = false;
          field.placeholder = S.ai_other_key;
          field.value = other.keyDraft;
          // Der Schlüssel gehört zu der Adresse, die eingetragen ist. Steht im
          // Feld darüber eine andere, die noch nicht übernommen wurde, wird
          // nichts gesichert – er ginge sonst an den Dienst von vorher.
          const pending = custom && other.urlDraft !== null && other.urlDraft.trim() !== (data.url || '');
          const save = button('tbtn primary', S.ai_key_save, () => {
            const value = field.value.trim();
            if (!value || pending) return;
            other.keyError = '';
            post({ c: 'aiOtherKey', key: value, url: data.url || '' });
          });
          field.disabled = pending;
          save.disabled = pending;
          if (pending) other.keyError = S.ai_key_other_url;
          else if (other.keyError === S.ai_key_other_url) other.keyError = '';
          field.addEventListener('input', () => {
            other.keyDraft = field.value;
          });
          field.addEventListener('keydown', (event) => {
            if (event.key === 'Enter') save.click();
          });
          keyLine.appendChild(field);
          keyLine.appendChild(save);
        }
        extra.appendChild(keyLine);
        if (other.keyError) extra.appendChild(el('div', 'small bad', other.keyError));
        // OpenRouter kann den Schlüssel auch selbst ausstellen – nach Anmeldung dort.
        if (data.openrouter && !data.key) {
          const sign = el('div', 'line');
          sign.appendChild(
            button('tbtn', data.signingIn ? S.ai_oauth_again : S.ai_oauth, () => post({ c: 'aiOAuthStart' }))
          );
          extra.appendChild(sign);
          extra.appendChild(el('div', 'small', data.signingIn ? S.ai_oauth_waiting : S.ai_oauth_hint));
        }
      }

      // Modell: aus der Liste des Dienstes – oder von Hand, wenn er keine nennt.
      const ready = data.local || data.key;
      if (ready && (data.models || []).length) {
        const names = data.models.map((name) => ({ id: name, name }));
        if (!data.model) names.unshift({ id: '', name: S.ai_r_other_model });
        extra.appendChild(picker(S.ai_model, names, data.model, 'otherModel'));
      } else if (ready) {
        const modelLine = el('div', 'line');
        const field = document.createElement('input');
        field.type = 'text';
        field.id = 'other-model';
        field.autocomplete = 'off';
        field.spellcheck = false;
        field.placeholder = S.ai_other_model;
        field.value = other.modelDraft !== null ? other.modelDraft : data.model || '';
        const apply = button('tbtn', S.ai_apply, () => {
          const value = field.value.trim();
          if (!value) return;
          other.modelDraft = null;
          post({ c: 'aiSetting', key: 'otherModel', value });
        });
        field.addEventListener('input', () => {
          other.modelDraft = field.value;
        });
        field.addEventListener('keydown', (event) => {
          if (event.key === 'Enter') apply.click();
        });
        modelLine.appendChild(field);
        modelLine.appendChild(apply);
        extra.appendChild(modelLine);
        if (data.status === 'checking') extra.appendChild(el('div', 'small', S.ai_other_checking));
      }
      extra.appendChild(
        el('div', 'small', data.local ? S.ai_other_local_hint : S.ai_other_remote_hint.replace('{host}', data.host || ''))
      );
    } else {
      extra.appendChild(el('div', 'small', S.ai_other_hint));
    }
    box.appendChild(extra);
    return box;
  }

  function refreshModels() {
    // Angefangene Eingaben und der Fokus überstehen das Neuzeichnen.
    const active = document.activeElement;
    const focused = active && ['key-field', 'other-url', 'other-key', 'other-model'].includes(active.id) ? active.id : '';
    renderModels();
    const again = focused ? $(focused) : null;
    if (again) again.focus();
  }

  // ------------------------------------------------------------------------
  // Eingabe
  // ------------------------------------------------------------------------

  function grow() {
    input.style.height = 'auto';
    input.style.height = Math.min(input.scrollHeight, 120) + 'px';
  }

  function submit() {
    const text = input.value.trim();
    if (!text || !page) return;
    input.value = '';
    grow();
    ask('ask', text);
  }

  input.addEventListener('input', () => {
    grow();
    renderComposer();
  });

  input.addEventListener('keydown', (event) => {
    if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      if (!running) submit();
    } else if (event.key === 'Escape' && running) {
      event.preventDefault();
      stop();
    }
  });

  $('send').addEventListener('click', () => {
    if (running) stop();
    else submit();
    input.focus();
  });

  $('engine').addEventListener('click', () => {
    view = 'models';
    post({ c: 'aiRefresh' });
    render();
  });

  $('models-done').addEventListener('click', () => {
    view = 'chat';
    render(true);
    input.focus();
  });

  $('recheck').addEventListener('click', () => post({ c: 'aiRefresh' }));

  $('clear').addEventListener('click', () => {
    const key = talkKey();
    if (key == null) return;
    if (running && running.tab === key) stop();
    const current = talks.get(key);
    // Ein laufender Vergleich bleibt bestehen – nur das Gespräch wird leer.
    if (current) current.items = [];
    render();
    input.focus();
  });

  $('close').addEventListener('click', () => post({ c: 'aiClose' }));

  // ------------------------------------------------------------------------
  // Nachrichten vom Rust-Teil
  // ------------------------------------------------------------------------

  function onPage(message) {
    page = message;
    // Geschlossene Tabs: ihre Unterhaltungen vergessen und aus Vergleichen nehmen.
    const alive = new Set(message.tabs || []);
    for (const id of Array.from(talks.keys())) {
      if (!alive.has(id)) talks.delete(id);
    }
    if (running && !alive.has(running.tab)) running = null;
    for (const other of talks.values()) {
      if (!other.others) continue;
      other.others = other.others.filter((entry) => alive.has(entry.id));
      if (!other.others.length) {
        other.others = null;
        other.items.push({ kind: 'mode', title: S.ai_compare_end });
      }
    }

    const current = talk();
    // Nur der Tab, dem die Unterhaltung gehört, bestimmt, ob eine neue Seite vorliegt.
    if (talkKey() === message.tab) {
      if (current.key !== message.key) {
        // Im selben Tab liegt jetzt eine andere Seite vor: Die Unterhaltung
        // bleibt sichtbar, das Modell fängt aber neu an.
        current.key = message.key;
        if (current.items.length) current.items.push({ kind: 'page', key: message.key, title: message.title });
      } else {
        const last = current.items[current.items.length - 1];
        if (last && last.kind === 'page' && last.key === message.key) last.title = message.title;
      }
    }
    if (view === 'pick') post({ c: 'aiTabs' });
    render(true);
  }

  window.__zunderAi = (message) => {
    switch (message.t) {
      case 'aiState':
        models = message;
        if (view === 'models') refreshModels();
        else if (view === 'chat') renderComposer();
        break;
      case 'aiPage':
        onPage(message);
        break;
      case 'aiTabs':
        tabList = message.tabs || [];
        for (const id of Array.from(pick.chosen)) {
          if (!tabList.some((tab) => tab.id === id && tab.ready)) pick.chosen.delete(id);
        }
        if (view === 'pick') renderPick();
        break;
      case 'aiFocus':
        if (view === 'chat') input.focus();
        break;
      case 'aiRun':
        if (message.action === 'pick') openPicker();
        else if (!running && page && page.readable) ask(message.action, '');
        break;
      case 'aiStarted':
        if (running && running.job == null) {
          running.job = message.job;
          update((answer) => {
            answer.engine = message.engine;
            answer.status = S.ai_reading;
          });
        } else {
          // Niemand wartet mehr auf diese Anfrage.
          post({ c: 'aiStop' });
        }
        break;
      case 'aiFailed':
        if (running && running.job == null) finish('error', message.text);
        break;
      case 'aiContext':
        if (mine(message)) {
          update((answer) => {
            answer.ctx = message.kind;
            answer.pages = message.pages;
            answer.status = S.ai_waiting;
          });
        }
        break;
      case 'aiStatus':
        if (mine(message)) update((answer) => (answer.status = message.text));
        break;
      case 'aiDelta':
        if (mine(message)) {
          update((answer) => {
            answer.text += message.text;
            answer.status = '';
          });
        }
        break;
      case 'aiReset':
        if (mine(message)) update((answer) => (answer.text = ''));
        break;
      case 'aiDone':
        if (mine(message)) {
          running.answer.text = message.text;
          running.answer.note = message.note || '';
          running.answer.asked = message.asked;
          finish('done');
        }
        break;
      case 'aiError':
        if (mine(message)) finish('error', message.text);
        break;
      case 'aiStopped':
        if (mine(message)) finish('stopped');
        break;
      case 'aiKeyResult':
        if (message.other) {
          other.keyError = message.ok ? '' : message.text || '';
          if (message.ok) other.keyDraft = '';
        } else if (message.ok) {
          keyDraft = '';
          keyError = '';
        } else {
          keyError = message.text || '';
        }
        if (view === 'models') refreshModels();
        break;
    }
  };

  // ------------------------------------------------------------------------
  // Start
  // ------------------------------------------------------------------------

  document.body.classList.toggle('private', !!DATA.private);
  $('close').title = S.close;
  $('clear').title = S.ai_clear;
  $('compare-end').title = S.ai_compare_end;
  $('recheck').textContent = S.ai_recheck;
  $('models-done').textContent = S.ai_done;
  $('pick-cancel').textContent = S.cancel;
  $('pick-go').textContent = S.ai_pick_go;

  // Das eingebaute Kontextmenü gibt es nur in Eingabefeldern.
  window.addEventListener('contextmenu', (event) => {
    if (!(event.target instanceof HTMLTextAreaElement || event.target instanceof HTMLInputElement)) {
      event.preventDefault();
    }
  });
  window.addEventListener('dragover', (event) => event.preventDefault());
  window.addEventListener('drop', (event) => event.preventDefault());

  render();
  post({ c: 'aiReady' });
})();
