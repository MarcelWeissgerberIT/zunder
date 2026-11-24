// Zeiterfassung – eine eingebaute App.
//
// Gebucht wird in einer Zeile: „9-11 #Kunde Besprechung“, „45m #Mails“,
// „1,5h Angebot“, oder nur „#Kunde“ – dann läuft die Uhr. Alles bleibt beim
// Browser auf diesem Mac (src/apps.rs): die Einträge und Projekte als einzelne
// Karteikarten, Einstellungen und die laufende Uhr auf dem Zettel der App.
// Ins Netz geht diese App gar nicht.
(() => {
  'use strict';
  const Z = window.ZunderApp;
  const S = Z.strings;
  const LANG = Z.data.lang === 'de' ? 'de' : 'en';
  const $ = (id) => document.getElementById(id);
  const text = (key, values) =>
    String(S[key] || '').replace(/\{(\w+)\}/g, (whole, name) => (values && name in values ? values[name] : whole));

  // So viele Pausen kennt ein Eintrag höchstens.
  const MAX_BREAKS = 12;
  const COLORS = ['#e8590c', '#1c7ed6', '#2f9e44', '#ae3ec9', '#f08c00', '#0c8599', '#e03131', '#5f3dc4', '#74b816', '#d6336c'];
  const DAY = 24 * 60;

  // ------------------------------------------------------------------------
  // Zeit
  // ------------------------------------------------------------------------

  const two = (value) => String(value).padStart(2, '0');
  const keyOf = (date) => date.getFullYear() + '-' + two(date.getMonth() + 1) + '-' + two(date.getDate());
  const dateOf = (key) => {
    const parts = String(key).split('-').map(Number);
    return new Date(parts[0], (parts[1] || 1) - 1, parts[2] || 1);
  };
  const validKey = (key) => typeof key === 'string' && /^\d{4}-\d{2}-\d{2}$/.test(key) && keyOf(dateOf(key)) === key;
  const today = () => keyOf(new Date());
  const shift = (key, days) => {
    const date = dateOf(key);
    date.setDate(date.getDate() + days);
    return keyOf(date);
  };
  const nowMinutes = () => {
    const now = new Date();
    return now.getHours() * 60 + now.getMinutes();
  };
  // Uhrzeit „09:05“ (Mitternacht am Ende des Tages: „24:00“) und Dauer „1:35“.
  const clock = (minutes) => (minutes >= DAY ? '24:00' : two(Math.floor(minutes / 60)) + ':' + two(minutes % 60));
  const span = (minutes) => (minutes < 0 ? '−' : '') + Math.floor(Math.abs(minutes) / 60) + ':' + two(Math.abs(minutes) % 60);
  // Montag der Woche, in der ein Tag liegt.
  const monday = (key) => shift(key, -((dateOf(key).getDay() + 6) % 7));

  // ------------------------------------------------------------------------
  // Zustand
  // ------------------------------------------------------------------------

  let kept = false;
  let ready = false;
  let shown = today();
  const settings = { targets: [0, 8, 8, 8, 8, 8, 0], step: 5, last: '' }; // Sollstunden: Sonntag … Samstag
  // Die laufende Uhr: Projekt, Notiz, Beginn; dazu die Pausen bisher ([von, bis]
  // in Millisekunden) und seit wann sie gerade pausiert. Führt sie einen
  // Eintrag weiter, stehen dabei seine Kennung und der Augenblick, ab dem es
  // weiterging – was davor liegt, bleibt auf die Minute, wie es gebucht war.
  let running = null; // { p, n, t0, id, since, breaks, pausedAt }
  const newRun = (project, note, t0) => ({ p: project, n: note, t0, id: '', since: 0, breaks: [], pausedAt: 0 });
  const projects = new Map(); // Kennung → { k: 'p', name, color, gone }
  const entries = new Map(); // Kennung → { k: 'e', d, s, e, p, n, b } – b: Pausen [von, bis] in Minuten
  let editing = '';
  // Der Eintrag, bei dem gerade gefragt wird: weiterführen oder neu beginnen?
  let choosing = '';
  let undo = null;
  // Der Tab war weg, während hier ein Eintrag in Bearbeitung war: Der Abgleich
  // mit dem Gesicherten steht noch aus.
  let stale = false;

  const roundTo = (minutes) => Math.round(minutes / settings.step) * settings.step;
  const newId = (prefix) => prefix + '-' + Date.now().toString(36) + Math.random().toString(36).slice(2, 8);
  const cleanName = (name) => Z.clip(String(name).replace(/"/g, '').replace(/\s+/g, ' ').trim(), 40);
  const cleanNote = (note) => Z.clip(String(note == null ? '' : note).replace(/\s+/g, ' ').trim(), 200);

  function cleanProject(value) {
    if (!value || typeof value !== 'object' || typeof value.name !== 'string' || !cleanName(value.name)) return null;
    return {
      k: 'p',
      name: cleanName(value.name),
      color: /^#[0-9a-f]{6}$/i.test(value.color) ? value.color : COLORS[0],
      gone: value.gone === true,
    };
  }

  function cleanEntry(value) {
    if (!value || typeof value !== 'object' || !validKey(value.d)) return null;
    const start = Number(value.s);
    const end = Number(value.e);
    if (!Number.isInteger(start) || !Number.isInteger(end) || start < 0 || end > DAY || end <= start) return null;
    const project = typeof value.p === 'string' && /^[A-Za-z0-9_-]{1,40}$/.test(value.p) ? value.p : '';
    const entry = { k: 'e', d: value.d, s: start, e: end, p: project, n: cleanNote(value.n) };
    // Pausen: ganze Minuten, innerhalb der Spanne, eine nach der anderen.
    const breaks = [];
    let edge = start;
    for (const pair of Array.isArray(value.b) ? value.b.slice(0, MAX_BREAKS) : []) {
      const from = Array.isArray(pair) ? pair[0] : NaN;
      const to = Array.isArray(pair) ? pair[1] : NaN;
      if (Number.isInteger(from) && Number.isInteger(to) && from > start && from >= edge && to > from && to < end) {
        breaks.push([from, to]);
        edge = to;
      }
    }
    if (breaks.length) entry.b = breaks;
    return entry;
  }

  // Gearbeitete Minuten eines Eintrags: seine Spanne ohne die Pausen darin.
  const pauseOf = (entry) => (entry.b || []).reduce((sum, [from, to]) => sum + (to - from), 0);
  const worked = (entry) => entry.e - entry.s - pauseOf(entry);

  // Die gearbeiteten Abschnitte eines Eintrags – zwischen ihnen liegen die Pausen.
  function segments(entry) {
    const list = [];
    let from = entry.s;
    for (const [start, end] of entry.b || []) {
      list.push([from, start]);
      from = end;
    }
    list.push([from, entry.e]);
    return list;
  }

  // Die wirklich gearbeitete Zeit mehrerer Einträge: die Vereinigung ihrer
  // Abschnitte. Überschneiden sich zwei Einträge, zählt die gemeinsame Zeit nur
  // einmal (die Überschneidungs-Warnung an der Zeile bleibt davon unberührt).
  function distinctWorked(list) {
    const segs = [];
    for (const entry of list) for (const seg of segments(entry)) if (seg[1] > seg[0]) segs.push(seg);
    segs.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
    let total = 0;
    let end = -1;
    for (const [from, to] of segs) {
      if (from >= end) {
        total += to - from;
        end = to;
      } else if (to > end) {
        total += to - end;
        end = to;
      }
    }
    return total;
  }

  // Bringt Pausen in Ordnung: in der Spanne, der Reihe nach, verschmolzen, wo sie
  // sich berühren. Eine Pause am Rand ist keine – sie verkürzt den Eintrag.
  function tidy(start, end, breaks) {
    const list = breaks
      .map(([from, to]) => [Math.max(start, from), Math.min(end, to)])
      .filter(([from, to]) => to > from)
      .sort((left, right) => left[0] - right[0]);
    const merged = [];
    for (const [from, to] of list) {
      const last = merged[merged.length - 1];
      if (last && from <= last[1]) last[1] = Math.max(last[1], to);
      else merged.push([from, to]);
    }
    let s = start;
    let e = end;
    while (merged.length && merged[0][0] <= s) s = merged.shift()[1];
    while (merged.length && merged[merged.length - 1][1] >= e) e = merged.pop()[0];
    return { s, e, b: merged.slice(0, MAX_BREAKS) };
  }

  // Ließ sich etwas nicht sichern, erfährt man es – außer im privaten Fenster,
  // wo ohnehin nichts bleibt.
  function stored(state) {
    if (!kept || state === true || state === 'ok') return;
    toast(state === 'full' ? S.tm_full : S.tm_save_failed);
  }

  function saveNote() {
    const run = running
      ? { p: running.p, n: running.n, t0: running.t0, id: running.id, si: running.since, b: running.breaks, pa: running.pausedAt }
      : null;
    Z.save({ v: 1, targets: settings.targets, step: settings.step, last: settings.last, run }).then(stored);
  }

  function putEntry(id, entry) {
    entries.set(id, entry);
    Z.records.put(id, entry).then(stored);
  }

  function dropEntry(id) {
    entries.delete(id);
    Z.records.remove(id).then(stored);
  }

  function putProject(id, project) {
    projects.set(id, project);
    Z.records.put(id, project).then(stored);
  }

  function projectByName(name) {
    const wanted = name.trim().toLowerCase();
    for (const [id, project] of projects) if (project.name.toLowerCase() === wanted) return id;
    return '';
  }

  // Wann ein Projekt zuletzt gebucht wurde – als Text, der sich ordnen lässt.
  function lastUses() {
    const used = new Map();
    for (const entry of entries.values()) {
      const stamp = entry.d + ' ' + String(entry.e).padStart(4, '0');
      if (entry.p && (!used.has(entry.p) || used.get(entry.p) < stamp)) used.set(entry.p, stamp);
    }
    return used;
  }

  // Welches Projekt ist gemeint? Das mit genau diesem Namen; sonst eines, das so
  // beginnt (von mehreren das zuletzt gebuchte); sonst ein neues. In
  // Anführungszeichen – #"Name" – gilt der Name wörtlich.
  function findProject(name, exact) {
    const clean = cleanName(name);
    if (!clean) return { id: '', name: '', fresh: false };
    const known = projectByName(clean);
    if (known) return { id: known, name: projects.get(known).name, fresh: false };
    if (!exact) {
      const lower = clean.toLowerCase();
      const used = lastUses();
      const hits = [...projects.entries()]
        .filter(([, project]) => !project.gone && project.name.toLowerCase().startsWith(lower))
        .sort((left, right) => String(used.get(right[0]) || '').localeCompare(String(used.get(left[0]) || '')) || left[1].name.localeCompare(right[1].name));
      if (hits.length) return { id: hits[0][0], name: hits[0][1].name, fresh: false };
    }
    return { id: '', name: clean, fresh: true };
  }

  // Das gefundene Projekt beim Buchen: ein abgelegtes kommt zurück, ein neues
  // entsteht – mit der nächsten Farbe.
  function projectFor(found) {
    if (!found.name) return '';
    if (found.id && projects.has(found.id)) {
      const project = projects.get(found.id);
      if (project.gone) {
        project.gone = false;
        putProject(found.id, project);
      }
      return found.id;
    }
    const id = newId('p');
    putProject(id, { k: 'p', name: found.name, color: COLORS[projects.size % COLORS.length], gone: false });
    return id;
  }

  const projectName = (id) => (projects.get(id) || {}).name || S.tm_no_project;
  const projectColor = (id) => (projects.get(id) || {}).color || '';

  function entriesOf(day) {
    const list = [];
    for (const [id, entry] of entries) if (entry.d === day) list.push([id, entry]);
    return list.sort((left, right) => left[1].s - right[1].s || left[1].e - right[1].e);
  }

  const targetOf = (day) => Math.round((Number(settings.targets[dateOf(day).getDay()]) || 0) * 60);

  // ------------------------------------------------------------------------
  // Schnellbuchung: eine Zeile lesen
  // ------------------------------------------------------------------------
  //
  //   [Tag] [Zeit] [#Projekt] Notiz
  //
  //   Tag      heute, gestern, vorgestern, mo … so, 23.11., 2026-11-23 – nur
  //            ganz am Anfang (sonst wäre „so“ in einer Notiz ein Sonntag)
  //   Zeit     9-11 · 9:15-11:30 · 9 bis 11        von … bis
  //            1,5h · 90m · 1h30 · 1:30             Dauer – wohin, sagt `placeFor`
  //            ab 9 · seit 9:30                     die Uhr läuft seit dann
  //            (nichts)                              die Uhr läuft ab jetzt
  //   Projekt  #Name (der Anfang genügt), #"Name mit Leerzeichen", #- für
  //            keines; ohne # gilt ein bekannter Projektname am Anfang der
  //            Notiz, sonst das zuletzt benutzte Projekt

  function toMinutes(hours, minutes) {
    const h = Number(hours);
    const m = Number(minutes || 0);
    return h >= 0 && h <= 24 && m >= 0 && m < 60 && h * 60 + m <= DAY ? h * 60 + m : -1;
  }

  function readQuick(input) {
    let rest = ' ' + input.replace(/\s+/g, ' ').trim() + ' ';
    // `day`: der genannte Tag – sonst der gezeigte (`given` sagt, welcher von beiden).
    const out = { day: shown, given: false, start: -1, end: -1, minutes: 0, from: -1, name: '', exact: false, none: false, note: '', error: '' };
    // Was gelesen ist, kommt aus der Zeile heraus; ein Leerzeichen bleibt stehen.
    const take = (pattern, use) => {
      const match = pattern.exec(rest);
      if (!match || use(match) === false) return false;
      rest = rest.slice(0, match.index) + rest.slice(match.index + match[0].length - 1);
      return true;
    };

    // Der Tag – nur als erstes Wort.
    const before = rest;
    const now = new Date();
    const weekdays = { so: 0, sun: 0, mo: 1, mon: 1, di: 2, tue: 2, mi: 3, wed: 3, do: 4, thu: 4, fr: 5, fri: 5, sa: 6, sat: 6 };
    take(/^ (heute|today|gestern|yesterday|vorgestern) /i, (match) => {
      const back = { heute: 0, today: 0, gestern: 1, yesterday: 1, vorgestern: 2 }[match[1].toLowerCase()];
      out.day = shift(today(), -back);
    }) ||
      take(/^ (\d{1,2})\.(\d{1,2})\.(\d{4}|\d{2})? /, (match) => {
        const tail = '-' + two(match[2]) + '-' + two(match[1]);
        // Ohne Jahr gilt das Datum, das heute am nächsten liegt: „30.12.“ meint
        // Anfang Januar das alte Jahr, nicht das kommende Jahresende.
        const years = match[3]
          ? [match[3].length === 2 ? 2000 + Number(match[3]) : Number(match[3])]
          : [now.getFullYear() - 1, now.getFullYear(), now.getFullYear() + 1];
        const distance = (key) => Math.abs(dateOf(key) - dateOf(today()));
        const key = years
          .map((year) => year + tail)
          .filter(validKey)
          .sort((left, right) => distance(left) - distance(right))[0];
        if (!key) return false;
        out.day = key;
      }) ||
      take(/^ (\d{4}-\d{2}-\d{2}) /, (match) => {
        if (!validKey(match[1])) return false;
        out.day = match[1];
      }) ||
      take(/^ (mo|di|mi|do|fr|sa|so|mon|tue|wed|thu|fri|sat|sun)\.? /i, (match) => {
        // Der letzte solche Wochentag, heute eingeschlossen.
        const wanted = weekdays[match[1].toLowerCase()];
        out.day = shift(today(), -((now.getDay() - wanted + 7) % 7));
      });

    out.given = rest !== before;

    // Das Projekt.
    take(/ [#@]- /, () => {
      out.none = true;
    }) ||
      take(/ [#@]"([^"]{1,40})" /, (match) => {
        out.name = match[1];
        out.exact = true;
      }) ||
      take(/ [#@]([^\s#@"]{1,40}) /, (match) => {
        out.name = match[1];
      });

    // Die Zeit steht vorn oder hinten – nie mitten in der Notiz: „Fehler seit 2
    // Wochen“ ist keine Uhrzeit. Vorn hat Vorrang. Je Stelle zuerst „von – bis“,
    // dann „ab“, dann eine Dauer.
    const time = '(\\d{1,2})(?:[:.](\\d{2}))?(?: ?(?:uhr|h))?';
    const hours = '(?:h|std\\.?|stunden?|hours?)';
    const mins = '(?:m|min\\.?|mins|minuten?|minutes?)';
    const lasts = (value) => {
      out.minutes = value;
    };
    const both = (match) => (Number(match[2]) > 59 ? false : lasts(Number(match[1]) * 60 + Number(match[2])));
    const kinds = [
      [
        ' ' + time + ' ?(?:-|–|bis|to) ?' + time + ' ',
        (match) => {
          const start = toMinutes(match[1], match[2]);
          const end = toMinutes(match[3], match[4]);
          if (start < 0 || end < 0) return false;
          if (end <= start) {
            out.error = S.tm_err_range;
            return true;
          }
          out.start = start;
          out.end = end;
        },
      ],
      [
        ' (?:ab|seit|from|since) ' + time + ' ',
        (match) => {
          const start = toMinutes(match[1], match[2]);
          if (start < 0) return false;
          out.from = start;
        },
      ],
      // „1h30“ ohne Lücke; mit Lücke nur samt Einheit („1h 30m“) – in „2h 15
      // Leute“ sind es zwei Stunden.
      [' (\\d{1,2}) ?' + hours + '(\\d{1,2}) ?' + mins + '? ', both],
      [' (\\d{1,2}) ?' + hours + ' (\\d{1,2}) ?' + mins + ' ', both],
      [' (\\d{1,2}(?:[.,]\\d{1,2})?) ?' + hours + ' ', (match) => lasts(Math.round(Number(match[1].replace(',', '.')) * 60))],
      [' (\\d{1,3}) ?' + mins + ' ', (match) => lasts(Number(match[1]))],
      [' (\\d{1,2}):(\\d{2}) ?' + hours + '? ', (match) => (toMinutes(match[1], match[2]) > 0 ? lasts(toMinutes(match[1], match[2])) : false)],
    ];
    if (!kinds.some(([source, use]) => take(new RegExp('^' + source, 'i'), use))) {
      kinds.some(([source, use]) => take(new RegExp(source + '$', 'i'), use));
    }

    out.note = cleanNote(rest);
    // Ohne #: Steht am Anfang der Name eines bekannten Projekts, ist es gemeint.
    if (!out.name && !out.none && out.note) {
      const lower = out.note.toLowerCase();
      let best = '';
      for (const project of projects.values()) {
        const name = project.name.toLowerCase();
        if (!project.gone && name.length > best.length && (lower === name || lower.startsWith(name + ' '))) best = project.name;
      }
      if (best) {
        out.name = best;
        out.exact = true;
        out.note = out.note.slice(best.length).trim();
      }
    }
    if (out.minutes > DAY) out.error = S.tm_err_long;
    return out;
  }

  // Der späteste freie Platz dieser Länge, der spätestens um `until` endet –
  // sein Anfang, oder -1, wenn es keinen gibt.
  function freeBefore(list, minutes, until) {
    let end = until;
    for (let round = 0; round <= list.length; round++) {
      let moved = false;
      for (const entry of list) {
        if (entry.s < end && entry.e > end - minutes) {
          end = entry.s;
          moved = true;
        }
      }
      if (!moved) break;
    }
    return end - minutes >= 0 ? end - minutes : -1;
  }

  // Wohin mit einer Dauer ohne Uhrzeit? Heute endet sie jetzt. Ist dort schon
  // gebucht, schließt sie an den letzten Eintrag an, solange sie damit kaum über
  // jetzt hinausreicht – sonst kommt sie in die späteste freie Lücke davor (so
  // lässt sich abends ein ganzer Tag nacheinander buchen). An anderen Tagen
  // schließt sie an den letzten Eintrag an; ein leerer Tag beginnt um neun.
  function placeFor(day, minutes) {
    const list = entriesOf(day).map(([, entry]) => entry);
    const hits = (start, end) => list.some((entry) => entry.s < end && entry.e > start);
    const now = Math.min(DAY, roundTo(nowMinutes()));
    if (day === today() && now - minutes >= 0) {
      if (!hits(now - minutes, now)) return [now - minutes, now];
      const before = list.filter((entry) => entry.s < now).map((entry) => entry.e);
      const lastEnd = before.length ? Math.max(...before) : -1;
      const slack = Math.max(15, settings.step);
      if (lastEnd >= 0 && lastEnd + minutes <= Math.min(DAY, now + slack) && !hits(lastEnd, lastEnd + minutes)) {
        return [lastEnd, lastEnd + minutes];
      }
      const free = freeBefore(list, minutes, now);
      // Bis jetzt ist alles voll: dann eben mit Überschneidung – die Liste zeigt sie an.
      return free >= 0 ? [free, free + minutes] : [now - minutes, now];
    }
    if (!list.length) {
      const start = Math.min(9 * 60, DAY - minutes);
      return [start, start + minutes];
    }
    const lastEnd = Math.max(...list.map((entry) => entry.e));
    if (lastEnd + minutes <= DAY) return [lastEnd, lastEnd + minutes];
    const free = freeBefore(list, minutes, DAY);
    return free >= 0 ? [free, free + minutes] : [DAY - minutes, DAY];
  }

  // Was aus der Zeile wird: ein Eintrag, eine laufende Uhr – oder nichts.
  function planQuick(input) {
    const read = readQuick(input);
    let project = { id: '', name: '', fresh: false };
    if (read.name) {
      project = findProject(read.name, read.exact);
    } else if (!read.none && projects.has(settings.last) && !projects.get(settings.last).gone) {
      project = { id: settings.last, name: projects.get(settings.last).name, fresh: false };
    }
    const plan = { kind: 'none', day: read.day, start: 0, end: 0, project, note: read.note, error: read.error };
    if (read.error) return plan;
    if (read.start >= 0) {
      plan.kind = 'entry';
      plan.start = read.start;
      plan.end = read.end;
    } else if (read.minutes > 0) {
      plan.kind = 'entry';
      [plan.start, plan.end] = placeFor(read.day, read.minutes);
    } else if (read.from >= 0) {
      // Die Uhr läuft immer heute – gleich, welcher Tag gerade gezeigt wird. Nur
      // ein ausdrücklich genannter anderer Tag passt nicht dazu.
      if ((read.given && read.day !== today()) || read.from > nowMinutes()) plan.error = S.tm_err_from;
      else {
        plan.kind = 'timer';
        plan.day = today();
        plan.start = read.from;
      }
    } else if (read.name || read.note) {
      if (read.given && read.day !== today()) plan.error = S.tm_err_need_time;
      else {
        plan.kind = 'timer';
        plan.day = today();
        plan.start = -1;
      }
    }
    return plan;
  }

  function previewQuick() {
    const box = $('quick-preview');
    const value = $('quick-input').value;
    box.textContent = '';
    box.className = '';
    const plan = ready ? planQuick(value) : { kind: 'none', error: '' };
    $('quick-book').disabled = plan.kind === 'none';
    $('quick-book').textContent = plan.kind === 'timer' ? S.tm_start : S.tm_book;
    if (!value.trim()) {
      box.textContent = S.tm_hint;
      return;
    }
    if (plan.error || plan.kind === 'none') {
      box.className = 'bad';
      box.textContent = plan.error || S.tm_err_unclear;
      return;
    }
    const strong = (content) => {
      const node = document.createElement('b');
      node.textContent = content;
      return node;
    };
    const project = plan.project.name ? plan.project.name + (plan.project.fresh ? ' (' + S.tm_new_project + ')' : '') : S.tm_no_project;
    if (plan.kind === 'entry') {
      const day = plan.day === today() ? S.tm_today : dateLabel(plan.day, true);
      box.append('→ ', strong(span(plan.end - plan.start)), '  ·  ' + day + ' ' + clock(plan.start) + '–' + clock(plan.end) + '  ·  ', strong(project));
    } else {
      const since = plan.start >= 0 ? text('tm_since', { time: clock(plan.start) }) : S.tm_from_now;
      box.append('▶ ', strong(S.tm_timer_runs), '  ·  ' + since + '  ·  ', strong(project));
    }
    if (plan.note) box.append('  ·  ' + plan.note);
  }

  function bookNow() {
    const input = $('quick-input');
    const plan = planQuick(input.value);
    if (plan.kind === 'none') return;
    const project = projectFor(plan.project);
    if (project) settings.last = project;
    if (plan.kind === 'entry') {
      const id = newId('e' + plan.day.replace(/-/g, ''));
      putEntry(id, { k: 'e', d: plan.day, s: plan.start, e: plan.end, p: project, n: plan.note });
      shown = plan.day;
      toast(text('tm_booked', { time: span(plan.end - plan.start), project: projectName(project) }), () => {
        if (entries.has(id)) dropEntry(id);
      });
    } else {
      // Läuft schon eine Uhr, wird sie erst gebucht.
      if (running) stopTimer();
      const start = new Date();
      if (plan.start >= 0) start.setHours(Math.floor(plan.start / 60), plan.start % 60, 0, 0);
      running = newRun(project, plan.note, start.getTime());
      shown = today();
      // Eine neue Uhr läuft: Was sich eben noch zurücknehmen ließ, gilt nicht ihr.
      hideToast();
    }
    saveNote();
    input.value = '';
    editing = '';
    choosing = '';
    render();
    input.focus();
  }

  const bookQuick = () => act(bookNow);

  // ------------------------------------------------------------------------
  // Die laufende Uhr
  // ------------------------------------------------------------------------

  // Minute des Tages, auf die ein Zeitpunkt fällt – 0 davor, 24:00 danach.
  function minuteIn(day, ms) {
    const date = new Date(ms);
    const key = keyOf(date);
    if (key !== day) return key < day ? 0 : DAY;
    return date.getHours() * 60 + date.getMinutes();
  }

  // Bucht die laufende Uhr: vom Beginn bis jetzt (pausiert sie, bis zum Beginn
  // der Pause), auf das Raster gerundet, mit ihren Pausen. Läuft sie über
  // Mitternacht, bekommt jeder Tag seinen Teil. Führt sie einen Eintrag weiter,
  // wird der fortgeschrieben statt eines neuen angelegt. Zurück kommt, was
  // geschrieben wurde: [Kennung, was vorher dort stand] – für „Rückgängig“.
  function stopTimer() {
    if (!running) return [];
    const run = running;
    const until = run.pausedAt || Date.now();
    const made = [];
    const lastDay = keyOf(new Date(until));
    let day = keyOf(new Date(run.t0));
    for (let guard = 0; guard < 8; guard++) {
      const last = day === lastDay;
      // Was vor dem Weiterführen schon gebucht war, bleibt auf die Minute stehen.
      const at = (ms) => (run.id && ms < run.since ? minuteIn(day, ms) : roundTo(minuteIn(day, ms)));
      let begin = at(run.t0);
      let end = at(until);
      const breaks = run.breaks.map(([from, to]) => [at(from), at(to)]);
      // Auch eine eben erst gestartete Uhr ergibt einen Eintrag: einen Schritt
      // lang, in dem Schritt, in dem sie begann – nicht in der Zukunft.
      if (last && end <= begin && !run.id && !breaks.length) {
        begin = Math.floor(minuteIn(day, run.t0) / settings.step) * settings.step;
        end = begin + settings.step;
      }
      if (!run.id) begin = Math.min(begin, DAY - settings.step);
      end = Math.min(DAY, end);
      // Der Eintrag, der weitergeführt wird: Kürzer als er war, wird er dabei nie.
      const was = run.id && entries.has(run.id) && entries.get(run.id).d === day ? entries.get(run.id) : null;
      if (was) end = Math.max(end, was.e);
      const shape = tidy(begin, end, breaks);
      if (shape.e > shape.s) {
        const id = was ? run.id : newId('e' + day.replace(/-/g, ''));
        const entry = { k: 'e', d: day, s: shape.s, e: shape.e, p: run.p, n: run.n };
        if (shape.b.length) entry.b = shape.b;
        made.push([id, was]);
        putEntry(id, entry);
      }
      if (last) break;
      day = shift(day, 1);
    }
    running = null;
    return made;
  }

  // Kann die Uhr diesen Eintrag weiterführen? Nur einen von heute, der schon zu
  // Ende ist, noch Platz für eine Pause hat und nicht ohnehin gerade läuft.
  function canContinue(id, entry) {
    return entry.d === today() && entry.e <= nowMinutes() && (entry.b || []).length < MAX_BREAKS && !(running && running.id === id);
  }

  function started() {
    if (running.p) settings.last = running.p;
    choosing = '';
    hideToast();
    saveNote();
    shown = today();
    render();
  }

  // Die Uhr läuft für dieselbe Arbeit noch einmal – als neuer Eintrag.
  function startAgain(entry) {
    if (running) stopTimer();
    running = newRun(projects.has(entry.p) ? entry.p : '', entry.n, Date.now());
    started();
  }

  // Die Uhr führt diesen Eintrag weiter: Er bleibt ein Eintrag, und die Zeit
  // seit seinem Ende steht als Pause darin.
  function continueEntry(id) {
    const entry = entries.get(id);
    if (!entry || !canContinue(id, entry)) return;
    if (running) stopTimer();
    const at = (minutes) => {
      const date = dateOf(entry.d);
      date.setHours(Math.floor(minutes / 60), minutes % 60, 0, 0);
      return date.getTime();
    };
    const now = Date.now();
    const breaks = (entry.b || []).map(([from, to]) => [at(from), at(to)]);
    if (now - at(entry.e) >= 60000) breaks.push([at(entry.e), now]);
    running = { p: projects.has(entry.p) ? entry.p : '', n: entry.n, t0: at(entry.s), id, since: now, breaks, pausedAt: 0 };
    started();
  }

  function tick() {
    $('running').hidden = !running;
    let title = S.app_time;
    if (running) {
      const until = running.pausedAt || Date.now();
      const pause = running.breaks.reduce((sum, [from, to]) => sum + (to - from), 0);
      const seconds = Math.max(0, Math.floor((until - running.t0 - pause) / 1000));
      const stamp = Math.floor(seconds / 3600) + ':' + two(Math.floor(seconds / 60) % 60);
      const hour = (ms) => {
        const date = new Date(ms);
        return two(date.getHours()) + ':' + two(date.getMinutes());
      };
      $('running').classList.toggle('paused', !!running.pausedAt);
      $('running-clock').textContent = stamp + ':' + two(seconds % 60);
      $('running-project').textContent = projectName(running.p);
      $('running-project').style.setProperty('--dot', projectColor(running.p) || '');
      $('running-note').textContent = running.n;
      let since = running.pausedAt ? text('tm_paused_since', { time: hour(running.pausedAt) }) : text('tm_since', { time: hour(running.t0) });
      if (!running.pausedAt && pause >= 60000) since += ' · ' + text('tm_breaks', { time: span(Math.round(pause / 60000)) });
      $('running-since').textContent = since;
      $('running-pause').textContent = running.pausedAt ? S.tm_resume : S.tm_pause;
      // Mehr Pausen, als ein Eintrag kennt, gibt es nicht.
      $('running-pause').hidden = !running.pausedAt && running.breaks.length >= MAX_BREAKS;
      title = (running.pausedAt ? '⏸ ' : '▶ ') + stamp + ' · ' + projectName(running.p);
    }
    // Der Titel steht am Tab – er wechselt nur mit der Minute, nicht im Sekundentakt.
    if (document.title !== title) document.title = title;
  }

  // Pause: Die Uhr hält an. Weiter: Sie läuft im selben Eintrag weiter, und die
  // Pause steht darin.
  $('running-pause').addEventListener('click', () =>
    act(() => {
      if (!running) return;
      const now = Date.now();
      if (running.pausedAt) {
        if (now > running.pausedAt) running.breaks.push([running.pausedAt, now]);
        running.pausedAt = 0;
      } else if (running.breaks.length < MAX_BREAKS) {
        running.pausedAt = now;
      }
      saveNote();
      render();
    })
  );

  $('running-stop').addEventListener('click', () =>
    act(() => {
      const before = running;
      const made = stopTimer();
      saveNote();
      shown = today();
      render();
      if (!made.length) return;
      const total = made.reduce((sum, [id]) => sum + worked(entries.get(id)), 0);
      // Zurücknehmen heißt: Was neu entstand, verschwindet, ein weitergeführter
      // Eintrag ist wieder, wie er war – und die Uhr läuft weiter, als wäre
      // nichts gewesen (außer es läuft inzwischen eine andere).
      toast(text('tm_booked', { time: span(total), project: projectName(entries.get(made[0][0]).p) }), () => {
        for (const [id, was] of made) {
          if (was) putEntry(id, was);
          else if (entries.has(id)) dropEntry(id);
        }
        if (!running) {
          running = before;
          saveNote();
        }
      });
    })
  );

  $('running-drop').addEventListener('click', () =>
    act(() => {
      const before = running;
      if (!before) return;
      running = null;
      saveNote();
      render();
      toast(S.tm_timer_dropped, () => {
        // Läuft inzwischen eine andere Uhr, bleibt sie.
        if (running) return;
        running = before;
        saveNote();
      });
    })
  );

  // ------------------------------------------------------------------------
  // Anzeige
  // ------------------------------------------------------------------------

  function icon(name) {
    const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    const use = document.createElementNS('http://www.w3.org/2000/svg', 'use');
    use.setAttribute('href', '#i-' + name);
    svg.appendChild(use);
    return svg;
  }

  function node(tag, className, content) {
    const element = document.createElement(tag);
    if (className) element.className = className;
    if (content !== undefined) element.textContent = content;
    return element;
  }

  function dateLabel(key, short) {
    const options = short ? { weekday: 'short', day: 'numeric', month: 'numeric' } : { weekday: 'long', day: 'numeric', month: 'long' };
    // Ein anderes Jahr steht dabei – sonst sähe der 30.12. des Vorjahrs aus wie der kommende.
    if (dateOf(key).getFullYear() !== new Date().getFullYear()) options.year = 'numeric';
    return dateOf(key).toLocaleDateString(LANG, options);
  }

  function tag(project) {
    const element = node('span', 'tag', projectName(project));
    const color = projectColor(project);
    if (color) element.style.setProperty('--dot', color);
    return element;
  }

  function render() {
    if (!ready) return;
    const isToday = shown === today();
    $('date-label').textContent = isToday ? S.tm_today + ' · ' + dateLabel(shown) : dateLabel(shown);
    $('today').hidden = isToday;
    renderChips();
    renderDay();
    renderWeek();
    renderCalendar();
    previewQuick();
    tick();
    // Stand der Abgleich mit dem Gesicherten noch aus (siehe `sync`), kommt er jetzt.
    if (stale && !editing && !pending) sync();
  }

  function renderChips() {
    const box = $('quick-chips');
    box.textContent = '';
    // Zuletzt gebuchte Projekte zuerst.
    const used = lastUses();
    const list = [...projects.entries()]
      .filter(([, project]) => !project.gone)
      .sort((left, right) => String(used.get(right[0]) || '').localeCompare(String(used.get(left[0]) || '')) || left[1].name.localeCompare(right[1].name))
      .slice(0, 8);
    for (const [id, project] of list) {
      const chip = node('button', 'chip' + (id === settings.last ? ' on' : ''), project.name);
      chip.type = 'button';
      chip.style.setProperty('--dot', project.color);
      chip.title = S.tm_chip_tip;
      chip.addEventListener('click', () => {
        act(() => {
          if (!projects.has(id)) return;
          settings.last = id;
          saveNote();
          renderChips();
          previewQuick();
        });
        $('quick-input').focus();
      });
      box.appendChild(chip);
    }
    // Ein Klick bucht so viel – auf das Projekt und mit der Notiz aus der Zeile,
    // sonst auf das vorgemerkte Projekt.
    for (const minutes of [15, 30, 60]) {
      const chip = node('button', 'chip plain', '+' + (minutes < 60 ? minutes + ' min' : '1 h'));
      chip.type = 'button';
      chip.addEventListener('click', () => {
        const read = readQuick($('quick-input').value);
        // Das Projekt so, wie die Zeile es verstanden hat – ausgeschrieben.
        const project = read.none ? '#- ' : read.name ? '#"' + findProject(read.name, read.exact).name + '" ' : '';
        $('quick-input').value = (read.day !== shown ? read.day + ' ' : '') + minutes + 'm ' + project + read.note;
        bookQuick();
      });
      box.appendChild(chip);
    }
  }

  function renderDay() {
    const list = entriesOf(shown);
    // Ein Eintrag, der nicht mehr da ist, ist auch nicht mehr in Bearbeitung.
    if (editing && !list.some(([id]) => id === editing)) editing = '';
    if (choosing && !list.some(([id]) => id === choosing)) choosing = '';
    const total = distinctWorked(list.map(([, entry]) => entry));
    const target = targetOf(shown);
    $('day-title').textContent = shown === today() ? S.tm_today : dateLabel(shown);
    const sum = $('day-sum');
    sum.textContent = span(total);
    if (target) sum.appendChild(node('small', '', ' ' + text('tm_of', { time: span(target) })));
    const meter = $('day-meter');
    meter.hidden = !target;
    meter.classList.toggle('done', target > 0 && total >= target);
    meter.firstElementChild.style.width = target ? Math.min(100, Math.round((total / target) * 100)) + '%' : '0';

    const box = $('entries');
    box.textContent = '';
    let previousEnd = -1;
    // Was bis hierher schon gearbeitet ist – die Pause eines Eintrags ist frei
    // für einen anderen.
    const busy = [];
    for (const [id, entry] of list) {
      // Lücken zwischen zwei Einträgen: Ein Klick trägt sie in die Zeile ein.
      if (previousEnd >= 0 && entry.s - previousEnd >= 5) {
        const gap = node('button', 'gap', '· ' + text('tm_gap', { time: span(entry.s - previousEnd) }) + ' ·');
        const from = previousEnd;
        gap.type = 'button';
        gap.addEventListener('click', () => {
          $('quick-input').value = clock(from) + '-' + clock(entry.s) + ' ';
          previewQuick();
          $('quick-input').focus();
        });
        box.appendChild(gap);
      }
      if (id === editing) {
        box.appendChild(editorFor(id, entry));
      } else {
        const mine = segments(entry);
        const clash = mine.some(([from, to]) => busy.some(([start, end]) => from < end && to > start));
        // Führt die Uhr diesen Eintrag gerade weiter, bleibt er, wie er ist, bis sie stoppt.
        const live = !!running && running.id === id;
        const row = node('div', 'entry' + (clash ? ' clash' : '') + (live ? ' live' : ''));
        const pause = pauseOf(entry);
        const about = node('span', 'text');
        if (live) about.append(node('small', 'mark', S.tm_live));
        if (pause) {
          const mark = node('small', 'mark', text('tm_breaks', { time: span(pause) }));
          mark.title = entry.b.map(([from, to]) => clock(from) + '–' + clock(to)).join(', ');
          about.append(mark);
        }
        about.append(entry.n);
        row.append(node('span', 'span', clock(entry.s) + '–' + clock(entry.e)), node('span', 'length', span(worked(entry))), tag(entry.p), about);
        if (clash) row.title = S.tm_clash;
        const tools = node('span', 'tools');
        const again = node('button', 'tool');
        again.type = 'button';
        again.title = S.tm_again;
        again.setAttribute('aria-label', S.tm_again);
        again.append(icon('play'));
        again.addEventListener('click', (event) => {
          event.stopPropagation();
          if (!ready) return;
          // Weiterführen oder neu beginnen? Wo nur „neu“ geht, braucht es keine Frage.
          if (!canContinue(id, entry)) {
            act(() => {
              const now = entries.get(id);
              if (now) startAgain(now);
            });
            return;
          }
          choosing = choosing === id ? '' : id;
          renderDay();
        });
        const remove = node('button', 'tool');
        remove.type = 'button';
        remove.title = S.tm_delete;
        remove.setAttribute('aria-label', S.tm_delete);
        remove.append(icon('trash'));
        remove.addEventListener('click', (event) => {
          event.stopPropagation();
          act(() => {
            const gone = entries.get(id);
            if (!gone) return;
            dropEntry(id);
            render();
            toast(S.tm_deleted, () => putEntry(id, gone));
          });
        });
        if (!live) {
          tools.append(again, remove);
          row.append(tools);
        }
        row.addEventListener('click', () => {
          if (!ready) return;
          if (live) {
            toast(S.tm_live_locked);
            return;
          }
          editing = id;
          choosing = '';
          renderDay();
        });
        // Ein Zeitstrahl des Eintrags: die gearbeiteten Abschnitte gefüllt, die
        // Pausen dazwischen als Lücken. Nur bei Einträgen mit Pausen.
        if (entry.b && entry.b.length && entry.e > entry.s) {
          const spanMin = entry.e - entry.s;
          const track = node('div', 'track');
          const place = (element, from, to) => {
            element.style.left = ((from - entry.s) / spanMin) * 100 + '%';
            element.style.width = (Math.max(0, to - from) / spanMin) * 100 + '%';
          };
          for (const [from, to] of segments(entry)) {
            const fill = node('span', 'fill');
            place(fill, from, to);
            track.appendChild(fill);
          }
          // Die Pausen als graue Abschnitte; beim Darüberfahren die Dauer.
          for (const [from, to] of entry.b) {
            const pause = node('span', 'pause');
            place(pause, from, to);
            pause.title = clock(from) + '–' + clock(to) + ' · ' + text('tm_breaks', { time: span(to - from) });
            track.appendChild(pause);
          }
          row.appendChild(track);
        }
        box.appendChild(row);
        if (choosing === id && !live) box.appendChild(choiceFor(id, entry));
      }
      busy.push(...segments(entry));
      previousEnd = Math.max(previousEnd, entry.e);
    }
    $('empty').hidden = list.length > 0;
    $('empty').textContent = S.tm_empty;
  }

  // Play an einem Eintrag von heute: diesen weiterführen – oder einen neuen
  // beginnen?
  function choiceFor(id, entry) {
    const strip = node('div', 'choice');
    const go = node('button', 'primary', S.tm_continue);
    go.type = 'button';
    go.addEventListener('click', () => act(() => continueEntry(id)));
    const fresh = node('button', 'ghost', S.tm_new_entry);
    fresh.type = 'button';
    fresh.addEventListener('click', () =>
      act(() => {
        const now = entries.get(id);
        if (now) startAgain(now);
      })
    );
    const hint = nowMinutes() > entry.e ? text('tm_continue_gap', { time: clock(entry.e) }) : S.tm_continue_now;
    strip.append(go, fresh, node('small', '', hint));
    return strip;
  }

  function editorFor(id, entry) {
    const form = node('form', 'editor');
    // Die Prüfung der Zeitfelder macht der Code selbst: eine halb getippte
    // Pause blockiert so nicht mit „Ungültiger Wert“, sie fällt beim Sichern
    // einfach weg.
    form.noValidate = true;
    const field = (type, value, label) => {
      const input = document.createElement('input');
      input.type = type;
      input.value = value;
      input.setAttribute('aria-label', label);
      input.title = label;
      return input;
    };
    // Zeitfeld mit Maske: Nach den zwei Stunden-Ziffern rutscht der Doppelpunkt
    // dazwischen, „1934“ wird zu „19:34“. `minutes` darf null sein (leer).
    const timeField = (minutes, label) => {
      const input = document.createElement('input');
      input.type = 'text';
      input.className = 'tf';
      input.inputMode = 'numeric';
      input.autocomplete = 'off';
      input.setAttribute('aria-label', label);
      input.title = label;
      input.placeholder = '--:--';
      input.maxLength = 5;
      input.value = minutes == null ? '' : clock(minutes);
      input.addEventListener('input', () => {
        const digits = input.value.replace(/\D/g, '').slice(0, 4);
        input.value = digits.length > 2 ? digits.slice(0, 2) + ':' + digits.slice(2) : digits;
      });
      // Beim Verlassen zu vollem HH:MM auffüllen und auf 23:59 begrenzen.
      input.addEventListener('blur', () => {
        const digits = input.value.replace(/\D/g, '');
        if (!digits) {
          input.value = '';
          return;
        }
        const hours = Math.min(23, parseInt(digits.slice(0, 2), 10) || 0);
        const mins = Math.min(59, parseInt(digits.slice(2, 4) || '0', 10) || 0);
        input.value = two(hours) + ':' + two(mins);
      });
      return input;
    };
    // Ein Zeitfeld kennt kein „24:00“ – das Ende des Tages steht dort als 23:59.
    const from = timeField(entry.s, S.tm_from);
    const until = timeField(entry.e >= DAY ? DAY - 1 : entry.e, S.tm_until);
    // Ein Projekt, das es nicht mehr gibt, steht nicht im Feld – sonst entstünde
    // beim Sichern eines mit dem Namen „Ohne Projekt“.
    const current = projects.has(entry.p) ? projects.get(entry.p).name : '';
    const project = field('text', current, S.tm_project);
    project.placeholder = S.tm_project;
    project.maxLength = 40;
    const names = document.createElement('datalist');
    names.id = 'names-' + id;
    for (const known of projects.values()) if (!known.gone) names.appendChild(new Option(known.name));
    project.setAttribute('list', names.id);
    const note = field('text', entry.n, S.tm_note);
    note.placeholder = S.tm_note;
    note.maxLength = 200;
    const actions = node('div', 'actions');
    const cancel = node('button', 'ghost', S.tm_cancel);
    cancel.type = 'button';
    cancel.addEventListener('click', () => {
      editing = '';
      render();
    });
    const save = node('button', 'primary', S.tm_save);
    save.type = 'submit';
    actions.append(cancel, save);
    form.append(from, until, project, note, names);
    // Die Pausen des Eintrags: jede mit Von/Bis bearbeitbar, und eigene lassen
    // sich einfügen. Beim Sichern bringt `tidy` sie in Ordnung (in der Spanne,
    // der Reihe nach, verschmolzen) – leere oder unsinnige Zeilen fallen weg.
    const pauses = node('div', 'pauses');
    const pausesHead = node('div', 'pauses-head');
    pausesHead.appendChild(node('span', 'pauses-title', S.tm_breaks_head));
    const addBreak = node('button', 'ghost', S.tm_add_break);
    addBreak.type = 'button';
    pausesHead.appendChild(addBreak);
    const brkList = node('div', 'brk-list');
    pauses.append(pausesHead, brkList);
    form.append(pauses);

    const breakRows = [];
    const syncAdd = () => {
      addBreak.disabled = breakRows.length >= MAX_BREAKS;
    };
    function addBreakRow(start, end) {
      if (breakRows.length >= MAX_BREAKS) return null;
      const row = node('div', 'brk-row');
      const breakFrom = timeField(start == null ? null : start, S.tm_break_from);
      const breakUntil = timeField(end == null ? null : end, S.tm_break_until);
      const remove = node('button', 'brk-rm');
      remove.type = 'button';
      remove.title = S.tm_remove_break;
      remove.setAttribute('aria-label', S.tm_remove_break);
      remove.appendChild(icon('close'));
      const record = { from: breakFrom, until: breakUntil };
      remove.addEventListener('click', () => {
        const at = breakRows.indexOf(record);
        if (at >= 0) breakRows.splice(at, 1);
        row.remove();
        syncAdd();
      });
      row.append(breakFrom, node('span', 'brk-sep', '–'), breakUntil, remove);
      brkList.appendChild(row);
      breakRows.push(record);
      syncAdd();
      return breakFrom;
    }
    for (const [start, end] of entry.b || []) addBreakRow(start, end);
    addBreak.addEventListener('click', () => {
      const firstField = addBreakRow();
      if (firstField) firstField.focus();
    });
    syncAdd();
    form.append(actions);
    form.addEventListener('submit', (event) => {
      event.preventDefault();
      const read = (value) => {
        const digits = String(value).replace(/\D/g, '');
        if (!digits) return -1;
        const hours = parseInt(digits.slice(0, 2), 10);
        const mins = digits.length > 2 ? parseInt(digits.slice(2, 4), 10) : 0;
        if (hours > 23 || mins > 59) return -1;
        return toMinutes(hours, mins);
      };
      const start = read(from.value);
      // Das Ende des Tages bleibt das Ende des Tages, solange das Feld unberührt ist.
      const end = entry.e >= DAY && until.value === '23:59' ? DAY : read(until.value);
      if (start < 0 || end <= start) {
        until.focus();
        return;
      }
      const typed = cleanName(project.value);
      const noted = cleanNote(note.value);
      act(() => {
        editing = '';
        // Anderswo gelöscht, während hier bearbeitet wurde: Dann bleibt er gelöscht.
        const now = entries.get(id);
        if (now) {
          // Dasselbe Projekt wie vorher bleibt, wie es ist – auch ein abgelegtes.
          const chosen = !typed ? '' : typed.toLowerCase() === current.toLowerCase() ? now.p : projectFor(findProject(typed, true));
          // Die Pausen aus den Zeilen lesen; ungültige (leer, Ende ≤ Beginn) weg.
          const picked = [];
          for (const record of breakRows) {
            const breakStart = read(record.from.value);
            const breakEnd = read(record.until.value);
            if (breakStart >= 0 && breakEnd > breakStart) picked.push([breakStart, breakEnd]);
          }
          const shape = tidy(start, end, picked);
          const next = { k: 'e', d: now.d, s: shape.s, e: shape.e, p: chosen, n: noted };
          if (shape.b.length) next.b = shape.b;
          if (next.e > next.s) putEntry(id, next);
        }
        render();
      });
    });
    form.addEventListener('keydown', (event) => {
      if (event.key === 'Escape') {
        event.stopPropagation();
        cancel.click();
      }
    });
    setTimeout(() => from.focus(), 0);
    return form;
  }

  function renderWeek() {
    const first = monday(shown);
    const days = [0, 1, 2, 3, 4, 5, 6].map((offset) => shift(first, offset));
    const lists = days.map((day) => entriesOf(day));
    const sums = lists.map((list) => distinctWorked(list.map(([, entry]) => entry)));
    const targets = days.map((day) => targetOf(day));
    const total = sums.reduce((sum, value) => sum + value, 0);
    const goal = targets.reduce((sum, value) => sum + value, 0);
    const short = (day) => dateOf(day).toLocaleDateString(LANG, { day: 'numeric', month: 'numeric' });
    $('week').querySelector('h2').textContent = text('tm_week', { from: short(first), to: short(days[6]) });
    const balance = $('week-balance');
    balance.textContent = span(total);
    if (goal) balance.appendChild(node('small', '', ' ' + text('tm_of', { time: span(goal) })));
    $('week-sum').textContent = text('tm_month', { time: span(monthMinutes(shown)) });

    const scale = Math.max(8 * 60, ...sums, ...targets);
    const bars = $('week-bars');
    bars.textContent = '';
    const perProject = new Map();
    days.forEach((day, index) => {
      const button = node('button', 'weekday' + (day === shown ? ' on' : ''));
      button.type = 'button';
      const column = node('span', 'column');
      const byProject = new Map();
      for (const [, entry] of lists[index]) {
        byProject.set(entry.p, (byProject.get(entry.p) || 0) + worked(entry));
        perProject.set(entry.p, (perProject.get(entry.p) || 0) + worked(entry));
      }
      for (const [project, minutes] of byProject) {
        const part = document.createElement('i');
        part.style.height = ((minutes / scale) * 100).toFixed(2) + '%';
        const color = projectColor(project);
        if (color) part.style.setProperty('--dot', color);
        column.appendChild(part);
      }
      if (targets[index]) {
        const line = node('span', 'goal');
        line.style.bottom = ((targets[index] / scale) * 100).toFixed(2) + '%';
        column.appendChild(line);
      }
      button.append(node('span', '', dateOf(day).toLocaleDateString(LANG, { weekday: 'short' })), column, node('b', '', sums[index] ? span(sums[index]) : '–'));
      button.addEventListener('click', () => go(day));
      bars.appendChild(button);
    });
    const legend = $('week-projects');
    legend.textContent = '';
    for (const [project, minutes] of [...perProject.entries()].sort((left, right) => right[1] - left[1])) {
      const item = document.createElement('div');
      const color = projectColor(project);
      if (color) item.style.setProperty('--dot', color);
      item.append(projectName(project) + ' ', node('b', '', span(minutes)));
      legend.appendChild(item);
    }
  }

  function monthMinutes(day) {
    const month = day.slice(0, 7);
    const byDay = new Map();
    for (const entry of entries.values()) {
      if (!entry.d.startsWith(month)) continue;
      if (!byDay.has(entry.d)) byDay.set(entry.d, []);
      byDay.get(entry.d).push(entry);
    }
    let sum = 0;
    for (const list of byDay.values()) sum += distinctWorked(list);
    return sum;
  }

  // ---- Meldung mit „Rückgängig“ -----------------------------------------------

  let toastTimer = 0;

  function toast(message, revert) {
    clearTimeout(toastTimer);
    undo = revert || null;
    $('toast-text').textContent = message;
    $('toast-undo').hidden = !revert;
    $('toast-undo').textContent = S.tm_undo;
    $('toast').hidden = false;
    toastTimer = setTimeout(() => {
      $('toast').hidden = true;
      undo = null;
    }, 6000);
  }

  function hideToast() {
    clearTimeout(toastTimer);
    undo = null;
    $('toast').hidden = true;
  }

  $('toast-undo').addEventListener('click', () => {
    const revert = undo;
    hideToast();
    act(() => {
      if (revert) revert();
      render();
    });
  });

  // ------------------------------------------------------------------------
  // Bedienung
  // ------------------------------------------------------------------------

  const quick = $('quick-input');
  quick.placeholder = S.tm_placeholder;
  quick.setAttribute('aria-label', S.tm_placeholder);
  quick.addEventListener('input', previewQuick);
  quick.addEventListener('keydown', (event) => {
    if (event.key === 'Enter') {
      event.preventDefault();
      bookQuick();
    } else if (event.key === 'Escape') {
      quick.value = '';
      previewQuick();
    }
  });
  $('quick-book').addEventListener('click', bookQuick);
  $('running-stop').textContent = S.tm_stop;
  $('running-drop').textContent = S.tm_drop;
  $('today').textContent = S.tm_today;
  $('prev').title = S.tm_prev;
  $('next').title = S.tm_next;
  $('prev').setAttribute('aria-label', S.tm_prev);
  $('next').setAttribute('aria-label', S.tm_next);
  $('settings-button').title = S.tm_settings;
  $('settings-button').setAttribute('aria-label', S.tm_settings);
  $('settings-close').setAttribute('aria-label', S.close);
  $('trouble-text').textContent = S.tm_load_failed;
  $('trouble-retry').textContent = S.tm_retry;
  document.title = S.app_time;

  function go(day) {
    if (!validKey(day)) return;
    shown = day;
    editing = '';
    choosing = '';
    render();
  }

  $('prev').addEventListener('click', () => go(shift(shown, -1)));
  $('next').addEventListener('click', () => go(shift(shown, 1)));
  $('today').addEventListener('click', () => go(today()));
  // ---- Kalender ---------------------------------------------------------------
  //
  // Ein Monat zum Anklicken, mit einem Punkt an den Tagen, an denen gebucht ist.
  // Ein eigener statt dem des Systems: Der bliebe nach der Auswahl offen stehen.

  // Der Monat, den der Kalender zeigt ('2026-11') – leer, wenn er zu ist.
  let calendarMonth = '';

  function closeCalendar() {
    if (!calendarMonth) return;
    calendarMonth = '';
    renderCalendar();
  }

  function renderCalendar() {
    const box = $('calendar');
    box.hidden = !calendarMonth;
    box.textContent = '';
    $('date').setAttribute('aria-expanded', calendarMonth ? 'true' : 'false');
    if (!calendarMonth) return;
    const [year, month] = calendarMonth.split('-').map(Number);
    const first = new Date(year, month - 1, 1);
    const turn = (label, name, by) => {
      const button = node('button', 'round');
      button.type = 'button';
      button.title = label;
      button.setAttribute('aria-label', label);
      button.append(icon(name));
      button.addEventListener('click', () => {
        const other = new Date(year, month - 1 + by, 1);
        calendarMonth = other.getFullYear() + '-' + two(other.getMonth() + 1);
        renderCalendar();
      });
      return button;
    };
    const head = node('div', 'cal-head');
    head.append(turn(S.tm_cal_prev, 'back', -1), node('b', '', first.toLocaleDateString(LANG, { month: 'long', year: 'numeric' })), turn(S.tm_cal_next, 'forward', 1));
    const grid = node('div', 'cal-grid');
    // Montag zuerst. (Der 1. Januar 2024 war ein Montag.)
    for (let weekday = 1; weekday <= 7; weekday++) {
      grid.append(node('span', 'cal-name', new Date(2024, 0, weekday).toLocaleDateString(LANG, { weekday: 'narrow' })));
    }
    const sums = new Map();
    const byDay = new Map();
    for (const entry of entries.values()) {
      if (!entry.d.startsWith(calendarMonth)) continue;
      if (!byDay.has(entry.d)) byDay.set(entry.d, []);
      byDay.get(entry.d).push(entry);
    }
    for (const [day, list] of byDay) sums.set(day, distinctWorked(list));
    for (let blank = (first.getDay() + 6) % 7; blank > 0; blank--) grid.append(node('span', ''));
    const days = new Date(year, month, 0).getDate();
    for (let number = 1; number <= days; number++) {
      const key = calendarMonth + '-' + two(number);
      const booked = sums.get(key) || 0;
      const cell = node('button', 'cal-day' + (key === shown ? ' on' : '') + (key === today() ? ' today' : '') + (booked ? ' booked' : ''), String(number));
      cell.type = 'button';
      const label = dateOf(key).toLocaleDateString(LANG, { weekday: 'long', day: 'numeric', month: 'long' }) + (booked ? ' · ' + span(booked) : '');
      cell.title = label;
      cell.setAttribute('aria-label', label);
      cell.addEventListener('click', () => {
        // Gewählt: Der Kalender geht zu, der Tag auf.
        calendarMonth = '';
        go(key);
      });
      grid.append(cell);
    }
    box.append(head, grid);
    box.style.left = $('date').offsetLeft + 'px';
  }

  $('date').addEventListener('click', (event) => {
    event.stopPropagation();
    calendarMonth = calendarMonth ? '' : shown.slice(0, 7);
    renderCalendar();
  });
  // Ein Klick daneben schließt ihn; einer darin nicht.
  $('calendar').addEventListener('click', (event) => event.stopPropagation());
  document.addEventListener('click', closeCalendar);

  document.addEventListener('keydown', (event) => {
    const typing = /^(input|select|textarea)$/i.test((event.target && event.target.tagName) || '');
    if (!$('settings').hidden) {
      if (event.key === 'Escape') closeSettings();
      return;
    }
    if (event.key === 'Escape' && calendarMonth) {
      closeCalendar();
      return;
    }
    if (event.key === 'Escape' && choosing) {
      choosing = '';
      renderDay();
      return;
    }
    if (typing || event.metaKey || event.ctrlKey || event.altKey) return;
    if (event.key === 'ArrowLeft') go(shift(shown, -1));
    else if (event.key === 'ArrowRight') go(shift(shown, 1));
    else if (event.key === 't' || event.key === 'h') go(today());
    else if (event.key === '/' || event.key === 'n') {
      event.preventDefault();
      quick.focus();
    }
  });

  // ------------------------------------------------------------------------
  // Einstellungen, Projekte, Ausgabe als Tabelle
  // ------------------------------------------------------------------------

  function openSettings() {
    if (!ready) return;
    // Was sich eben noch zurücknehmen ließ, passt nicht mehr zu dem, was hier geändert wird.
    hideToast();
    const sheet = $('settings');
    sheet.querySelector('h2').textContent = S.tm_settings;
    $('set-target-title').textContent = S.tm_set_target;
    $('set-round-title').textContent = S.tm_set_round;
    $('set-projects-title').textContent = S.tm_set_projects;
    $('set-export-title').textContent = S.tm_set_export;
    $('set-export-text').textContent = S.tm_set_export_text;
    $('export-month').textContent = dateOf(shown).toLocaleDateString(LANG, { month: 'long', year: 'numeric' });
    $('export-all').textContent = S.tm_export_all;
    $('set-where').textContent = kept ? S.tm_where : S.tm_private;

    const targets = $('set-targets');
    targets.textContent = '';
    // Montag zuerst. (Der 1. Januar 2024 war ein Montag.)
    for (const weekday of [1, 2, 3, 4, 5, 6, 0]) {
      const label = document.createElement('label');
      const input = document.createElement('input');
      input.type = 'number';
      input.min = '0';
      input.max = '24';
      input.step = '0.25';
      input.value = String(settings.targets[weekday]);
      input.addEventListener('change', () => {
        const hours = Math.round(Math.min(24, Math.max(0, Number(input.value) || 0)) * 4) / 4;
        input.value = String(hours);
        act(() => {
          settings.targets[weekday] = hours;
          saveNote();
          render();
        });
      });
      label.append(new Date(2024, 0, weekday === 0 ? 7 : weekday).toLocaleDateString(LANG, { weekday: 'short' }), input);
      targets.appendChild(label);
    }

    const rounding = $('set-round');
    rounding.textContent = '';
    for (const step of [1, 5, 15]) {
      const button = node('button', step === settings.step ? 'on' : '', text('tm_minutes', { n: step }));
      button.type = 'button';
      button.addEventListener('click', () =>
        act(() => {
          settings.step = step;
          saveNote();
          openSettings();
        })
      );
      rounding.appendChild(button);
    }

    const list = $('set-projects');
    list.textContent = '';
    const usedProjects = new Set([...entries.values()].map((entry) => entry.p));
    if (running) usedProjects.add(running.p);
    for (const [id, project] of [...projects.entries()].sort((left, right) => left[1].name.localeCompare(right[1].name))) {
      const row = node('div', 'project-row' + (project.gone ? ' gone' : ''));
      const color = document.createElement('input');
      color.type = 'color';
      color.value = project.color;
      color.setAttribute('aria-label', S.tm_color);
      color.addEventListener('change', () => {
        const picked = color.value;
        if (!/^#[0-9a-f]{6}$/i.test(picked)) return;
        act(() => {
          const now = projects.get(id);
          if (!now) return;
          now.color = picked;
          putProject(id, now);
          render();
        });
      });
      const name = document.createElement('input');
      name.type = 'text';
      name.value = project.name;
      name.maxLength = 40;
      name.setAttribute('aria-label', S.tm_project);
      name.addEventListener('change', () => {
        const wanted = cleanName(name.value);
        act(() => {
          const now = projects.get(id);
          // Kein leerer und kein doppelter Name.
          if (now && wanted && !(projectByName(wanted) && projectByName(wanted) !== id)) {
            now.name = wanted;
            putProject(id, now);
            render();
          }
          openSettings();
        });
      });
      // Ein Projekt ohne Einträge verschwindet ganz; eines mit Einträgen wird
      // nur abgelegt – seine Zeiten behalten ihren Namen.
      const toggle = node('button', 'ghost', !usedProjects.has(id) ? S.tm_delete : project.gone ? S.tm_restore : S.tm_archive);
      toggle.type = 'button';
      toggle.addEventListener('click', () =>
        act(() => {
          const now = projects.get(id);
          if (now) {
            // Ob es Zeiten auf dem Projekt gibt, zählt jetzt – nicht beim Öffnen
            // der Einstellungen.
            const used = (running && running.p === id) || [...entries.values()].some((entry) => entry.p === id);
            if (used) {
              now.gone = !now.gone;
              putProject(id, now);
            } else {
              projects.delete(id);
              Z.records.remove(id).then(stored);
            }
            if (settings.last === id) settings.last = '';
            saveNote();
            render();
          }
          openSettings();
        })
      );
      row.append(color, name, toggle);
      list.appendChild(row);
    }
    if (!projects.size) list.appendChild(node('p', 'note', S.tm_no_projects));
    sheet.hidden = false;
  }

  function closeSettings() {
    $('settings').hidden = true;
  }

  $('settings-button').addEventListener('click', openSettings);
  $('settings-close').addEventListener('click', closeSettings);
  $('settings').addEventListener('click', (event) => {
    if (event.target === $('settings')) closeSettings();
  });

  // Eine Tabelle zum Weiterrechnen: Semikolon als Trenner, Stunden mit Komma im
  // Deutschen. Felder, die eine Tabellenkalkulation als Formel läse, bekommen ein
  // Hochkomma davor.
  let exporting = false;

  function exportTable(prefix, name) {
    if (exporting || !ready) return;
    const cell = (value) => {
      let content = String(value);
      if (/^[=+\-@\t\r]/.test(content)) content = "'" + content;
      return /[";\n\r]/.test(content) ? '"' + content.replace(/"/g, '""') + '"' : content;
    };
    const decimal = (minutes) => (minutes / 60).toFixed(2).replace('.', LANG === 'de' ? ',' : '.');
    // Die Stunden sind gerundet; die Minuten daneben stimmen auf die Minute.
    const rows = [[S.tm_col_date, S.tm_col_from, S.tm_col_until, S.tm_col_break, S.tm_col_hours, S.tm_col_minutes, S.tm_project, S.tm_note].map(cell).join(';')];
    const list = [...entries.values()].filter((entry) => entry.d.startsWith(prefix)).sort((left, right) => left.d.localeCompare(right.d) || left.s - right.s);
    for (const entry of list) {
      const project = projects.has(entry.p) ? projects.get(entry.p).name : '';
      rows.push([entry.d, clock(entry.s), clock(entry.e), pauseOf(entry), decimal(worked(entry)), worked(entry), project, entry.n].map(cell).join(';'));
    }
    // Das Zeichen am Anfang sagt Excel, dass die Datei in UTF-8 geschrieben ist.
    const mark = String.fromCharCode(0xfeff);
    const blob = new Blob([mark + rows.join('\r\n') + '\r\n'], { type: 'text/csv;charset=utf-8' });
    exporting = true;
    Z.saveFile(name, blob).then((saved) => {
      exporting = false;
      toast(saved ? text('tm_exported', { n: list.length }) : S.tm_save_failed);
    });
  }

  $('export-month').addEventListener('click', () => exportTable(shown.slice(0, 7), S.tm_file + '-' + shown.slice(0, 7) + '.csv'));
  $('export-all').addEventListener('click', () => exportTable('', S.tm_file + '.csv'));

  // ------------------------------------------------------------------------
  // Laden – beim Start und wenn der Tab wieder nach vorn kommt
  // ------------------------------------------------------------------------

  function load() {
    return Promise.all([Z.load(), Z.records.all()]).then(([note, cards]) => {
      // Ohne vollständigen Stand wird nichts angezeigt und nichts geschrieben –
      // sonst überschriebe ein leerer Zettel den gesicherten.
      const ok = note.ok && cards.ok;
      $('trouble').hidden = ok;
      if (!ok) {
        ready = false;
        $('quick-book').disabled = true;
        return;
      }
      kept = note.kept && cards.kept;
      const value = note.value;
      if (Array.isArray(value.targets) && value.targets.length === 7) {
        settings.targets = value.targets.map((hours) => Math.min(24, Math.max(0, Number(hours) || 0)));
      }
      if ([1, 5, 15].includes(value.step)) settings.step = value.step;
      settings.last = typeof value.last === 'string' ? value.last : '';
      running = null;
      const run = value.run;
      // Eine Uhr, die länger als eine Woche läuft, hat jemand vergessen.
      const now = Date.now();
      if (run && typeof run === 'object' && Number.isFinite(run.t0) && run.t0 <= now && now - run.t0 < 7 * 24 * 3600000) {
        running = newRun(typeof run.p === 'string' ? run.p : '', cleanNote(run.n), run.t0);
        // Ihre Pausen: der Reihe nach, zwischen Beginn und jetzt.
        let edge = run.t0;
        for (const pair of Array.isArray(run.b) ? run.b.slice(0, MAX_BREAKS) : []) {
          const from = Array.isArray(pair) ? pair[0] : NaN;
          const to = Array.isArray(pair) ? pair[1] : NaN;
          if (Number.isFinite(from) && Number.isFinite(to) && from >= edge && to > from && to <= now) {
            running.breaks.push([from, to]);
            edge = to;
          }
        }
        if (Number.isFinite(run.pa) && run.pa >= edge && run.pa <= now && run.pa > 0) running.pausedAt = run.pa;
        if (typeof run.id === 'string' && /^[A-Za-z0-9_-]{1,40}$/.test(run.id)) running.id = run.id;
        if (Number.isFinite(run.si) && run.si >= run.t0 && run.si <= now) running.since = run.si;
      }
      projects.clear();
      entries.clear();
      for (const [id, record] of cards.list) {
        if (id.startsWith('p-')) {
          const project = cleanProject(record);
          if (project) projects.set(id, project);
        } else if (id.startsWith('e')) {
          const entry = cleanEntry(record);
          if (entry) entries.set(id, entry);
        }
      }
      $('private-note').hidden = kept;
      $('private-note').textContent = S.tm_private;
      ready = true;
      render();
    });
  }

  // ---- Abgleich mit dem Gesicherten ---------------------------------------------
  //
  // In einem zweiten Tab oder Fenster kann inzwischen gebucht worden sein. War
  // dieser Tab weg – verdeckt oder ohne Eingabe –, liest er den Stand neu, und
  // jede Änderung wartet darauf: Sonst überschriebe ein veralteter Tab etwa die
  // Uhr, die im anderen gerade gestartet wurde.

  // Der laufende Abgleich.
  let pending = null;

  function sync() {
    if (!pending) {
      stale = false;
      pending = load()
        .catch(() => {
          ready = false;
          $('trouble').hidden = false;
        })
        .then(() => {
          pending = null;
        });
    }
    return pending;
  }

  // Führt eine Änderung aus: auf dem abgeglichenen Stand – und nur, wenn einer
  // geladen ist.
  function act(change) {
    if (stale || pending) {
      sync().then(() => act(change));
      return;
    }
    if (ready) change();
  }

  $('trouble-retry').addEventListener('click', () => sync());

  setInterval(tick, 1000);
  // Um Mitternacht wird aus „heute“ ein anderer Tag.
  let lastToday = today();
  setInterval(() => {
    if (today() !== lastToday) {
      if (shown === lastToday) shown = today();
      lastToday = today();
      render();
    }
  }, 30000);

  let awaySince = 0;
  const away = () => {
    if (!awaySince) awaySince = Date.now();
  };
  const back = () => {
    const gone = awaySince ? Date.now() - awaySince : 0;
    awaySince = 0;
    // Ein privates Fenster hat nichts Gesichertes, mit dem es sich abgleichen
    // könnte: Dort gilt, was im Tab steht.
    if (gone < 400 || !kept) return;
    stale = true;
    // Mitten in der Bearbeitung eines Eintrags wartet der Abgleich, bis sie zu
    // Ende ist (`render`) – oder bis die nächste Änderung ihn verlangt (`act`).
    if (editing) return;
    sync().then(() => {
      if (!$('settings').hidden) openSettings();
    });
  };
  document.addEventListener('visibilitychange', () => (document.hidden ? away() : back()));
  window.addEventListener('blur', away);
  window.addEventListener('focus', back);

  sync().then(() => quick.focus());
})();
