// Aufgaben – eine eingebaute App.
//
// Todos und Notizen auf Kanban-Boards. Jede Karte liegt in einer Spalte eines
// Boards; ziehen verschiebt sie. Alles bleibt beim Browser auf diesem Mac
// (src/apps.rs): Boards und Karten als einzelne Karteikarten, die Einstellungen
// (welches Board, welche Ansicht) auf dem Zettel der App. Ins Netz geht die App
// nicht. Ein Board lässt sich als Markdown-Checkliste sichern.
(() => {
  'use strict';
  const Z = window.ZunderApp;
  const S = Z.strings;
  const LANG = Z.data.lang === 'de' ? 'de' : 'en';
  const $ = (id) => document.getElementById(id);
  const text = (key, values) =>
    String(S[key] || '').replace(/\{(\w+)\}/g, (whole, name) => (values && name in values ? values[name] : whole));

  // Farben einer Karte. Leer = ohne Rand; sonst „wichtig“ mit farbigem Streifen.
  const COLORS = ['', '#e03131', '#f08c00', '#2f9e44', '#1c7ed6', '#ae3ec9'];

  let kept = false;
  let ready = false;
  const boards = new Map(); // id → { k:'b', name, cols:[{id,name}], order }
  const cards = new Map(); // id → { k:'c', b, col, kind, title, body, done, due, color, order, when }
  const settings = { board: '', view: 'board' };
  let editing = ''; // Karte im Editor
  let edDraft = null; // vorgemerkte Änderungen im Editor (erst beim Sichern übernommen)
  let undo = null;

  const newId = (p) => p + '-' + Date.now().toString(36) + Math.random().toString(36).slice(2, 7);
  const clip = (value, max) => Z.clip(String(value == null ? '' : value).replace(/\s+/g, ' ').trim(), max);
  // Wie `clip`, behält aber Zeilenumbrüche (und Tabs) – für die längere Notiz.
  function cleanBody(value, max) {
    const out = [];
    for (const ch of String(value == null ? '' : value)) {
      const code = ch.charCodeAt(0);
      if (ch.length === 1 && code >= 0xd800 && code <= 0xdfff) continue;
      if (code === 10 || code === 9 || (code >= 32 && code !== 127)) {
        out.push(ch);
        if (out.length >= max) break;
      }
    }
    return out.join('').replace(/[ \t]+\n/g, '\n').replace(/\n{3,}/g, '\n\n').trim();
  }

  function node(tag, className, content) {
    const element = document.createElement(tag);
    if (className) element.className = className;
    if (content !== undefined) element.textContent = content;
    return element;
  }

  function icon(name) {
    const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    const use = document.createElementNS('http://www.w3.org/2000/svg', 'use');
    use.setAttribute('href', '#i-' + name);
    svg.appendChild(use);
    return svg;
  }

  // ------------------------------------------------------------------------
  // Datum
  // ------------------------------------------------------------------------

  const two = (value) => String(value).padStart(2, '0');
  const keyOf = (date) => date.getFullYear() + '-' + two(date.getMonth() + 1) + '-' + two(date.getDate());
  const dateOf = (key) => {
    const parts = String(key).split('-').map(Number);
    return new Date(parts[0], (parts[1] || 1) - 1, parts[2] || 1);
  };
  const validDay = (key) => typeof key === 'string' && /^\d{4}-\d{2}-\d{2}$/.test(key) && keyOf(dateOf(key)) === key;
  const today = () => keyOf(new Date());

  // Ein Datumswort aus einer Zeile: „heute“, „morgen“, Wochentag, 23.12., 2026-12-23.
  function readDue(word) {
    const lower = word.toLowerCase();
    const now = new Date();
    // Nur eindeutige Wörter – bloße Wochentagskürzel wie „do“/„so“ wären in
    // einer englischen Zeile („to do“) zu leicht zu verwechseln.
    if (lower === 'heute' || lower === 'today') return today();
    if (lower === 'morgen' || lower === 'tomorrow') {
      now.setDate(now.getDate() + 1);
      return keyOf(now);
    }
    let match = /^(\d{1,2})\.(\d{1,2})\.(\d{4}|\d{2})?$/.exec(word);
    if (match) {
      const year = match[3] ? (match[3].length === 2 ? 2000 + Number(match[3]) : Number(match[3])) : now.getFullYear();
      const key = year + '-' + two(match[2]) + '-' + two(match[1]);
      return validDay(key) ? key : '';
    }
    if (validDay(word)) return word;
    return '';
  }

  function dueLabel(due) {
    if (due === today()) return S.tk_today;
    const now = today();
    const tomorrow = keyOf(new Date(Date.now() + 86400000));
    if (due === tomorrow) return S.tk_tomorrow;
    const options = dateOf(due).getFullYear() === new Date().getFullYear() ? { day: 'numeric', month: 'short' } : { day: 'numeric', month: 'short', year: 'numeric' };
    return dateOf(due).toLocaleDateString(LANG, options);
  }

  // ------------------------------------------------------------------------
  // Ablage
  // ------------------------------------------------------------------------

  function stored(state) {
    if (!kept || state === true || state === 'ok') return;
    toast(state === 'full' ? S.tk_full : S.tk_save_failed);
  }

  function saveNote() {
    Z.save({ v: 1, board: settings.board, view: settings.view }).then(stored);
  }

  function putBoard(id, board) {
    boards.set(id, board);
    Z.records.put(id, board).then(stored);
  }

  function dropBoard(id) {
    boards.delete(id);
    Z.records.remove(id).then(stored);
  }

  // Eine Karteikarte darf höchstens ~1500 Bytes groß sein (src/apps.rs). Ist sie
  // zu groß (lange Notiz), wird die Notiz gekürzt, damit das Sichern nie an der
  // Größe scheitert und die Karte nicht beim nächsten Laden fehlt.
  function fit(card) {
    const budget = 1400;
    const size = (value) => new TextEncoder().encode(JSON.stringify(value)).length;
    if (size(card) <= budget) return card;
    let body = card.body;
    while (body && size({ ...card, body }) > budget) {
      body = body.slice(0, Math.max(0, body.length - Math.max(4, Math.ceil(body.length / 16))));
    }
    card.body = body.trimEnd();
    return card;
  }

  function putCard(id, card) {
    fit(card);
    cards.set(id, card);
    Z.records.put(id, card).then(stored);
  }

  function dropCard(id) {
    cards.delete(id);
    Z.records.remove(id).then(stored);
  }

  function cleanBoard(value) {
    if (!value || typeof value !== 'object' || typeof value.name !== 'string') return null;
    const cols = [];
    for (const col of Array.isArray(value.cols) ? value.cols.slice(0, 20) : []) {
      if (col && typeof col.id === 'string' && /^[A-Za-z0-9_-]{1,40}$/.test(col.id) && typeof col.name === 'string') {
        cols.push({ id: col.id, name: clip(col.name, 40) || '–' });
      }
    }
    if (!cols.length) return null;
    return { k: 'b', name: clip(value.name, 60) || S.tk_default_board, cols, order: Number(value.order) || 0 };
  }

  function cleanCard(value) {
    if (!value || typeof value !== 'object') return null;
    if (typeof value.b !== 'string' || typeof value.col !== 'string') return null;
    const kind = value.kind === 'note' ? 'note' : 'task';
    const color = COLORS.includes(value.color) ? value.color : '';
    const due = validDay(value.due) ? value.due : '';
    return {
      k: 'c',
      b: value.b,
      col: value.col,
      kind,
      title: clip(value.title, 200),
      body: cleanBody(value.body, 1200),
      done: value.done === true,
      due,
      color,
      order: Number(value.order) || 0,
      when: Number(value.when) || 0,
    };
  }

  // ------------------------------------------------------------------------
  // Boards und Spalten
  // ------------------------------------------------------------------------

  function orderedBoards() {
    return [...boards.entries()].sort((a, b) => a[1].order - b[1].order || a[1].name.localeCompare(b[1].name));
  }

  function currentBoard() {
    if (boards.has(settings.board)) return [settings.board, boards.get(settings.board)];
    const first = orderedBoards()[0];
    if (first) settings.board = first[0];
    return first || null;
  }

  function makeBoard(name) {
    const id = newId('b');
    const cols = [
      { id: 'c' + Math.random().toString(36).slice(2, 7), name: S.tk_col_todo },
      { id: 'c' + Math.random().toString(36).slice(2, 7), name: S.tk_col_doing },
      { id: 'c' + Math.random().toString(36).slice(2, 7), name: S.tk_col_done },
    ];
    const order = orderedBoards().reduce((max, [, b]) => Math.max(max, b.order), 0) + 1;
    putBoard(id, { k: 'b', name: clip(name, 60) || S.tk_new_board, cols, order });
    return id;
  }

  function cardsOf(boardId, colId) {
    const list = [];
    for (const [id, card] of cards) if (card.b === boardId && card.col === colId) list.push([id, card]);
    return list.sort((a, b) => a[1].order - b[1].order || a[1].when - b[1].when);
  }

  // Die Karten einer Spalte neu durchnummerieren; nur geänderte werden gesichert.
  function reindex(boardId, colId) {
    cardsOf(boardId, colId).forEach(([id, card], index) => {
      if (card.order !== index) {
        card.order = index;
        putCard(id, card);
      }
    });
  }

  // ------------------------------------------------------------------------
  // Schnelleingabe: Titel, „!“ wichtig, ein Datumswort am Ende
  // ------------------------------------------------------------------------

  function addCard(boardId, colId, raw) {
    let title = raw.replace(/\s+/g, ' ').trim();
    if (!title) return;
    let color = '';
    if (/(^|\s)!($|\s)/.test(title)) {
      color = COLORS[1];
      title = title.replace(/(^|\s)!($|\s)/, ' ').trim();
    }
    let due = '';
    const words = title.split(' ');
    if (words.length > 1) {
      const maybe = readDue(words[words.length - 1]);
      if (maybe) {
        due = maybe;
        words.pop();
        title = words.join(' ');
      }
    }
    const order = cardsOf(boardId, colId).reduce((max, [, c]) => Math.max(max, c.order), -1) + 1;
    const id = newId('c');
    putCard(id, { k: 'c', b: boardId, col: colId, kind: 'task', title: clip(title, 200), body: '', done: false, due, color, order, when: Date.now() });
    return id;
  }

  // ------------------------------------------------------------------------
  // Anzeige
  // ------------------------------------------------------------------------

  function render() {
    if (!ready) return;
    renderBar();
    const board = currentBoard();
    $('empty').hidden = !!board;
    $('view-board').hidden = !board || settings.view !== 'board';
    $('view-list').hidden = !board || settings.view !== 'list';
    if (!board) {
      $('empty').textContent = S.tk_empty_board;
      return;
    }
    if (settings.view === 'board') renderBoardView(board);
    else renderListView(board);
  }

  function renderBar() {
    const box = $('boards');
    box.textContent = '';
    for (const [id, board] of orderedBoards()) {
      const pill = node('button', 'board-pill' + (id === settings.board ? ' on' : ''));
      pill.type = 'button';
      pill.append(board.name);
      if (id === settings.board) {
        const dot = node('span', 'menu-dot');
        dot.appendChild(icon('chevron-down'));
        dot.addEventListener('click', (event) => {
          event.stopPropagation();
          openBoardMenu(id, dot);
        });
        pill.appendChild(dot);
      }
      pill.addEventListener('click', () => {
        settings.board = id;
        saveNote();
        render();
      });
      box.appendChild(pill);
    }
    const add = node('button', 'round', '');
    add.id = 'add-board';
    add.type = 'button';
    add.title = S.tk_add_board;
    add.setAttribute('aria-label', S.tk_add_board);
    add.appendChild(icon('plus'));
    add.addEventListener('click', () => {
      const id = makeBoard('');
      settings.board = id;
      saveNote();
      render();
    });
    box.appendChild(add);

    const view = $('view');
    view.textContent = '';
    for (const [key, label] of [['board', S.tk_view_board], ['list', S.tk_view_list]]) {
      const button = node('button', settings.view === key ? 'on' : '', label);
      button.type = 'button';
      button.addEventListener('click', () => {
        settings.view = key;
        saveNote();
        render();
      });
      view.appendChild(button);
    }
    $('export').textContent = S.tk_export;
  }

  // ---- Board-Ansicht ----------------------------------------------------------

  function renderBoardView([boardId, board]) {
    const wrap = $('view-board');
    wrap.textContent = '';
    for (const col of board.cols) {
      wrap.appendChild(columnEl(boardId, board, col));
    }
    const add = node('button', '', '+ ' + S.tk_add_column_full);
    add.id = 'add-column';
    add.type = 'button';
    add.addEventListener('click', () => {
      board.cols.push({ id: 'c' + Math.random().toString(36).slice(2, 7), name: S.tk_add_column });
      putBoard(boardId, board);
      render();
    });
    wrap.appendChild(add);
  }

  function columnEl(boardId, board, col) {
    const list = cardsOf(boardId, col.id);
    const column = node('div', 'column');
    column.dataset.col = col.id;

    const head = node('div', 'col-head');
    const name = node('span', 'name', col.name);
    name.addEventListener('dblclick', () => renameColumn(boardId, board, col, head, name));
    const count = node('span', 'count', String(list.length));
    const menu = node('button', 'round');
    menu.type = 'button';
    menu.title = S.tk_column_name;
    menu.setAttribute('aria-label', S.tk_rename_column);
    menu.appendChild(icon('chevron-down'));
    menu.addEventListener('click', (event) => {
      event.stopPropagation();
      openColumnMenu(boardId, board, col, menu, head, name);
    });
    head.append(name, count, menu);
    column.appendChild(head);

    const cardBox = node('div', 'col-cards');
    cardBox.dataset.col = col.id;
    for (const [id, card] of list) cardBox.appendChild(cardEl(id, card));
    column.appendChild(cardBox);

    const addRow = node('div', 'col-add');
    const input = document.createElement('input');
    input.type = 'text';
    input.placeholder = S.tk_add_card;
    input.setAttribute('aria-label', S.tk_add_card);
    input.addEventListener('keydown', (event) => {
      if (event.key === 'Enter' && input.value.trim()) {
        addCard(boardId, col.id, input.value);
        input.value = '';
        render();
        // Nach dem Neuzeichnen das passende Feld wieder in den Fokus holen.
        focusAdd(col.id);
      } else if (event.key === 'Escape') {
        input.value = '';
      }
    });
    addRow.appendChild(input);
    column.appendChild(addRow);
    return column;
  }

  function focusAdd(colId) {
    const input = document.querySelector('.column[data-col="' + CSS.escape(colId) + '"] .col-add input');
    if (input) input.focus();
  }

  function cardEl(id, card) {
    const element = node('div', 'card' + (card.done ? ' done' : '') + (card.color ? ' important' : ''));
    element.dataset.id = id;
    if (card.color) element.style.setProperty('--dot', card.color);

    const top = node('div', 'card-top');
    if (card.kind === 'task') {
      const tick = node('span', 'tick' + (card.done ? ' on' : ''));
      tick.setAttribute('role', 'checkbox');
      tick.setAttribute('aria-checked', String(card.done));
      if (card.done) tick.appendChild(icon('check'));
      tick.addEventListener('pointerdown', (event) => event.stopPropagation());
      tick.addEventListener('click', (event) => {
        event.stopPropagation();
        card.done = !card.done;
        putCard(id, card);
        render();
      });
      top.appendChild(tick);
    }
    top.appendChild(node('span', 'title', card.title || '–'));
    const grip = node('span', 'grip');
    grip.appendChild(icon('grip'));
    top.appendChild(grip);
    element.appendChild(top);

    if (card.body) element.appendChild(node('p', 'snippet', card.body));

    const meta = node('div', 'meta');
    if (card.kind === 'note') {
      const tag = node('span', 'chip note-tag');
      tag.append(icon('edit'), S.tk_kind_note);
      meta.appendChild(tag);
    }
    if (card.due) {
      const overdue = card.due < today() && !card.done;
      const chip = node('span', 'chip' + (overdue ? ' overdue' : card.due === today() ? ' due-today' : ''));
      chip.append(icon('calendar'), overdue ? dueLabel(card.due) + ' · ' + S.tk_overdue : dueLabel(card.due));
      meta.appendChild(chip);
    }
    if (meta.children.length) element.appendChild(meta);

    element.addEventListener('pointerdown', (event) => startDrag(event, id, element));
    return element;
  }

  // ---- Listen-Ansicht ---------------------------------------------------------

  function renderListView([boardId, board]) {
    const wrap = $('view-list');
    wrap.textContent = '';
    for (const col of board.cols) {
      const list = cardsOf(boardId, col.id);
      const group = node('div', 'group');
      const title = node('h2', '');
      title.append(col.name, node('span', 'count', ' ' + list.length));
      group.appendChild(title);
      for (const [id, card] of list) {
        const row = node('div', 'row' + (card.done ? ' done' : ''));
        row.dataset.id = id;
        if (card.kind === 'task') {
          const tick = node('span', 'tick' + (card.done ? ' on' : ''));
          if (card.done) tick.appendChild(icon('check'));
          tick.addEventListener('click', (event) => {
            event.stopPropagation();
            card.done = !card.done;
            putCard(id, card);
            render();
          });
          row.appendChild(tick);
        } else {
          row.appendChild(node('span', 'tick', ''));
        }
        const main = node('div', 'title');
        main.append(node('span', '', card.title || '–'));
        if (card.due) {
          const overdue = card.due < today() && !card.done;
          main.append(' ', node('span', 'chip' + (overdue ? ' overdue' : card.due === today() ? ' due-today' : ''), dueLabel(card.due)));
        }
        if (card.body) main.appendChild(node('p', 'snippet', card.body));
        row.appendChild(main);
        row.addEventListener('click', () => openEditor(id));
        group.appendChild(row);
      }
      const addRow = node('div', 'col-add');
      const input = document.createElement('input');
      input.type = 'text';
      input.placeholder = S.tk_add_card;
      input.setAttribute('aria-label', S.tk_add_card);
      input.style.cssText = 'width:100%;height:32px;padding:0 8px;border:0;border-radius:8px;background:var(--field);color:inherit';
      input.addEventListener('keydown', (event) => {
        if (event.key === 'Enter' && input.value.trim()) {
          addCard(boardId, col.id, input.value);
          input.value = '';
          render();
        }
      });
      addRow.appendChild(input);
      group.appendChild(addRow);
      wrap.appendChild(group);
    }
  }

  // ------------------------------------------------------------------------
  // Karte im Board anklicken: Editor
  // ------------------------------------------------------------------------

  $('view-board').addEventListener('click', (event) => {
    const card = event.target.closest('.card');
    if (card && !dragMoved) openEditor(card.dataset.id);
  });

  function openEditor(id) {
    const card = cards.get(id);
    if (!card) return;
    editing = id;
    // Änderungen erst vormerken; „Sichern“ übernimmt sie, „Abbrechen“ verwirft sie.
    edDraft = { kind: card.kind, color: card.color || '' };
    const sheet = $('editor');
    const kind = $('ed-kind');
    kind.textContent = '';
    for (const [value, label] of [['task', S.tk_kind_task], ['note', S.tk_kind_note]]) {
      const button = node('button', card.kind === value ? 'on' : '', label);
      button.type = 'button';
      button.addEventListener('click', () => {
        edDraft.kind = value;
        for (const other of kind.children) other.classList.toggle('on', other === button);
      });
      kind.appendChild(button);
    }
    $('ed-title').value = card.title;
    $('ed-title').placeholder = S.tk_title;
    $('ed-title').maxLength = 200;
    $('ed-note').value = card.body;
    $('ed-note').placeholder = S.tk_note;
    $('ed-note').maxLength = 1200;
    $('ed-due-label').textContent = S.tk_due;
    $('ed-due').value = card.due || '';
    const colors = $('ed-colors');
    colors.textContent = '';
    for (const color of COLORS) {
      const dot = node('button', color === (card.color || '') ? 'on' : '');
      dot.type = 'button';
      dot.style.background = color || 'var(--field)';
      dot.title = color ? S.tk_important : S.tk_color;
      dot.addEventListener('click', () => {
        edDraft.color = color;
        for (const other of colors.children) other.classList.toggle('on', other === dot);
      });
      colors.appendChild(dot);
    }
    $('ed-delete').textContent = S.tk_delete;
    $('ed-cancel').textContent = S.tk_cancel;
    $('ed-save').textContent = S.tk_save;
    sheet.hidden = false;
    setTimeout(() => $('ed-title').focus(), 0);
  }

  function closeEditor() {
    editing = '';
    edDraft = null;
    $('editor').hidden = true;
  }

  $('editor').querySelector('form').addEventListener('submit', (event) => {
    event.preventDefault();
    const card = cards.get(editing);
    if (card && edDraft) {
      card.title = clip($('ed-title').value, 200);
      card.body = cleanBody($('ed-note').value, 1200);
      card.due = validDay($('ed-due').value) ? $('ed-due').value : '';
      card.kind = edDraft.kind;
      card.color = edDraft.color;
      if (card.kind === 'note') card.done = false;
      putCard(editing, card);
    }
    closeEditor();
    render();
  });
  $('ed-cancel').addEventListener('click', closeEditor);
  $('ed-close').addEventListener('click', closeEditor);
  $('ed-delete').addEventListener('click', () => {
    const id = editing;
    const card = cards.get(id);
    closeEditor();
    if (card) {
      dropCard(id);
      render();
      toast(S.tk_deleted, () => putCard(id, card));
    }
  });
  $('editor').addEventListener('click', (event) => {
    if (event.target === $('editor')) closeEditor();
  });

  // ------------------------------------------------------------------------
  // Board- und Spalten-Menü
  // ------------------------------------------------------------------------

  function placeMenu(menu, anchor) {
    const box = anchor.getBoundingClientRect();
    menu.style.left = Math.min(box.left, window.innerWidth - 200) + 'px';
    menu.style.top = box.bottom + 6 + 'px';
  }

  function openBoardMenu(id, anchor) {
    const board = boards.get(id);
    const menu = $('board-menu');
    menu.textContent = '';
    const nameInput = document.createElement('input');
    nameInput.value = board.name;
    nameInput.maxLength = 60;
    nameInput.setAttribute('aria-label', S.tk_board_name);
    nameInput.addEventListener('keydown', (event) => {
      if (event.key === 'Enter') {
        const wanted = clip(nameInput.value, 60);
        if (wanted) {
          board.name = wanted;
          putBoard(id, board);
        }
        closeMenu();
        render();
      }
    });
    menu.appendChild(nameInput);
    const del = node('button', 'danger');
    del.type = 'button';
    del.append(icon('trash'), S.tk_delete_board);
    del.addEventListener('click', () => {
      closeMenu();
      deleteBoard(id);
    });
    // Löschen nur anbieten, wenn es mehr als ein Board gibt.
    if (boards.size > 1) menu.appendChild(del);
    placeMenu(menu, anchor);
    menu.hidden = false;
    setTimeout(() => nameInput.focus(), 0);
  }

  function deleteBoard(id) {
    const board = boards.get(id);
    const gone = [...cards.entries()].filter(([, c]) => c.b === id);
    dropBoard(id);
    for (const [cardId] of gone) dropCard(cardId);
    const next = orderedBoards()[0];
    settings.board = next ? next[0] : '';
    saveNote();
    render();
    toast(S.tk_board_deleted, () => {
      putBoard(id, board);
      for (const [cardId, card] of gone) putCard(cardId, card);
      settings.board = id;
      saveNote();
      render();
    });
  }

  function openColumnMenu(boardId, board, col, anchor, head, name) {
    const menu = $('board-menu');
    menu.textContent = '';
    const rename = node('button', '');
    rename.type = 'button';
    rename.append(icon('edit'), S.tk_rename_column);
    rename.addEventListener('click', () => {
      closeMenu();
      renameColumn(boardId, board, col, head, name);
    });
    menu.appendChild(rename);
    if (board.cols.length > 1) {
      const del = node('button', 'danger');
      del.type = 'button';
      del.append(icon('trash'), S.tk_delete_column);
      del.addEventListener('click', () => {
        closeMenu();
        deleteColumn(boardId, board, col);
      });
      menu.appendChild(del);
    }
    placeMenu(menu, anchor);
    menu.hidden = false;
  }

  function renameColumn(boardId, board, col, head, name) {
    const input = document.createElement('input');
    input.value = col.name;
    input.maxLength = 40;
    name.replaceWith(input);
    input.focus();
    input.select();
    let settled = false;
    const commit = () => {
      if (settled) return;
      settled = true;
      col.name = clip(input.value, 40) || col.name;
      putBoard(boardId, board);
      render();
    };
    input.addEventListener('keydown', (event) => {
      if (event.key === 'Enter') commit();
      else if (event.key === 'Escape') {
        settled = true;
        render();
      }
    });
    input.addEventListener('blur', commit);
  }

  function deleteColumn(boardId, board, col) {
    const index = board.cols.findIndex((other) => other.id === col.id);
    const fallback = board.cols[index === 0 ? 1 : index - 1];
    const moved = cardsOf(boardId, col.id);
    board.cols = board.cols.filter((other) => other.id !== col.id);
    putBoard(boardId, board);
    // Die Karten der Spalte wandern in die Nachbarspalte, statt verloren zu gehen.
    for (const [id, card] of moved) {
      card.col = fallback.id;
      putCard(id, card);
    }
    reindex(boardId, fallback.id);
    render();
  }

  function closeMenu() {
    $('board-menu').hidden = true;
  }
  document.addEventListener('click', (event) => {
    if (!$('board-menu').hidden && !$('board-menu').contains(event.target)) closeMenu();
  });

  // ------------------------------------------------------------------------
  // Ziehen (Zeiger): Karte in eine andere Spalte oder Position schieben
  // ------------------------------------------------------------------------

  let drag = null;
  let dragMoved = false;

  function startDrag(event, id, element) {
    if (event.button !== 0) return;
    dragMoved = false;
    drag = { id, element, startX: event.clientX, startY: event.clientY, ghost: null };
    element.setPointerCapture(event.pointerId);
    element.addEventListener('pointermove', onDragMove);
    element.addEventListener('pointerup', endDrag);
    element.addEventListener('pointercancel', endDrag);
  }

  function onDragMove(event) {
    if (!drag) return;
    if (!dragMoved) {
      if (Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY) < 6) return;
      // Jetzt ist es ein Ziehen: eine schwebende Kopie der Karte anlegen.
      dragMoved = true;
      drag.element.classList.add('dragging');
      const ghost = drag.element.cloneNode(true);
      ghost.id = 'drag-ghost';
      ghost.classList.remove('dragging');
      document.body.appendChild(ghost);
      drag.ghost = ghost;
    }
    drag.ghost.style.left = event.clientX - 20 + 'px';
    drag.ghost.style.top = event.clientY - 16 + 'px';
    // Spalte unter dem Zeiger hervorheben.
    const column = document.elementFromPoint(event.clientX, event.clientY);
    const target = column && column.closest('.column');
    for (const col of document.querySelectorAll('.column')) col.classList.toggle('drop', col === target);
  }

  function endDrag(event) {
    if (!drag) return;
    const element = drag.element;
    element.removeEventListener('pointermove', onDragMove);
    element.removeEventListener('pointerup', endDrag);
    element.removeEventListener('pointercancel', endDrag);
    element.classList.remove('dragging');
    if (drag.ghost) drag.ghost.remove();
    for (const col of document.querySelectorAll('.column')) col.classList.remove('drop');

    if (dragMoved) {
      const under = document.elementFromPoint(event.clientX, event.clientY);
      const column = under && under.closest('.column');
      const card = cards.get(drag.id);
      if (column && card) {
        const colId = column.dataset.col;
        const index = insertIndex(column, event.clientY, drag.id);
        moveCard(drag.id, colId, index);
      }
      // Ein Ziehen ist kein Klick: der Editor soll nicht aufgehen.
      setTimeout(() => (dragMoved = false), 0);
    }
    drag = null;
  }

  // An welche Stelle der Spalte? Vor die erste Karte, deren Mitte unter dem Zeiger liegt.
  function insertIndex(column, y, dragId) {
    const list = [...column.querySelectorAll('.col-cards .card')].filter((card) => card.dataset.id !== dragId);
    let index = list.length;
    for (let i = 0; i < list.length; i++) {
      const box = list[i].getBoundingClientRect();
      if (y < box.top + box.height / 2) {
        index = i;
        break;
      }
    }
    return index;
  }

  function moveCard(id, colId, index) {
    const card = cards.get(id);
    if (!card) return;
    const board = currentBoard();
    if (!board || !board[1].cols.some((col) => col.id === colId)) return;
    const fromCol = card.col;
    // Die Zielspalte ohne die gezogene Karte, dann an der Zielstelle einfügen.
    const rest = cardsOf(card.b, colId).filter(([other]) => other !== id);
    rest.splice(Math.min(index, rest.length), 0, [id, card]);
    card.col = colId;
    rest.forEach(([cardId, c], at) => {
      if (c.order !== at || cardId === id) {
        c.order = at;
        putCard(cardId, c);
      }
    });
    if (fromCol !== colId) reindex(card.b, fromCol);
    render();
  }

  // ------------------------------------------------------------------------
  // Als Markdown sichern
  // ------------------------------------------------------------------------

  $('export').addEventListener('click', () => {
    const board = currentBoard();
    if (!board) return;
    const [boardId, data] = board;
    const lines = ['# ' + data.name, ''];
    const notes = [];
    for (const col of data.cols) {
      const list = cardsOf(boardId, col.id);
      if (!list.length) continue;
      lines.push('## ' + col.name, '');
      for (const [, card] of list) {
        if (card.kind === 'note') {
          notes.push(card);
          continue;
        }
        let line = '- [' + (card.done ? 'x' : ' ') + '] ' + (card.title || '–');
        if (card.due) line += ' (' + dueLabel(card.due) + ')';
        lines.push(line);
        if (card.body) for (const part of card.body.split('\n')) lines.push('  ' + part);
      }
      lines.push('');
    }
    if (notes.length) {
      lines.push('## ' + S.tk_notes_heading, '');
      for (const note of notes) {
        lines.push('### ' + (note.title || '–'));
        if (note.body) lines.push(note.body);
        lines.push('');
      }
    }
    const name = S.tk_file + '-' + (data.name.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 40) || 'board') + '.md';
    Z.saveFile(name, new Blob([lines.join('\n')], { type: 'text/markdown;charset=utf-8' })).then((ok) => toast(ok ? S.tk_exported : S.tk_save_failed));
  });

  // ---- Meldung mit „Rückgängig“ -----------------------------------------------

  let toastTimer = 0;
  function toast(message, revert) {
    clearTimeout(toastTimer);
    undo = revert || null;
    $('toast-text').textContent = message;
    $('toast-undo').hidden = !revert;
    $('toast-undo').textContent = S.tk_undo;
    $('toast').hidden = false;
    toastTimer = setTimeout(() => {
      $('toast').hidden = true;
      undo = null;
    }, 6000);
  }
  $('toast-undo').addEventListener('click', () => {
    const revert = undo;
    undo = null;
    $('toast').hidden = true;
    if (revert) revert();
  });

  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') {
      if (!$('editor').hidden) closeEditor();
      else if (!$('board-menu').hidden) closeMenu();
    }
  });

  $('export').textContent = S.tk_export;
  $('trouble-text').textContent = S.tk_load_failed;
  $('trouble-retry').textContent = S.tk_retry;
  document.title = S.app_tasks;

  // ------------------------------------------------------------------------
  // Laden
  // ------------------------------------------------------------------------

  function load() {
    return Promise.all([Z.load(), Z.records.all()]).then(([note, records]) => {
      const ok = note.ok && records.ok;
      $('trouble').hidden = ok;
      if (!ok) {
        ready = false;
        return;
      }
      kept = note.kept && records.kept;
      const value = note.value;
      settings.board = typeof value.board === 'string' ? value.board : '';
      settings.view = value.view === 'list' ? 'list' : 'board';
      boards.clear();
      cards.clear();
      for (const [id, record] of records.list) {
        if (id.startsWith('b-')) {
          const board = cleanBoard(record);
          if (board) boards.set(id, board);
        } else if (id.startsWith('c-')) {
          const card = cleanCard(record);
          if (card) cards.set(id, card);
        }
      }
      // Immer ein Board bereitstellen, damit die App nicht leer startet – im
      // privaten Fenster lebt es nur für dieses Fenster (nichts wird gesichert).
      if (!boards.size) {
        settings.board = makeBoard(S.tk_default_board);
        saveNote();
      }
      $('private-note').hidden = kept;
      $('private-note').textContent = S.tk_private;
      ready = true;
      render();
    });
  }

  $('trouble-retry').addEventListener('click', load);

  // Kommt der Tab wieder nach vorn, den Stand neu lesen (zweiter Tab/Fenster) –
  // nicht mitten im Editieren oder Ziehen.
  let awaySince = 0;
  document.addEventListener('visibilitychange', () => {
    if (document.hidden) awaySince = Date.now();
    else if (kept && awaySince && Date.now() - awaySince > 1500 && !editing && !drag && $('editor').hidden && $('board-menu').hidden) {
      load();
    }
  });

  load();
})();
