//! KI-Seitenleiste: Befehle aus ihrer Oberfläche, Seitentext holen, Antworten
//! weiterreichen. Die Modelle selbst spricht `ai.rs` an.
//!
//! Zwei Regeln gelten hier:
//!
//!   - Von selbst geht nie etwas ins Netz. Ohne ausdrückliche Wahl arbeitet der
//!     Assistent nur mit Modellen auf dem Mac; in privaten Fenstern immer.
//!   - Was aus einer Seite kommt (Text, Auswahl, Sprachangabe), ist Material.
//!     Titel und Adresse stammen vom Browser, nicht von der Seite.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tao::window::WindowId;

use crate::ai::{self, Action, Engine};
use crate::app::{App, BrowserWindow, TabId, UserEvent};
use crate::i18n::{self, t, Lang};
use crate::native;
use crate::omnibox;
use crate::webviews::build_assistant;

/// Liest den Text der Seite aus. Läuft in der Seite – das Ergebnis ist Material.
const EXTRACT_JS: &str = include_str!("../ui/extract.js");

/// Die laufende Anfrage eines Fensters.
pub struct AiJob {
    pub id: u64,
    pub tab: TabId,
    pub cancel: Arc<AtomicBool>,
    /// Gesetzt, solange noch auf den Seitentext gewartet wird.
    waiting: Option<Waiting>,
}

struct Waiting {
    engine: Engine,
    action: Action,
    question: String,
    history: Vec<(String, String)>,
    /// Die Seiten der Anfrage: zuerst die eigentliche, danach die zum Vergleich.
    sources: Vec<Source>,
    /// Wie viele davon ihren Text noch nicht geliefert haben.
    outstanding: usize,
}

/// Eine Seite, deren Text für die Anfrage gebraucht wird. Titel und Adresse
/// kennt der Browser selbst; `json` ist die Antwort des Auslese-Skripts.
struct Source {
    title: String,
    url: String,
    json: Option<String>,
}

fn engine_id(engine: &Engine) -> &'static str {
    match engine {
        Engine::Device => "device",
        Engine::Ollama(_) => "ollama",
        Engine::Claude(_) => "claude",
    }
}

fn engine_label(engine: &Engine) -> String {
    match engine {
        Engine::Device => t("ai_e_device_label").to_string(),
        Engine::Ollama(model) => format!("Ollama · {model}"),
        Engine::Claude(model) => ai::CLAUDE_MODELS
            .iter()
            .find(|(id, _)| id == model)
            .map(|(_, name)| name.to_string())
            .unwrap_or_else(|| model.clone()),
    }
}

/// Räumt ausgelesenen Seitentext auf: keine Leerzeilen-Stapel, keine
/// Leerzeichen-Ketten, keine unsichtbaren Steuerzeichen.
fn tidy(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank = 0;
    for line in text.lines() {
        let mut compact = String::with_capacity(line.len());
        let mut space = false;
        for c in line.chars() {
            if c == ' ' || c == '\t' || c == '\u{a0}' {
                space = true;
            } else if c.is_control() || matches!(c, '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{feff}') {
                // unsichtbar – weglassen
            } else {
                if space && !compact.is_empty() {
                    compact.push(' ');
                }
                space = false;
                compact.push(c);
            }
        }
        if compact.is_empty() {
            blank += 1;
            if blank == 1 && !out.is_empty() {
                out.push('\n');
            }
        } else {
            blank = 0;
            out.push_str(&compact);
            out.push('\n');
        }
    }
    out.trim().to_string()
}

/// Macht aus der Antwort des Auslese-Skripts den Text für das Modell.
fn read_page(json: &str, title: &str, url: &str) -> Option<ai::Page> {
    let mut value: Value = serde_json::from_str(json).ok()?;
    if let Some(inner) = value.as_str() {
        value = serde_json::from_str(inner).ok()?;
    }
    let field = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or("");
    let selection = tidy(field("selection"));
    let is_selection = selection.chars().filter(|c| !c.is_whitespace()).count() >= 12;
    let text = if is_selection { selection } else { tidy(field("text")) };
    if text.is_empty() {
        return None;
    }
    Some(ai::Page {
        title: title.to_string(),
        url: url.to_string(),
        text,
        is_selection,
        lang: field("lang").chars().take(12).collect(),
        truncated: !is_selection && value.get("truncated").and_then(Value::as_bool).unwrap_or(false),
    })
}

