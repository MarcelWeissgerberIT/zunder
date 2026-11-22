//! Seiten an Ort und Stelle übersetzen.
//!
//! Zunders Skript in der abgeschotteten Welt der Seite sammelt die sichtbaren
//! Textstellen (`ui/translate.js`). Der Browser bündelt sie zu Sätzen, schickt
//! sie in Portionen an das gewählte Modell und setzt die Antworten als reinen
//! Text wieder ein. Das Original bleibt gemerkt und kommt auf Wunsch zurück.
//!
//! Welches Modell übersetzt, entscheidet dieselbe Regel wie beim Assistenten:
//! aus einem privaten oder einem Stealth-Fenster nur eines, das auf dem Mac
//! läuft – der Text einer Seite verlässt den Mac von dort nie.
//!
//! Der Seitentext ist Material, keine Anweisung: Das Modell bekommt ihn als
//! nummerierte Zeilen, und was zurückkommt, wird nie anders verwendet denn als
//! Text für genau die Stelle mit derselben Nummer.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;
use tao::window::WindowId;

use crate::ai::{self, Engine};
use crate::app::{App, NoticeAction, TabId, UserEvent};
use crate::i18n::{self, t, Lang};
use crate::native;

const TRANSLATE_JS: &str = include_str!("../ui/translate.js");

/// Obergrenzen für das, was eine Seite an Text liefern darf. Das Skript hält
/// sie selbst ein – geprüft wird trotzdem.
const MAX_NODES: usize = 1_500;
const MAX_TEXT: usize = 6_000;

/// Eine Adresse ohne das Stück hinter `#` – daran zeigt sich, ob ein Tab noch
/// dieselbe Seite zeigt.
pub fn page_of(url: &str) -> &str {
    url.split('#').next().unwrap_or(url)
}

/// Der Aufruf von Zunders Skript in der Seite; die Daten kommen als Argument an.
fn script(mode: &str) -> String {
    format!("return ({TRANSLATE_JS})('{mode}', JSON.parse(payload));")
}

/// Die Übersetzung eines Tabs: läuft noch oder steht in der Seite.
pub struct Translation {
    job: u64,
    /// Die Seite, um die es geht (ohne `#abschnitt`). Zeigt der Tab eine
    /// andere, gilt der Auftrag nicht mehr.
    url: String,
    cancel: Arc<AtomicBool>,
    engine: Engine,
    /// Wie das Modell in den Hinweisen heißt.
    label: String,
    to_german: bool,
    /// Portionen insgesamt, zurückgekommene, davon gescheiterte.
    total: usize,
    done: usize,
    failed: usize,
    /// Textstellen, die jetzt übersetzt in der Seite stehen.
    changed: usize,
    error: String,
    /// Die Seite war zu lang: Übersetzt wurde nur ihr Anfang.
    truncated: bool,
    /// Der Hinweis, der den Fortschritt zeigt.
    notice: u64,
    finished: bool,
}

impl Drop for Translation {
    /// Wird die Übersetzung verworfen (neue Seite, Tab zu, Original zurück),
    /// hören auch die Anfragen an das Modell auf.
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

pub enum TranslateEvent {
    /// Die Textstellen der Seite (JSON aus der Seite, also Material).
    Collected { window: WindowId, tab: TabId, job: u64, json: Option<String> },
    /// Eine Portion ist zurück: Paare aus Textstelle und Übersetzung.
    Batch { window: WindowId, tab: TabId, job: u64, result: Result<Vec<(usize, String)>, String> },
    /// Es kam gar nicht erst zu einer Anfrage (kein Schlüssel für diesen Dienst).
    Failed { window: WindowId, tab: TabId, job: u64, message: String },
}

impl std::fmt::Debug for TranslateEvent {
    /// Ohne den Text der Seite.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TranslateEvent::Collected { job, json, .. } => {
                write!(f, "Collected(job {job}, {} bytes)", json.as_ref().map_or(0, String::len))
            }
            TranslateEvent::Batch { job, result, .. } => match result {
                Ok(pairs) => write!(f, "Batch(job {job}, {} texts)", pairs.len()),
                Err(_) => write!(f, "Batch(job {job}, failed)"),
            },
            TranslateEvent::Failed { job, .. } => write!(f, "Failed(job {job})"),
        }
    }
}

#[derive(Deserialize)]
struct Collected {
    #[serde(default)]
    lang: String,
    #[serde(default)]
    texts: Vec<String>,
    #[serde(default)]
    cont: Vec<u8>,
    #[serde(default)]
    truncated: bool,
}

