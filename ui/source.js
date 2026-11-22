// Quelltext einer Seite. Der Text ist Material und wird nur als Text
// eingesetzt – Zeile für Zeile, ohne dass etwas davon ausgeführt würde.
(() => {
  'use strict';
  const DATA = JSON.parse(document.getElementById('data').textContent);
  const S = DATA.strings;
  const $ = (id) => document.getElementById(id);
  const text = String(DATA.text || '');
  const MAX_LINES = 20000;

  document.title = S.src_title.replace('{page}', DATA.host || '');
  $('src-url').textContent = DATA.url || '';
  $('src-wrap-label').textContent = S.src_wrap;

  const lines = text.split(/\r\n|\r|\n/);
  const list = $('src');
  const part = document.createDocumentFragment();
  for (let i = 0; i < lines.length && i < MAX_LINES; i++) {
    const line = document.createElement('li');
    line.textContent = lines[i].length > 4000 ? lines[i].slice(0, 4000) + ' …' : lines[i];
    part.appendChild(line);
  }
  list.appendChild(part);

  const note = $('src-note');
  if (!text) {
    note.textContent = S.src_empty;
    note.hidden = false;
  } else if (DATA.truncated || lines.length > MAX_LINES) {
    note.textContent = S.src_truncated;
    note.hidden = false;
  }

  $('src-wrap').addEventListener('change', (event) => {
    document.body.classList.toggle('wrap', event.target.checked);
  });
})();
