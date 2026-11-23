// Wetter und Regenradar – eine eingebaute App.
//
// Die Seite holt selbst, was sie zeigt: die Vorhersage und die Ortssuche bei
// Open-Meteo, das Regenradar (Daten des Deutschen Wetterdienstes) bei Bright
// Sky. Wohin sie sonst im Netz dürfte: nirgends – das legt die Schutzregel
// fest, mit der der Browser sie ausliefert (src/apps.rs). Den Standort bekommt
// sie vom Browser nur nach Zustimmung und nur gerundet.
//
// Alles, was von außen kommt (Ortsnamen, Zahlen), gilt als Material: Es wird
// geprüft und nur als Text in die Seite gesetzt.
(() => {
  'use strict';
  const Z = window.ZunderApp;
  const S = Z.strings;
  const LANG = Z.data.lang === 'de' ? 'de' : 'en';
  const MAP = window.ZunderMap || { places: [], lines: [] };
  const $ = (id) => document.getElementById(id);
  const text = (key, values) =>
    String(S[key] || '').replace(/\{(\w+)\}/g, (whole, name) => (values && name in values ? values[name] : whole));

  const FORECAST = 'https://api.open-meteo.com/v1/forecast';
  const SEARCH = 'https://geocoding-api.open-meteo.com/v1/search';
  // Postleitzahlen kennt Open-Meteo nur lückenhaft (deutsche gar nicht) – die
  // schlägt die App bei Nominatim nach, dem Suchdienst von OpenStreetMap.
  const POSTAL = 'https://nominatim.openstreetmap.org/search';
  // Das Land dieses Macs: Dort wird eine Postleitzahl zuerst gesucht.
  const REGION = /^[a-z]{2}$/.test(Z.data.region || '') ? Z.data.region : '';
  const RADAR = 'https://api.brightsky.dev/radar';
  // Das Satellitenbild hinter dem Radar: NASA Blue Marble, als ein Bild für den
  // gezeigten Ausschnitt.
  const SATELLITE = 'https://gibs.earthdata.nasa.gov/wms/epsg4326/best/wms.cgi';
  // Das deutsche Radar reicht über Deutschland und einen Streifen der Nachbarn.
  const RADAR_AREA = { south: 45.9, north: 55.6, west: 2.0, east: 18.2 };
  const RADII = [50, 100, 150];

  document.title = S.app_weather;

  // ------------------------------------------------------------------------
  // Kleine Helfer
  // ------------------------------------------------------------------------

  const finite = (value) => typeof value === 'number' && Number.isFinite(value);

  // Liest eine Antwort aus dem Netz als JSON – aber nur bis zu der Größe, die
  // zu ihr passt. Was darüber hinausgeht, wird gar nicht erst geladen.
  async function jsonOf(response, limit) {
    if (!response.body || typeof response.body.getReader !== 'function') return response.json();
    const reader = response.body.getReader();
    const parts = [];
    let size = 0;
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.length;
      if (size > limit) {
        reader.cancel();
        throw new Error('size');
      }
      parts.push(value);
    }
    const whole = new Uint8Array(size);
    let at = 0;
    for (const part of parts) {
      whole.set(part, at);
      at += part.length;
    }
    return JSON.parse(new TextDecoder().decode(whole));
  }
  const SMALL = 600000; // Vorhersage, Ortssuche
  const LARGE = 60000000; // Radar: viele Bilder, jedes ein Feld von Zahlen
  const number = (value, digits) =>
    finite(value) ? value.toLocaleString(LANG, { maximumFractionDigits: digits || 0, minimumFractionDigits: 0 }) : '–';
  // Gerundet – und ohne „-0°“ knapp unter null.
  const degrees = (value) => (finite(value) ? (Math.round(value) || 0).toLocaleString(LANG) + '°' : '–');
  // Zeiten kommen als Ortszeit des gewählten Ortes: „2026-11-23T05:45“.
  const clock = (iso) => (typeof iso === 'string' && /T\d\d:\d\d/.test(iso) ? iso.slice(11, 16) : '');
  const two = (value) => String(value).padStart(2, '0');

  function icon(name, className) {
    const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    const use = document.createElementNS('http://www.w3.org/2000/svg', 'use');
    use.setAttribute('href', '#i-' + name);
    svg.appendChild(use);
    if (className) svg.setAttribute('class', className);
    return svg;
  }

  function node(tag, className, content) {
    const element = document.createElement(tag);
    if (className) element.className = className;
    if (content !== undefined) element.textContent = content;
    return element;
  }

  // Wetterlage nach den Kennzahlen der Weltorganisation für Meteorologie:
  // Text, Symbol und der Himmel dahinter.
  function condition(code, day) {
    const sun = day ? 'sun' : 'moon';
    const partly = day ? 'cloud-sun' : 'cloud-moon';
    const table = {
      0: ['wx_c_clear', sun, 'clear'],
      1: ['wx_c_mostly_clear', sun, 'clear'],
      2: ['wx_c_partly', partly, 'cloudy'],
      3: ['wx_c_overcast', 'cloud', 'cloudy'],
      45: ['wx_c_fog', 'fog', 'fog'],
      48: ['wx_c_fog', 'fog', 'fog'],
      51: ['wx_c_drizzle', 'drizzle', 'rain'],
      53: ['wx_c_drizzle', 'drizzle', 'rain'],
      55: ['wx_c_drizzle', 'drizzle', 'rain'],
      56: ['wx_c_freezing_drizzle', 'drizzle', 'rain'],
      57: ['wx_c_freezing_drizzle', 'drizzle', 'rain'],
      61: ['wx_c_rain_light', 'rain', 'rain'],
      63: ['wx_c_rain', 'rain', 'rain'],
      65: ['wx_c_rain_heavy', 'rain', 'rain'],
      66: ['wx_c_freezing_rain', 'rain', 'rain'],
      67: ['wx_c_freezing_rain', 'rain', 'rain'],
      71: ['wx_c_snow_light', 'snow', 'snow'],
      73: ['wx_c_snow', 'snow', 'snow'],
      75: ['wx_c_snow_heavy', 'snow', 'snow'],
      77: ['wx_c_snow_grains', 'snow', 'snow'],
      80: ['wx_c_showers', 'rain', 'rain'],
      81: ['wx_c_showers', 'rain', 'rain'],
      82: ['wx_c_showers_heavy', 'rain', 'rain'],
      85: ['wx_c_snow_showers', 'snow', 'snow'],
      86: ['wx_c_snow_showers', 'snow', 'snow'],
      95: ['wx_c_thunder', 'thunder', 'thunder'],
      96: ['wx_c_thunder_hail', 'thunder', 'thunder'],
      99: ['wx_c_thunder_hail', 'thunder', 'thunder'],
    };
    const entry = table[code] || ['wx_c_overcast', 'cloud', 'cloudy'];
    return { text: S[entry[0]] || '', icon: entry[1], sky: entry[2] };
  }

  function compass(direction) {
    const names = String(S.wx_dir || 'N,NE,E,SE,S,SW,W,NW').split(',');
    return finite(direction) ? names[Math.round((((direction % 360) + 360) % 360) / 45) % 8] || '' : '';
  }

  // ------------------------------------------------------------------------
  // Zustand
  // ------------------------------------------------------------------------

  let kept = false; // merkt sich der Browser den Zettel dieser App?
  let saved = []; // gemerkte Orte
  let place = null; // { name, where, lat, lon, mine }
  let forecast = null;
  let loadedAt = 0;
  let loading = 0;

  function cleanPlace(entry) {
    if (!entry || typeof entry !== 'object') return null;
    const lat = Number(entry.lat);
    const lon = Number(entry.lon);
    if (!finite(lat) || !finite(lon) || Math.abs(lat) > 90 || Math.abs(lon) > 180) return null;
    return {
      name: Z.clip(entry.name || '', 80),
      where: Z.clip(entry.where || '', 80),
      lat: Math.round(lat * 100) / 100,
      lon: Math.round(lon * 100) / 100,
      mine: entry.mine === true,
    };
  }

  function remember() {
    if (!kept) return;
    const last = place ? { name: place.name, where: place.where, lat: place.lat, lon: place.lon, mine: place.mine } : null;
    Z.save({ v: 1, last, places: saved.slice(0, 8) });
  }

  // ------------------------------------------------------------------------
  // Ortswahl
  // ------------------------------------------------------------------------

  const places = $('places');
  const search = $('search');
  let searchTimer = 0;
  let searchRun = 0;
  // Die Suche zur Eingabe, die noch auf ihren Augenblick wartet.
  let searchPending = null;

  $('place-button').addEventListener('click', () => togglePlaces(places.hidden));
  $('intro-search').addEventListener('click', () => togglePlaces(true));
  search.placeholder = S.wx_search;
  search.setAttribute('aria-label', S.wx_search);
  $('saved-title').textContent = S.wx_saved;
  $('refresh').title = S.wx_refresh;
  $('refresh').setAttribute('aria-label', S.wx_refresh);
  $('refresh').addEventListener('click', () => loadAll(true));
  $('search-form').addEventListener('submit', (event) => {
    event.preventDefault();
    // Wartet die Suche zur Eingabe noch, beginnt sie jetzt. Sonst gilt der
    // erste Treffer – die Liste gehört immer zu dem, was im Feld steht.
    if (searchPending) {
      const start = searchPending;
      clearTimeout(searchTimer);
      searchPending = null;
      start();
      return;
    }
    const first = $('results').querySelector('button');
    if (first) first.click();
  });

  function togglePlaces(open) {
    places.hidden = !open;
    if (open) {
      setMineNote('');
      renderSaved();
      search.focus();
      search.select();
    }
  }

  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && !places.hidden) togglePlaces(false);
  });

  function placeRow(entry, removable) {
    const row = node('button', 'row');
    row.type = 'button';
    row.append(icon(entry.mine ? 'location' : 'globe'), node('span', '', entry.name));
    if (entry.where) row.append(node('span', 'where', entry.where));
    row.addEventListener('click', () => {
      choose(entry);
    });
    if (removable) {
      const drop = node('span', 'drop');
      drop.title = S.wx_remove;
      drop.setAttribute('role', 'button');
      drop.setAttribute('aria-label', S.wx_remove);
      drop.append(icon('close'));
      drop.addEventListener('click', (event) => {
        event.stopPropagation();
        saved = saved.filter((other) => !(other.lat === entry.lat && other.lon === entry.lon));
        remember();
        renderSaved();
      });
      row.append(drop);
    }
    return row;
  }

  function renderSaved() {
    const box = $('saved');
    box.textContent = '';
    for (const entry of saved) box.appendChild(placeRow(entry, true));
    $('saved-title').hidden = saved.length === 0;
    $('private-note').hidden = kept;
    $('private-note').textContent = S.wx_private;
    $('mine-label').textContent = place && place.mine ? S.wx_mine_stop : S.wx_mine;
    // Die Ortung trifft nicht immer genau: Dann hilft die Postleitzahl.
    if (place && place.mine) setMineNote(S.wx_mine_hint);
  }

  // Sieht die Eingabe wie eine Postleitzahl aus? Kurz, mit Ziffern, ohne Wort.
  function looksPostal(query) {
    return /^[A-Za-z0-9][A-Za-z0-9 -]{2,9}$/.test(query) && /\d{3}/.test(query.replace(/[ -]/g, '')) && !/[A-Za-z]{4}/.test(query);
  }

  function showResults(run, entries) {
    if (run !== searchRun) return;
    const box = $('results');
    box.textContent = '';
    for (const entry of entries) box.appendChild(placeRow(entry, false));
    if (!box.children.length) box.appendChild(node('p', 'note', S.wx_no_results));
  }

  // Eine Postleitzahl: erst im eigenen Land, und nur wenn es sie dort nicht
  // gibt, überall. Jede Anfrage steht für eine fertig getippte Eingabe – der
  // Dienst ist kein Vorschlagsgeber für jeden Tastendruck.
  function postalSearch(run, query, everywhere) {
    const country = !everywhere && REGION ? '&countrycodes=' + REGION : '';
    const url = POSTAL + '?format=jsonv2&addressdetails=1&limit=6&accept-language=' + LANG + country + '&postalcode=' + encodeURIComponent(query);
    return fetch(url)
      .then((response) => (response.ok ? jsonOf(response, SMALL) : Promise.reject(new Error('postal'))))
      .then((data) => {
        if (run !== searchRun) return;
        const entries = [];
        for (const hit of Array.isArray(data) ? data.slice(0, 6) : []) {
          const address = hit && typeof hit.address === 'object' && hit.address ? hit.address : {};
          const town = [address.city, address.town, address.village, address.municipality, address.county].find((part) => typeof part === 'string' && part);
          const code = typeof address.postcode === 'string' ? address.postcode : query;
          const where = [address.state, address.country].filter((part) => typeof part === 'string' && part).join(', ');
          const entry = cleanPlace({ name: (code + ' ' + (town || '')).trim(), where, lat: Number(hit.lat), lon: Number(hit.lon) });
          if (entry && entry.name) entries.push(entry);
        }
        if (!entries.length && country) return postalSearch(run, query, true);
        showResults(run, entries);
      });
  }

  search.addEventListener('input', () => {
    clearTimeout(searchTimer);
    searchPending = null;
    const query = search.value.trim();
    const run = ++searchRun;
    // Die Treffer von eben gehören zur Eingabe von eben.
    $('results').textContent = '';
    if (query.length < 2) return;
    const later = (start, wait) => {
      searchPending = start;
      searchTimer = setTimeout(() => {
        searchPending = null;
        start();
      }, wait);
    };
    if (looksPostal(query)) {
      // Erst suchen, wenn die Eingabe steht – und lang genug für eine
      // Postleitzahl ist: Der Dienst bekommt keine halben.
      if (query.replace(/[ -]/g, '').length < 4) return;
      later(() => {
        $('results').textContent = '';
        $('results').appendChild(node('p', 'note', S.wx_searching));
        postalSearch(run, query.slice(0, 12), false).catch(() => showResults(run, []));
      }, 900);
      return;
    }
    later(() => {
      const url = SEARCH + '?count=6&format=json&language=' + LANG + '&name=' + encodeURIComponent(query.slice(0, 80));
      fetch(url)
        .then((response) => (response.ok ? jsonOf(response, SMALL) : Promise.reject(new Error('search'))))
        .then((data) => {
          if (run !== searchRun) return;
          const box = $('results');
          box.textContent = '';
          const found = Array.isArray(data && data.results) ? data.results.slice(0, 6) : [];
          for (const hit of found) {
            const where = [hit.admin1, hit.country].filter((part) => typeof part === 'string' && part).join(', ');
            const entry = cleanPlace({ name: hit.name, where, lat: hit.latitude, lon: hit.longitude });
            if (entry && entry.name) box.appendChild(placeRow(entry, false));
          }
          if (!box.children.length) box.appendChild(node('p', 'note', S.wx_no_results));
        })
        .catch(() => {
          if (run !== searchRun) return;
          $('results').textContent = '';
          $('results').appendChild(node('p', 'note', S.wx_no_results));
        });
    }, 280);
  });

  function choose(entry) {
    place = entry;
    if (!entry.mine && !saved.some((other) => other.lat === entry.lat && other.lon === entry.lon)) {
      saved.unshift({ name: entry.name, where: entry.where, lat: entry.lat, lon: entry.lon, mine: false });
      saved = saved.slice(0, 8);
    }
    search.value = '';
    $('results').textContent = '';
    togglePlaces(false);
    remember();
    loadAll(true);
  }

  // ---- Standort ---------------------------------------------------------------

  function setMineNote(message) {
    for (const id of ['mine-note']) {
      $(id).textContent = message;
      $(id).hidden = !message;
    }
    const intro = $('intro').querySelector('p');
    if (!$('intro').hidden && message) intro.textContent = message;
  }

  // Der nächste größere Ort aus der Liste des Radars – nur als Beschriftung.
  function nearestName(lat, lon) {
    let best = null;
    let bestDistance = 30;
    for (const [name, placeLat, placeLon] of MAP.places) {
      const dy = (placeLat - lat) * 111.2;
      const dx = (placeLon - lon) * 111.2 * Math.cos((lat * Math.PI) / 180);
      const distance = Math.hypot(dx, dy);
      if (distance < bestDistance) {
        bestDistance = distance;
        best = name;
      }
    }
    return best;
  }

  function useMine() {
    // Schon in Gebrauch: Dann nimmt der Knopf die Erlaubnis zurück.
    if (place && place.mine) {
      Z.forgetLocation();
      place = null;
      // Was für diesen Ort noch unterwegs ist, kommt nicht mehr an.
      loading++;
      radar.run++;
      clearTimeout(radar.timer);
      radar.frames = [];
      forecast = null;
      remember();
      togglePlaces(false);
      showOnly('intro');
      return;
    }
    setMineNote(S.wx_locating);
    Z.location((progress) => {
      setMineNote(progress.state === 'asking' ? S.wx_asking : S.wx_locating);
    }).then((answer) => {
      if (answer.state === 'ok' && finite(answer.lat) && finite(answer.lon)) {
        const near = nearestName(answer.lat, answer.lon);
        const entry = cleanPlace({
          name: S.wx_mine_short,
          where: near ? text('wx_near', { place: near }) : '',
          lat: answer.lat,
          lon: answer.lon,
          mine: true,
        });
        setMineNote('');
        choose(entry);
        return;
      }
      // Die Frage in der Leiste blieb ohne Antwort: kein Fehler – alles wie vorher.
      if (answer.state === 'unanswered') {
        setMineNote('');
        if (!$('intro').hidden) showOnly('intro');
        return;
      }
      const why = { denied: S.wx_denied, refused: answer.why === 'stealth' ? S.wx_stealth : S.wx_failed }[answer.state];
      setMineNote(why || S.wx_failed);
    });
  }

  $('mine').addEventListener('click', useMine);
  $('intro-mine').addEventListener('click', useMine);

  // ------------------------------------------------------------------------
  // Vorhersage laden und zeigen
  // ------------------------------------------------------------------------

  function showOnly(id) {
    for (const other of ['content', 'intro', 'trouble']) $(other).hidden = other !== id;
    $('bar').hidden = id === 'intro';
    if (id === 'intro') {
      $('intro').querySelector('h1').textContent = S.wx_intro_title;
      $('intro').querySelector('p').textContent = S.wx_intro_text;
      $('intro-mine').textContent = S.wx_mine;
      $('intro-search').textContent = S.wx_intro_search;
      setSky('clear', false);
    }
    if (id === 'trouble') {
      $('trouble').querySelector('p').textContent = S.wx_trouble;
      $('retry').textContent = S.wx_retry;
    }
  }

  $('retry').addEventListener('click', () => loadAll(true));

  // Die Vorhersage kommt aus dem Netz. Bevor die Anzeige sie liest, wird sie auf
  // das gestutzt, was dort erwartet wird: kurze, je Block gleich lange Listen.
  function tidyForecast(data) {
    if (!data || typeof data !== 'object' || !data.current || typeof data.current !== 'object') return null;
    const block = (source, keys, max) => {
      const input = source && typeof source === 'object' ? source : {};
      const lists = keys.map((key) => (Array.isArray(input[key]) ? input[key] : []));
      const count = Math.min(max, ...lists.map((list) => list.length));
      const out = {};
      keys.forEach((key, index) => {
        out[key] = lists[index].slice(0, count);
      });
      return out;
    };
    return {
      current: data.current,
      utc_offset_seconds: data.utc_offset_seconds,
      minutely_15: block(data.minutely_15, ['time', 'precipitation'], 8),
      hourly: block(data.hourly, ['time', 'temperature_2m', 'weather_code', 'precipitation_probability', 'precipitation', 'is_day'], 24),
      daily: block(
        data.daily,
        ['time', 'weather_code', 'temperature_2m_max', 'temperature_2m_min', 'precipitation_sum', 'precipitation_probability_max', 'sunrise', 'sunset'],
        7
      ),
    };
  }

  function loadAll(fresh) {
    if (!place) {
      showOnly('intro');
      return;
    }
    const run = ++loading;
    $('place-name').textContent = place.where && place.mine ? place.name + ' · ' + place.where : place.name;
    $('refresh').classList.add('busy');
    const query = [
      'latitude=' + place.lat,
      'longitude=' + place.lon,
      'current=temperature_2m,apparent_temperature,relative_humidity_2m,is_day,precipitation,weather_code,wind_speed_10m,wind_direction_10m,wind_gusts_10m,pressure_msl',
      'minutely_15=precipitation',
      'forecast_minutely_15=8',
      'hourly=temperature_2m,weather_code,precipitation_probability,precipitation,is_day',
      'forecast_hours=24',
      'daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_sum,precipitation_probability_max,sunrise,sunset',
      'forecast_days=7',
      'timezone=auto',
    ].join('&');
    fetch(FORECAST + '?' + query)
      .then((response) => (response.ok ? jsonOf(response, SMALL) : Promise.reject(new Error('forecast'))))
      .then((data) => {
        if (run !== loading) return;
        const tidy = tidyForecast(data);
        if (!tidy) throw new Error('forecast');
        forecast = tidy;
        loadedAt = Date.now();
        render();
        showOnly('content');
        $('refresh').classList.remove('busy');
      })
      .catch(() => {
        if (run !== loading) return;
        $('refresh').classList.remove('busy');
        // Eine ältere Vorhersage bleibt stehen; ohne eine gibt es die Störungsseite.
        if (!forecast || fresh) showOnly('trouble');
      });
    loadRadar(run);
  }

  function fact(iconName, label, value) {
    const box = document.createElement('div');
    box.append(icon(iconName), node('dt', '', label), node('dd', '', value));
    return box;
  }

  function render() {
    const now = forecast.current || {};
    const day = now.is_day !== 0;
    const state = condition(now.weather_code, day);
    setSky(state.sky, !day);

    $('now-icon').firstElementChild.setAttribute('href', '#i-' + state.icon);
    $('now-temp').textContent = degrees(now.temperature_2m);
    $('now-text').textContent = state.text;
    const daily = forecast.daily || {};
    const high = (daily.temperature_2m_max || [])[0];
    const low = (daily.temperature_2m_min || [])[0];
    $('now-range').textContent =
      text('wx_feels', { t: degrees(now.apparent_temperature) }) + (finite(high) && finite(low) ? '  ·  ' + degrees(low) + ' / ' + degrees(high) : '');

    const facts = $('facts');
    facts.textContent = '';
    const wind = number(now.wind_speed_10m) + ' km/h ' + compass(now.wind_direction_10m);
    facts.append(
      fact('wind', S.wx_wind, wind),
      fact('wind', S.wx_gusts, number(now.wind_gusts_10m) + ' km/h'),
      fact('drop', S.wx_humidity, number(now.relative_humidity_2m) + ' %'),
      fact('cloud', S.wx_pressure, number(now.pressure_msl) + ' hPa')
    );
    const rise = clock((daily.sunrise || [])[0]);
    const set = clock((daily.sunset || [])[0]);
    if (rise && set) facts.append(fact('sun', S.wx_sunrise, rise), fact('moon', S.wx_sunset, set));

    renderSoon();
    renderHours();
    renderDays();
    renderSources();
    $('updated').textContent = text('wx_updated', { time: clock(now.time) });
  }

  // Die nächsten zwei Stunden in Viertelstunden.
  function renderSoon() {
    const box = $('soon');
    box.querySelector('h2').textContent = S.wx_soon_title;
    const times = ((forecast.minutely_15 || {}).time || []).slice(0, 8);
    const amounts = ((forecast.minutely_15 || {}).precipitation || []).slice(0, 8).map((value) => (finite(value) ? value : 0));
    const bars = $('soon-bars');
    bars.textContent = '';
    const top = Math.max(0.6, ...amounts);
    for (const amount of amounts) {
      const bar = document.createElement('i');
      if (amount >= 0.05) bar.className = 'wet';
      bar.style.height = Math.max(4, Math.round((amount / top) * 100)) + '%';
      bar.title = number(amount, 1) + ' mm';
      bars.appendChild(bar);
    }
    const axis = $('soon-axis');
    axis.textContent = '';
    for (const index of [0, 2, 4, 6]) if (times[index]) axis.appendChild(node('span', '', clock(times[index])));
    if (times.length) axis.appendChild(node('span', '', '+2 h'));

    const wet = amounts.map((amount) => amount >= 0.05);
    let sentence = S.wx_soon_dry;
    if (wet.length && wet.every(Boolean)) sentence = S.wx_soon_all;
    else if (wet[0]) sentence = text('wx_soon_until', { time: clock(times[wet.indexOf(false)]) });
    else if (wet.includes(true)) sentence = text('wx_soon_from', { time: clock(times[wet.indexOf(true)]) });
    $('soon-text').textContent = sentence;
  }

  function renderHours() {
    $('hours').querySelector('h2').textContent = S.wx_hours_title;
    const hourly = forecast.hourly || {};
    const list = $('hours-list');
    list.textContent = '';
    const count = Math.min(24, (hourly.time || []).length);
    for (let index = 0; index < count; index++) {
      const state = condition(hourly.weather_code[index], hourly.is_day[index] !== 0);
      const item = node('div', index === 0 ? 'hour now' : 'hour');
      item.append(node('time', '', index === 0 ? S.wx_now : clock(hourly.time[index])), icon(state.icon), node('b', '', degrees(hourly.temperature_2m[index])));
      const chance = hourly.precipitation_probability ? hourly.precipitation_probability[index] : null;
      item.append(node('em', '', finite(chance) && chance >= 20 ? number(chance) + ' %' : ''));
      item.title = state.text;
      list.appendChild(item);
    }
  }

  function renderDays() {
    $('days').querySelector('h2').textContent = S.wx_days_title;
    const daily = forecast.daily || {};
    const list = $('days-list');
    list.textContent = '';
    const lows = (daily.temperature_2m_min || []).filter(finite);
    const highs = (daily.temperature_2m_max || []).filter(finite);
    if (!lows.length || !highs.length) return;
    const bottom = Math.min(...lows);
    const range = Math.max(1, Math.max(...highs) - bottom);
    const count = Math.min(7, (daily.time || []).length);
    for (let index = 0; index < count; index++) {
      const parts = String(daily.time[index]).split('-').map(Number);
      const date = new Date(parts[0], (parts[1] || 1) - 1, parts[2] || 1);
      const name = index === 0 ? S.wx_today : date.toLocaleDateString(LANG, { weekday: 'long' });
      const state = condition(daily.weather_code[index], true);
      const low = daily.temperature_2m_min[index];
      const high = daily.temperature_2m_max[index];
      const row = node('div', 'day');
      const chance = daily.precipitation_probability_max ? daily.precipitation_probability_max[index] : null;
      const span = node('span', 'span');
      const fill = document.createElement('i');
      fill.style.left = Math.round(((low - bottom) / range) * 100) + '%';
      fill.style.right = Math.round(100 - ((high - bottom) / range) * 100) + '%';
      span.appendChild(fill);
      row.append(
        node('span', '', name),
        icon(state.icon),
        node('span', 'wet', finite(chance) && chance >= 20 ? number(chance) + ' %' : ''),
        span,
        node('span', 'low', degrees(low)),
        node('span', 'high', degrees(high))
      );
      row.title = state.text;
      list.appendChild(row);
    }
  }

  function renderSources() {
    const box = $('sources');
    box.textContent = '';
    const link = (label, address) => {
      const anchor = node('a', '', label);
      anchor.href = address;
      // In einem neuen Tab und ohne Verbindung zurück: Die App bleibt, wo sie ist.
      anchor.target = '_blank';
      anchor.rel = 'noopener noreferrer';
      return anchor;
    };
    box.append(
      S.wx_src_weather + ' ',
      link('Open-Meteo.com', 'https://open-meteo.com/'),
      ' (CC BY 4.0)  ·  ' + S.wx_src_radar + ' ',
      link('Bright Sky', 'https://brightsky.dev/'),
      '  ·  ' + S.wx_src_satellite + ' ',
      link('NASA GIBS', 'https://www.earthdata.nasa.gov/engage/open-data-services-software/earthdata-developer-portal/gibs-api'),
      '  ·  ' + S.wx_src_places + '  ·  ' + S.wx_src_postal
    );
  }

  // ------------------------------------------------------------------------
  // Regenradar
  // ------------------------------------------------------------------------
  //
  // Bright Sky liefert zu einem Ort einen quadratischen Ausschnitt des
  // deutschen Radars: ein Gitter mit einem Kilometer je Zelle, alle fünf
  // Minuten ein Bild – die letzte Stunde gemessen, bis zu zwei Stunden
  // vorhergesagt. Gezeichnet wird es hier selbst; Kartenkacheln eines fremden
  // Dienstes braucht es dafür nicht.

  const canvas = $('radar-canvas');
  const radar = {
    radius: 100,
    frames: [],
    size: 0,
    position: null, // Spalte und Zeile des Ortes im Gitter
    origin: null, // derselbe Ort in Kilometern der Abbildung
    index: 0,
    playing: true,
    timer: 0,
    run: 0,
    at: 0,
    backdrop: null, // das Satellitenbild, in das Gitter des Radars gelegt
    backdropKey: '',
    view: '', // Ort und Ausschnitt, zu denen die Bilder gehören
  };

  // Die Abbildung des Radars: polar-stereografisch auf dem WGS84-Ellipsoid,
  // längentreu bei 60° Nord, Mittelmeridian 10° Ost. Ergebnis in Kilometern.
  function project(lat, lon) {
    const a = 6378.137;
    const b = 6356.7523142451802;
    const e = Math.sqrt(1 - (b * b) / (a * a));
    const t = (phi) => Math.tan(Math.PI / 4 - phi / 2) / Math.pow((1 - e * Math.sin(phi)) / (1 + e * Math.sin(phi)), e / 2);
    const phi = (lat * Math.PI) / 180;
    const lam = ((lon - 10) * Math.PI) / 180;
    const phic = Math.PI / 3;
    const mc = Math.cos(phic) / Math.sqrt(1 - e * e * Math.sin(phic) * Math.sin(phic));
    const rho = (a * mc * t(phi)) / t(phic);
    return [rho * Math.sin(lam), -rho * Math.cos(lam)];
  }

  // Der Rückweg: aus Kilometern der Abbildung wieder Breite und Länge.
  function unproject(x, y) {
    const a = 6378.137;
    const b = 6356.7523142451802;
    const e = Math.sqrt(1 - (b * b) / (a * a));
    const t = (phi) => Math.tan(Math.PI / 4 - phi / 2) / Math.pow((1 - e * Math.sin(phi)) / (1 + e * Math.sin(phi)), e / 2);
    const phic = Math.PI / 3;
    const mc = Math.cos(phic) / Math.sqrt(1 - e * e * Math.sin(phic) * Math.sin(phic));
    const small = (Math.hypot(x, y) * t(phic)) / (a * mc);
    let phi = Math.PI / 2 - 2 * Math.atan(small);
    for (let round = 0; round < 6; round++) {
      phi = Math.PI / 2 - 2 * Math.atan(small * Math.pow((1 - e * Math.sin(phi)) / (1 + e * Math.sin(phi)), e / 2));
    }
    return [(phi * 180) / Math.PI, 10 + (Math.atan2(x, -y) * 180) / Math.PI];
  }

  // Breite und Länge einer Stelle im Gitter (Spalte, Zeile).
  function placeOf(column, row) {
    return unproject(radar.origin[0] + (column - radar.position.x), radar.origin[1] - (row - radar.position.y));
  }

  // Wo ein Ort im Gitter liegt (Spalte, Zeile; die Zeilen zählen nach Süden).
  function cellOf(lat, lon) {
    const [x, y] = project(lat, lon);
    return [radar.position.x + (x - radar.origin[0]), radar.position.y - (y - radar.origin[1])];
  }

  // Farben nach Stärke (Millimeter je Stunde). Die Werte kommen in
  // Hundertstel Millimetern je fünf Minuten.
  const STOPS = [
    [0.1, 150, 214, 255, 120],
    [0.5, 96, 178, 255, 175],
    [1, 52, 132, 246, 200],
    [2, 38, 196, 128, 210],
    [5, 238, 222, 66, 220],
    [10, 250, 152, 44, 230],
    [25, 236, 62, 52, 235],
    [50, 204, 44, 172, 240],
    [100, 255, 255, 255, 245],
  ];
  const COLORS = new Uint8ClampedArray(1024 * 4);
  for (let value = 1; value < 1024; value++) {
    const rate = value * 0.12;
    if (rate < STOPS[0][0]) continue;
    let upper = STOPS.findIndex((stop) => stop[0] > rate);
    if (upper < 0) upper = STOPS.length - 1;
    const lower = Math.max(0, upper - 1);
    const from = STOPS[lower];
    const to = STOPS[upper];
    const share = from === to ? 0 : Math.min(1, Math.max(0, (Math.log(rate) - Math.log(from[0])) / (Math.log(to[0]) - Math.log(from[0]))));
    for (let channel = 0; channel < 4; channel++) {
      COLORS[value * 4 + channel] = Math.round(from[channel + 1] + (to[channel + 1] - from[channel + 1]) * share);
    }
  }

  function radarNote(message) {
    $('radar-note').textContent = message;
    $('radar-note').hidden = !message;
    $('radar-controls').hidden = !!message;
    $('radar-foot').hidden = !!message;
    if (message) {
      const context = canvas.getContext('2d');
      context.clearRect(0, 0, canvas.width, canvas.height);
    }
  }

  function loadRadar(run) {
    $('radar').querySelector('h2').textContent = S.wx_radar_title;
    // Dasselbe Bild wie eben (gleicher Ort, gleicher Ausschnitt)? Dann läuft es
    // weiter, bis das frische da ist. Sonst gehört das alte nicht hierher.
    const view = [place.lat, place.lon, radar.radius].join('/');
    if (radar.view !== view) {
      radar.view = view;
      radar.frames = [];
      clearTimeout(radar.timer);
    }
    const inside = place.lat > RADAR_AREA.south && place.lat < RADAR_AREA.north && place.lon > RADAR_AREA.west && place.lon < RADAR_AREA.east;
    if (!inside) {
      radar.frames = [];
      clearTimeout(radar.timer);
      radarNote(S.wx_radar_none);
      return;
    }
    if (!radar.frames.length) radarNote(S.wx_radar_loading);
    const radarRun = ++radar.run;
    const from = new Date(Date.now() - 60 * 60000).toISOString();
    const until = new Date(Date.now() + 120 * 60000).toISOString();
    const url =
      RADAR + '?format=plain&lat=' + place.lat + '&lon=' + place.lon + '&distance=' + radar.radius * 1000 + '&date=' + encodeURIComponent(from) + '&last_date=' + encodeURIComponent(until);
    fetch(url)
      .then((response) => {
        if (response.status === 404) return Promise.reject(new Error('outside'));
        return response.ok ? jsonOf(response, LARGE) : Promise.reject(new Error('radar'));
      })
      .then((data) => {
        if (run !== loading || radarRun !== radar.run) return;
        // Wer angehalten und eine Zeit gewählt hat, bleibt bei ihr.
        const chosen = !radar.playing && radar.frames[radar.index] ? radar.frames[radar.index].time : NaN;
        const frames = readRadar(data);
        if (!frames) throw new Error('radar');
        radar.frames = frames;
        radar.at = Date.now();
        // Sonst: anfangen beim jüngsten gemessenen Bild.
        const measured = frames.map((frame, index) => (frame.forecast ? -1 : index)).filter((index) => index >= 0);
        const again = frames.findIndex((frame) => frame.time === chosen);
        radar.index = again >= 0 ? again : measured.length ? measured[measured.length - 1] : 0;
        $('radar-time').max = String(frames.length - 1);
        radarNote('');
        renderZoom();
        renderLegend();
        resizeRadar();
        play(radar.playing);
        loadBackdrop();
      })
      .catch((error) => {
        if (run !== loading || radarRun !== radar.run) return;
        const outside = error && error.message === 'outside';
        // Ein Auffrischen, das misslingt, nimmt das Bild von eben nicht weg –
        // solange es noch etwas über das Wetter sagt.
        if (!outside && radar.frames.length && Date.now() - radar.at < 30 * 60000) return;
        radar.frames = [];
        clearTimeout(radar.timer);
        radarNote(outside ? S.wx_radar_none : S.wx_radar_failed);
      });
  }

  // Prüft die Antwort und macht aus jedem Bild ein Feld kleiner Zahlen.
  function readRadar(data) {
    const list = data && Array.isArray(data.radar) ? data.radar : null;
    const position = data && data.latlon_position;
    if (!list || !list.length || !position || !finite(position.x) || !finite(position.y)) return null;
    const first = list[0].precipitation_5;
    const size = Array.isArray(first) ? first.length : 0;
    if (size < 20 || size > 420) return null;
    const frames = [];
    for (const entry of list.slice(0, 48)) {
      const rows = entry.precipitation_5;
      const time = Date.parse(entry.timestamp);
      if (!Array.isArray(rows) || rows.length !== size || !Number.isFinite(time)) return null;
      const grid = new Uint16Array(size * size);
      for (let row = 0; row < size; row++) {
        const cells = rows[row];
        if (!Array.isArray(cells) || cells.length !== size) return null;
        for (let column = 0; column < size; column++) {
          const value = cells[column];
          grid[row * size + column] = finite(value) && value > 0 ? Math.min(1023, Math.round(value)) : 0;
        }
      }
      // Die Quelle nennt den Lauf, aus dem das Bild stammt: Liegt es danach,
      // ist es eine Vorhersage.
      const runTime = Date.parse(String(entry.source || '').split('::').pop());
      frames.push({ time, grid, forecast: Number.isFinite(runTime) ? time > runTime : time > Date.now(), image: null });
    }
    radar.size = size;
    radar.position = { x: position.x, y: position.y };
    radar.origin = project(place.lat, place.lon);
    return frames;
  }

  // Das Satellitenbild: ein Bild in Breite und Länge für den Ausschnitt, hier
  // Punkt für Punkt in das Gitter des Radars gelegt (das steht leicht schräg
  // und ist nach Norden hin gestaucht). Fehlt es, bleibt der Hintergrund dunkel.
  function loadBackdrop() {
    const size = radar.size;
    const key = [place.lat, place.lon, size].join('/');
    if (radar.backdropKey === key) return;
    radar.backdropKey = key;
    radar.backdrop = null;
    // Der Ausschnitt in Breite und Länge: die Ränder des Gitters, mit etwas Luft.
    let south = 90;
    let north = -90;
    let west = 180;
    let east = -180;
    for (let step = 0; step <= 8; step++) {
      const along = (size * step) / 8;
      for (const [column, row] of [[along, 0], [along, size], [0, along], [size, along]]) {
        const [lat, lon] = placeOf(column, row);
        south = Math.min(south, lat);
        north = Math.max(north, lat);
        west = Math.min(west, lon);
        east = Math.max(east, lon);
      }
    }
    if (!(north > south && east > west)) return;
    const pad = 0.02;
    south -= pad;
    north += pad;
    west -= pad;
    east += pad;
    const width = 1024;
    const height = Math.max(256, Math.min(1024, Math.round((width * (north - south)) / ((east - west) * Math.cos((place.lat * Math.PI) / 180)))));
    const box = [south, west, north, east].map((value) => value.toFixed(4)).join(',');
    const image = new Image();
    image.crossOrigin = 'anonymous';
    image.onload = () => {
      if (radar.backdropKey !== key || !radar.frames.length) return;
      try {
        // Bestellt war ein Bild dieser Größe – ein anderes wird nicht ausgepackt.
        if (image.naturalWidth !== width || image.naturalHeight !== height) throw new Error('size');
        radar.backdrop = warp(image, { south, north, west, east });
        drawRadar();
      } catch (error) {
        radar.backdrop = null;
      }
    };
    // Kam es nicht an, versucht es das nächste Auffrischen des Radars noch einmal.
    image.onerror = () => {
      if (radar.backdropKey === key) radar.backdropKey = '';
    };
    image.src =
      SATELLITE + '?SERVICE=WMS&REQUEST=GetMap&VERSION=1.3.0&LAYERS=BlueMarble_ShadedRelief_Bathymetry&STYLES=&FORMAT=image/jpeg&CRS=EPSG:4326&BBOX=' + box + '&WIDTH=' + width + '&HEIGHT=' + height;
  }

  function warp(image, area) {
    const size = radar.size;
    const side = 720;
    const source = document.createElement('canvas');
    source.width = image.naturalWidth;
    source.height = image.naturalHeight;
    const sourceContext = source.getContext('2d', { willReadFrequently: true });
    sourceContext.drawImage(image, 0, 0);
    const from = sourceContext.getImageData(0, 0, source.width, source.height).data;
    const target = document.createElement('canvas');
    target.width = side;
    target.height = side;
    const targetContext = target.getContext('2d');
    const picture = targetContext.createImageData(side, side);
    const to = picture.data;
    // Breite und Länge nur an einem groben Raster ausrechnen, dazwischen mitteln.
    const mesh = 24;
    const knots = side / mesh;
    const lats = new Float64Array((mesh + 1) * (mesh + 1));
    const lons = new Float64Array((mesh + 1) * (mesh + 1));
    for (let j = 0; j <= mesh; j++) {
      for (let i = 0; i <= mesh; i++) {
        const [lat, lon] = placeOf((i * knots * size) / side - 0.5, (j * knots * size) / side - 0.5);
        lats[j * (mesh + 1) + i] = lat;
        lons[j * (mesh + 1) + i] = lon;
      }
    }
    const scaleX = source.width / (area.east - area.west);
    const scaleY = source.height / (area.north - area.south);
    for (let y = 0; y < side; y++) {
      const j = Math.min(mesh - 1, Math.floor(y / knots));
      const v = y / knots - j;
      for (let x = 0; x < side; x++) {
        const i = Math.min(mesh - 1, Math.floor(x / knots));
        const u = x / knots - i;
        const a = j * (mesh + 1) + i;
        const b = a + mesh + 1;
        const lat = (lats[a] * (1 - u) + lats[a + 1] * u) * (1 - v) + (lats[b] * (1 - u) + lats[b + 1] * u) * v;
        const lon = (lons[a] * (1 - u) + lons[a + 1] * u) * (1 - v) + (lons[b] * (1 - u) + lons[b + 1] * u) * v;
        const sx = Math.min(source.width - 1, Math.max(0, Math.round((lon - area.west) * scaleX)));
        const sy = Math.min(source.height - 1, Math.max(0, Math.round((area.north - lat) * scaleY)));
        const fromIndex = (sy * source.width + sx) * 4;
        const toIndex = (y * side + x) * 4;
        to[toIndex] = from[fromIndex];
        to[toIndex + 1] = from[fromIndex + 1];
        to[toIndex + 2] = from[fromIndex + 2];
        to[toIndex + 3] = 255;
      }
    }
    targetContext.putImageData(picture, 0, 0);
    return target;
  }

  const stage = document.createElement('canvas');

  function imageOf(frame) {
    if (frame.image) return frame.image;
    const size = radar.size;
    const image = new ImageData(size, size);
    const pixels = image.data;
    for (let cell = 0; cell < frame.grid.length; cell++) {
      const value = frame.grid[cell];
      if (!value) continue;
      const from = value * 4;
      const to = cell * 4;
      pixels[to] = COLORS[from];
      pixels[to + 1] = COLORS[from + 1];
      pixels[to + 2] = COLORS[from + 2];
      pixels[to + 3] = COLORS[from + 3];
    }
    frame.image = image;
    return image;
  }

  function resizeRadar() {
    const side = Math.round(canvas.clientWidth * (window.devicePixelRatio || 1));
    if (side > 0 && (canvas.width !== side || canvas.height !== side)) {
      canvas.width = side;
      canvas.height = side;
    }
    drawRadar();
  }

  function drawRadar() {
    const frame = radar.frames[radar.index];
    if (!frame || !canvas.width || !place) return;
    const context = canvas.getContext('2d');
    const side = canvas.width;
    const size = radar.size;
    const scale = side / size;
    const ratio = window.devicePixelRatio || 1;
    context.setTransform(1, 0, 0, 1, 0, 0);
    context.fillStyle = '#0a0f1c';
    context.fillRect(0, 0, side, side);
    // Das Satellitenbild – abgedunkelt, damit Niederschlag und Schrift davor stehen.
    if (radar.backdrop) {
      context.imageSmoothingEnabled = true;
      context.imageSmoothingQuality = 'high';
      context.drawImage(radar.backdrop, 0, 0, side, side);
      context.fillStyle = 'rgba(6, 10, 20, 0.42)';
      context.fillRect(0, 0, side, side);
    }

    // Entfernungsringe um den Ort.
    const centerX = (radar.position.x + 0.5) * scale;
    const centerY = (radar.position.y + 0.5) * scale;
    context.lineWidth = ratio;
    context.font = 11 * ratio + 'px -apple-system, sans-serif';
    context.textAlign = 'left';
    context.textBaseline = 'middle';
    for (const ring of [25, 50, 100]) {
      if (ring > radar.radius) continue;
      context.strokeStyle = 'rgba(255, 255, 255, 0.22)';
      context.beginPath();
      context.arc(centerX, centerY, ring * scale, 0, Math.PI * 2);
      context.stroke();
      context.fillStyle = 'rgba(255, 255, 255, 0.6)';
      context.fillText(ring + ' km', centerX + ring * scale * 0.707 + 4 * ratio, centerY - ring * scale * 0.707);
    }

    // Grenzen und Küsten, wenn die Karte welche mitbringt.
    context.strokeStyle = 'rgba(255, 255, 255, 0.28)';
    context.lineWidth = ratio;
    for (const line of MAP.lines) {
      context.beginPath();
      let drawing = false;
      for (let point = 0; point + 1 < line.length; point += 2) {
        const [column, row] = cellOf(line[point + 1], line[point]);
        const visible = column > -40 && row > -40 && column < size + 40 && row < size + 40;
        if (!visible) {
          drawing = false;
          continue;
        }
        if (drawing) context.lineTo((column + 0.5) * scale, (row + 0.5) * scale);
        else context.moveTo((column + 0.5) * scale, (row + 0.5) * scale);
        drawing = true;
      }
      context.stroke();
    }

    // Der Niederschlag – weich vergrößert.
    stage.width = size;
    stage.height = size;
    stage.getContext('2d').putImageData(imageOf(frame), 0, 0);
    context.imageSmoothingEnabled = true;
    context.imageSmoothingQuality = 'high';
    context.drawImage(stage, 0, 0, size, size, 0, 0, side, side);

    // Städte zur Orientierung.
    const rank = radar.radius >= 150 ? 2 : 3;
    context.font = '500 ' + 12 * ratio + 'px -apple-system, sans-serif';
    context.textBaseline = 'middle';
    for (const [name, lat, lon, level] of MAP.places) {
      if (level > rank) continue;
      const [column, row] = cellOf(lat, lon);
      if (column < 4 || row < 4 || column > size - 4 || row > size - 4) continue;
      const x = (column + 0.5) * scale;
      const y = (row + 0.5) * scale;
      // Am Ort selbst steht schon die Markierung.
      if (Math.hypot(x - centerX, y - centerY) < 14 * ratio) continue;
      context.fillStyle = 'rgba(255, 255, 255, 0.9)';
      context.beginPath();
      context.arc(x, y, 2.2 * ratio, 0, Math.PI * 2);
      context.fill();
      const right = x < side - 90 * ratio;
      context.textAlign = right ? 'left' : 'right';
      context.lineWidth = 3 * ratio;
      context.strokeStyle = 'rgba(8, 12, 22, 0.75)';
      context.strokeText(name, x + (right ? 6 : -6) * ratio, y);
      context.fillText(name, x + (right ? 6 : -6) * ratio, y);
    }

    // Der gewählte Ort.
    const pulse = (Date.now() % 2000) / 2000;
    context.strokeStyle = 'rgba(255, 255, 255, ' + (0.55 * (1 - pulse)).toFixed(3) + ')';
    context.lineWidth = 2 * ratio;
    context.beginPath();
    context.arc(centerX, centerY, (6 + pulse * 14) * ratio, 0, Math.PI * 2);
    context.stroke();
    context.fillStyle = '#ffffff';
    context.beginPath();
    context.arc(centerX, centerY, 5 * ratio, 0, Math.PI * 2);
    context.fill();
    context.fillStyle = '#2f8cff';
    context.beginPath();
    context.arc(centerX, centerY, 3.2 * ratio, 0, Math.PI * 2);
    context.fill();

    // Wo Norden ist: Das Gitter des Radars steht nur am 10. Längengrad genau
    // aufrecht, sonst ein paar Grad schräg.
    const north = cellOf(place.lat + 0.2, place.lon);
    const angle = Math.atan2(north[0] - radar.position.x, radar.position.y - north[1]);
    context.save();
    context.translate(side - 26 * ratio, 28 * ratio);
    context.rotate(angle);
    context.fillStyle = 'rgba(255, 255, 255, 0.85)';
    context.beginPath();
    context.moveTo(0, -11 * ratio);
    context.lineTo(5 * ratio, 6 * ratio);
    context.lineTo(0, 2.5 * ratio);
    context.lineTo(-5 * ratio, 6 * ratio);
    context.closePath();
    context.fill();
    context.restore();
    context.textAlign = 'center';
    context.font = '600 ' + 10 * ratio + 'px -apple-system, sans-serif';
    context.fillStyle = 'rgba(255, 255, 255, 0.7)';
    context.fillText('N', side - 26 * ratio, 46 * ratio);

    labelRadar(frame);
  }

  function labelRadar(frame) {
    // Den Schieber nur anfassen, wenn er woanders steht – sonst risse ihn das
    // regelmäßige Neuzeichnen dem aus der Hand, der ihn gerade zieht.
    const slider = $('radar-time');
    if (slider.value !== String(radar.index)) slider.value = String(radar.index);
    const label = $('radar-label');
    label.textContent = '';
    const offset = forecast && finite(forecast.utc_offset_seconds) ? forecast.utc_offset_seconds : 0;
    const local = new Date(frame.time + offset * 1000);
    const minutes = Math.round((frame.time - Date.now()) / 300000) * 5;
    const when = minutes === 0 ? S.wx_now : minutes < 0 ? text('wx_minutes_ago', { n: -minutes }) : text('wx_minutes_ahead', { n: minutes });
    label.append(two(local.getUTCHours()) + ':' + two(local.getUTCMinutes()) + '  ·  ' + when);
    label.append(node('small', '', frame.forecast ? S.wx_forecast : S.wx_measured));
  }

  function play(on) {
    radar.playing = on;
    clearTimeout(radar.timer);
    const button = $('radar-play');
    button.firstElementChild.firstElementChild.setAttribute('href', on ? '#i-pause' : '#i-play');
    button.title = on ? S.wx_pause : S.wx_play;
    button.setAttribute('aria-label', button.title);
    drawRadar();
    if (!on || !radar.frames.length) return;
    const step = () => {
      if (!radar.playing || !radar.frames.length) return;
      if (!document.hidden) {
        radar.index = (radar.index + 1) % radar.frames.length;
        drawRadar();
      }
      // Am Ende kurz stehen bleiben, dann von vorn.
      radar.timer = setTimeout(step, radar.index === radar.frames.length - 1 ? 1500 : 220);
    };
    radar.timer = setTimeout(step, 600);
  }

  $('radar-play').addEventListener('click', () => play(!radar.playing));
  // Am Schieber eine Zeit wählen – durch Klick auf die Leiste oder durch
  // Ziehen. Erst die gewählte Stelle übernehmen, dann anhalten: Das Anhalten
  // zeichnet neu und stellt den Schieber dabei auf das gezeigte Bild.
  $('radar-time').addEventListener('input', () => {
    radar.index = Math.min(radar.frames.length - 1, Math.max(0, Math.round(Number($('radar-time').value) || 0)));
    play(false);
  });
  window.addEventListener('resize', resizeRadar);

  function renderZoom() {
    const box = $('radar-zoom');
    box.textContent = '';
    for (const radius of RADII) {
      const button = node('button', radius === radar.radius ? 'on' : '', radius + ' km');
      button.type = 'button';
      button.addEventListener('click', () => {
        if (radius === radar.radius) return;
        radar.radius = radius;
        loadRadar(loading);
      });
      box.appendChild(button);
    }
  }

  function renderLegend() {
    const box = $('radar-legend');
    box.textContent = '';
    const bar = document.createElement('i');
    const colors = STOPS.map((stop) => 'rgb(' + stop[1] + ',' + stop[2] + ',' + stop[3] + ')');
    bar.style.background = 'linear-gradient(90deg, ' + colors.join(', ') + ')';
    box.append(node('span', '', S.wx_light), bar, node('span', '', S.wx_heavy));
  }

  // ------------------------------------------------------------------------
  // Der Himmel dahinter
  // ------------------------------------------------------------------------

  const sky = $('sky');
  const still = window.matchMedia('(prefers-reduced-motion: reduce)');
  let weather = { kind: 'clear', night: true };
  let drops = [];
  let skyFrame = 0;

  function setSky(kind, night) {
    weather = { kind, night };
    const name = kind === 'clear' ? (night ? 'sky-night' : 'sky-clear-day') : 'sky-' + kind + (night ? '-night' : '-day');
    document.body.className = name;
    seedSky();
  }

  function seedSky() {
    const width = sky.clientWidth || window.innerWidth;
    const height = sky.clientHeight || window.innerHeight;
    const amount = { rain: 150, thunder: 190, snow: 110, clear: weather.night ? 90 : 0, cloudy: 0, fog: 0 }[weather.kind] || 0;
    drops = [];
    for (let index = 0; index < amount; index++) {
      drops.push({ x: Math.random() * width, y: Math.random() * height, z: 0.4 + Math.random() * 0.6, seed: Math.random() * 1000 });
    }
  }

  function paintSky(now) {
    skyFrame = requestAnimationFrame(paintSky);
    if (document.hidden) return;
    const ratio = Math.min(2, window.devicePixelRatio || 1);
    const width = sky.clientWidth;
    const height = sky.clientHeight;
    if (sky.width !== Math.round(width * ratio) || sky.height !== Math.round(height * ratio)) {
      sky.width = Math.round(width * ratio);
      sky.height = Math.round(height * ratio);
    }
    const context = sky.getContext('2d');
    context.setTransform(ratio, 0, 0, ratio, 0, 0);
    context.clearRect(0, 0, width, height);
    const time = now / 1000;
    const kind = weather.kind;

    if (kind === 'clear' && !weather.night) {
      // Die Sonne: ein weiches Leuchten oben rechts.
      const glow = context.createRadialGradient(width * 0.82, height * 0.12, 0, width * 0.82, height * 0.12, Math.max(width, height) * 0.6);
      glow.addColorStop(0, 'rgba(255, 244, 200, 0.75)');
      glow.addColorStop(0.18, 'rgba(255, 236, 170, 0.28)');
      glow.addColorStop(1, 'rgba(255, 236, 170, 0)');
      context.fillStyle = glow;
      context.fillRect(0, 0, width, height);
    }

    if (kind === 'clear' && weather.night) {
      for (const star of drops) {
        const light = 0.35 + 0.65 * Math.abs(Math.sin(time * 0.6 * star.z + star.seed));
        context.fillStyle = 'rgba(255, 255, 255, ' + (light * 0.8).toFixed(3) + ')';
        context.fillRect(star.x, star.y * 0.75, 1.4 * star.z, 1.4 * star.z);
      }
    }

    if (kind === 'cloudy' || kind === 'fog' || kind === 'rain' || kind === 'thunder' || kind === 'snow') {
      // Wolken: weiche Flecken, die langsam ziehen.
      const shade = weather.night ? '170, 185, 215' : '255, 255, 255';
      const strength = kind === 'fog' ? 0.2 : kind === 'cloudy' ? 0.16 : 0.1;
      for (let index = 0; index < 6; index++) {
        const span = width + 600;
        const x = ((index * 310 + time * (6 + index * 1.7)) % span) - 300;
        const y = height * (0.08 + 0.11 * index) + Math.sin(time * 0.1 + index) * 12;
        const radius = 220 + (index % 3) * 70;
        const cloud = context.createRadialGradient(x, y, 0, x, y, radius);
        cloud.addColorStop(0, 'rgba(' + shade + ', ' + strength + ')');
        cloud.addColorStop(1, 'rgba(' + shade + ', 0)');
        context.fillStyle = cloud;
        context.fillRect(x - radius, y - radius, radius * 2, radius * 2);
      }
    }

    if (kind === 'rain' || kind === 'thunder') {
      context.strokeStyle = 'rgba(200, 225, 255, 0.38)';
      context.lineWidth = 1.1;
      context.beginPath();
      for (const drop of drops) {
        drop.y += 9 * drop.z;
        drop.x -= 1.6 * drop.z;
        if (drop.y > height) {
          drop.y = -20;
          drop.x = Math.random() * (width + 80);
        }
        context.moveTo(drop.x, drop.y);
        context.lineTo(drop.x - 2.4 * drop.z, drop.y + 14 * drop.z);
      }
      context.stroke();
      if (kind === 'thunder' && Math.sin(time * 0.9) > 0.995) {
        context.fillStyle = 'rgba(255, 255, 255, 0.16)';
        context.fillRect(0, 0, width, height);
      }
    }

    if (kind === 'snow') {
      context.fillStyle = 'rgba(255, 255, 255, 0.8)';
      for (const flake of drops) {
        flake.y += 0.9 * flake.z;
        flake.x += Math.sin(time * 0.8 + flake.seed) * 0.5;
        if (flake.y > height) {
          flake.y = -6;
          flake.x = Math.random() * width;
        }
        context.beginPath();
        context.arc(flake.x, flake.y, 1.7 * flake.z, 0, Math.PI * 2);
        context.fill();
      }
    }
  }

  function runSky() {
    cancelAnimationFrame(skyFrame);
    if (still.matches) {
      const context = sky.getContext('2d');
      context.clearRect(0, 0, sky.width, sky.height);
      return;
    }
    skyFrame = requestAnimationFrame(paintSky);
  }

  still.addEventListener('change', runSky);
  window.addEventListener('resize', seedSky);

  // ------------------------------------------------------------------------
  // Auf dem Laufenden bleiben
  // ------------------------------------------------------------------------

  // Die Markierung des Ortes pulsiert auch, wenn das Radar steht.
  setInterval(() => {
    if (!document.hidden && !$('content').hidden && radar.frames.length && !radar.playing) drawRadar();
  }, 120);

  // Die Vorhersage alle zehn Minuten, das Radar alle fünf – aber nur, solange
  // jemand hinsieht.
  function refreshIfStale() {
    if (document.hidden || !place) return;
    if (Date.now() - loadedAt > 10 * 60000) loadAll(false);
    else if (radar.frames.length && Date.now() - radar.at > 5 * 60000) loadRadar(loading);
  }

  setInterval(refreshIfStale, 30000);
  document.addEventListener('visibilitychange', refreshIfStale);

  // ------------------------------------------------------------------------
  // Los
  // ------------------------------------------------------------------------

  runSky();
  Z.load().then((note) => {
    kept = note.kept;
    saved = (Array.isArray(note.value.places) ? note.value.places : []).map(cleanPlace).filter(Boolean).slice(0, 8);
    const last = cleanPlace(note.value.last);
    if (last && last.name) {
      place = last;
      loadAll(true);
      // Der eigene Standort wird im Hintergrund aufgefrischt – ohne Rückfrage
      // geht das nur, solange die Erlaubnis noch gilt.
      if (last.mine) {
        Z.location(() => {}).then((answer) => {
          if (answer.state !== 'ok' || !place || !place.mine) return;
          if (answer.lat === place.lat && answer.lon === place.lon) return;
          const near = nearestName(answer.lat, answer.lon);
          place = cleanPlace({ name: S.wx_mine_short, where: near ? text('wx_near', { place: near }) : '', lat: answer.lat, lon: answer.lon, mine: true });
          remember();
          loadAll(false);
        });
      }
    } else {
      showOnly('intro');
    }
  });
})();
