//! Das Gedächtnis: Seiten, die der Mensch bewusst gemerkt hat, als Text auf
//! der Platte (`app_records`-Kind „memory"). Der Assistent sucht darin die zur
//! Frage passenden heraus und bekommt sie als EINE Seite zu lesen – so
//! antwortet er aus dem, was man gesehen hat. Nichts davon verlässt das Gerät,
//! außer an den KI-Anbieter, den man ohnehin gewählt hat, und nur für die
//! Frage, die man gerade stellt.

use serde::{Deserialize, Serialize};

use crate::store::Store;

/// So viele Seiten passen ins Gedächtnis.
pub const LIMIT: usize = 300;
/// So viel Text wird je gemerkter Seite höchstens abgelegt (Bytes).
pub const TEXT_MAX: usize = 20_000;

/// Eine gemerkte Seite: Adresse, Titel, ihr Text und wann sie gemerkt wurde.
#[derive(Clone, Serialize, Deserialize)]
pub struct Record {
    #[serde(default = "kind")]
    pub k: String,
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub when: u64,
}

fn kind() -> String {
    "m".to_string()
}

/// Lädt alle gemerkten Seiten samt Text (für die Suche).
pub fn load(store: &Store) -> Vec<Record> {
    let mut list: Vec<Record> = Vec::new();
    if let Some((records, _)) = store.app_records("memory", "", LIMIT, LIMIT * (TEXT_MAX + 512)) {
        for (_id, data) in records {
            if let Ok(record) = serde_json::from_str::<Record>(&data) {
                if !record.text.trim().is_empty() {
                    list.push(record);
                }
            }
        }
    }
    list
}

/// Zerlegt eine Frage in brauchbare Suchwörter (klein, ab drei Zeichen).
fn terms(question: &str) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for word in question.split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() >= 3) {
        let word = word.to_lowercase();
        if !seen.contains(&word) {
            seen.push(word);
        }
        if seen.len() >= 24 {
            break;
        }
    }
    seen
}

/// Zählt, wie oft `needle` in `haystack` vorkommt (beide bereits klein).
fn count(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    let mut total = 0;
    let mut rest = haystack;
    while let Some(pos) = rest.find(needle) {
        total += 1;
        rest = &rest[pos + needle.len()..];
    }
    total
}

/// Bewertet eine Seite gegen die Suchwörter – ein Titeltreffer zählt stärker.
fn score(lower_title: &str, lower_text: &str, terms: &[String]) -> usize {
    terms.iter().map(|term| count(lower_title, term) * 5 + count(lower_text, term)).sum()
}

/// Sucht die zur Frage passenden Seiten heraus und schreibt sie als EINEN
/// Lesetext zusammen (je Seite eine Quellenüberschrift). Leer, wenn nichts da
/// ist. Ohne Treffer kommen die neuesten Seiten. `budget`: Gesamttext in Bytes.
pub fn gather(records: &[Record], question: &str, max_pages: usize, budget: usize) -> String {
    if records.is_empty() || max_pages == 0 {
        return String::new();
    }
    let terms = terms(question);
    let mut scored: Vec<(usize, u64, usize)> = records
        .iter()
        .enumerate()
        .map(|(index, record)| {
            let points = if terms.is_empty() {
                0
            } else {
                score(&record.title.to_lowercase(), &record.text.to_lowercase(), &terms)
            };
            (points, record.when, index)
        })
        .collect();
    // Beste zuerst; bei Gleichstand das Neuere. Gibt es Treffer, fallen die
    // ohne Treffer weg; gibt es keine, kommen die neuesten Seiten.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    let any_hit = scored.first().is_some_and(|(points, _, _)| *points > 0);
    let chosen: Vec<usize> = scored
        .into_iter()
        .filter(|(points, _, _)| !any_hit || *points > 0)
        .take(max_pages)
        .map(|(_, _, index)| index)
        .collect();

    let mut out = String::new();
    let per = (budget / chosen.len().max(1)).max(500);
    for index in chosen {
        let record = &records[index];
        let header = format!("## {}\n{}\n\n", record.title.trim(), record.url);
        if out.len() + header.len() >= budget {
            break;
        }
        out.push_str(&header);
        let room = per.min(budget.saturating_sub(out.len()));
        out.push_str(clip(&record.text, room));
        out.push_str("\n\n");
    }
    out
}

/// Schneidet an einer Zeichengrenze.
fn clip(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(title: &str, text: &str, when: u64) -> Record {
        Record { k: "m".into(), url: format!("https://example.com/{when}"), title: title.into(), text: text.into(), when }
    }

    #[test]
    fn matching_pages_come_first_and_others_are_left_out() {
        let records = vec![
            rec("Rust Ownership", "Borrowing and lifetimes explained.", 1),
            rec("Pasta Recipe", "Boil water, add salt and pasta.", 2),
            rec("Rust Async", "Futures and the async runtime.", 3),
        ];
        let out = gather(&records, "how does rust ownership work", 2, 10_000);
        assert!(out.contains("Rust Ownership"), "best title match missing: {out}");
        assert!(!out.contains("Pasta"), "unrelated page should be excluded: {out}");
        let pasta = gather(&records, "pasta salt", 3, 10_000);
        assert!(pasta.contains("Pasta Recipe"));
    }

    #[test]
    fn without_usable_terms_the_newest_pages_come() {
        let records = vec![rec("Old", "old text here", 1), rec("New", "new text here", 5)];
        // „ok" hat nur zwei Zeichen – keine Suchwörter, also die neueste Seite.
        let out = gather(&records, "ok", 1, 10_000);
        assert!(out.contains("New"));
        assert!(!out.contains("Old"));
    }

    #[test]
    fn the_budget_is_respected() {
        let big = "wort ".repeat(10_000);
        let records = vec![rec("Groß", &big, 1)];
        let out = gather(&records, "wort", 1, 500);
        assert!(out.len() <= 600, "over budget: {}", out.len());
    }

    #[test]
    fn empty_memory_yields_nothing() {
        assert_eq!(gather(&[], "irgendwas", 3, 1000), "");
    }
}
