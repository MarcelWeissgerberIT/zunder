// Sprechblase des Passwort-Tresors: Vorschläge an einem Anmeldefeld und die
// Frage, ob ein Passwort gesichert werden soll. Sie zeigt nur an, was der
// Browser ihr schickt, und meldet Klicks zurück. Passwörter bekommt sie nie zu
// sehen – außer einem neu vorgeschlagenen, das der Mensch ja lesen soll.
//
// Alles, was von Webseiten stammt (Rechnername, Benutzername), wird nur als
// Text eingesetzt.
(() => {
  'use strict';

  const DATA = JSON.parse(document.getElementById('data').textContent);
  const S = DATA.strings;
  const SVG_NS = 'http://www.w3.org/2000/svg';
  const post = (message) => window.ipc.postMessage(JSON.stringify(message));
  const card = document.getElementById('card');

  let seq = 0;
  let shownAt = 0;
  let timer = null;

  function icon(name) {
    const svg = document.createElementNS(SVG_NS, 'svg');
    svg.setAttribute('class', 'ic');
    const use = document.createElementNS(SVG_NS, 'use');
    use.setAttribute('href', '#i-' + name);
    svg.appendChild(use);
    return svg;
  }

  function el(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text != null) node.textContent = text;
    return node;
  }

  // Ein Klick zählt erst einen Augenblick nach dem Erscheinen. So lässt sich
  // niemand dazu bringen, auf eine Stelle zu klicken, an der die Blase dann
  // unter dem Mauszeiger auftaucht.
  function send(command, extra) {
    if (performance.now() - shownAt < 350) return;
    post(Object.assign({ c: command, seq }, extra || {}));
  }

  function head(iconName, title) {
    const box = el('div', 'head');
    box.appendChild(icon(iconName));
    box.appendChild(el('span', 't', title));
    const close = el('button', 'ibtn sm');
    close.type = 'button';
    close.title = S.close;
    close.appendChild(icon('close'));
    close.addEventListener('click', () => post({ c: 'pwBubbleClose', seq }));
    box.appendChild(close);
    return box;
  }

  function account(iconName, title, sub, code, onClick) {
    const button = el('button', 'acct');
    button.type = 'button';
    button.appendChild(icon(iconName));
    const text = el('span', 'txt');
    text.appendChild(el('div', 't', title));
    if (sub) text.appendChild(el('div', code ? 's code' : 's', sub));
    button.appendChild(text);
    button.addEventListener('click', onClick);
    return button;
  }

  function renderFill(message) {
    card.appendChild(head('key', message.host));
    const accounts = message.accounts || [];
    const hint = message.auth ? S.pw_fill_touch : S.pw_fill_plain;
    const suggestion = () => {
      card.appendChild(account('key', S.pw_suggest, message.suggest, true, () => send('pwUse')));
      card.appendChild(el('div', 'note', S.pw_suggest_hint));
    };
    // In einem Feld für ein neues Passwort steht der Vorschlag oben.
    if (message.suggest) {
      suggestion();
      if (accounts.length) card.appendChild(el('div', 'sep'));
    }
    for (const entry of accounts) {
      card.appendChild(
        account('user', entry.user || S.pw_no_user, hint, false, () => send('pwFill', { id: entry.id }))
      );
    }
    if (!accounts.length && !message.suggest) card.appendChild(el('div', 'note', S.pw_none_here));
    card.appendChild(el('div', 'sep'));
    const manage = el('button', 'link', S.pw_manage);
    manage.type = 'button';
    manage.addEventListener('click', () => send('pwManage'));
    card.appendChild(manage);
  }

  function renderSave(message) {
    card.appendChild(head('key', message.update ? S.pw_update_q : S.pw_save_q));
    const who = el('div', 'who');
    who.appendChild(el('div', 't', (message.user || S.pw_no_user) + ' · ' + message.host));
    who.appendChild(el('div', 's', '•'.repeat(Math.max(4, Number(message.dots) || 8))));
    card.appendChild(who);
    if (message.insecure) card.appendChild(el('div', 'note warn', S.pw_insecure_note));

    const buttons = el('div', 'btns');
    const later = el('button', 'tbtn', S.pw_not_now);
    later.type = 'button';
    later.addEventListener('click', () => send('pwDismiss'));
    const save = el('button', 'tbtn primary', message.update ? S.pw_update : S.pw_save);
    save.type = 'button';
    save.addEventListener('click', () => send('pwSave'));
    buttons.append(later, save);
    card.appendChild(buttons);

    const never = el('button', 'link', S.pw_never);
    never.type = 'button';
    never.addEventListener('click', () => send('pwNever'));
    card.appendChild(never);

    // Unbeantwortet verschwindet die Frage nach einer Weile; am Schlüssel-Knopf
    // der Adressleiste bleibt sie erreichbar.
    const mine = message.seq;
    timer = setTimeout(() => post({ c: 'pwTimeout', seq: mine }), 25000);
  }

  window.__zunderBubble = (message) => {
    clearTimeout(timer);
    card.textContent = '';
    if (message.t === 'hide') {
      card.hidden = true;
      return;
    }
    seq = message.seq;
    shownAt = performance.now();
    if (message.t === 'fill') renderFill(message);
    else if (message.t === 'save') renderSave(message);
    card.hidden = false;
    // Die Blase ist noch unsichtbar; der Browser zeigt sie, sobald er ihre Höhe kennt.
    post({ c: 'pwBubbleSize', seq, h: Math.ceil(card.getBoundingClientRect().height) + 20 });
  };

  document.body.classList.toggle('private', !!DATA.private);

  window.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') post({ c: 'pwBubbleClose', seq });
  });
  window.addEventListener('contextmenu', (event) => event.preventDefault());
  window.addEventListener('dragover', (event) => event.preventDefault());
  window.addEventListener('drop', (event) => event.preventDefault());

  post({ c: 'pwBubbleReady' });
})();
