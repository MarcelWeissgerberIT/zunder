// Startseite eines neuen Tabs. Die Daten (Lesezeichen, oft Besuchtes) hat der
// Browser beim Ausliefern mitgegeben; die Seite selbst kann ihm nichts befehlen.
(() => {
  'use strict';
  const DATA = JSON.parse(document.getElementById('data').textContent);
  const S = DATA.strings;
  const $ = (id) => document.getElementById(id);

  document.title = S.new_tab_title;

  // Eingebaute Apps: Kacheln, die zu ihren Seiten führen. Öffnen kann eine App
  // außer dem Browser nur diese Seite hier – über genau diese Adressen.
  function showApps() {
    const section = $('apps');
    const box = section.querySelector('.tiles');
    for (const app of DATA.apps || []) {
      if (!/^zunder:\/\/app-[a-z]+\/$/.test(app.url)) continue;
      const link = document.createElement('a');
      link.className = 'tile app';
      link.href = app.url;
      link.title = app.about || app.name;
      const picture = document.createElement('span');
      picture.className = 'icon';
      const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
      const use = document.createElementNS('http://www.w3.org/2000/svg', 'use');
      use.setAttribute('href', '#i-' + String(app.icon).replace(/[^a-z0-9-]/g, ''));
      svg.appendChild(use);
      picture.appendChild(svg);
      const label = document.createElement('span');
      label.className = 'label';
      label.textContent = app.name;
      const where = document.createElement('span');
      where.className = 'where';
      where.textContent = app.about || '';
      link.append(picture, label, where);
      box.appendChild(link);
    }
    if (box.children.length) {
      section.querySelector('h2').textContent = S.nt_apps;
      section.hidden = false;
    }
  }

  // Stealth-Fenster: sagt offen, was es leistet – und was nicht.
  if (DATA.stealth) {
    document.body.classList.add('private', 'stealth');
    $('logo').hidden = true;
    const box = $('stealth');
    box.hidden = false;
    box.querySelector('h1').textContent = S.nt_stealth_title;
    box.querySelector('.lead').textContent = S.nt_stealth_lead;
    const list = $('stealth-list');
    const add = (ok, title, text) => {
      const item = document.createElement('li');
      item.className = ok ? 'ok' : 'open';
      const mark = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
      const use = document.createElementNS('http://www.w3.org/2000/svg', 'use');
      use.setAttribute('href', ok ? '#i-check' : '#i-warning');
      mark.appendChild(use);
      const body = document.createElement('div');
      const head = document.createElement('b');
      head.textContent = title;
      const more = document.createElement('span');
      more.textContent = text;
      body.append(head, more);
      item.append(mark, body);
      list.appendChild(item);
    };
    add(true, S.nt_st_local, S.nt_st_local_text);
    add(true, S.nt_st_island, S.nt_st_island_text);
    add(true, S.nt_st_block, S.nt_st_block_text);
    add(true, S.nt_st_print, S.nt_st_print_text);
    if (DATA.tor) {
      add(true, S.nt_st_tor_on, S.nt_st_tor_on_text.replace('{port}', DATA.tor));
    } else {
      add(false, S.nt_st_tor_off, S.nt_st_tor_off_text);
      const how = $('stealth-how');
      how.hidden = false;
      how.querySelector('p').textContent = S.nt_st_tor_how;
      how.querySelector('code').textContent = 'brew install tor && brew services start tor';
    }
    box.querySelector('.note').textContent = S.nt_st_note;
    $('hint').textContent = S.nt_hint;
    return;
  }

  if (DATA.private) {
    document.body.classList.add('private');
    $('logo').hidden = true;
    $('private').hidden = false;
    $('private').querySelector('h1').textContent = S.nt_private_title;
    $('private').querySelector('p').textContent = S.nt_private_text;
    $('hint').textContent = S.nt_hint;
    showApps();
    return;
  }

  function hostOf(url) {
    try {
      return new URL(url).hostname.replace(/^www\./, '');
    } catch (error) {
      return '';
    }
  }

  // Farbe aus dem Namen ableiten, damit jede Seite ihre feste Kachel hat.
  function colorFor(text) {
    let hash = 0;
    for (let i = 0; i < text.length; i++) hash = (hash * 31 + text.charCodeAt(i)) | 0;
    return 'hsl(' + (Math.abs(hash) % 360) + ' 52% 46%)';
  }

  // Bild oder Anfangsbuchstabe für eine Seite – in voller oder kleiner Größe.
  function iconFor(entry, className) {
    const host = hostOf(entry.url);
    const icon = document.createElement('span');
    icon.className = className;
    const letter = () => {
      icon.textContent = (host[0] || '?').toUpperCase();
      icon.style.background = colorFor(host);
    };
    if (entry.icon && entry.icon.startsWith('data:image/png;base64,')) {
      const img = document.createElement('img');
      img.addEventListener('error', letter, { once: true });
      img.src = entry.icon;
      icon.appendChild(img);
    } else {
      letter();
    }
    return icon;
  }

  function tile(entry) {
    if (!/^https?:\/\//i.test(entry.url)) return null;
    const host = hostOf(entry.url);
    const link = document.createElement('a');
    link.className = 'tile';
    link.href = entry.url;
    link.title = entry.title || entry.url;

    const label = document.createElement('span');
    label.className = 'label';
    label.textContent = entry.title || host;

    // Den Titel bestimmt die Seite selbst – darunter steht deshalb immer, wohin
    // die Kachel wirklich führt.
    const where = document.createElement('span');
    where.className = 'where';
    where.textContent = host;

    link.append(iconFor(entry, 'icon'), label, where);
    return link;
  }

  function fill(sectionId, heading, entries) {
    const section = $(sectionId);
    const box = section.querySelector('.tiles');
    for (const entry of entries || []) {
      const node = tile(entry);
      if (node) box.appendChild(node);
    }
    if (box.children.length) {
      section.querySelector('h2').textContent = heading;
      section.hidden = false;
    }
  }

  const data = DATA.data || {};
  showApps();

  // ------------------------------------------------------------------------
  // Schreibtisch: Lesezeichen als Kacheln, die sich durch Ziehen anordnen und
  // in Ordner legen lassen. Die Seite meldet dem Browser nur, was wohin
  // gezogen wurde – anlegen oder löschen kann sie nichts.
  // ------------------------------------------------------------------------

  const marks = (data.bookmarks || []).filter((entry) => /^https?:\/\//i.test(entry.url));
  const folders = data.folders || [];
  let dragged = null;
  let openFolder = null;

  function tell(change) {
    try {
      window.webkit.messageHandlers.ipc.postMessage('b:' + JSON.stringify(change));
    } catch (error) {
      /* ohne Browser dahinter bleibt es bei der Anzeige */
    }
  }

  // Legt ein Lesezeichen in einen Ordner ('' = keinen) vor ein anderes – oder
  // ans Ende –, hier in der Anzeige und beim Browser.
  function move(id, folder, before) {
    const from = marks.findIndex((entry) => entry.id === id);
    if (from < 0 || id === before) return;
    const [entry] = marks.splice(from, 1);
    entry.folder = folder;
    let to = before ? marks.findIndex((other) => other.id === before) : -1;
    if (to < 0) {
      // Ans Ende seines Ordners: hinter das letzte, das dort liegt.
      to = marks.length;
      for (let i = marks.length - 1; i >= 0; i--) {
        if ((marks[i].folder || '') === folder) {
          to = i + 1;
          break;
        }
      }
    }
    marks.splice(to, 0, entry);
    tell({ id, folder, before: before || 0 });
    renderDesk();
  }

  function dropZone(node, folder, before) {
    node.addEventListener('dragover', (event) => {
      if (dragged == null || dragged === before) return;
      event.preventDefault();
      event.stopPropagation();
      event.dataTransfer.dropEffect = 'move';
      node.classList.add('drop');
    });
    node.addEventListener('dragleave', () => node.classList.remove('drop'));
    node.addEventListener('drop', (event) => {
      event.preventDefault();
      event.stopPropagation();
      node.classList.remove('drop');
      const id = dragged;
      dragged = null;
      if (id != null) move(id, folder, before);
    });
  }

  function deskTile(entry) {
    const node = tile(entry);
    node.draggable = true;
    node.addEventListener('dragstart', (event) => {
      dragged = entry.id;
      node.classList.add('dragging');
      event.dataTransfer.effectAllowed = 'move';
      event.dataTransfer.setData('text/uri-list', entry.url);
      event.dataTransfer.setData('text/plain', entry.url);
    });
    node.addEventListener('dragend', () => {
      dragged = null;
      node.classList.remove('dragging');
    });
    dropZone(node, entry.folder || '', entry.id);
    return node;
  }

  function folderTile(name) {
    const inside = marks.filter((entry) => entry.folder === name);
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'tile folder';
    button.title = name;
    const stack = document.createElement('span');
    stack.className = 'icon stack';
    for (const entry of inside.slice(0, 4)) stack.appendChild(iconFor(entry, 'mini'));
    const label = document.createElement('span');
    label.className = 'label';
    label.textContent = name;
    const where = document.createElement('span');
    where.className = 'where';
    where.textContent = String(inside.length);
    button.append(stack, label, where);
    button.addEventListener('click', () => {
      openFolder = name;
      renderDesk();
    });
    dropZone(button, name, 0);
    return button;
  }

  function renderDesk() {
    const section = $('bookmarks');
    const box = section.querySelector('.tiles');
    box.textContent = '';
    for (const entry of marks.filter((entry) => !entry.folder)) box.appendChild(deskTile(entry));
    for (const name of folders) box.appendChild(folderTile(name));
    section.querySelector('h2').textContent = S.nt_favorites;
    section.hidden = !box.children.length;

    const window_ = $('desk-folder');
    window_.hidden = openFolder == null;
    if (openFolder != null) {
      window_.querySelector('h3').textContent = openFolder;
      const inner = window_.querySelector('.tiles');
      inner.textContent = '';
      const inside = marks.filter((entry) => entry.folder === openFolder);
      for (const entry of inside) inner.appendChild(deskTile(entry));
      const note = window_.querySelector('.out');
      note.textContent = inside.length ? S.nt_folder_out : S.bm_folder_empty;
    }
  }

  // Der Ordner als kleines Fenster über dem Schreibtisch: Ein Klick daneben
  // schließt ihn, und wer eine Kachel daneben fallen lässt, legt sie zurück
  // auf den Schreibtisch.
  $('desk-folder').addEventListener('click', (event) => {
    if (event.target === $('desk-folder')) {
      openFolder = null;
      renderDesk();
    }
  });
  window.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && openFolder != null) {
      openFolder = null;
      renderDesk();
    }
  });
  dropZone($('desk-folder'), '', 0);
  $('desk-folder').addEventListener('drop', () => {
    openFolder = null;
    renderDesk();
  });
  // Im Fenster selbst: ans Ende dieses Ordners.
  const folderWindow = $('desk-folder').querySelector('.win');
  folderWindow.addEventListener('dragover', (event) => {
    if (dragged == null) return;
    event.preventDefault();
    event.stopPropagation();
  });
  folderWindow.addEventListener('drop', (event) => {
    event.preventDefault();
    event.stopPropagation();
    const id = dragged;
    dragged = null;
    if (id != null && openFolder != null) move(id, openFolder, 0);
  });

  renderDesk();
  const bookmarked = new Set(marks.map((b) => hostOf(b.url)));
  fill('top', S.nt_top, (data.top || []).filter((t) => !bookmarked.has(hostOf(t.url))).slice(0, 12));
  $('hint').textContent = marks.length > 1 ? S.nt_hint_desk : S.nt_hint;

  // ------------------------------------------------------------------------
  // Live-Hintergrund: eine kurze 3D-Szene, die sich nahtlos wiederholt. Das
  // Standbild ist sofort da; das Video blendet darüber ein, sobald es läuft.
  // ------------------------------------------------------------------------

  const walls = data.wallpapers || [];
  if (!walls.length) return;

  let chosen = data.wallpaper || '';
  const still = $('wall-still');
  const video = $('wall-video');
  const calm = window.matchMedia('(prefers-reduced-motion: reduce)');

  function showWall(id) {
    const wall = walls.find((entry) => entry.id === id);
    document.body.classList.toggle('walled', !!wall);
    document.body.classList.toggle('wall-light', !!wall && wall.light);
    $('wall').hidden = !wall;
    video.classList.remove('on');
    video.pause();
    video.removeAttribute('src');
    video.load();
    if (!wall) {
      still.removeAttribute('src');
      return;
    }
    still.src = 'wp/' + wall.id + '.jpg';
    // Wer Bewegung am Mac reduziert hat, bekommt nur das Standbild.
    if (calm.matches) return;
    video.src = 'wp/' + wall.id + '.mp4';
    run();
  }

  // Das Video läuft nur, solange die Startseite wirklich zu sehen ist – nicht
  // in einem Tab im Hintergrund und nicht in einem verdeckten Fenster.
  function run() {
    if (!video.getAttribute('src')) return;
    if (document.hidden) video.pause();
    else video.play().catch(() => {});
  }

  video.addEventListener('playing', () => video.classList.add('on'));
  document.addEventListener('visibilitychange', run);

  const pickerButton = $('wall-button');
  const picker = $('wall-picker');

  function choose(id) {
    chosen = id;
    showWall(id);
    renderPicker();
    // Der Browser merkt sich die Wahl. Mehr als das kann die Startseite ihm
    // nicht mitteilen – und er nimmt nur eingebaute Hintergründe an.
    try {
      window.webkit.messageHandlers.ipc.postMessage('w:' + id);
    } catch (error) {}
  }

  function option(id, name, thumb) {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'wp' + (id === chosen ? ' on' : '');
    const picture = document.createElement('span');
    picture.className = 'pic';
    if (thumb) {
      const img = document.createElement('img');
      img.src = thumb;
      img.alt = '';
      picture.appendChild(img);
    }
    const label = document.createElement('span');
    label.className = 'name';
    label.textContent = name;
    button.append(picture, label);
    button.addEventListener('click', () => choose(id));
    return button;
  }

  function renderPicker() {
    picker.textContent = '';
    picker.appendChild(option('', S.wp_none, null));
    for (const wall of walls) picker.appendChild(option(wall.id, wall.name, 'wp/' + wall.id + '-thumb.jpg'));
  }

  pickerButton.hidden = false;
  pickerButton.title = S.nt_wallpaper;
  pickerButton.addEventListener('click', (event) => {
    event.stopPropagation();
    if (picker.hidden) renderPicker();
    picker.hidden = !picker.hidden;
  });
  document.addEventListener('click', (event) => {
    // Ein Klick in die Auswahl lässt sie offen – so lassen sich mehrere
    // Hintergründe nacheinander ansehen.
    if (!picker.hidden && !event.composedPath().includes(picker)) picker.hidden = true;
  });
  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') picker.hidden = true;
  });

  showWall(chosen);
})();