/// Ein Satz oder Satzstück: die Textstellen, die nur durch Formatierung
/// getrennt sind. Steht derselbe Text mehrfach in der Seite, wird er einmal
/// übersetzt – `places` nennt jede Stelle.
#[derive(Clone, Debug, PartialEq)]
struct Unit {
    texts: Vec<String>,
    places: Vec<Vec<usize>>,
}

/// Bündelt die Textstellen zu Sätzen. `cont[i] == 1` heißt: Stelle `i` setzt
/// den Satz der Stelle davor fort. Ein Satz fasst höchstens `max_lines`
/// Stellen und `max_chars` Zeichen – danach beginnt ein neuer.
fn units(texts: &[String], cont: &[u8], max_lines: usize, max_chars: usize) -> Vec<Unit> {
    let mut groups: Vec<(Vec<usize>, usize)> = Vec::new();
    for (index, text) in texts.iter().enumerate() {
        let chars = text.chars().count();
        let continues = index > 0 && cont.get(index).copied() == Some(1);
        match groups.last_mut() {
            Some((group, used)) if continues && group.len() < max_lines && *used + chars <= max_chars => {
                group.push(index);
                *used += chars;
            }
            _ => groups.push((vec![index], chars)),
        }
    }
    let mut seen: HashMap<Vec<&str>, usize> = HashMap::new();
    let mut units: Vec<Unit> = Vec::new();
    for (group, _) in groups {
        let key: Vec<&str> = group.iter().map(|index| texts[*index].as_str()).collect();
        match seen.get(&key) {
            Some(at) => units[*at].places.push(group),
            None => {
                units.push(Unit { texts: key.iter().map(|text| text.to_string()).collect(), places: vec![group] });
                seen.insert(key, units.len() - 1);
            }
        }
    }
    units
}

/// Teilt die Sätze in Portionen, die das Modell in einer Anfrage verkraftet.
fn batches(units: Vec<Unit>, chars: usize, lines: usize) -> Vec<Vec<Unit>> {
    let mut out: Vec<Vec<Unit>> = Vec::new();
    let (mut used_chars, mut used_lines) = (0, 0);
    for unit in units {
        let unit_chars: usize = unit.texts.iter().map(|text| text.chars().count()).sum();
        let unit_lines = unit.texts.len();
        let fits = used_chars + unit_chars <= chars && used_lines + unit_lines <= lines;
        match out.last_mut() {
            Some(batch) if fits => batch.push(unit),
            _ => {
                out.push(vec![unit]);
                (used_chars, used_lines) = (0, 0);
            }
        }
        used_chars += unit_chars;
        used_lines += unit_lines;
    }
    out
}

/// Verteilt die Antwort des Modells auf die Stellen der Seite. Zurück kommen
/// die Paare (Stelle, Übersetzung) und die Sätze, die nicht vollständig
/// zurückkamen – ein halber Satz neben seinem Original wäre Kauderwelsch.
fn settle(batch: &[Unit], found: &[Option<String>]) -> (Vec<(usize, String)>, Vec<usize>) {
    let mut pairs = Vec::new();
    let mut again = Vec::new();
    let mut at = 0;
    for (index, unit) in batch.iter().enumerate() {
        let count = unit.texts.len();
        let Some(part) = found.get(at..at + count) else {
            break;
        };
        at += count;
        if part.iter().all(Option::is_some) || count == 1 {
            place(unit, part, &mut pairs);
        } else {
            again.push(index);
        }
    }
    (pairs, again)
}

fn place(unit: &Unit, part: &[Option<String>], pairs: &mut Vec<(usize, String)>) {
    for (slot, text) in part.iter().enumerate() {
        let (Some(text), Some(original)) = (text, unit.texts.get(slot)) else {
            continue;
        };
        // Was gleich blieb, braucht nicht angefasst zu werden. Und eine Antwort,
        // die um ein Vielfaches länger ist als ihre Vorlage, ist keine Übersetzung.
        if text == original || text.chars().count() > original.chars().count() * 6 + 200 {
            continue;
        }
        for nodes in &unit.places {
            if let Some(node) = nodes.get(slot) {
                pairs.push((*node, text.clone()));
            }
        }
    }
}