fn readable(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://") || url.starts_with("file://")
}

impl BrowserWindow {
    /// Schickt eine Nachricht an die Seitenleiste dieses Fensters.
    pub fn assistant_send(&mut self, message: Value) {
        let Some(view) = &self.assistant else {
            return;
        };
        let script = format!("window.__zunderAi&&window.__zunderAi({message})");
        if self.assistant_ready {
            let _ = view.evaluate_script(&script);
        } else {
            self.assistant_queue.push(script);
        }
    }
}

impl App {
    // -----------------------------------------------------------------------
    // Seitenleiste
    // -----------------------------------------------------------------------

    /// Öffnet oder schließt die Seitenleiste; `None` schaltet um.
    pub fn set_assistant(&mut self, window: WindowId, open: Option<bool>) -> bool {
        let Some(win) = self.windows.get_mut(&window) else {
            return false;
        };
        let open = open.unwrap_or(!win.assistant_open);
        if open != win.assistant_open {
            if open && win.assistant.is_none() {
                win.assistant_open = true; // damit die Maße schon stimmen
                match build_assistant(&win.shared, win.assistant_rect()) {
                    Ok(view) => win.assistant = Some(view),
                    Err(error) => {
                        win.assistant_open = false;
                        eprintln!("zunder: Seitenleiste konnte nicht angelegt werden: {error}");
                        return false;
                    }
                }
            }
            win.assistant_open = open;
            if let Some(view) = &win.assistant {
                let _ = view.set_visible(open);
            }
            win.dirty = true;
            self.layout(window);
        }
        if open {
            self.ai_refresh(false);
            self.assistant_page(window);
            if let Some(win) = self.windows.get_mut(&window) {
                if win.overlay {
                    native::bring_to_front(&win.chrome);
                } else if let Some(view) = &win.assistant {
                    let _ = view.focus();
                }
                win.assistant_send(json!({ "t": "aiFocus" }));
            }
        } else {
            self.focus_content(window);
        }
        open
    }

