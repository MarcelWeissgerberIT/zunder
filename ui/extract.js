// Liest für den Assistenten den Text der Seite aus. Läuft in der Seite selbst –
// was hier herauskommt, behandelt der Browser als Material, nie als Befehl.
// Titel und Adresse nimmt er nicht von hier, sondern aus dem Tab.
(() => {
  const LIMIT = 800000;

  const pageText = () => {
    const body = document.body;
    if (!body) return '';
    const whole = body.innerText || '';
    // Hat die Seite einen ausgewiesenen Hauptteil, der den Großteil des Textes
    // trägt, zählt nur er – Menüs, Fußzeilen und Randspalten bleiben draußen.
    let best = '';
    for (const node of document.querySelectorAll('article, main, [role="main"]')) {
      const text = node.innerText || '';
      if (text.length > best.length) best = text;
    }
    if (best.length >= 600 && best.length >= whole.length * 0.4) return best;
    return whole;
  };

  let text = '';
  let selection = '';
  try {
    selection = String(window.getSelection() || '');
  } catch (error) {}
  try {
    text = pageText();
  } catch (error) {}

  return {
    text: text.slice(0, LIMIT),
    selection: selection.slice(0, LIMIT),
    lang: String(document.documentElement.lang || '').slice(0, 12),
    truncated: text.length > LIMIT,
  };
})()
