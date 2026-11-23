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
        Engine::Other(_) => "other",
    }
}

/// Dienste, die sich mit einem Klick eintragen lassen (Kennung, Name, Adresse).
const SERVICES: &[(&str, &str, &str)] = &[
    ("lmstudio", "LM Studio", "http://localhost:1234/v1"),
    ("llamacpp", "llama.cpp", "http://localhost:8080/v1"),
    ("openai", "OpenAI", "https://api.openai.com/v1"),
    ("openrouter", "OpenRouter", ai::OPENROUTER_BASE),
    ("mistral", "Mistral", "https://api.mistral.ai/v1"),
];

pub(crate) fn engine_label(engine: &Engine) -> String {
    match engine {
        Engine::Device => t("ai_e_device_label").to_string(),
        Engine::Ollama(model) => format!("Ollama · {model}"),
        Engine::Claude(model) => ai::CLAUDE_MODELS
            .iter()
            .find(|(id, _)| id == model)
            .map(|(_, name)| name.to_string())
            .unwrap_or_else(|| model.clone()),
        Engine::Other(service) => {
            let host = ai::service_base(&service.base).map(|url| url.host).unwrap_or_default();
            format!("{host} · {}", service.model)
        }
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
            "aiOtherKey" => self.ai_other_key(window, &text("key"), &text("url")),
            "aiOtherKeyDelete" => {
                self.ai_other_forget_key();
                self.ai_other_status.clear();
                self.ai_cancel_all();
                self.assistant_state_all();
            }
            "aiOtherCheck" => {
                self.ai_other_check();
                self.assistant_state_all();
            }
            "aiOAuthStart" => self.ai_oauth_start(window),
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
    // Gedächtnis
    // -----------------------------------------------------------------------

    /// Legt die aktuelle Seite ins Gedächtnis: Zunder liest ihren Text (wie für
    /// den Assistenten, abgeschottet und als Text) und sichert ihn lokal. Der
    /// Text kommt erst gleich – das Sichern erledigt `remember_store`.
    pub fn remember_page(&mut self, window: WindowId) {
        // In privaten Fenstern wird nichts gemerkt – das Gedächtnis ist dauerhaft.
        if self.windows.get(&window).is_some_and(|win| win.private) {
            self.notice(window, t("gd_private").to_string(), None, 2600);
            return;
        }
        let proxy = self.proxy.clone();
        let started = {
            let Some(win) = self.windows.get(&window) else {
                return;
            };
            let Some(tab) = win.active_tab() else {
                return;
            };
            match (readable(&tab.url), tab.webview.as_ref()) {
                (true, Some(webview)) => {
                    let title = tab.display_title();
                    let url = omnibox::bar_url(tab.shown_url());
                    native::eval_isolated(webview, &format!("JSON.stringify({EXTRACT_JS})"), move |json| {
                        let json = json.filter(|j| j.len() <= 8_000_000).unwrap_or_default();
                        let _ = proxy.send_event(UserEvent::Remembered {
                            window,
                            title: title.clone(),
                            url: url.clone(),
                            json,
                        });
                    });
                    true
                }
                _ => false,
            }
        };
        if !started {
            self.notice(window, t("mem_not_a_page").to_string(), None, 2400);
        }
    }

    /// Der Text der zu merkenden Seite ist da: als Karteikarte ins Gedächtnis
    /// (`app_records`-Kind „memory"), wie die Gedächtnis-App sie liest.
    pub fn remember_store(&mut self, window: WindowId, title: String, url: String, json: String) {
        let mut text = read_page(&json, &title, &url).map(|page| page.text).unwrap_or_default();
        if text.len() > crate::memory::TEXT_MAX {
            let mut end = crate::memory::TEXT_MAX;
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
        }
        if text.trim().is_empty() {
            self.notice(window, t("mem_empty").to_string(), None, 2400);
            return;
        }
        // Je Aufruf eine andere Kennung, auch bei zwei Seiten in derselben Millisekunde.
        static MEM_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let seq = MEM_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let id = format!("m-{now:x}{seq:x}");
        let record = json!({ "k": "m", "url": url, "title": title, "text": text, "when": now }).to_string();
        let message = match self.store.put_app_record("memory", &id, &record, crate::memory::LIMIT) {
            crate::store::RecordPut::Stored => "mem_saved",
            crate::store::RecordPut::Full => "mem_full",
            crate::store::RecordPut::Failed => "mem_failed",
        };
        self.notice(window, t(message).to_string(), None, 2400);
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
        // Seiten, deren Übersetzung darauf gewartet hat – sofern sie noch vorn
        // stehen und der Tab noch dieselbe Seite zeigt wie beim Auftrag.
        for (window, tab, url) in std::mem::take(&mut self.translate_deferred) {
            let same = self
                .windows
                .get(&window)
                .and_then(|win| win.active_tab())
                .is_some_and(|active| active.id == tab && crate::translate::page_of(&active.url) == url);
            if same {
                self.translate_page(window);
            }
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

    // --- Anderer Dienst ------------------------------------------------------

    fn ai_other_url(&self) -> Option<ai::ServiceUrl> {
        self.store.setting("ai_other_url").and_then(|url| ai::service_base(&url))
    }

    /// Gibt es einen Schlüssel – und gehört er zu genau diesem Dienst? „Genau
    /// dieser“ heißt: dieselbe Basisadresse, nicht bloß derselbe Rechner – ein
    /// anderer Port oder Pfad kann ein anderer Betreiber sein.
    fn ai_other_keyed(&self, url: &ai::ServiceUrl) -> bool {
        self.store.setting("ai_other_key").as_deref() == Some("1")
            && self.store.setting("ai_other_key_base").as_deref() == Some(url.base.as_str())
    }

    /// Vergisst den hinterlegten Schlüssel – samt dem Vermerk, wem er gehört.
    fn ai_other_forget_key(&mut self) {
        if let Some(owner) = self.store.setting("ai_other_key_base").filter(|base| !base.is_empty()) {
            ai::delete_other_key(&owner);
        }
        self.store.set_setting("ai_other_key", "0");
        self.store.set_setting("ai_other_key_base", "");
        self.store.set_setting("ai_other_key_host", "");
    }

    fn ai_other_model(&self) -> String {
        self.store.setting("ai_other_model").filter(|model| ai::model_name_ok(model)).unwrap_or_default()
    }

    fn ai_other_models(&self) -> Vec<String> {
        self.store
            .setting("ai_other_models")
            .and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok())
            .unwrap_or_default()
    }

    /// Der eingetragene Dienst, sofern alles Nötige da ist: Adresse, Modell
    /// und – außer auf diesem Mac – ein Schlüssel.
    fn ai_other_service(&self) -> Option<ai::Service> {
        let url = self.ai_other_url()?;
        let model = self.ai_other_model();
        let keyed = self.ai_other_keyed(&url);
        (!model.is_empty() && (url.local || keyed)).then(|| ai::Service { base: url.base, model, local: url.local, keyed })
    }

    /// Trägt einen Dienst ein. Wechselt dabei der Rechner, gelten weder das
    /// alte Modell noch der alte Schlüssel weiter – ein Schlüssel geht nie an
    /// einen anderen Dienst als den, für den er eingetragen wurde.
    fn ai_other_set_url(&mut self, input: &str) -> bool {
        let Some(url) = ai::service_base(input) else {
            return false;
        };
        let before = self.ai_other_url();
        if self.ai_other_status == "badurl" {
            self.ai_other_status.clear();
        }
        if before.as_ref().map(|old| old.base.as_str()) != Some(url.base.as_str()) {
            self.store.set_setting("ai_other_models", "[]");
            self.store.set_setting("ai_other_model", "");
            self.ai_other_status.clear();
        }
        if self.store.setting("ai_other_key").as_deref() == Some("1") && !self.ai_other_keyed(&url) {
            self.ai_other_forget_key();
        }
        self.store.set_setting("ai_other_url", &url.base);
        true
    }

    /// Fragt den Dienst im Hintergrund, welche Modelle er anbietet.
    fn ai_other_check(&mut self) {
        let Some(url) = self.ai_other_url() else {
            return;
        };
        let keyed = self.ai_other_keyed(&url);
        if !url.local && !keyed {
            self.ai_other_status.clear();
            return;
        }
        self.ai_other_status = "checking".to_string();
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let result = ai::other_models_for(&url, keyed);
            let _ = proxy.send_event(UserEvent::AiOther(ai::OtherEvent::Models { base: url.base, result }));
        });
    }

    pub fn ai_other_event(&mut self, event: ai::OtherEvent) {
        match event {
            ai::OtherEvent::Models { base, result } => {
                // Inzwischen ein anderer Dienst eingetragen? Dann ist die Antwort überholt.
                let Some(url) = self.ai_other_url().filter(|url| url.base == base) else {
                    return;
                };
                match result {
                    Ok(models) => {
                        self.ai_other_status = "ok".to_string();
                        // Auf diesem Mac kostet die Wahl nichts: das erste Modell nehmen,
                        // wenn noch keines gewählt ist. Im Netz wählt der Mensch.
                        if url.local && self.ai_other_model().is_empty() {
                            if let Some(first) = models.first() {
                                self.store.set_setting("ai_other_model", first);
                            }
                        }
                        self.store.set_setting("ai_other_models", &serde_json::to_string(&models).unwrap_or_default());
                    }
                    Err(401) | Err(403) => self.ai_other_status = "denied".to_string(),
                    Err(0) => self.ai_other_status = "off".to_string(),
                    // Manche Dienste haben keine Modell-Liste – nutzbar sind sie trotzdem.
                    Err(_) => self.ai_other_status = "error".to_string(),
                }
                self.assistant_state_all();
            }
            ai::OtherEvent::Key(result) => {
                let window = self.focused();
                match result {
                    Ok(key) => {
                        // Erst der Dienst, dann sein Schlüssel – unter genau dessen Adresse.
                        self.ai_other_set_url(&ai::openrouter_base());
                        let saved = self.ai_other_url().filter(|url| ai::save_other_key(&url.base, &key).is_ok());
                        let Some(url) = saved else {
                            if let Some(window) = window {
                                self.warn(window, t("ai_oauth_failed").to_string(), 9000);
                            }
                            return;
                        };
                        self.store.set_setting("ai_other_key", "1");
                        self.store.set_setting("ai_other_key_base", &url.base);
                        self.store.set_setting("ai_engine", "other");
                        // Was noch mit dem alten Stand läuft, hört auf.
                        self.ai_cancel_all();
                        self.ai_other_check();
                        if let Some(window) = window {
                            self.notice(window, t("ai_oauth_ok").to_string(), None, 9000);
                        }
                    }
                    Err(text) => {
                        if let Some(window) = window {
                            self.warn(window, text, 9000);
                        }
                    }
                }
                self.assistant_state_all();
            }
        }
    }

    /// Beginnt die Anmeldung bei OpenRouter: Die Zustimmung gibt der Mensch
    /// dort selbst, in einem gewöhnlichen Tab. Zunder bekommt danach einen
    /// eigenen Schlüssel für sein Konto – das Passwort sieht es nie.
    fn ai_oauth_start(&mut self, window: WindowId) {
        // Nicht aus einem privaten oder einem Stealth-Fenster: Der Tausch des
        // Codes gegen den Schlüssel ginge am Tunnel vorbei, und ein solches
        // Fenster richtet im Profil nichts ein.
        if self.windows.get(&window).is_none_or(|win| win.private || win.stealth) {
            return;
        }
        let Some(verifier) = ai::oauth_verifier() else {
            return;
        };
        self.ai_cancel_all();
        self.ai_other_set_url(&ai::openrouter_base());
        let Some(index) = self.new_tab(window, Some(ai::oauth_url(&verifier)), true, None) else {
            return;
        };
        let Some(tab) = self.windows.get(&window).and_then(|w| w.tabs.get(index)).map(|t| t.id) else {
            return;
        };
        self.oauth = Some(crate::app::OAuthFlow { verifier, at: Instant::now(), tab });
        native::expect_oauth(true);
        self.assistant_state_all();
    }

    /// Der Tab der Anmeldung ist bei der Rücksprung-Adresse angekommen
    /// (abgefangen in `native::nav_policy`): Code herausnehmen und einlösen.
    pub fn oauth_returned(&mut self, window: WindowId, index: usize, url: &str) {
        let Some(started) = self.oauth.as_ref() else {
            return;
        };
        let here = self.windows.get(&window).and_then(|w| w.tabs.get(index));
        let tab = here.map(|t| t.id);
        // Ein anderer Tab steuert die Rücksprung-Adresse an: Das ist nicht die
        // Anmeldung. Sein Abruf ist schon abgebrochen – die Anmeldung läuft weiter.
        if tab != Some(started.tab) {
            return;
        }
        // Der Rücksprung muss von der Seite kommen, auf der man zustimmt. Hat
        // der Tab inzwischen eine andere Website gezeigt, könnte die einen Code
        // für ein fremdes Konto unterschieben.
        let from_provider = here.is_some_and(|t| crate::shield::host_of(&t.url) == ai::oauth_host());
        let Some(flow) = self.oauth.take() else {
            return;
        };
        native::expect_oauth(false);
        let code = url::Url::parse(url)
            .ok()
            .and_then(|parsed| parsed.query_pairs().find(|(key, _)| key == "code").map(|(_, value)| value.into_owned()))
            .filter(|code| (4..=512).contains(&code.len()) && code.chars().all(|c| c.is_ascii_graphic()));
        // Nur der Tab, in dem die Anmeldung begann, und nur binnen zehn Minuten.
        let fresh = flow.at.elapsed() < Duration::from_secs(600) && from_provider;
        let (Some(code), true) = (code, fresh) else {
            self.warn(window, t("ai_oauth_failed").to_string(), 9000);
            self.assistant_state_all();
            return;
        };
        // Der Tab hat seinen Dienst getan.
        if self.windows.get(&window).is_some_and(|w| w.tabs.len() > 1) {
            self.close_tab(window, index);
        } else {
            self.navigate(window, index, crate::omnibox::NEWTAB, false);
        }
        let proxy = self.proxy.clone();
        let german = i18n::lang() == Lang::De;
        std::thread::spawn(move || {
            let result = ai::oauth_exchange(&code, &flow.verifier, german);
            let _ = proxy.send_event(UserEvent::AiOther(ai::OtherEvent::Key(result)));
        });
        self.assistant_state_all();
    }

    /// Welches Modell eine Anfrage aus einem Fenster bekommt.
    pub(crate) fn ai_engine(&self, private: bool) -> Option<Engine> {
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
            // Ein Dienst im Netz nie aus einem privaten Fenster.
            "other" => {
                if let Some(service) = self.ai_other_service().filter(|service| service.local || !private) {
                    return Some(Engine::Other(service));
                }
            }
            _ => {}
        }
        // Sonst Apples Modell – nie von selbst ein Dienst im Netz. Auch Ollama
        // nimmt Zunder nur, wenn es ausdrücklich gewählt ist: An seinem Anschluss
        // auf diesem Mac kann jedes Programm lauschen, und wer dort antwortet,
        // bekommt den Text der Seite.
        device.then_some(Engine::Device)
    }

    fn assistant_state(&mut self, window: WindowId) {
        let Some(private) = self.windows.get(&window).map(|w| w.private) else {
            return;
        };
        let probe = self.ai_probe.as_ref();
        let engine = self.ai_engine(private);
        let other_url = self.ai_other_url();
        let models = probe.and_then(|p| p.ollama.clone());
        let state = json!({
            "t": "aiState",
            "probed": probe.is_some(),
            "choice": self.ai_choice(),
            "engine": engine.as_ref().map(|e| json!({
                "id": engine_id(e),
                "label": engine_label(e),
                "local": e.is_local(),
                // Wohin der Seitentext geht, wenn nicht zu Anthropic.
                "host": match e {
                    Engine::Other(service) => ai::service_base(&service.base).map(|url| url.host),
                    _ => None,
                },
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
            "other": {
                "url": other_url.as_ref().map(|url| url.base.clone()).unwrap_or_default(),
                "host": other_url.as_ref().map(|url| url.host.clone()).unwrap_or_default(),
                "local": other_url.as_ref().is_some_and(|url| url.local),
                "key": other_url.as_ref().is_some_and(|url| self.ai_other_keyed(url)),
                "model": self.ai_other_model(),
                "models": self.ai_other_models(),
                "usable": self.ai_other_service().is_some(),
                "status": self.ai_other_status,
                "service": other_url
                    .as_ref()
                    .and_then(|url| SERVICES.iter().find(|(_, _, base)| *base == url.base))
                    .map(|(id, _, _)| *id)
                    .unwrap_or(if other_url.is_some() { "custom" } else { "" }),
                "services": SERVICES
                    .iter()
                    .map(|(id, name, base)| json!({ "id": id, "name": name, "local": base.starts_with("http://") }))
                    .collect::<Vec<_>>(),
                "openrouter": other_url.as_ref().is_some_and(|url| url.base == ai::openrouter_base()),
                "signingIn": self.oauth.as_ref().is_some_and(|flow| flow.at.elapsed() < Duration::from_secs(600)),
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
            "engine" if matches!(value, "device" | "ollama" | "claude" | "other") => {
                self.store.set_setting("ai_engine", value);
            }
            // Einen der bekannten Dienste eintragen – oder eine eigene Adresse.
            "otherService" => {
                let Some((_, _, base)) = SERVICES.iter().find(|(id, _, _)| *id == value) else {
                    return;
                };
                self.ai_other_set_url(base);
                self.ai_other_check();
            }
            "otherUrl" => {
                if !self.ai_other_set_url(value) {
                    // Sagen, warum nichts passiert ist.
                    self.ai_other_status = "badurl".to_string();
                    self.assistant_state_all();
                    return;
                }
                self.ai_other_check();
            }
            "otherModel" if ai::model_name_ok(value) => {
                self.store.set_setting("ai_other_model", value);
                // Wer ein Modell wählt, will den Dienst benutzen.
                if self.ai_other_service().is_some() {
                    self.store.set_setting("ai_engine", "other");
                }
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
        // Auch das Übersetzen: Es liefe sonst mit dem alten Dienst weiter.
        self.translate_cancel_all();
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
        self.ai_cancel_all();
        if let Some(win) = self.windows.get_mut(&window) {
            win.assistant_send(json!({ "t": "aiKeyResult", "ok": true }));
        }
        self.assistant_state_all();
    }

    /// Sichert den Schlüssel für den eingetragenen Dienst im Schlüsselbund –
    /// zusammen mit dem Vermerk, zu welchem Rechner er gehört.
    /// `intended`: die Adresse, die die Oberfläche beim Sichern gezeigt hat.
    /// Stimmt sie nicht mit der eingetragenen überein (die Adresse wurde
    /// geändert, aber nicht übernommen), wird nichts gesichert – der Schlüssel
    /// ginge sonst an den Dienst von vorher.
    fn ai_other_key(&mut self, window: WindowId, key: &str, intended: &str) {
        let key = key.trim();
        let url = self.ai_other_url();
        let plausible = (8..=400).contains(&key.len()) && key.chars().all(|c| c.is_ascii_graphic());
        let error = match &url {
            None => Some("ai_r_other_url"),
            Some(url) if url.base != intended => Some("ai_key_other_url"),
            Some(_) if !plausible => Some("ai_key_invalid"),
            Some(url) if ai::save_other_key(&url.base, key).is_err() => Some("ai_key_error"),
            Some(_) => None,
        };
        if let Some(error) = error {
            if let Some(win) = self.windows.get_mut(&window) {
                win.assistant_send(json!({ "t": "aiKeyResult", "ok": false, "text": t(error), "other": true }));
            }
            return;
        }
        if let Some(url) = url {
            self.store.set_setting("ai_other_key", "1");
            self.store.set_setting("ai_other_key_base", &url.base);
        }
        self.ai_cancel_all();
        if let Some(win) = self.windows.get_mut(&window) {
            win.assistant_send(json!({ "t": "aiKeyResult", "ok": true, "other": true }));
        }
        self.ai_other_check();
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
        // Gedächtnis-Modus: Die Frage geht nicht an die Seite vor der Leiste,
        // sondern an die gemerkten Seiten, die am besten dazu passen.
        let memory = message.get("memory").and_then(Value::as_bool).unwrap_or(false);

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
            // Im Gedächtnis-Modus spielt die Seite vor der Leiste keine Rolle.
            if (gone || moved) && !memory {
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
            if memory {
                // Die passenden gemerkten Seiten als EIN Lesetext – er liegt
                // schon auf der Platte, es muss nichts von einer Seite kommen.
                // `run` kürzt den Text später auf das, was das Modell fasst.
                let records = crate::memory::load(&self.store);
                let text = crate::memory::gather(&records, &question, 4, 40_000);
                if text.trim().is_empty() {
                    win.assistant_send(json!({ "t": "aiFailed", "text": t("mem_none") }));
                    return;
                }
                sources.push(Source {
                    title: t("mem_source_title").to_string(),
                    url: String::new(),
                    json: Some(json!({ "text": text, "truncated": false }).to_string()),
                });
            } else {
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
        // War das der Tab der Anmeldung bei OpenRouter, ist sie damit beendet.
        if self.oauth.as_ref().is_some_and(|flow| flow.tab == tab) {
            self.oauth = None;
            native::expect_oauth(false);
            self.assistant_state_all();
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