    /// Befehl aus der Oberfläche der Seitenleiste.
    pub fn handle_assistant(&mut self, window: WindowId, command: &str, message: &Value) {
        let text = |key: &str| {
            message
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        match command {
            "aiReady" => {
                if let Some(win) = self.windows.get_mut(&window) {
                    win.assistant_ready = true;
                    win.assistant_page.clear();
                    let queued = std::mem::take(&mut win.assistant_queue);
                    if let Some(view) = &win.assistant {
                        for script in queued {
                            let _ = view.evaluate_script(&script);
                        }
                    }
                }
                self.assistant_state(window);
                self.assistant_page(window);
            }
            "aiClose" => {
                self.set_assistant(window, Some(false));
            }
            "aiAsk" => self.ai_ask(window, message),
            "aiStop" => self.ai_stop(window),
            "aiTabs" => self.assistant_tabs(window),
            "aiSplit" => {
                // Den Vergleichs-Tab neben den aktuellen stellen (0 beendet das).
                let tab = message.get("tab").and_then(Value::as_u64).filter(|id| *id != 0);
                // Die Seite, von der der Vergleich ausgeht, steht links – und
                // bleibt vorn, wenn er endet. So bleibt auch ihr Gespräch zu sehen.
                let base = message.get("base").and_then(Value::as_u64);
                if let Some(index) = base.and_then(|base| self.tab_index(window, base)) {
                    let in_pair = self
                        .windows
                        .get(&window)
                        .is_some_and(|w| w.active != index && (tab.is_some() || w.in_split(w.tabs[index].id)));
                    if in_pair {
                        self.activate_tab(window, index);
                    }
                }
                self.set_split(window, tab);
                if let Some(win) = self.windows.get(&window) {
                    if let Some(view) = &win.assistant {
                        let _ = view.focus();
                    }
                }
            }
            "aiRefresh" => self.ai_refresh(true),
            "aiSetting" => self.ai_setting(&text("key"), &text("value")),
            "aiKey" => self.ai_key(window, &text("key")),
            "aiKeyDelete" => {
                ai::delete_key();
                self.store.set_setting("ai_claude_key", "0");
                self.ai_cancel_all();
                self.assistant_state_all();
            }
            _ => {}
        }
    }

    /// Sagt der Seitenleiste, welche Seite gerade vor ihr liegt.
    pub fn assistant_page(&mut self, window: WindowId) {
        let Some(win) = self.windows.get_mut(&window) else {
            return;
        };
        if !win.assistant_open {
            return;
        }
        let Some(tab) = win.active_tab() else {
            return;
        };
        let can_read = tab.webview.is_some() && readable(&tab.url);
        let message = json!({
            "t": "aiPage",
            "tab": tab.id,
            // Den Titel bestimmt die Seite: ohne Zeichen, die die Schreibrichtung
            // umdrehen oder Zeilen vortäuschen, und mit dem Rechner davor – so
            // kann er in der Leiste nicht wie eine Zeile des Assistenten aussehen.
            "title": if can_read {
                let title: String = tab
                    .display_title()
                    .chars()
                    .filter(|c| !c.is_control() && !matches!(c, '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
                    .take(120)
                    .collect();
                let host = crate::shield::host_of(&tab.url);
                if host.is_empty() || title.contains(&host) { title } else { format!("{host} – {title}") }
            } else {
                String::new()
            },
            "url": if can_read { omnibox::pretty_url(&tab.url) } else { String::new() },
            // Dieselbe Seite bleibt dieselbe, auch wenn nur der Anker wechselt.
            "key": tab.url.split('#').next().unwrap_or(""),
            "readable": can_read,
            "tabs": win.tabs.iter().map(|t| t.id).collect::<Vec<_>>(),
            // Stehen zwei Tabs nebeneinander: (links, rechts).
            "pair": win.split.map(|(left, right)| vec![left, right]),
        });
        let text = message.to_string();
        if text != win.assistant_page {
            win.assistant_page = text;
            win.assistant_send(message);
        }
    }

    /// Die anderen Tabs des Fensters – zur Auswahl für einen Vergleich.
    fn assistant_tabs(&mut self, window: WindowId) {
        let Some(win) = self.windows.get_mut(&window) else {
            return;
        };
        let active = win.active_tab().map(|t| t.id);
        let private = win.private;
        let tabs: Vec<Value> = win
            .tabs
            .iter()
            .map(|tab| {
                json!({
                    "id": tab.id,
                    "title": tab.display_title(),
                    "url": omnibox::pretty_url(&tab.url),
                    "favicon": if private { None } else { tab.favicon.clone() },
                    "current": Some(tab.id) == active,
                    // Vergleichen lässt sich nur, was gerade geladen ist.
                    "ready": tab.webview.is_some() && readable(&tab.url),
                    "web": readable(&tab.url),
                })
            })
            .collect();
        win.assistant_send(json!({ "t": "aiTabs", "tabs": tabs }));
    }

    // -----------------------------------------------------------------------
    // Modelle
    // -----------------------------------------------------------------------

    /// Stellt im Hintergrund fest, welche Modelle nutzbar sind.
    pub fn ai_refresh(&mut self, force: bool) {
        if self.ai_probing {
            return;
        }
        let fresh = self
            .ai_probe_at
            .is_some_and(|at| at.elapsed() < Duration::from_secs(20));
        if fresh && !force {
            return;
        }
        self.ai_probing = true;
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let _ = proxy.send_event(UserEvent::AiProbe(ai::probe()));
        });
    }

    pub fn ai_probe_done(&mut self, probe: ai::Probe) {
        self.ai_probe = Some(probe);
        self.ai_probe_at = Some(Instant::now());
        self.ai_probing = false;
        self.assistant_state_all();
        // Fragen, die auf diese Auskunft gewartet haben.
        for (window, message) in std::mem::take(&mut self.ai_deferred) {
            self.ai_ask(window, &message);
        }
    }

    fn ai_choice(&self) -> String {
        self.store.setting("ai_engine").unwrap_or_default()
    }

    /// Ob ein Schlüssel hinterlegt ist, merkt sich Zunder getrennt – so muss
    /// der Schlüsselbund erst angefasst werden, wenn wirklich eine Anfrage ansteht.
    fn ai_has_key(&self) -> bool {
        self.store.setting("ai_claude_key").as_deref() == Some("1") || ai::debug_key_present()
    }

    fn ai_ollama_model(&self) -> Option<String> {
        let models = self.ai_probe.as_ref()?.ollama.as_ref()?;
        let saved = self.store.setting("ai_ollama_model").unwrap_or_default();
        if models.iter().any(|m| *m == saved) {
            Some(saved)
        } else {
            models.first().cloned()
        }
    }

    fn ai_claude_model(&self) -> String {
        let saved = self.store.setting("ai_claude_model").unwrap_or_default();
        if ai::CLAUDE_MODELS.iter().any(|(id, _)| *id == saved) {
            saved
        } else {
            ai::CLAUDE_MODELS[0].0.to_string()
        }
    }

    /// Welches Modell eine Anfrage aus einem Fenster bekommt.
    fn ai_engine(&self, private: bool) -> Option<Engine> {
        let device = self.ai_probe.as_ref().is_some_and(|p| p.device);
        let ollama = self.ai_ollama_model();
        match self.ai_choice().as_str() {
            "claude" if self.ai_has_key() && !private => {
                return Some(Engine::Claude(self.ai_claude_model()));
            }
            "ollama" => {
                if let Some(model) = &ollama {
                    return Some(Engine::Ollama(model.clone()));
                }
            }
            _ => {}
        }
        // Sonst das, was auf dem Mac läuft – nie von selbst ein Dienst im Netz.
        if device {
            return Some(Engine::Device);
        }
        ollama.map(Engine::Ollama)
    }

    fn assistant_state(&mut self, window: WindowId) {
        let Some(private) = self.windows.get(&window).map(|w| w.private) else {
            return;
        };
        let probe = self.ai_probe.as_ref();
        let engine = self.ai_engine(private);
        let models = probe.and_then(|p| p.ollama.clone());
        let state = json!({
            "t": "aiState",
            "probed": probe.is_some(),
            "choice": self.ai_choice(),
            "engine": engine.as_ref().map(|e| json!({
                "id": engine_id(e),
                "label": engine_label(e),
                "local": e.is_local(),
            })),
            "device": {
                "ok": probe.is_some_and(|p| p.device),
                "reason": probe.map(|p| p.device_reason.clone()).unwrap_or_default(),
            },
            "ollama": {
                "running": models.is_some(),
                "models": models.unwrap_or_default(),
                "model": self.ai_ollama_model(),
            },
            "claude": {
                "key": self.ai_has_key(),
                "model": self.ai_claude_model(),
                "models": ai::CLAUDE_MODELS
                    .iter()
                    .map(|(id, name)| json!({ "id": id, "name": name }))
                    .collect::<Vec<_>>(),
            },
            "private": private,
        });
        if let Some(win) = self.windows.get_mut(&window) {
            win.assistant_send(state);
        }
    }

    fn assistant_state_all(&mut self) {
        let windows: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|(_, w)| w.assistant.is_some())
            .map(|(id, _)| *id)
            .collect();
        for window in windows {
            self.assistant_state(window);
        }
    }