/// Übersetzt eine Portion. Sätze, die das Modell nicht Stück für Stück
/// zurückgibt, gehen noch einmal hin – dann jedes Stück für sich.
fn run_batch(
    engine: &Engine,
    key: Option<&str>,
    batch: &[Unit],
    to_german: bool,
    german_ui: bool,
    cancel: &AtomicBool,
) -> Result<Vec<(usize, String)>, String> {
    let lines: Vec<(String, bool)> = batch
        .iter()
        .flat_map(|unit| unit.texts.iter().enumerate().map(|(slot, text)| (text.clone(), slot > 0)))
        .collect();
    let found = ai::translate_lines(engine, key, &lines, to_german, german_ui, cancel)?;
    let (mut pairs, again) = settle(batch, &found);
    if !again.is_empty() && !cancel.load(Ordering::Relaxed) {
        let loose: Vec<Unit> = again
            .iter()
            .flat_map(|index| {
                let unit = &batch[*index];
                unit.texts.iter().enumerate().map(|(slot, text)| Unit {
                    texts: vec![text.clone()],
                    places: unit.places.iter().filter_map(|nodes| nodes.get(slot).map(|node| vec![*node])).collect(),
                })
            })
            .collect();
        let lines: Vec<(String, bool)> = loose.iter().map(|unit| (unit.texts[0].clone(), false)).collect();
        // Scheitert der zweite Anlauf, bleibt es bei dem, was der erste brachte.
        if let Ok(found) = ai::translate_lines(engine, key, &lines, to_german, german_ui, cancel) {
            pairs.extend(settle(&loose, &found).0);
        }
    }
    Ok(pairs)
}

/// Häufige Wörter, an denen sich Deutsch und Englisch erkennen lassen – ohne
/// die, die es in beiden Sprachen gibt („in“, „was“, „die“, „will“ …).
const GERMAN_WORDS: &[&str] = &[
    "der", "das", "und", "ist", "nicht", "mit", "für", "auf", "ein", "eine", "von", "zu", "den", "dem", "sich", "auch",
    "wird", "sind", "oder", "bei", "wie", "im", "aus", "nach", "noch", "über", "einen", "wir", "sie", "ich", "nur",
    "wenn", "aber", "dass", "zum", "zur", "als", "mehr", "werden", "kann", "diese", "einer", "haben", "durch",
];
const ENGLISH_WORDS: &[&str] = &[
    "the", "and", "of", "to", "is", "that", "for", "with", "on", "are", "this", "be", "by", "at", "from", "have", "has",
    "you", "not", "or", "it", "as", "your", "we", "can", "more", "about", "were", "which", "their", "but", "they",
    "been", "its", "our", "what", "how", "when", "would", "there", "these",
];

/// Ist der Text überwiegend deutsch (`Some(true)`) oder englisch
/// (`Some(false)`)? `None`: weder noch, oder zu wenig Text für ein Urteil.
fn german_or_english(texts: &[String]) -> Option<bool> {
    let german: HashSet<&str> = GERMAN_WORDS.iter().copied().collect();
    let english: HashSet<&str> = ENGLISH_WORDS.iter().copied().collect();
    let (mut words, mut de, mut en) = (0usize, 0usize, 0usize);
    let (mut de_kinds, mut en_kinds): (HashSet<String>, HashSet<String>) = (HashSet::new(), HashSet::new());
    'all: for text in texts {
        for word in text.split(|c: char| !c.is_alphabetic()).filter(|word| !word.is_empty()) {
            let word = word.to_lowercase();
            words += 1;
            if german.contains(word.as_str()) {
                de += 1;
                de_kinds.insert(word);
            } else if english.contains(word.as_str()) {
                en += 1;
                en_kinds.insert(word);
            }
            if words >= 4_000 {
                break 'all;
            }
        }
    }
    if words < 30 {
        return None;
    }
    // Jedes zehnte Wort eines der häufigen, deutlich mehr als aus der anderen
    // Sprache, und nicht immer dasselbe: So sieht ein Text in der Sprache aus.
    let clear = |hits: usize, other: usize, kinds: usize| hits * 10 >= words && hits >= other * 2 && kinds >= 5;
    if clear(de, en, de_kinds.len()) {
        Some(true)
    } else if clear(en, de, en_kinds.len()) {
        Some(false)
    } else {
        None
    }
}

/// Die Sprache aus einer Angabe wie „de-CH“ oder „en_US“.
fn primary(lang: &str) -> &str {
    let code = lang.split(['-', '_']).next().unwrap_or("").trim();
    if (2..=3).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_alphabetic()) {
        code
    } else {
        ""
    }
}

/// Wohin übersetzt wird: in die Sprache der Oberfläche – und steht die Seite
/// schon in ihr, in die jeweils andere der beiden.
fn target_is_german(ui_german: bool, texts: &[String], lang: &str) -> bool {
    let page = german_or_english(texts).or_else(|| match primary(lang).to_ascii_lowercase().as_str() {
        "de" => Some(true),
        "en" => Some(false),
        _ => None,
    });
    match (ui_german, page) {
        (true, Some(true)) => false,
        (true, _) => true,
        (false, Some(false)) => true,
        (false, _) => false,
    }
}

/// Gibt die Seite selbst eine andere Sprache an als die der Oberfläche? Danach
/// richtet sich nur, ob die Adressleiste das Übersetzen von sich aus anbietet.
pub fn foreign(lang: &str) -> bool {
    let code = primary(lang).to_ascii_lowercase();
    !code.is_empty() && code != i18n::lang_code()
}

fn language_name(to_german: bool) -> &'static str {
    t(if to_german { "tr_lang_de" } else { "tr_lang_en" })
}

impl Translation {
    /// Läuft die Übersetzung noch?
    pub fn running(&self) -> bool {
        !self.finished
    }

    /// Der Hinweis, der ihren Fortschritt zeigt.
    pub fn notice_id(&self) -> u64 {
        self.notice
    }

    /// Für die Prüfhilfe: der Stand in Zahlen.
    pub fn dump(&self) -> serde_json::Value {
        json!({
            "engine": self.label, "toGerman": self.to_german, "total": self.total, "done": self.done,
            "failed": self.failed, "changed": self.changed, "truncated": self.truncated, "finished": self.finished,
        })
    }

    fn progress(&self) -> String {
        let text = if self.total > 1 { t("tr_progress_n") } else { t("tr_progress") };
        text.replace("{lang}", language_name(self.to_german))
            .replace("{engine}", &self.label)
            // Gezeigt wird der Teil, an dem gerade gearbeitet wird.
            .replace("{done}", &(self.done + 1).min(self.total).to_string())
            .replace("{total}", &self.total.to_string())
    }
}