    fn ai_setting(&mut self, key: &str, value: &str) {
        match key {
            "engine" if matches!(value, "device" | "ollama" | "claude") => {
                self.store.set_setting("ai_engine", value);
            }
            "ollamaModel" => {
                let known = self
                    .ai_probe
                    .as_ref()
                    .and_then(|p| p.ollama.as_ref())
                    .is_some_and(|models| models.iter().any(|m| m == value));
                if !known {
                    return;
                }
                self.store.set_setting("ai_ollama_model", value);
            }
            "claudeModel" if ai::CLAUDE_MODELS.iter().any(|(id, _)| *id == value) => {
                self.store.set_setting("ai_claude_model", value);
            }
            _ => return,
        }
        self.ai_cancel_all();
        self.assistant_state_all();
    }

    /// Bricht alle laufenden Anfragen ab – nach einem Wechsel des Modells oder
    /// dem Entfernen des Schlüssels soll keine Anfrage mit dem alten Stand
    /// weiterlaufen.
    fn ai_cancel_all(&mut self) {
        self.ai_deferred.clear();
        for win in self.windows.values_mut() {
            if let Some(job) = win.ai_job.take() {
                job.cancel.store(true, Ordering::Relaxed);
                win.assistant_send(json!({ "t": "aiStopped", "job": job.id }));
            }
        }
    }

    /// Legt den API-Schlüssel in den Schlüsselbund. Er wird nirgends sonst
    /// gespeichert und nie ausgegeben.
    fn ai_key(&mut self, window: WindowId, key: &str) {
        let key = key.trim();
        let plausible = (20..=400).contains(&key.len()) && key.chars().all(|c| c.is_ascii_graphic());
        let error = if !plausible {
            Some("ai_key_invalid")
        } else if ai::save_key(key).is_err() {
            Some("ai_key_error")
        } else {
            None
        };
        if let Some(error) = error {
            if let Some(win) = self.windows.get_mut(&window) {
                win.assistant_send(json!({ "t": "aiKeyResult", "ok": false, "text": t(error) }));
            }
            return;
        }
        self.store.set_setting("ai_claude_key", "1");
        // Wer einen Schlüssel einträgt, will Claude benutzen.
        self.store.set_setting("ai_engine", "claude");
        if let Some(win) = self.windows.get_mut(&window) {
            win.assistant_send(json!({ "t": "aiKeyResult", "ok": true }));
        }
        self.assistant_state_all();
    }

    // -----------------------------------------------------------------------
    // Anfragen
    // -----------------------------------------------------------------------

    fn ai_ask(&mut self, window: WindowId, message: &Value) {
        if self.ai_probe.is_none() {
            // Erst feststellen, welche Modelle es gibt; die Frage wartet so lange.
            self.ai_deferred.push((window, message.clone()));
            self.ai_refresh(true);
            return;
        }
        let Some(private) = self.windows.get(&window).map(|w| w.private) else {
            return;
        };
        let fail = |app: &mut App, key: &str| {
            if let Some(win) = app.windows.get_mut(&window) {
                win.assistant_send(json!({ "t": "aiFailed", "text": t(key) }));
            }
        };
        let Some(engine) = self.ai_engine(private) else {
            let key = if private && self.ai_choice() == "claude" && self.ai_has_key() {
                "ai_none_private"
            } else {
                "ai_none_text"
            };
            fail(self, key);
            return;
        };
        let action = Action::parse(message.get("action").and_then(Value::as_str).unwrap_or(""));
        let question: String = message
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .chars()
            .take(4000)
            .collect();
        if action == Action::Ask && question.trim().is_empty() {
            fail(self, "ai_empty_question");
            return;
        }
        let history: Vec<(String, String)> = message
            .get("history")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|pair| {
                        let asked = pair.get(0)?.as_str()?;
                        let answer = pair.get(1)?.as_str()?;
                        Some((asked.to_string(), answer.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Um welche Seiten es geht: die Ausgangsseite (sonst der aktive Tab) und,
        // beim Vergleichen, bis zu drei weitere Tabs dieses Fensters.
        let base = message.get("base").and_then(Value::as_u64);
        let wanted: Vec<u64> = message
            .get("others")
            .and_then(Value::as_array)
            .map(|ids| ids.iter().filter_map(Value::as_u64).collect())
            .unwrap_or_default();

        self.ai_next_job += 1;
        let id = self.ai_next_job;
        let proxy = self.proxy.clone();
        let mut outstanding = 0;
        {
            let Some(win) = self.windows.get_mut(&window) else {
                return;
            };
            if let Some(previous) = win.ai_job.take() {
                previous.cancel.store(true, Ordering::Relaxed);
            }
            let base_index = base
                .and_then(|id| win.tabs.iter().position(|t| t.id == id))
                .unwrap_or(win.active);
            let Some(base_id) = win.tabs.get(base_index).map(|t| t.id) else {
                return;
            };
            // Die Frage galt einer bestimmten Seite. Zeigt der Tab inzwischen
            // eine andere (oder gibt es ihn nicht mehr), geht nichts an ein
            // Modell – sonst läse es eine Seite, nach der niemand gefragt hat.
            let asked_page = message.get("page").and_then(Value::as_str);
            let gone = base.is_some_and(|id| id != base_id);
            let moved = asked_page.is_some_and(|key| win.tabs[base_index].url.split('#').next().unwrap_or("") != key);
            if gone || moved {
                win.assistant_send(json!({ "t": "aiFailed", "text": t("ai_page_changed") }));
                return;
            }
            let mut indices = vec![base_index];
            for other in wanted {
                if indices.len() >= 4 {
                    break;
                }
                if let Some(index) = win.tabs.iter().position(|t| t.id == other) {
                    if !indices.contains(&index) {
                        indices.push(index);
                    }
                }
            }

            let mut sources: Vec<Source> = Vec::new();
            for (slot, index) in indices.iter().enumerate() {
                let tab = &win.tabs[*index];
                let mut asked = false;
                if let (true, Some(webview)) = (readable(&tab.url), tab.webview.as_ref()) {
                    // In Zunders abgeschotteter Welt und als Text: Die Skripte der
                    // Seite können dem Auslesen nichts unterschieben, und der
                    // Umfang ist dort begrenzt, wo die Seite nicht hinkommt.
                    let page_proxy = proxy.clone();
                    native::eval_isolated(webview, &format!("JSON.stringify({EXTRACT_JS})"), move |json| {
                        let json = json.filter(|j| j.len() <= 8_000_000).unwrap_or_default();
                        let _ = page_proxy.send_event(UserEvent::AiPage { window, job: id, slot, json });
                    });
                    asked = true;
                }
                if asked {
                    outstanding += 1;
                }
                sources.push(Source {
                    title: tab.display_title(),
                    url: tab.url.clone(),
                    // Von einer nicht lesbaren Seite kommt auch später nichts.
                    json: if asked { None } else { Some(String::new()) },
                });
            }
            win.assistant_send(json!({
                "t": "aiStarted",
                "job": id,
                "tab": base_id,
                "engine": engine_label(&engine),
                "local": engine.is_local(),
            }));
            win.ai_job = Some(AiJob {
                id,
                tab: base_id,
                cancel: Arc::new(AtomicBool::new(false)),
                waiting: Some(Waiting { engine, action, question, history, sources, outstanding }),
            });
        }
        if outstanding > 0 {
            // Antwortet eine Seite nicht (abgestürzt, hängt fest), geht es nach
            // ein paar Sekunden ohne ihren Text weiter.
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(6));
                let _ = proxy.send_event(UserEvent::AiPage {
                    window,
                    job: id,
                    slot: usize::MAX,
                    json: String::new(),
                });
            });
        } else {
            self.ai_page_ready(window, id, usize::MAX, String::new());
        }
    }