impl App {
    /// Übersetzt die Seite des aktiven Tabs – oder holt, wenn sie übersetzt
    /// ist oder gerade wird, das Original zurück.
    pub fn translate_page(&mut self, window: WindowId) {
        let Some(win) = self.windows.get(&window) else {
            return;
        };
        let Some(tab) = win.active_tab() else {
            return;
        };
        let tab_id = tab.id;
        if tab.translation.is_some() {
            self.translate_undo(window, tab_id);
            return;
        }
        let web = tab.url.starts_with("http://") || tab.url.starts_with("https://");
        if !web || tab.webview.is_none() {
            self.notice(window, t("tr_not_here").to_string(), None, 4000);
            return;
        }
        // Aus einem privaten oder einem Stealth-Fenster geht kein Seitentext ins Netz.
        let private = win.private || win.stealth;
        let reader = tab.reader;
        let engine = self.ai_engine(private);
        // Welche Modelle es gibt, ist noch nicht bekannt oder nicht mehr frisch
        // (Ollama kann gerade erst gestartet sein): erst nachsehen, dann weiter.
        let stale = self.ai_probe_at.is_none_or(|at| at.elapsed() > Duration::from_secs(20));
        if stale && (self.ai_probe.is_none() || engine.is_none()) {
            // Gemerkt wird die Seite, nicht bloß der Tab: Zeigt er nachher eine
            // andere, wird nichts übersetzt, wonach niemand gefragt hat.
            let entry = (window, tab_id, page_of(&tab.url).to_string());
            if !self.translate_deferred.contains(&entry) {
                self.translate_deferred.push(entry);
            }
            self.ai_refresh(true);
            return;
        }
        let Some(engine) = engine else {
            let key = if private && self.ai_engine(false).is_some() { "tr_private" } else { "tr_no_model" };
            self.notice(window, t(key).to_string(), None, 9000);
            return;
        };
        // Der Lesemodus legt einen Auszug über die Seite. Übersetzt wird die
        // Seite selbst – danach zeigt der Lesemodus den übersetzten Text.
        if reader {
            self.toggle_reader(window);
        }
        self.next_translation += 1;
        let job = self.next_translation;
        let limit = ai::translate_limit(&engine);
        let label = crate::assistant::engine_label(&engine);
        let proxy = self.proxy.clone();
        let Some(win) = self.windows.get_mut(&window) else {
            return;
        };
        let Some(tab) = win.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };
        let Some(webview) = &tab.webview else {
            return;
        };
        let payload = json!({ "limit": limit, "job": job }).to_string();
        let page = page_of(&tab.url).to_string();
        native::call_isolated(webview, &script("collect"), &payload, move |result| {
            let event = TranslateEvent::Collected { window, tab: tab_id, job, json: result };
            let _ = proxy.send_event(UserEvent::Translate(event));
        });
        tab.translation = Some(Translation {
            job,
            url: page,
            cancel: Arc::new(AtomicBool::new(false)),
            engine,
            label,
            to_german: true,
            total: 0,
            done: 0,
            failed: 0,
            changed: 0,
            error: String::new(),
            truncated: false,
            notice: 0,
            finished: false,
        });
        win.dirty = true;
    }

    /// Bricht eine laufende Übersetzung ab und zeigt wieder das Original.
    pub fn translate_undo(&mut self, window: WindowId, tab_id: TabId) {
        let Some(win) = self.windows.get_mut(&window) else {
            return;
        };
        let Some(tab) = win.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };
        let Some(translation) = tab.translation.take() else {
            return;
        };
        if let Some(webview) = &tab.webview {
            // Das Skript stellt nur wieder her, wenn in der Seite genau dieser Auftrag steht.
            native::call_isolated(webview, &script("restore"), &json!({ "job": translation.job }).to_string(), |_| {});
        }
        win.dirty = true;
        // Der Hinweis mit dem Fortschritt hat sich erledigt.
        win.send(json!({ "t": "noticeHide", "id": translation.notice }));
        self.notices.remove(&translation.notice);
    }

    pub fn translate_event(&mut self, event: TranslateEvent) {
        match event {
            TranslateEvent::Collected { window, tab, job, json } => self.translate_collected(window, tab, job, json),
            TranslateEvent::Batch { window, tab, job, result } => self.translate_batch(window, tab, job, result),
            TranslateEvent::Failed { window, tab, job, message } => {
                let running = self
                    .windows
                    .get(&window)
                    .and_then(|win| win.tabs.iter().find(|candidate| candidate.id == tab))
                    .and_then(|tab| tab.translation.as_ref())
                    .is_some_and(|translation| translation.job == job);
                if running {
                    self.translate_undo(window, tab);
                    self.warn(window, t("tr_failed").replace("{error}", &message), 9000);
                }
            }
        }
    }

    /// Bricht jede laufende Übersetzung ab – nach einem Wechsel des Modells,
    /// des Dienstes oder eines Schlüssels soll nichts mit dem alten Stand
    /// weiterlaufen. Was fertig übersetzt dasteht, bleibt.
    pub fn translate_cancel_all(&mut self) {
        self.translate_deferred.clear();
        let running: Vec<(WindowId, TabId)> = self
            .windows
            .iter()
            .flat_map(|(id, win)| {
                win.tabs
                    .iter()
                    .filter(|tab| tab.translation.as_ref().is_some_and(Translation::running))
                    .map(|tab| (*id, tab.id))
            })
            .collect();
        for (window, tab) in running {
            self.translate_undo(window, tab);
        }
    }

    /// Verwirft die Übersetzung eines Tabs, sofern es noch dieser Auftrag ist.
    fn translate_drop(&mut self, window: WindowId, tab_id: TabId, job: u64) {
        let Some(win) = self.windows.get_mut(&window) else {
            return;
        };
        if let Some(tab) = win.tabs.iter_mut().find(|tab| tab.id == tab_id) {
            if tab.translation.as_ref().is_some_and(|translation| translation.job == job) {
                tab.translation = None;
                win.dirty = true;
            }
        }
    }

    /// Die Textstellen der Seite sind da: zu Sätzen bündeln, in Portionen
    /// teilen und an das Modell schicken.
    fn translate_collected(&mut self, window: WindowId, tab_id: TabId, job: u64, json: Option<String>) {
        let current = self
            .windows
            .get(&window)
            .and_then(|win| win.tabs.iter().find(|tab| tab.id == tab_id))
            .and_then(|tab| tab.translation.as_ref())
            .filter(|translation| translation.job == job)
            .map(|translation| translation.engine.clone());
        let Some(engine) = current else {
            return;
        };
        let found = json.as_deref().and_then(|json| serde_json::from_str::<Collected>(json).ok());
        let Some(mut found) = found else {
            self.translate_drop(window, tab_id, job);
            self.notice(window, t("tr_nothing").to_string(), None, 5000);
            return;
        };
        // Was aus der Seite kommt, wird hier noch einmal begrenzt.
        let limit = ai::translate_limit(&engine);
        let mut truncated = found.truncated || found.texts.len() > MAX_NODES;
        found.texts.truncate(MAX_NODES);
        let mut total = 0;
        let mut keep = 0;
        for text in &found.texts {
            let chars = text.chars().count();
            if chars > MAX_TEXT || total + chars > limit {
                truncated = true;
                break;
            }
            total += chars;
            keep += 1;
        }
        found.texts.truncate(keep);
        if found.texts.is_empty() {
            self.translate_drop(window, tab_id, job);
            self.notice(window, t("tr_nothing").to_string(), None, 5000);
            return;
        }
        let german_ui = i18n::lang() == Lang::De;
        let to_german = target_is_german(german_ui, &found.texts, &found.lang);
        let (chars, lines) = ai::translate_batch(&engine);
        let portions = batches(units(&found.texts, &found.cont, lines, chars), chars, lines);
        let total = portions.len();

        let Some(translation) = self
            .windows
            .get_mut(&window)
            .and_then(|win| win.tabs.iter_mut().find(|tab| tab.id == tab_id))
            .and_then(|tab| tab.translation.as_mut())
        else {
            return;
        };
        translation.to_german = to_german;
        translation.total = total;
        translation.truncated = truncated;
        let cancel = translation.cancel.clone();
        let text = translation.progress();
        let action = NoticeAction::TranslateUndo(window, tab_id);
        let notice = self.notice_id(window, text, Some((t("cancel"), action)), 0);
        if let Some(translation) = self
            .windows
            .get_mut(&window)
            .and_then(|win| win.tabs.iter_mut().find(|tab| tab.id == tab_id))
            .and_then(|tab| tab.translation.as_mut())
        {
            translation.notice = notice;
        }

        // Die Portionen liegen in einer Schlange; auf dem Mac arbeitet sie einer
        // ab, bei einem Dienst im Netz mehrere nebeneinander.
        let workers = ai::translate_workers(&engine).min(total).max(1);
        let queue = Arc::new(Mutex::new(portions.into_iter().collect::<VecDeque<_>>()));
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            // Der Schlüssel wird einmal geholt, für genau diesen Dienst, und
            // gilt für den ganzen Auftrag. Trägt jemand inzwischen einen anderen
            // Dienst ein, bekommt dieser hier dessen Schlüssel nie zu sehen.
            let key = match ai::translate_key(&engine, german_ui) {
                Ok(key) => Arc::new(key),
                Err(message) => {
                    let event = TranslateEvent::Failed { window, tab: tab_id, job, message };
                    let _ = proxy.send_event(UserEvent::Translate(event));
                    return;
                }
            };
            for _ in 0..workers {
                let (queue, engine, cancel, proxy, key) =
                    (queue.clone(), engine.clone(), cancel.clone(), proxy.clone(), key.clone());
                std::thread::spawn(move || loop {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let Some(batch) = queue.lock().ok().and_then(|mut queue| queue.pop_front()) else {
                        break;
                    };
                    let result = run_batch(&engine, key.as_deref(), &batch, to_german, german_ui, &cancel);
                    // Nach einem Abbruch interessiert das Ergebnis niemanden mehr.
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let event = TranslateEvent::Batch { window, tab: tab_id, job, result };
                    if proxy.send_event(UserEvent::Translate(event)).is_err() {
                        break;
                    }
                });
            }
        });
    }

    /// Eine Portion ist zurück: einsetzen, Fortschritt zeigen, am Ende Bilanz ziehen.
    fn translate_batch(
        &mut self,
        window: WindowId,
        tab_id: TabId,
        job: u64,
        result: Result<Vec<(usize, String)>, String>,
    ) {
        let Some(win) = self.windows.get_mut(&window) else {
            return;
        };
        let visible = win.shows(tab_id);
        let Some(tab) = win.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };
        let Some(translation) = tab.translation.as_mut().filter(|translation| translation.job == job && !translation.finished)
        else {
            return;
        };
        // Der Tab zeigt nicht mehr die Seite, um die es ging (eine Seite, die
        // ihre Adresse selbst wechselt): Dann wird nichts mehr eingesetzt.
        if page_of(&tab.url) != translation.url {
            self.translate_undo(window, tab_id);
            return;
        }
        translation.done += 1;
        match result {
            Ok(pairs) => {
                if let (false, Some(webview)) = (pairs.is_empty(), tab.webview.as_ref()) {
                    translation.changed += pairs.len();
                    // Mit der Nummer des Auftrags: Das Skript setzt nur ein, wenn in
                    // der Seite genau dieser Auftrag steht – nicht in eine andere
                    // Seite, die inzwischen in den Tab zurückgekehrt ist.
                    let payload = json!({ "job": job, "pairs": pairs }).to_string();
                    native::call_isolated(webview, &script("apply"), &payload, |_| {});
                }
            }
            Err(message) => {
                translation.failed += 1;
                if !message.is_empty() {
                    translation.error = message;
                }
            }
        }
        // Zwei Fehlschläge und noch kein Erfolg: Da geht gerade nichts – der
        // Rest der Seite braucht es nicht auch noch zu versuchen.
        let hopeless = translation.failed == translation.done && translation.failed >= translation.total.min(2);
        if hopeless {
            translation.cancel.store(true, Ordering::Relaxed);
        }
        let notice = translation.notice;
        if !hopeless && translation.done < translation.total {
            let text = translation.progress();
            win.send(json!({ "t": "noticeText", "id": notice, "text": text }));
            return;
        }

        translation.finished = true;
        let language = language_name(translation.to_german);
        let already = t(if translation.to_german { "tr_in_de" } else { "tr_in_en" });
        let outcome = if translation.failed == translation.done {
            Err(t("tr_failed").replace("{error}", &translation.error))
        } else if translation.changed == 0 {
            Err(t("tr_same").replace("{in}", already))
        } else {
            let mut text = t("tr_done").replace("{lang}", language).replace("{engine}", &translation.label);
            if translation.truncated {
                text.push(' ');
                text.push_str(t("tr_done_part"));
            }
            if translation.failed > 0 {
                text.push(' ');
                text.push_str(&t("tr_done_skipped").replace("{n}", &translation.failed.to_string()));
            }
            Ok(text)
        };
        win.dirty = true;
        match outcome {
            // Nichts wurde übersetzt: Es gibt auch kein Original zurückzuholen.
            Err(text) => {
                let failed = translation.failed > 0;
                tab.translation = None;
                win.send(json!({ "t": "noticeHide", "id": notice }));
                self.notices.remove(&notice);
                if failed {
                    self.warn(window, text, 9000);
                } else {
                    self.notice(window, text, None, 7000);
                }
            }
            Ok(text) => {
                self.notices.remove(&notice);
                if visible {
                    let action = NoticeAction::TranslateUndo(window, tab_id);
                    let shown = self.notice_id(window, text, Some((t("tr_original"), action)), 15000);
                    // Gemerkt, damit auch dieser Hinweis verschwindet, wenn die Seite den Tab verlässt.
                    if let Some(translation) = self
                        .windows
                        .get_mut(&window)
                        .and_then(|win| win.tabs.iter_mut().find(|tab| tab.id == tab_id))
                        .and_then(|tab| tab.translation.as_mut())
                    {
                        translation.notice = shown;
                    }
                } else if let Some(win) = self.windows.get_mut(&window) {
                    // Der Tab ist nicht zu sehen: kein Hinweis über einer anderen Seite.
                    win.send(json!({ "t": "noticeHide", "id": notice }));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|text| text.to_string()).collect()
    }

    #[test]
    fn sentences_split_by_formatting_stay_together() {
        let texts = strings(&["Home", "Click", "here", "to continue.", "Home", "Read more", "Click", "here", "to continue."]);
        let cont = [0, 0, 1, 1, 0, 0, 0, 1, 1];
        let found = units(&texts, &cont, 20, 1_000);
        // Derselbe Text an zwei Stellen wird einmal übersetzt.
        assert_eq!(
            found,
            vec![
                Unit { texts: strings(&["Home"]), places: vec![vec![0], vec![4]] },
                Unit { texts: strings(&["Click", "here", "to continue."]), places: vec![vec![1, 2, 3], vec![6, 7, 8]] },
                Unit { texts: strings(&["Read more"]), places: vec![vec![5]] },
            ]
        );
        // Ein Satz mit zu vielen Stücken wird geteilt, ein fehlendes Kennzeichen heißt „neu“.
        let long = units(&strings(&["a1", "b2", "c3", "d4"]), &[1, 1, 1], 2, 1_000);
        assert_eq!(long.iter().map(|unit| unit.texts.len()).collect::<Vec<_>>(), vec![2, 1, 1]);
        // … und einer mit zu viel Text ebenso.
        let wide = units(&strings(&["aaaa", "bbbb", "cc"]), &[0, 1, 1], 20, 6);
        assert_eq!(wide.iter().map(|unit| unit.texts.len()).collect::<Vec<_>>(), vec![1, 2]);

        // Portionen: nie mehr Zeilen oder Zeichen als erlaubt – außer ein Satz allein ist schon größer.
        let portions = batches(found, 14, 3);
        assert_eq!(portions.iter().map(Vec::len).collect::<Vec<_>>(), vec![1, 1, 1]);
        let one = batches(units(&texts, &cont, 20, 1_000), 1_000, 100);
        assert_eq!(one.len(), 1);
        assert!(batches(Vec::new(), 10, 10).is_empty());
    }

    #[test]
    fn answers_land_on_every_place_and_half_sentences_go_again() {
        let batch = vec![
            Unit { texts: strings(&["Home"]), places: vec![vec![0], vec![4]] },
            Unit { texts: strings(&["Click", "here"]), places: vec![vec![1, 2]] },
            Unit { texts: strings(&["Zunder", "is fast"]), places: vec![vec![5, 6]] },
            Unit { texts: strings(&["Missing"]), places: vec![vec![7]] },
        ];
        let found = vec![
            Some("Start".to_string()),
            Some("Klicke".to_string()),
            None,
            Some("Zunder".to_string()),
            Some("ist schnell".to_string()),
            None,
        ];
        let (pairs, again) = settle(&batch, &found);
        // „Zunder“ blieb gleich und wird nicht angefasst; der halbe Satz geht noch einmal hin.
        assert_eq!(pairs, vec![(0, "Start".to_string()), (4, "Start".to_string()), (6, "ist schnell".to_string())]);
        assert_eq!(again, vec![1]);
        // Eine Antwort, die viel länger ist als ihre Vorlage, wird verworfen.
        let wall = vec![Some("x".repeat(400))];
        assert!(settle(&batch[..1], &wall).0.is_empty());
        // Zu wenige Antworten: Was fehlt, bleibt einfach stehen.
        assert_eq!(settle(&batch, &found[..1]).0.len(), 2);
    }

    #[test]
    fn language_of_a_page_is_read_from_its_words() {
        let german = strings(&[
            "Der Browser ist schnell und einfach zu bedienen.",
            "Mit dem Werbeblocker werden Seiten nicht nur ruhiger, sie laden auch schneller.",
            "Wenn eine Seite nach dem Standort fragt, wird das für diese Website gemerkt.",
            "Das ist nicht alles: Über die Einstellungen kann man noch mehr anpassen.",
        ]);
        let english = strings(&[
            "The browser is fast and easy to use.",
            "With the ad blocker, pages are not only calmer, they also load faster for you.",
            "When a page asks for your location, this is remembered for that website.",
            "That is not all: there are more settings which you can change at any time.",
        ]);
        let french = strings(&[
            "Le navigateur est rapide et facile à utiliser pour tout le monde.",
            "Avec le bloqueur de publicité, les pages sont plus calmes et se chargent plus vite.",
            "Quand une page demande votre position, ce choix est retenu pour ce site.",
            "Ce n'est pas tout : il y a encore des réglages que vous pouvez changer.",
        ]);
        assert_eq!(german_or_english(&german), Some(true));
        assert_eq!(german_or_english(&english), Some(false));
        assert_eq!(german_or_english(&french), None);
        assert_eq!(german_or_english(&strings(&["Der Browser ist schnell."])), None);

        // Deutsche Oberfläche: alles wird deutsch – nur Deutsches wird englisch.
        assert!(target_is_german(true, &english, ""));
        assert!(target_is_german(true, &french, "fr"));
        assert!(!target_is_german(true, &german, "en"));
        // Englische Oberfläche: alles wird englisch – nur Englisches wird deutsch.
        assert!(!target_is_german(false, &german, ""));
        assert!(!target_is_german(false, &french, "fr-FR"));
        assert!(target_is_german(false, &english, "de"));
        // Zu wenig Text: Dann gilt, was die Seite über sich sagt.
        let short = strings(&["Willkommen"]);
        assert!(!target_is_german(true, &short, "de-CH"));
        assert!(target_is_german(true, &short, ""));
        assert!(target_is_german(false, &short, "en_US"));

        assert_eq!(primary("de-CH"), "de");
        assert_eq!(primary("EN_us"), "EN");
        assert_eq!(primary("x"), "");
        assert_eq!(primary("<script>"), "");
    }
}