    /// Eine Seite hat ihren Text geliefert (oder das Warten ist vorbei). Sind
    /// alle da, geht die Anfrage ans Modell.
    pub fn ai_page_ready(&mut self, window: WindowId, job: u64, slot: usize, json: String) {
        let Some(win) = self.windows.get_mut(&window) else {
            return;
        };
        let Some(current) = win.ai_job.as_mut().filter(|j| j.id == job) else {
            return;
        };
        let Some(waiting) = current.waiting.as_mut() else {
            return;
        };
        match waiting.sources.get_mut(slot) {
            Some(source) if source.json.is_none() => {
                source.json = Some(json);
                waiting.outstanding = waiting.outstanding.saturating_sub(1);
            }
            Some(_) => return,
            // Kein Platz gemeint: nicht länger warten.
            None => waiting.outstanding = 0,
        }
        if waiting.outstanding > 0 {
            return;
        }
        let Some(waiting) = current.waiting.take() else {
            return;
        };
        let cancel = current.cancel.clone();
        let mut pages = waiting
            .sources
            .iter()
            .map(|source| read_page(source.json.as_deref().unwrap_or(""), &source.title, &source.url));
        let page = pages.next().flatten();
        let others: Vec<ai::Page> = pages.flatten().collect();
        win.assistant_send(json!({
            "t": "aiContext",
            "job": job,
            "kind": match &page {
                _ if !others.is_empty() => "compare",
                Some(page) if page.is_selection => "selection",
                Some(_) => "page",
                None => "none",
            },
            "pages": others.len() + usize::from(page.is_some()),
        }));
        let request = ai::Job {
            engine: waiting.engine,
            action: waiting.action,
            question: waiting.question,
            page,
            others,
            history: waiting.history,
            german: i18n::lang() == Lang::De,
        };
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            // Text kommt Zeichen für Zeichen; an die Oberfläche geht er in
            // Portionen, höchstens etwa zwanzigmal je Sekunde.
            let buffer = RefCell::new(String::new());
            let last = Cell::new(Instant::now());
            let send = |ev: ai::Event| {
                let _ = proxy.send_event(UserEvent::Ai { window, job, ev });
            };
            ai::run(request, cancel, |event| match event {
                ai::Event::Delta(piece) => {
                    let mut pending = buffer.borrow_mut();
                    pending.push_str(&piece);
                    if last.get().elapsed() >= Duration::from_millis(45) {
                        last.set(Instant::now());
                        send(ai::Event::Delta(std::mem::take(&mut *pending)));
                    }
                }
                other => {
                    let rest = std::mem::take(&mut *buffer.borrow_mut());
                    // Vor einem Neuansatz ist der Rest hinfällig.
                    if !rest.is_empty() && !matches!(other, ai::Event::Reset) {
                        send(ai::Event::Delta(rest));
                    }
                    send(other);
                }
            });
        });
    }

    /// Zwischenstand oder Ergebnis aus dem Modell-Thread.
    pub fn ai_event(&mut self, window: WindowId, job: u64, event: ai::Event) {
        let Some(win) = self.windows.get_mut(&window) else {
            return;
        };
        if !win.ai_job.as_ref().is_some_and(|j| j.id == job) {
            return; // überholt oder angehalten
        }
        let message = match event {
            ai::Event::Status(text) => json!({ "t": "aiStatus", "job": job, "text": text }),
            ai::Event::Delta(text) => json!({ "t": "aiDelta", "job": job, "text": text }),
            ai::Event::Reset => json!({ "t": "aiReset", "job": job }),
            ai::Event::Done { text, note, asked } => {
                win.ai_job = None;
                json!({ "t": "aiDone", "job": job, "text": text, "note": note, "asked": asked })
            }
            ai::Event::Error(text) => {
                win.ai_job = None;
                json!({ "t": "aiError", "job": job, "text": text })
            }
        };
        win.assistant_send(message);
    }

    fn ai_stop(&mut self, window: WindowId) {
        // Auch eine Frage, die noch auf die Modell-Auskunft wartet, ist damit erledigt.
        self.ai_deferred.retain(|(waiting, _)| *waiting != window);
        let Some(win) = self.windows.get_mut(&window) else {
            return;
        };
        if let Some(job) = win.ai_job.take() {
            job.cancel.store(true, Ordering::Relaxed);
            win.assistant_send(json!({ "t": "aiStopped", "job": job.id }));
        }
    }

    /// Mit dem Tab endet auch die Anfrage, die zu ihm gehört.
    pub fn ai_tab_closed(&mut self, window: WindowId, tab: TabId) {
        let Some(win) = self.windows.get_mut(&window) else {
            return;
        };
        if win.ai_job.as_ref().is_some_and(|j| j.tab == tab) {
            if let Some(job) = win.ai_job.take() {
                job.cancel.store(true, Ordering::Relaxed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_text_is_tidied() {
        let raw = "  Titel\u{a0}\u{a0}der   Seite \r\n\r\n\n\n\tErster\u{200b} Absatz.  \n\n\nZweiter Absatz.\n";
        assert_eq!(tidy(raw), "Titel der Seite\n\nErster Absatz.\n\nZweiter Absatz.");
    }

    #[test]
    fn selection_wins_over_page_text() {
        let json = r#"{"text":"Der ganze lange Text der Seite.","selection":"  nur diese markierte Stelle ","lang":"de","truncated":true}"#;
        let page = read_page(json, "Titel", "https://a.de/").unwrap();
        assert!(page.is_selection);
        assert_eq!(page.text, "nur diese markierte Stelle");
        assert!(!page.truncated);
        assert_eq!(page.title, "Titel");

        let json = r#"{"text":"Der ganze Text.","selection":"kurz","lang":"en-GB","truncated":false}"#;
        let page = read_page(json, "T", "https://a.de/").unwrap();
        assert!(!page.is_selection);
        assert_eq!(page.text, "Der ganze Text.");
        assert_eq!(page.lang, "en-GB");
    }

    #[test]
    fn unreadable_answers_give_no_page() {
        assert!(read_page("", "T", "https://a.de/").is_none());
        assert!(read_page("null", "T", "https://a.de/").is_none());
        assert!(read_page(r#"{"text":"   ","selection":""}"#, "T", "https://a.de/").is_none());
        // Doppelt verpackt kommt es je nach WebKit-Version auch vor.
        let wrapped = serde_json::to_string(r#"{"text":"Inhalt der Seite","selection":""}"#).unwrap();
        assert_eq!(read_page(&wrapped, "T", "https://a.de/").unwrap().text, "Inhalt der Seite");
    }
}
