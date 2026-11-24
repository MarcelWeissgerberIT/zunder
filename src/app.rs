//! Zustand der App und die Ereignisschleife.
//!
//! Alles, was den Zustand verändert, läuft hier auf dem Hauptthread zusammen:
//! Eingaben aus der Oberfläche (`commands.rs`), Ereignisse der Seiten
//! (`pages.rs`), Menübefehle und Zeitgeber. Die Oberfläche bekommt nach jeder
//! Runde höchstens eine Aktualisierung je Fenster.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tao::dpi::{LogicalPosition, LogicalSize};
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget};
use tao::platform::macos::{WindowBuilderExtMacOS, WindowExtMacOS};
use tao::window::{Window, WindowBuilder, WindowId};
use wry::WebView;

use crate::ai;
use crate::assistant::AiJob;
use crate::blocker::Blocker;
use crate::i18n::{self, t};
use crate::native::{self, NavEvent};
use crate::omnibox::{self, NEWTAB, NEWTAB_PRIVATE};
use crate::store::Store;
use crate::webviews::{build_chrome, build_content, rect, PageEvent, Popup, Shared};

pub type TabId = u64;
pub type Target = EventLoopWindowTarget<UserEvent>;

/// Höhe des Rahmens (Tab-Leiste plus Symbolleiste) in Punkten.
pub const CHROME_HEIGHT: f64 = 84.0;
/// Lage der Fensterknöpfe: Abstand von links und von der Oberkante – so sitzen
/// sie auf der Mittellinie der Tabs.
const LIGHTS_LEFT: f64 = 14.0;
const LIGHTS_TOP: f64 = 14.0;
/// Einzug, mit dem macOS 26 die Knöpfe auf `LIGHTS_TOP` setzt. Andere
/// Versionen rechnen anders; deshalb wird beim Anlegen eines Fensters
/// nachgemessen (`align_traffic_lights`).
const LIGHTS_INSET: f64 = 30.0;

/// Stellt den Einzug der Fensterknöpfe so ein, dass sie auf `LIGHTS_TOP`
/// sitzen. Der Zusammenhang zwischen Einzug und tatsächlicher Lage wird an
/// zwei Stellen gemessen; dazu wird das Fenster jeweils sofort neu gezeichnet,
/// denn erst dabei wendet tao den Einzug an.
fn align_traffic_lights(window: &Window, chrome: &WebView) {
    let measure = |inset: f64| -> Option<f64> {
        window.set_traffic_light_inset(LogicalPosition::new(LIGHTS_LEFT, inset));
        native::force_display(window.ns_view());
        native::traffic_light_top(chrome)
    };
    let (low, high) = (14.0, 34.0);
    let (Some(top_low), Some(top_high)) = (measure(low), measure(high)) else {
        return;
    };
    let slope = (top_high - top_low) / (high - low);
    if slope.abs() < 0.1 {
        // Die Messung greift nicht – beim erprobten Wert bleiben.
        window.set_traffic_light_inset(LogicalPosition::new(LIGHTS_LEFT, LIGHTS_INSET));
        return;
    }
    let mut inset = (low + (LIGHTS_TOP - top_low) / slope).clamp(0.0, 80.0);
    if let Some(top) = measure(inset) {
        if (top - LIGHTS_TOP).abs() >= 0.75 {
            inset = (inset + (LIGHTS_TOP - top) / slope).clamp(0.0, 80.0);
            measure(inset);
        }
    }
}
/// Zoomstufen eines Tabs.
pub const ZOOM_STEPS: [f64; 13] = [
    0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0,
];

#[derive(Debug)]
pub enum UserEvent {
    /// Befehl aus der Oberfläche eines Fensters (JSON) – und von welcher ihrer
    /// drei Flächen er kommt. Jede darf nur, was zu ihrer Aufgabe gehört.
    Ui { window: WindowId, from: crate::assets::Role, body: String },
    Page { window: WindowId, tab: TabId, ev: PageEvent },
    Nav(NavEvent),
    Menu(String),
    FindResult { window: WindowId, tab: TabId, json: String },
    Suggestions { window: WindowId, seq: u64, query: String, items: Vec<String> },
    BlockerReady,
    /// Die Filterlisten sind übersetzt oder aufgefrischt (aus dem Hintergrund).
    Filters(crate::blocker::FilterEvent),
    /// Nur aufwachen: Es gibt etwas zu zeigen (Zähler der gesperrten Abrufe).
    Wake,
    /// Das Ergebnis eines Sicherheitschecks (aus dem Hintergrund) für die App.
    Scan { window: WindowId, tab: TabId, app: &'static str, n: u32, result: serde_json::Value },
    /// Die Antwort auf eine Anfrage des Request-Inspektors (aus dem Hintergrund).
    Inspect { window: WindowId, tab: TabId, app: &'static str, n: u32, result: serde_json::Value },
    /// Ergebnis eines Dienst-Abrufs des Posteingangs (`inbox::fetch`).
    Api { window: WindowId, tab: TabId, app: &'static str, n: u32, result: serde_json::Value },
    /// Die Liste der Websites mit abgelegten Daten ist da (für die Seitenleiste).
    SiteData { window: WindowId, query: String, sites: Vec<(String, Vec<&'static str>)> },
    /// Website-Daten wurden gelöscht – die Liste neu holen.
    SiteDataChanged { window: WindowId, query: String },
    /// Im Dialog zu Kamera und Mikrofon wurde „merken“ angekreuzt.
    MediaRule { site: String, rule: crate::media::Rule },
    /// Text einer Seite für eine Anfrage an den Assistenten (JSON aus der Seite,
    /// also Material). `slot` zählt die Seiten der Anfrage durch; `usize::MAX`
    /// heißt: nicht länger warten.
    AiPage { window: WindowId, job: u64, slot: usize, json: String },
    /// Der Text einer Seite ist da, die ins Gedächtnis gelegt werden soll.
    Remembered { window: WindowId, title: String, url: String, json: String },
    /// Zwischenstand oder Ergebnis einer Anfrage an den Assistenten.
    Ai { window: WindowId, job: u64, ev: ai::Event },
    /// Welche Modelle gerade nutzbar sind.
    AiProbe(ai::Probe),
    /// Modell-Liste oder Anmeldung beim „anderen Dienst“.
    AiOther(ai::OtherEvent),
    /// WebKit hat die Website-Daten gelöscht. Die Zahl sagt, welcher Auftrag
    /// das war – auf den beim Beenden wartet die App.
    DataCleared(u64),
    /// Nachricht aus der Welt einer Seite (inject.js, Start- und Fehlerseite).
    Ping(native::PagePing),
    /// Das Vorschaubild eines Tabs für die Übersicht ist fertig.
    Thumb { window: WindowId, tab: TabId, bytes: Vec<u8> },
    /// Die Seite konnte nicht selbst noch einmal losgehen (bereinigte Adresse):
    /// Der Browser lädt sie dann still an ihrer Stelle.
    Reissue { window: WindowId, tab: TabId, url: String },
    /// Nachricht von Zunders eigenem Skript in einer Seite (Passwort-Tresor).
    Bridge(native::BridgeMessage),
    /// macOS hat den Standort genannt oder verweigert – für die eingebauten Apps.
    Geo(crate::geo::Fix),
    /// Antwort auf eine Abfrage per Touch ID bzw. Passwort des Macs.
    Auth { token: u64, ok: bool },
    /// Ergebnis des Ausfüllens in einer Seite.
    Filled { window: WindowId, tab: TabId, login: i64, result: Option<String> },
    /// Ein vorgeschlagenes Passwort wurde angenommen: für welche Herkunft es
    /// angeboten wurde und welcher Name im Formular steht.
    Suggested { window: WindowId, tab: TabId, origin: String, user: String, password: String },
    /// Im Dateidialog wurde gewählt (oder abgebrochen).
    FileChosen { window: WindowId, purpose: &'static str, path: Option<String> },
    /// Ein kopiertes Passwort soll wieder aus der Zwischenablage verschwinden.
    ClipboardExpire(isize),
    /// Ergebnis einer Abfrage an Zunders Skript in einer Seite (Lesemodus).
    PageAnswer { window: WindowId, tab: TabId, what: &'static str, result: Option<String> },
    /// Seite übersetzen: Die Textstellen sind gesammelt oder eine Portion ist übersetzt.
    Translate(crate::translate::TranslateEvent),
    /// Seitencheck: Aufbau der Seite oder Merkmale ihrer Cookies sind da.
    SiteCheck(crate::sitecheck::Event),
    /// Zeile von der Standardeingabe (nur mit ZUNDER_DEBUG).
    Debug(String),
    DebugOutput(String),
}

/// Eine Tab-Gruppe: ein Name, eine Farbe, und sie lässt sich zusammenklappen.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TabGroup {
    pub id: u32,
    pub name: String,
    /// Eine von acht Farben der Oberfläche (0–7).
    pub color: u8,
    #[serde(default)]
    pub collapsed: bool,
}

/// Ein Container: ein Name, eine Farbe und eine Kennung für seinen eigenen,
/// dauerhaften Datenspeicher. Tabs in diesem Container sind von allem anderen
/// getrennt – so lassen sich etwa zwei Konten derselben Website gleichzeitig
/// offen halten. Dauerhaft gesichert (`app_records`), gilt für alle Fenster.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Container {
    pub id: u32,
    pub name: String,
    /// Eine von acht Farben der Oberfläche (0–7).
    pub color: u8,
    /// Kennung (UUID) des dauerhaften WebKit-Datenspeichers dieses Containers.
    pub uuid: String,
}

impl Container {
    /// Lädt die gesicherten Container, nach Kennung geordnet.
    fn load(store: &Store) -> Vec<Container> {
        let mut list: Vec<Container> = Vec::new();
        if let Some((records, _)) = store.app_records("container", "", 64, 64 * 1024) {
            for (_id, data) in records {
                if let Ok(container) = serde_json::from_str::<Container>(&data) {
                    if !container.uuid.is_empty() {
                        list.push(container);
                    }
                }
            }
        }
        list.sort_by_key(|container| container.id);
        list
    }
}

pub struct Tab {
    pub id: TabId,
    /// Angeheftet: steht schmal ganz links und kommt nach jedem Start wieder.
    pub pinned: bool,
    /// Gehört zu dieser Tab-Gruppe des Fensters.
    pub group: Option<u32>,
    /// Läuft in diesem Container – mit eigenem, dauerhaftem Datenspeicher, von
    /// allem anderen getrennt. `None`: das normale Profil. Nur in gewöhnlichen
    /// Fenstern (private und Stealth-Fenster haben ohnehin eigene Speicher).
    pub container: Option<u32>,
    /// Entsteht erst, wenn der Tab zum ersten Mal gezeigt wird.
    pub webview: Option<WebView>,
    /// Zuletzt übernommene Adresse – oder die, die beim ersten Zeigen geladen wird.
    pub url: String,
    /// Adresse, die gerade geladen wird, aber noch nicht übernommen ist.
    pub pending: Option<String>,
    /// Die Adressleiste darf die ladende Adresse schon zeigen: Der Browser
    /// selbst hat dorthin geschickt (Eingabe, Lesezeichen, neuer Tab). Schickt
    /// eine Seite woandershin, bleibt bis zur Übernahme die alte Adresse
    /// stehen – sonst stünde über ihrem Inhalt eine fremde Adresse.
    pub pending_shown: bool,
    pub title: String,
    pub favicon: Option<String>,
    pub loading: bool,
    pub progress: f64,
    pub zoom: f64,
    /// Die laufende Navigation wurde in der Adressleiste eingegeben.
    pub typed: bool,
    /// Läuft gerade der Versuch, eine unverschlüsselte Adresse verschlüsselt zu laden?
    pub upgrade: Option<Upgrade>,
    /// Wie oft kurz hintereinander für denselben Rechner (Rechner, Anzahl, seit wann) –
    /// damit eine Seite, die immer wieder ins Unverschlüsselte zurückschickt, keine Schleife dreht.
    pub upgrade_tries: (String, u8, Instant),
    pub opener: Option<TabId>,
    /// Zählt Takte, in denen WebKit nichts mehr lädt, der Tab aber noch als ladend gilt.
    pub idle_ticks: u8,
    /// Wann der Tab zuletzt sichtbar war – für das Entladen ungenutzter Tabs.
    pub last_used: Instant,
    /// Was der Tab über Anmeldungen auf seiner Seite weiß (Passwort-Tresor).
    pub login: crate::logins::TabLogin,
    /// Die Seite wird gerade im Lesemodus gezeigt.
    pub reader: bool,
    /// Die Seite hat genug Fließtext, dass sich der Lesemodus lohnt (Hinweis des Skripts).
    pub readable: bool,
    /// Der Tab gibt sich als iPhone aus – Seiten liefern dann ihre mobile Fassung.
    pub mobile: bool,
    /// Wie oft kurz hintereinander Tracking-Anhängsel entfernt wurden – hängt ein
    /// Server sie hartnäckig wieder an, wird die Adresse einmal gelassen, wie sie ist.
    pub rewrites: (u8, Instant),
    /// Stealth-Fenster: was die Seite über das Gerät abgefragt hat, je Rahmen
    /// (Rechnername; leer steht für die Seite selbst) und Art. Siehe `ui/watch.js`.
    pub asked: HashMap<String, std::collections::BTreeMap<&'static str, u32>>,
    /// Bremse für diese Meldungen: Beginn der laufenden Sekunde und Zahl darin.
    pub asked_rate: (Instant, u32),
    /// Websites, bei denen in diesem Tab ein Passwort eingegeben wurde – dort
    /// weiß die Website, wer man ist, gleich wie gut alles andere getarnt ist.
    pub signed_in: HashSet<String>,
    /// Die Sprache, die die Seite über sich angibt (`lang` am Wurzelelement) –
    /// nur ein Hinweis: Danach bietet die Adressleiste das Übersetzen an.
    pub lang: String,
    /// Die Seite wird gerade übersetzt oder steht übersetzt da.
    pub translation: Option<crate::translate::Translation>,
    /// Seitencheck: was über die Angriffsflächen der gezeigten Seite bekannt ist.
    pub check: crate::sitecheck::State,
}

impl Tab {
    pub fn new(id: TabId, url: String) -> Self {
        Self {
            id,
            pinned: false,
            group: None,
            container: None,
            webview: None,
            url,
            pending: None,
            pending_shown: false,
            title: String::new(),
            favicon: None,
            loading: false,
            progress: 0.0,
            zoom: 1.0,
            typed: false,
            upgrade: None,
            upgrade_tries: (String::new(), 0, Instant::now()),
            opener: None,
            idle_ticks: 0,
            last_used: Instant::now(),
            login: Default::default(),
            reader: false,
            readable: false,
            mobile: false,
            rewrites: (0, Instant::now()),
            asked: HashMap::new(),
            asked_rate: (Instant::now(), 0),
            signed_in: HashSet::new(),
            lang: String::new(),
            translation: None,
            check: crate::sitecheck::State::default(),
        }
    }

    /// Die Adresse, die in der Adressleiste steht.
    pub fn shown_url(&self) -> &str {
        match (&self.pending, self.pending_shown) {
            (Some(pending), true) => pending,
            _ => &self.url,
        }
    }

    pub fn display_title(&self) -> String {
        if !self.title.is_empty() {
            return self.title.clone();
        }
        let shown = self.shown_url();
        // Ein leeres Fenster, das eine Seite geöffnet hat, ist keine Startseite
        // – und soll auch nicht so heißen.
        if shown.is_empty() && self.opener.is_some() {
            return "about:blank".to_string();
        }
        if omnibox::is_newtab(shown) || shown.is_empty() {
            return t("new_tab_title").to_string();
        }
        let pretty = omnibox::pretty_url(shown);
        if pretty.is_empty() {
            t("untitled").to_string()
        } else {
            pretty
        }
    }
}

/// Der Versuch, statt einer unverschlüsselten Adresse die verschlüsselte zu laden.
pub struct Upgrade {
    pub host: String,
    /// Die ursprüngliche, unverschlüsselte Adresse.
    pub plain: String,
    pub at: Instant,
}

/// Ein Browserfenster. Die Reihenfolge der Felder ist auch die Reihenfolge
/// beim Aufräumen: erst die Tabs, dann der Rahmen, zuletzt das Fenster selbst.
pub struct BrowserWindow {
    /// Die Tab-Gruppen dieses Fensters.
    pub groups: Vec<TabGroup>,
    /// Stealth-Fenster (immer auch privat) – und ob es über Tor läuft.
    pub stealth: bool,
    pub tor: bool,
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub chrome: WebView,
    /// Seitenleiste des Assistenten; entsteht beim ersten Öffnen.
    pub assistant: Option<WebView>,
    pub assistant_open: bool,
    pub assistant_ready: bool,
    pub assistant_queue: Vec<String>,
    /// Zuletzt gemeldete Seite – damit dieselbe Meldung nicht doppelt hinausgeht.
    pub assistant_page: String,
    /// Laufende Anfrage an den Assistenten.
    pub ai_job: Option<AiJob>,
    /// Sprechblase des Passwort-Tresors (Vorschläge am Feld, Frage „sichern?“);
    /// entsteht beim ersten Gebrauch.
    pub bubble: Option<WebView>,
    pub bubble_ready: bool,
    pub bubble_queue: Vec<String>,
    pub bubble_state: Option<crate::logins::BubbleState>,
    pub bubble_height: f64,
    pub bubble_seq: u64,
    /// Zwei Tabs nebeneinander (links, rechts) – „Compared Browsing“. Beide sind
    /// sichtbar; aktiv ist der, in dem gerade gearbeitet wird.
    pub split: Option<(TabId, TabId)>,
    pub shared: Rc<Shared>,
    pub private: bool,
    /// Der Rahmen deckt gerade das ganze Fenster ab (Menü, Vorschläge, Seitenleiste).
    pub overlay: bool,
    /// Beim Start: Die Seiten bleiben unsichtbar, bis das Ladebild zu sehen ist.
    pub hold_content: bool,
    /// Die Tarn-Ampel hat Neues zu zeigen (Abfragen der Seite).
    pub ampel_pending: bool,
    /// Welche Zahl gesperrter Abrufe das Schild gerade zeigt.
    pub blocked_shown: u32,
    /// Aktuelle Höhe des Rahmens – mit Suchleiste oder Hinweis höher als normal.
    pub chrome_height: f64,
    pub fullscreen: bool,
    pub ui_ready: bool,
    pub dirty: bool,
    pub last_progress: f64,
    /// Zuletzt gesetzter Fenstertitel (für Mission Control und das Fenster-Menü).
    pub window_title: String,
    queue: Vec<String>,
    pub window: Rc<Window>,
}

impl BrowserWindow {
    /// Schickt eine Nachricht an die Oberfläche dieses Fensters.
    pub fn send(&mut self, message: Value) {
        let script = format!("window.__zunder&&window.__zunder({message})");
        if self.ui_ready {
            let _ = self.chrome.evaluate_script(&script);
        } else {
            self.queue.push(script);
        }
    }

    pub fn ui_became_ready(&mut self) {
        self.ui_ready = true;
        for script in std::mem::take(&mut self.queue) {
            let _ = self.chrome.evaluate_script(&script);
        }
        self.dirty = true;
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    pub fn active_webview(&self) -> Option<&WebView> {
        self.active_tab().and_then(|t| t.webview.as_ref())
    }

    pub fn size(&self) -> (f64, f64) {
        let size = self.window.inner_size().to_logical::<f64>(self.window.scale_factor());
        (size.width, size.height)
    }

    /// Breite der Seitenleiste des Assistenten; 0, solange sie zu ist.
    pub fn assistant_width(&self) -> f64 {
        if !self.assistant_open {
            return 0.0;
        }
        let (w, _) = self.size();
        if w < 720.0 {
            // Schmales Fenster: die Hälfte, aber nie weniger als lesbar.
            (w * 0.5).max(300.0).min(w)
        } else {
            (w * 0.3).clamp(340.0, 440.0)
        }
    }

    /// Gehört der Tab zum nebeneinander gezeigten Paar?
    pub fn in_split(&self, tab: TabId) -> bool {
        self.split.is_some_and(|(left, right)| left == tab || right == tab)
    }

    /// Fläche, auf der ein Tab gezeigt wird (x, y, Breite, Höhe): die ganze
    /// Seitenfläche – oder ihre linke bzw. rechte Hälfte, wenn er zum Paar gehört.
    pub fn pane_frame(&self, tab: TabId) -> (f64, f64, f64, f64) {
        let (w, h) = self.size();
        let width = w - self.assistant_width();
        let height = h - self.chrome_height;
        match self.split {
            Some((left, right)) if tab == left || tab == right => {
                // Ein Punkt bleibt dazwischen frei – als Trennlinie.
                let half = (width / 2.0).floor();
                if tab == left {
                    (0.0, self.chrome_height, half - 1.0, height)
                } else {
                    (half, self.chrome_height, width - half, height)
                }
            }
            _ => (0.0, self.chrome_height, width, height),
        }
    }

    pub fn pane_rect(&self, tab: TabId) -> wry::Rect {
        let (x, y, width, height) = self.pane_frame(tab);
        rect(x, y, width, height)
    }

    pub fn assistant_rect(&self) -> wry::Rect {
        let (w, h) = self.size();
        let width = self.assistant_width();
        rect(w - width, self.chrome_height, width, h - self.chrome_height)
    }
}

impl BrowserWindow {
    /// Stellt die Ordnung der Tab-Leiste her: Angeheftete stehen vorn (und
    /// gehören keiner Gruppe an), die Tabs einer Gruppe stehen beieinander,
    /// leere Gruppen verschwinden, und der aktive Tab ist nie weggeklappt.
    pub fn tidy_tabs(&mut self) {
        if self.tabs.is_empty() {
            return;
        }
        let active_id = self.tabs[self.active.min(self.tabs.len() - 1)].id;
        let known: HashSet<u32> = self.groups.iter().map(|group| group.id).collect();
        for tab in &mut self.tabs {
            if tab.pinned || tab.group.is_some_and(|group| !known.contains(&group)) {
                tab.group = None;
            }
        }
        let (pinned, rest): (Vec<Tab>, Vec<Tab>) = std::mem::take(&mut self.tabs).into_iter().partition(|tab| tab.pinned);
        let mut ordered = pinned;
        let mut waiting: Vec<Option<Tab>> = rest.into_iter().map(Some).collect();
        for index in 0..waiting.len() {
            let Some(tab) = waiting[index].take() else {
                continue;
            };
            let group = tab.group;
            ordered.push(tab);
            if group.is_some() {
                // Die übrigen Tabs derselben Gruppe rücken hier auf.
                for later in waiting.iter_mut().skip(index + 1) {
                    if later.as_ref().is_some_and(|tab| tab.group == group) {
                        ordered.extend(later.take());
                    }
                }
            }
        }
        self.tabs = ordered;
        let used: HashSet<u32> = self.tabs.iter().filter_map(|tab| tab.group).collect();
        self.groups.retain(|group| used.contains(&group.id));
        self.active = self.tabs.iter().position(|tab| tab.id == active_id).unwrap_or(0);
        if let Some(group) = self.tabs[self.active].group {
            if let Some(group) = self.groups.iter_mut().find(|g| g.id == group) {
                group.collapsed = false;
            }
        }
    }
}

pub struct Settings {
    pub engine: String,
    pub suggest: bool,
    pub restore: bool,
    pub blocker: bool,
    /// Lange ungenutzte Tabs entladen.
    pub saver: bool,
    /// Sicherheit – siehe `shield.rs` und `native::Guards`.
    pub https_only: bool,
    pub strip_tracking: bool,
    pub hardened: bool,
    /// Vor Adressen warnen, die einer bekannten Seite zum Verwechseln ähneln.
    pub lookalike: bool,
    /// Websites aus dem Internet vom eigenen Netz fernhalten.
    pub local_net: bool,
    /// Entwicklerwerkzeuge („Element untersuchen“) in Seiten anbieten.
    pub devtools: bool,
    /// Die Lesezeichenleiste unter der Adresszeile zeigen.
    pub bm_bar: bool,
    /// Beim Start das Ladebild zeigen (fünf Sekunden Animation).
    pub splash: bool,
    /// Die Lesezeichen als Kacheln auf der Startseite zeigen (der
    /// „Schreibtisch“). Von Haus aus aus: Die Startseite bleibt leer und ruhig.
    pub start_bookmarks: bool,
    /// Die Filterlisten des Werbeblockers von selbst auffrischen.
    pub filter_update: bool,
    /// Vor gemeldeten Betrugs- und Schadseiten schützen.
    pub guard: bool,
    /// Beim Beenden Verlauf und Website-Daten löschen.
    pub clear_on_quit: bool,
    /// Nach dem Anmelden fragen, ob das Passwort gesichert werden soll.
    /// (Ob vor dem Ausfüllen Touch ID verlangt wird, steht nicht hier, sondern
    /// beim Schlüssel im Schlüsselbund – siehe `vault.rs`.)
    pub pw_offer: bool,
}

impl Settings {
    pub fn guards(&self) -> native::Guards {
        native::Guards {
            https_only: self.https_only,
            strip_tracking: self.strip_tracking,
            hardened: self.hardened,
            lookalike: self.lookalike,
            local_net: self.local_net,
            devtools: self.devtools,
        }
    }

    fn load(store: &Store) -> Self {
        let flag = |key: &str, default: bool| {
            store
                .setting(key)
                .map(|v| v == "1")
                .unwrap_or(default)
        };
        Self {
            engine: store.setting("engine").unwrap_or_else(|| "google".into()),
            suggest: flag("suggest", true),
            restore: flag("restore", true),
            blocker: flag("blocker", true),
            saver: flag("saver", true),
            https_only: flag("https_only", true),
            strip_tracking: flag("strip_tracking", true),
            hardened: flag("hardened", false),
            lookalike: flag("lookalike", true),
            local_net: flag("local_net", true),
            devtools: flag("devtools", false),
            bm_bar: flag("bm_bar", false),
            splash: flag("splash", true),
            start_bookmarks: flag("start_bookmarks", false),
            filter_update: flag("filter_update", true),
            guard: flag("guard", true),
            clear_on_quit: flag("clear_on_quit", false),
            pw_offer: flag("pw_offer", true),
        }
    }
}

pub struct ActiveDownload {
    /// Kennung in der Download-Liste; 0 in privaten Fenstern (dort wird nichts vermerkt).
    pub id: i64,
    pub url: String,
    pub path: PathBuf,
}

/// Eine laufende Anmeldung bei OpenRouter: das Geheimnis, mit dem allein sich
/// der Einmal-Code einlösen lässt, und der Tab, in dem sie stattfindet.
pub struct OAuthFlow {
    pub verifier: String,
    pub at: Instant,
    pub tab: TabId,
}

/// Was der Knopf eines Hinweises auslöst.
pub enum NoticeAction {
    OpenExternal(String),
    Reveal(PathBuf),
    /// Die Entwicklerwerkzeuge einschalten und gleich öffnen.
    EnableDevtools,
    /// Einer Website den Zugriff auf Geräte im eigenen Netz erlauben (Fenster,
    /// Tab, Website) – danach lädt der Tab neu.
    AllowLocal(WindowId, TabId, String),
    /// Eine Adresse im eigenen Netz, die eine Website in einem neuen Fenster
    /// öffnen wollte, nach Zustimmung in einem neuen Tab öffnen.
    OpenTab(String),
    /// Den Werbeblocker für eine Website abschalten, die Besucher mit Blocker
    /// aussperrt (Fenster, Tab, Website) – der Tab geht danach zu ihrer Startseite.
    BlockerOff(WindowId, TabId, String),
    /// Das Übersetzen einer Seite abbrechen bzw. ihr Original zurückholen (Fenster, Tab).
    TranslateUndo(WindowId, TabId),
    /// Einer eingebauten App den Standort erlauben (Fenster, Tab, App, Nummer
    /// ihrer Anfrage – unter der bekommt sie die Antwort).
    AppLocation(WindowId, TabId, &'static str, u32),
}

#[derive(Serialize, Deserialize)]
struct SessionTab {
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    group: Option<u32>,
    #[serde(default)]
    container: Option<u32>,
}

#[derive(Serialize, Deserialize)]
struct SessionWindow {
    tabs: Vec<SessionTab>,
    #[serde(default)]
    groups: Vec<TabGroup>,
    #[serde(default)]
    active: usize,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[derive(Serialize, Deserialize, Default)]
struct Session {
    windows: Vec<SessionWindow>,
}

pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

pub struct App {
    pub proxy: EventLoopProxy<UserEvent>,
    pub store: Store,
    pub settings: Settings,
    /// Die eingerichteten Container (Name, Farbe, Datenspeicher-Kennung).
    pub containers: Vec<Container>,
    pub windows: HashMap<WindowId, BrowserWindow>,
    /// Fenster in der Reihenfolge ihrer letzten Benutzung, das aktuelle zuletzt.
    pub focus_order: Vec<WindowId>,
    pub ids: Rc<Cell<TabId>>,
    pub popups: Rc<RefCell<Vec<Popup>>>,
    pub newtab: Arc<Mutex<String>>,
    /// Zuletzt geschlossene Tabs (Adresse, Titel) zum Wiederöffnen.
    pub closed: Vec<(String, String)>,
    pub active_downloads: Vec<ActiveDownload>,
    pub notices: HashMap<u64, NoticeAction>,
    /// Zu welcher Seite ein Hinweis mit Knopf gehört (Fenster, Tab, Website).
    /// Wechselt der Tab die Website, geht er zu oder nach hinten, hat sich der
    /// Hinweis erledigt – sonst gälte ein Klick einer anderen Seite als der,
    /// die gefragt hat.
    pub notice_owners: HashMap<u64, (WindowId, TabId, String)>,
    pub next_notice: u64,
    /// Zählt die Aufträge zum Übersetzen einer Seite durch.
    pub next_translation: u64,
    /// Tabs, deren Übersetzung wartet, bis bekannt ist, welche Modelle es gibt.
    pub translate_deferred: Vec<(WindowId, TabId, String)>,
    /// Zählt die Läufe des Seitenchecks durch.
    pub next_check: u64,
    pub blocker: Blocker,
    pub ua_suffix: String,
    pub data_dir: PathBuf,
    pub session_dirty: Option<Instant>,
    pub last_tick: Instant,
    pub last_sweep: Instant,
    /// Nach dieser Zeit ohne Benutzung wird ein Hintergrund-Tab entladen.
    pub unload_after: Duration,
    /// Welche Modelle der Assistent nutzen kann – erst bekannt, wenn er zum
    /// ersten Mal geöffnet wurde.
    pub ai_probe: Option<ai::Probe>,
    pub ai_probe_at: Option<Instant>,
    pub ai_probing: bool,
    /// Stand der letzten Prüfung des „anderen Dienstes“ (leer, `checking`, `ok`, `off`, `denied`, `error`).
    pub ai_other_status: String,
    /// Eine laufende Anmeldung bei OpenRouter.
    pub oauth: Option<OAuthFlow>,
    /// Fragen, die warten, bis die Modelle festgestellt sind.
    pub ai_deferred: Vec<(WindowId, Value)>,
    pub ai_next_job: u64,
    /// Das Löschen beim Beenden läuft; danach endet die App.
    pub quitting: bool,
    pub quit: bool,
    /// Passwort-Tresor: hält den Schlüssel, sobald er einmal gebraucht wurde.
    pub vault: crate::vault::Vault,
    /// Was nach bestandener Abfrage per Touch ID geschehen soll – und wofür sie gilt.
    pub auth_waiting: HashMap<u64, (crate::logins::AuthThen, crate::logins::Gate)>,
    pub next_auth: u64,
    /// Bis wann nach einer Abfrage ohne erneute ausgefüllt wird (Anmeldungen in zwei Schritten).
    pub fill_unlocked_until: Option<Instant>,
    /// Bis wann die Verwaltung Passwörter zeigt, kopiert und ändert.
    pub panel_unlocked_until: Option<Instant>,
    /// Bis wann die eine freigegebene Ausfuhr ihr Ziel wählen darf.
    pub export_until: Option<Instant>,
    /// Zählerstand der Zwischenablage nach dem Kopieren eines Passworts.
    pub clipboard_count: Option<isize>,
    /// Websites, die Geräte im eigenen Netz ansprechen dürfen.
    pub local_sites: Vec<String>,
    /// Dasselbe, aber nur bis zum Beenden: Erlaubnisse aus privaten und
    /// Stealth-Fenstern. Sie stehen in keiner Einstellung.
    pub local_sites_session: Vec<String>,
    /// Websites, auf denen der Werbeblocker abgeschaltet ist (gespeichert –
    /// was nur in einem privaten Fenster abgeschaltet wurde, steht hier nicht).
    pub blocker_off: Vec<String>,
    /// Was sich Zunder je Website zu Kamera und Mikrofon gemerkt hat.
    pub media_sites: crate::media::Rules,
    /// Das nächste Fenster, das entsteht, wird ein Stealth-Fenster.
    pub stealth_next: bool,
    /// Websites, bei denen der Hinweis auf ihre Blocker-Sperre schon kam.
    pub wall_noticed: HashSet<String>,
    /// Wann zuletzt geprüft wurde, ob die Filterlisten aufzufrischen sind.
    pub last_filter_check: Instant,
    /// Websites, deren verhinderter Zugriff ins eigene Netz schon gemeldet wurde.
    pub local_noticed: HashSet<String>,
    /// Das Wissen über bekannte Seiten ist überholt (Lesezeichen, Passwörter
    /// oder Verlauf haben sich geändert).
    pub sites_stale: bool,
    /// Wovor in dieser Sitzung schon gewarnt wurde: (Website, Kennung der Anmeldung).
    pub reuse_warned: HashSet<(String, i64)>,
    /// Wann zuletzt nach dem Öffnen eines anderen Programms gefragt wurde.
    pub last_external: Option<Instant>,
    /// Tabs, die neu laden, sobald die Schutzlisten neu übersetzt sind.
    pub reload_when_ready: Vec<(WindowId, TabId)>,
    /// Wann zuletzt gemeldet wurde, dass ein Download nicht zugelassen wurde.
    pub last_refusal: Option<Instant>,
    /// Websites, bei denen schon gemeldet wurde, dass Zunder ein eingebettetes
    /// PDF nicht ungefragt heruntergeladen hat (Stealth-Fenster für sich gezählt).
    pub frame_files_noticed: HashSet<(bool, String)>,
    /// Eingebaute Apps, die auf den Standort warten: Fenster, Tab, App und die
    /// Nummer ihrer Anfrage.
    pub geo_waiters: Vec<(WindowId, TabId, &'static str, u32)>,
    /// Hinweise dieser Art, die gerade zu sehen sind oder warten: Kennung →
    /// (Website, seit wann). Verdrängt einen gleich ein anderer, oder kommt
    /// er gar nicht erst dran, zählt er nicht als gezeigt.
    pub frame_file_notices: HashMap<u64, ((bool, String), Instant)>,
    /// Quelltexte, die gerade in einem Tab angezeigt werden (nur im Arbeitsspeicher).
    pub sources: crate::assets::Sources,
    pub next_source: u64,
    /// Vorschaubilder der Tabs für die Übersicht (nur im Arbeitsspeicher).
    pub thumbs: crate::assets::Thumbs,
    pub thumb_serial: u64,
    /// Die App fährt gerade hoch: Fenster, die jetzt entstehen, beginnen mit dem Ladebild.
    pub starting: bool,
    _menu: muda::Menu,
}

static STARTED: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// Aufträge zum Löschen der Website-Daten: ein gewöhnlicher und der beim Beenden.
pub const CLEAR_PLAIN: u64 = 0;
const CLEAR_ON_QUIT: u64 = 1;

/// Prüfhilfe: schreibt, wie viele Millisekunden seit dem Start vergangen sind.
pub fn debug_timing(label: &str) {
    static DEBUG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *DEBUG.get_or_init(crate::dev::enabled) {
        if let Some(started) = STARTED.get() {
            println!("TIMING {label} {} ms", started.elapsed().as_millis());
            let _ = std::io::stdout().flush();
        }
    }
}

pub fn run() {
    let _ = STARTED.set(Instant::now());
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let menu_proxy = proxy.clone();
    muda::MenuEvent::set_event_handler(Some(move |event: muda::MenuEvent| {
        let _ = menu_proxy.send_event(UserEvent::Menu(event.id.0));
    }));

    let mut app: Option<App> = None;
    // Adressen von der Befehlszeile oder aus „Öffnen mit“, bevor die App steht.
    let mut early_urls: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .collect();

    event_loop.run(move |event, target, control_flow| {
        match event {
            Event::NewEvents(StartCause::Init) => {
                debug_timing("schleife_gestartet");
                let mut created = App::new(proxy.clone());
                debug_timing("app_bereit");
                created.start(target, std::mem::take(&mut early_urls));
                app = Some(created);
            }
            Event::Opened { urls } => {
                let urls: Vec<String> = urls.into_iter().map(|u| u.to_string()).collect();
                match app.as_mut() {
                    Some(app) => app.open_urls(target, urls),
                    None => early_urls.extend(urls),
                }
            }
            other => {
                if let Some(app) = app.as_mut() {
                    app.handle(other, target);
                }
            }
        }
        *control_flow = match app.as_ref() {
            Some(app) => app.control_flow(),
            None => ControlFlow::Wait,
        };
    });
}

/// Der Benutzerordner außerhalb der Sandbox. In der Sandbox zeigt `HOME` auf
/// den Behälter der App (…/Library/Containers/<Kennung>/Data); was davor
/// steht, ist der eigentliche Benutzerordner. Ohne Sandbox: `None`.
pub fn home_outside_sandbox() -> Option<PathBuf> {
    std::env::var_os("APP_SANDBOX_CONTAINER_ID")?;
    let home = std::env::var_os("HOME")?;
    let home = home.to_string_lossy();
    let (real, _) = home.split_once("/Library/Containers/")?;
    Some(PathBuf::from(real))
}

/// Einmal nach dem Umzug in die Sandbox: das Profil von vorher herüberholen
/// (Verlauf, Lesezeichen, gesicherte Anmeldungen, letzte Sitzung). Lesen darf
/// die App den alten Ordner eigens dafür – siehe assets/Zunder.entitlements.
fn adopt_old_profile(new_dir: &std::path::Path) {
    if crate::dev::BUILT_IN || new_dir.join("zunder.db").exists() {
        return;
    }
    let Some(old) = home_outside_sandbox().map(|home| home.join("Library/Application Support/Zunder")) else {
        return;
    };
    if !old.join("zunder.db").is_file() {
        return;
    }
    let _ = std::fs::create_dir_all(new_dir);
    for name in ["zunder.db", "zunder.db-wal", "zunder.db-shm", "session.json"] {
        let _ = std::fs::copy(old.join(name), new_dir.join(name));
    }
}

fn data_dir() -> PathBuf {
    if let Some(dir) = crate::dev::build_var_os("ZUNDER_PROFILE_DIR") {
        return PathBuf::from(dir);
    }
    // Ein Bau mit Prüfhilfe arbeitet nie im Profil der echten App.
    let name = if crate::dev::BUILT_IN { "Zunder-Probe" } else { "Zunder" };
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join(name)
}

/// Macht aus einer gespeicherten oder übergebenen Adresse eine, die sicher
/// geladen werden kann; alles Unbrauchbare wird zur Startseite.
fn loadable(url: &str, private: bool) -> String {
    let blank = if private { NEWTAB_PRIVATE } else { NEWTAB };
    if url.is_empty() || omnibox::is_newtab(url) {
        return blank.to_string();
    }
    // Eine eingebaute App – unter genau ihrer Adresse.
    if crate::apps::is_root(url) {
        return url.to_string();
    }
    let url = omnibox::error_retry(url).unwrap_or_else(|| url.to_string());
    match url::Url::parse(&url) {
        // Von den internen Seiten lädt ein Tab von sich aus nur die Startseite.
        Ok(parsed) if matches!(parsed.scheme(), "http" | "https" | "file" | "about" | "data") => parsed.to_string(),
        _ => blank.to_string(),
    }
}

impl App {
    fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        native::set_proxy(proxy.clone());
        i18n::init(&native::preferred_language());

        let data_dir = data_dir();
        adopt_old_profile(&data_dir);
        let store = Store::open(&data_dir).unwrap_or_else(|_| {
            // Eine beschädigte Datenbank soll den Browser nicht für immer am
            // Start hindern: Sie wird zur Seite gelegt, und Zunder beginnt neu.
            let stamp = crate::store::now();
            for name in ["zunder.db", "zunder.db-wal", "zunder.db-shm"] {
                let _ = std::fs::rename(data_dir.join(name), data_dir.join(format!("{name}.defekt-{stamp}")));
            }
            Store::open(&data_dir).expect("Datenbank lässt sich nicht anlegen")
        });
        let settings = Settings::load(&store);
        let containers = Container::load(&store);
        let debug = crate::dev::enabled();
        if debug {
            let debug_proxy = proxy.clone();
            std::thread::spawn(move || {
                for line in std::io::stdin().lock().lines().map_while(Result::ok) {
                    // "sleep <ms>" taktet die folgenden Befehle.
                    if let Some(ms) = line.strip_prefix("sleep ") {
                        std::thread::sleep(Duration::from_millis(ms.trim().parse().unwrap_or(0)));
                        continue;
                    }
                    if debug_proxy.send_event(UserEvent::Debug(line)).is_err() {
                        break;
                    }
                }
            });
        }

        native::set_guards(settings.guards());
        let local_sites: Vec<String> = store
            .setting("local_sites")
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        native::set_local_sites(HashSet::new());
        let local_sites: Vec<String> = local_sites.into_iter().filter(|site| native::allow_local_site(site)).take(200).collect();
        let blocker_off: Vec<String> = store
            .setting("blocker_off")
            .and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|site| !site.is_empty() && site.len() <= 253 && site.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-')))
            .take(500)
            .collect();
        let media_sites = store.setting("media_sites").map(|text| crate::media::parse(&text)).unwrap_or_default();
        native::set_media_rules(media_sites.clone());
        let blocker = Blocker::new(
            settings.blocker,
            settings.local_net,
            &local_sites,
            blocker_off.iter().cloned().collect(),
            settings.filter_update,
            settings.guard,
            &data_dir,
            proxy.clone(),
        );
        Self {
            ua_suffix: format!("Version/{} Safari/605.1.15", native::safari_version()),
            _menu: crate::menu::build(),
            proxy,
            store,
            settings,
            containers,
            windows: HashMap::new(),
            focus_order: Vec::new(),
            ids: Rc::new(Cell::new(0)),
            popups: Rc::new(RefCell::new(Vec::new())),
            newtab: Arc::new(Mutex::new(String::new())),
            closed: Vec::new(),
            active_downloads: Vec::new(),
            notices: HashMap::new(),
            notice_owners: HashMap::new(),
            next_notice: 0,
            next_translation: 0,
            translate_deferred: Vec::new(),
            next_check: 0,
            blocker,
            data_dir,
            session_dirty: None,
            last_tick: Instant::now(),
            last_sweep: Instant::now(),
            unload_after: Duration::from_secs(
                crate::dev::var("ZUNDER_UNLOAD_SECS")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(3 * 60 * 60),
            ),
            ai_probe: None,
            ai_probe_at: None,
            ai_probing: false,
            ai_other_status: String::new(),
            oauth: None,
            ai_deferred: Vec::new(),
            ai_next_job: 0,
            quitting: false,
            quit: false,
            vault: crate::vault::Vault::new(),
            auth_waiting: HashMap::new(),
            next_auth: 0,
            fill_unlocked_until: None,
            panel_unlocked_until: None,
            export_until: None,
            clipboard_count: None,
            local_sites,
            local_sites_session: Vec::new(),
            local_noticed: HashSet::new(),
            blocker_off,
            media_sites,
            stealth_next: false,
            wall_noticed: HashSet::new(),
            last_filter_check: Instant::now(),
            sites_stale: true,
            reuse_warned: HashSet::new(),
            last_external: None,
            reload_when_ready: Vec::new(),
            last_refusal: None,
            frame_files_noticed: HashSet::new(),
            frame_file_notices: HashMap::new(),
            geo_waiters: Vec::new(),
            sources: Default::default(),
            next_source: 0,
            thumbs: Default::default(),
            thumb_serial: 0,
            starting: false,
        }
    }

    fn start(&mut self, target: &Target, urls: Vec<String>) {
        // Sollte beim letzten Beenden gelöscht werden und es kam nicht mehr
        // dazu (etwa beim Abmelden), wird es jetzt nachgeholt – vor der ersten Seite.
        if self.store.setting("clear_pending").as_deref() == Some("1") {
            native::clear_website_data(CLEAR_PLAIN);
            self.store.set_setting("clear_pending", "0");
        }
        self.refresh_newtab();
        // Fenster, die beim Hochfahren entstehen, beginnen mit dem Ladebild. In
        // einer Probe-App nur auf Wunsch – sie soll sofort prüfbar sein.
        self.starting = if crate::dev::BUILT_IN {
            crate::dev::var("ZUNDER_SPLASH").as_deref() == Some("1")
        } else {
            self.settings.splash
        };
        // Angeheftete Tabs kommen immer wieder – auch wenn die letzte Sitzung
        // sonst nicht wiederhergestellt werden soll.
        self.restore_session(target, !self.settings.restore);
        if !urls.is_empty() {
            self.open_urls(target, urls);
        } else if self.windows.is_empty() {
            self.create_window(target, false, Vec::new(), 0, None);
        }
        self.starting = false;
    }

    /// Öffnet Adressen von außen (Befehlszeile, anderes Programm) in neuen Tabs.
    pub fn open_urls(&mut self, target: &Target, urls: Vec<String>) {
        // Von außen kommen nur Webadressen und Dateien herein.
        let urls: Vec<String> = urls
            .iter()
            .filter_map(|u| omnibox::as_url(u))
            .filter(|u| u.starts_with("http://") || u.starts_with("https://") || u.starts_with("file://"))
            .collect();
        if urls.is_empty() {
            return;
        }
        let window = self
            .focus_order
            .iter()
            .rev()
            .find(|id| self.windows.get(id).is_some_and(|w| !w.private))
            .copied();
        match window {
            Some(window) => {
                for url in urls {
                    self.new_tab(window, Some(url), true, None);
                }
                if let Some(win) = self.windows.get(&window) {
                    win.window.set_focus();
                }
            }
            None => {
                let seeds = urls.into_iter().map(|u| (u, String::new())).collect::<Vec<_>>();
                let last = seeds.len() - 1;
                self.create_window(target, false, seeds, last, None);
            }
        }
    }

    pub fn focused(&self) -> Option<WindowId> {
        self.focus_order
            .iter()
            .rev()
            .find(|id| self.windows.contains_key(id))
            .copied()
    }

    // -----------------------------------------------------------------------
    // Ereignisschleife
    // -----------------------------------------------------------------------

    fn handle(&mut self, event: Event<'_, UserEvent>, target: &Target) {
        match event {
            Event::UserEvent(event) => self.on_user_event(event, target),
            Event::WindowEvent { window_id, event, .. } => match event {
                WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                    self.layout(window_id);
                    self.bubble_context_changed(window_id);
                    if let Some(win) = self.windows.get_mut(&window_id) {
                        let fullscreen = win.window.fullscreen().is_some();
                        if fullscreen != win.fullscreen {
                            win.fullscreen = fullscreen;
                            win.dirty = true;
                        }
                    }
                    self.session_changed();
                }
                WindowEvent::Moved(_) => self.session_changed(),
                WindowEvent::CloseRequested => self.close_window(window_id),
                WindowEvent::Focused(true) => {
                    self.focus_order.retain(|id| *id != window_id);
                    self.focus_order.push(window_id);
                }
                _ => {}
            },
            Event::Reopen { .. } => {
                // Klick auf das Dock-Symbol ohne offenes Fenster.
                if self.windows.is_empty() {
                    self.create_window(target, false, Vec::new(), 0, None);
                }
            }
            Event::MainEventsCleared => {
                if self.last_tick.elapsed() >= Duration::from_millis(80) {
                    self.tick();
                }
                self.flush();
            }
            Event::LoopDestroyed => {
                if let Some(count) = self.clipboard_count.take() {
                    native::clear_clipboard_if(count);
                }
                if self.settings.clear_on_quit {
                    if !self.quitting {
                        // Beendet, ohne dass Zunder löschen konnte: Verlauf und
                        // Sitzung jetzt, die Website-Daten beim nächsten Start.
                        self.wipe_local();
                        self.store.set_setting("clear_pending", "1");
                    }
                } else {
                    self.save_session();
                }
            }
            _ => {}
        }
    }

    fn on_user_event(&mut self, event: UserEvent, target: &Target) {
        match event {
            UserEvent::Ui { window, from, body } => {
                if crate::commands::allowed_from(from, &body) {
                    self.handle_ui(window, &body, target);
                }
            }
            UserEvent::Page { window, tab, ev } => self.handle_page(window, tab, ev),
            UserEvent::Nav(nav) => self.handle_nav(nav),
            UserEvent::Menu(id) => {
                let window = self.focused();
                self.command(window, &id, target);
            }
            UserEvent::FindResult { window, tab, json } => {
                if let Some(win) = self.windows.get_mut(&window) {
                    if win.active_tab().is_some_and(|t| t.id == tab) {
                        let result: Value = serde_json::from_str(&json).unwrap_or(Value::Null);
                        win.send(json!({ "t": "findResult", "result": result }));
                    }
                }
            }
            UserEvent::Suggestions { window, seq, query, items } => {
                let engine = omnibox::engine(&self.settings.engine);
                let items: Vec<Value> = items
                    .iter()
                    .map(|s| json!({ "k": "search", "title": s, "url": omnibox::search_url(engine, s) }))
                    .collect();
                if let Some(win) = self.windows.get_mut(&window) {
                    win.send(json!({ "t": "suggestMore", "seq": seq, "query": query, "items": items }));
                }
            }
            UserEvent::BlockerReady => self.blocker_ready(),
            UserEvent::Filters(event) => {
                self.blocker.on_event(event);
                self.send_filter_info();
            }
            UserEvent::Wake => {}
            UserEvent::SiteData { window, query, sites } => {
                let needle = query.to_lowercase();
                let items: Vec<Value> = sites
                    .iter()
                    .filter(|(site, _)| needle.is_empty() || site.to_lowercase().contains(&needle))
                    .take(800)
                    .map(|(site, kinds)| json!({ "site": site, "kinds": kinds }))
                    .collect();
                if let Some(win) = self.windows.get_mut(&window) {
                    win.send(json!({ "t": "panel", "name": "sitedata", "q": query, "data": { "items": items, "total": sites.len() } }));
                }
            }
            UserEvent::SiteDataChanged { window, query } => self.send_panel(window, "sitedata", &query),
            UserEvent::MediaRule { site, rule } => self.media_remembered(&site, rule),
            UserEvent::AiPage { window, job, slot, json } => self.ai_page_ready(window, job, slot, json),
            UserEvent::Remembered { window, title, url, json } => self.remember_store(window, title, url, json),
            UserEvent::Ai { window, job, ev } => self.ai_event(window, job, ev),
            UserEvent::AiProbe(probe) => self.ai_probe_done(probe),
            UserEvent::AiOther(event) => self.ai_other_event(event),
            UserEvent::Ping(ping) => {
                let Some((window, index)) = self.find_webview(ping.webview) else {
                    return;
                };
                let Some(tab) = self.windows.get(&window).and_then(|w| w.tabs.get(index)).map(|t| t.id) else {
                    return;
                };
                self.handle_page(window, tab, PageEvent::Ping { origin: ping.origin, body: ping.body, main: ping.main_frame });
            }
            UserEvent::Thumb { window, tab, bytes } => {
                // Nur für Tabs, die es noch gibt; alte Bilder geschlossener Tabs fallen weg.
                let alive: Vec<TabId> = self.windows.values().flat_map(|w| w.tabs.iter().map(|t| t.id)).collect();
                if let Ok(mut thumbs) = self.thumbs.lock() {
                    thumbs.retain(|id, _| alive.contains(id));
                    if alive.contains(&tab) && thumbs.len() < 120 {
                        thumbs.insert(tab, bytes);
                    }
                }
                self.thumb_serial += 1;
                let serial = self.thumb_serial;
                if let Some(win) = self.windows.get_mut(&window) {
                    win.send(json!({ "t": "thumb", "id": tab, "v": serial }));
                }
            }
            UserEvent::Geo(fix) => self.geo_arrived(fix),
            UserEvent::Scan { window, tab, app, n, result } => self.app_reply(window, tab, app, n, result),
            UserEvent::Inspect { window, tab, app, n, result } => self.app_reply(window, tab, app, n, result),
            UserEvent::Api { window, tab, app, n, result } => self.app_reply(window, tab, app, n, result),
            UserEvent::Reissue { window, tab, url } => {
                if let Some(index) = self.tab_index(window, tab) {
                    self.load_quietly(window, index, &url);
                }
            }
            UserEvent::DataCleared(tag) => {
                if self.quitting && tag == CLEAR_ON_QUIT {
                    self.store.set_setting("clear_pending", "0");
                    self.quit = true;
                }
            }
            UserEvent::Bridge(message) => self.handle_bridge(message),
            UserEvent::Auth { token, ok } => self.auth_result(token, ok),
            UserEvent::Filled { window, tab, login, result } => self.filled(window, tab, login, result),
            UserEvent::Suggested { window, tab, origin, user, password } => {
                self.suggestion_accepted(window, tab, origin, user, password)
            }
            UserEvent::FileChosen { window, purpose, path } => self.file_chosen(window, purpose, path),
            UserEvent::ClipboardExpire(count) => self.clipboard_expire(count),
            UserEvent::PageAnswer { window, tab, what, result } => self.page_answer(window, tab, what, result),
            UserEvent::Translate(event) => self.translate_event(event),
            UserEvent::SiteCheck(event) => self.check_event(event),
            // Befehle der Prüfhilfe: In der ausgelieferten App gibt es sie nicht.
            UserEvent::Debug(line) => {
                if crate::dev::BUILT_IN {
                    self.handle_debug(&line, target);
                }
            }
            UserEvent::DebugOutput(line) => {
                println!("{line}");
                let _ = std::io::stdout().flush();
            }
        }
    }

    pub fn control_flow(&self) -> ControlFlow {
        if self.quit {
            return ControlFlow::Exit;
        }
        let loading = self
            .windows
            .values()
            .any(|w| w.active_tab().is_some_and(|t| t.loading));
        // Der Zähler der gesperrten Abrufe läuft auch nach dem Laden weiter.
        if loading || crate::blocker::blocked_pending() || self.windows.values().any(|w| w.ampel_pending) {
            return ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(90));
        }
        if let Some(since) = self.session_dirty {
            return ControlFlow::WaitUntil(since + Duration::from_millis(1500));
        }
        // Gibt es Tabs, die entladen werden könnten, gelegentlich nachsehen –
        // sonst schläft die App, bis wieder etwas passiert.
        let candidates = self.settings.saver
            && self.windows.values().any(|w| {
                w.tabs
                    .iter()
                    .enumerate()
                    .any(|(i, t)| i != w.active && t.webview.is_some())
            });
        if candidates {
            ControlFlow::WaitUntil(self.last_sweep + self.sweep_interval())
        } else {
            ControlFlow::Wait
        }
    }

    fn sweep_interval(&self) -> Duration {
        (self.unload_after / 6).clamp(Duration::from_secs(2), Duration::from_secs(300))
    }

    /// Fragt bei lange ungenutzten Hintergrund-Tabs nach, ob dort noch etwas
    /// abgespielt wird. Nur wenn nicht, werden sie entladen (`media_answer`).
    fn sweep_idle_tabs(&mut self) {
        self.last_sweep = Instant::now();
        if !self.settings.saver {
            return;
        }
        for win in self.windows.values() {
            for (index, tab) in win.tabs.iter().enumerate() {
                let idle = index != win.active
                    && !win.in_split(tab.id)
                    && !tab.loading
                    && tab.last_used.elapsed() >= self.unload_after
                    && (tab.url.starts_with("http://") || tab.url.starts_with("https://"));
                if let (true, Some(webview)) = (idle, tab.webview.as_ref()) {
                    native::request_media_state(webview);
                }
            }
        }
    }

    /// Entlädt einen ungenutzten Tab, in dem nichts abgespielt wird. Adresse,
    /// Titel und Icon bleiben; beim nächsten Öffnen lädt die Seite neu.
    pub fn media_answer(&mut self, id: WindowId, index: usize, playing: bool) {
        let unload_after = self.unload_after;
        let Some(win) = self.windows.get_mut(&id) else {
            return;
        };
        let active = win.active;
        let split = win.split;
        let Some(tab) = win.tabs.get_mut(index) else {
            return;
        };
        if playing || split.is_some_and(|(left, right)| left == tab.id || right == tab.id) {
            // Läuft noch etwas oder steht der Tab sichtbar daneben, gilt er als benutzt.
            tab.last_used = Instant::now();
            return;
        }
        // Ein Tab, der gerade übersetzt wird, arbeitet noch.
        let translating = tab.translation.as_ref().is_some_and(|translation| translation.running());
        if index != active && !tab.loading && !translating && tab.last_used.elapsed() >= unload_after {
            tab.webview = None;
            tab.pending = None;
            // Mit der Seite ist auch ihre Übersetzung fort.
            tab.translation = None;
        }
    }

    /// Regelmäßige Arbeit, solange etwas lädt: Fortschritt nachführen und
    /// hängen gebliebene Ladeanzeigen auflösen; außerdem die Sitzung sichern.
    fn tick(&mut self) {
        self.last_tick = Instant::now();
        for win in self.windows.values_mut() {
            let active = win.active;
            let mut progress = None;
            if let Some(tab) = win.tabs.get_mut(active) {
                if let (true, Some(webview)) = (tab.loading, tab.webview.as_ref()) {
                    if native::is_loading(webview) {
                        tab.idle_ticks = 0;
                        let p = native::progress(webview).clamp(0.1, 1.0);
                        if (p - tab.progress).abs() > 0.01 {
                            tab.progress = p;
                            progress = Some(p);
                        }
                    } else {
                        // WebKit lädt nichts mehr, ein Abschluss-Ereignis blieb aber aus.
                        tab.idle_ticks += 1;
                        if tab.idle_ticks > 5 {
                            tab.loading = false;
                            tab.pending = None;
                            tab.idle_ticks = 0;
                            win.dirty = true;
                        }
                    }
                }
            }
            if let Some(p) = progress {
                if (p - win.last_progress).abs() > 0.01 {
                    win.last_progress = p;
                    win.send(json!({ "t": "progress", "p": p }));
                }
            }
        }
        if self
            .session_dirty
            .is_some_and(|since| since.elapsed() >= Duration::from_millis(1500))
        {
            self.save_session();
        }
        if self.last_sweep.elapsed() >= self.sweep_interval() {
            self.sweep_idle_tabs();
        }
        // Gesperrte Abrufe: den Zähler am Schild nachführen.
        if crate::blocker::take_blocked_pending() {
            for win in self.windows.values_mut() {
                let count = win
                    .active_tab()
                    .and_then(|tab| tab.webview.as_ref())
                    .map(|webview| crate::blocker::blocked(native::key(webview)))
                    .unwrap_or(0);
                if count != win.blocked_shown && win.ui_ready {
                    win.blocked_shown = count;
                    win.send(json!({ "t": "blocked", "n": count }));
                }
            }
        }
        // Tarn-Ampel: neue Abfragen der Seite nachführen.
        let pending: Vec<WindowId> =
            self.windows.iter().filter(|(_, w)| w.ampel_pending && w.ui_ready).map(|(id, _)| *id).collect();
        for id in pending {
            let ampel = self.ampel(id);
            if let Some(win) = self.windows.get_mut(&id) {
                win.ampel_pending = false;
                win.send(json!({ "t": "ampel", "ampel": ampel }));
            }
        }
        // Filterlisten: gelegentlich nachsehen, ob es Zeit zum Auffrischen ist –
        // aber nicht in den ersten Sekunden, in denen die Seiten laden.
        if self.last_filter_check.elapsed() >= Duration::from_secs(60) {
            self.last_filter_check = Instant::now();
            let live: HashSet<usize> = self
                .windows
                .values()
                .flat_map(|win| win.tabs.iter())
                .filter_map(|tab| tab.webview.as_ref().map(native::key))
                .collect();
            crate::blocker::keep_only(&live);
            native::keep_stealth(&live);
            native::keep_responses(&live);
            if STARTED.get().is_some_and(|at| at.elapsed() >= Duration::from_secs(45)) && self.blocker.refresh(false) {
                self.send_filter_info();
            }
        }
        // Antwortet ein Rechner auf den verschlüsselten Versuch gar nicht, soll
        // niemand eine Minute warten: nach ein paar Sekunden kommt die Nachfrage.
        let mut stalled: Vec<(WindowId, usize, String)> = Vec::new();
        for (id, win) in &self.windows {
            for (index, tab) in win.tabs.iter().enumerate() {
                if let Some(upgrade) = &tab.upgrade {
                    let waiting = tab.loading
                        && tab
                            .pending
                            .as_deref()
                            .is_some_and(|p| p.starts_with("https://") && crate::shield::host_of(p) == upgrade.host);
                    if waiting && upgrade.at.elapsed() >= Duration::from_secs(6) {
                        stalled.push((*id, index, upgrade.plain.clone()));
                    }
                }
            }
        }
        for (id, index, plain) in stalled {
            if let Some(webview) = self.windows.get(&id).and_then(|w| w.tabs.get(index)).and_then(|t| t.webview.as_ref()) {
                native::stop(webview);
            }
            self.show_insecure_warning(id, index, &plain, None);
        }
    }

    /// Verlauf, geschlossene Tabs und die gespeicherte Sitzung vergessen.
    pub fn wipe_local(&mut self) {
        self.sites_stale = true;
        self.store.clear_history();
        self.closed.clear();
        self.session_dirty = None;
        let _ = std::fs::remove_file(self.session_file());
    }

    /// Beenden mit „Löschen beim Beenden“: erst alle Fenster zu, dann Verlauf
    /// und Website-Daten weg – die App endet, sobald WebKit fertig ist.
    pub fn clean_quit(&mut self) {
        if self.quitting {
            return;
        }
        self.quitting = true;
        self.popups.borrow_mut().clear();
        for win in self.windows.values_mut() {
            if let Some(job) = win.ai_job.take() {
                job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        self.windows.clear();
        self.focus_order.clear();
        self.wipe_local();
        self.store.set_setting("clear_pending", "1");
        native::clear_website_data(CLEAR_ON_QUIT);
        // Meldet sich WebKit nicht, endet die App trotzdem – gelöscht wird dann beim nächsten Start.
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(4));
            let _ = proxy.send_event(UserEvent::Menu("quit_now".into()));
        });
    }

    /// Legt alle Tabs neu an – nötig, wenn sich ändert, womit ein Tab entsteht
    /// (Härtung). Die Seiten laden dabei neu.
    pub fn rebuild_tabs(&mut self) {
        let windows: Vec<WindowId> = self.windows.keys().copied().collect();
        for id in windows {
            let mut shown: Vec<usize> = Vec::new();
            if let Some(win) = self.windows.get_mut(&id) {
                let active = win.active;
                let split = win.split;
                for (index, tab) in win.tabs.iter_mut().enumerate() {
                    let visible = index == active || split.is_some_and(|(left, right)| tab.id == left || tab.id == right);
                    if tab.webview.take().is_some() {
                        tab.loading = false;
                        tab.pending = None;
                        tab.upgrade = None;
                        tab.translation = None;
                    }
                    if visible {
                        shown.push(index);
                    }
                }
                win.dirty = true;
            }
            for index in shown {
                self.ensure_webview(id, index);
                if let Some(webview) = self.windows.get(&id).and_then(|w| w.tabs.get(index)).and_then(|t| t.webview.as_ref()) {
                    let _ = webview.set_visible(true);
                }
            }
            self.layout(id);
            if let Some(win) = self.windows.get(&id) {
                if win.overlay {
                    native::bring_to_front(&win.chrome);
                }
            }
        }
    }

    /// Bringt die Oberfläche aller geänderten Fenster auf den aktuellen Stand.
    fn flush(&mut self) {
        if self.sites_stale {
            self.refresh_sites();
        }
        let dirty: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|(_, w)| w.dirty && w.ui_ready)
            .map(|(id, _)| *id)
            .collect();
        for id in dirty {
            self.sync_ui(id);
        }
    }

    // -----------------------------------------------------------------------
    // Fenster
    // -----------------------------------------------------------------------

    pub fn create_window(
        &mut self,
        target: &Target,
        private: bool,
        seeds: Vec<(String, String)>,
        active: usize,
        frame: Option<Frame>,
    ) -> Option<WindowId> {
        let mut builder = WindowBuilder::new()
            .with_title("Zunder")
            .with_min_inner_size(LogicalSize::new(420.0, 300.0))
            .with_titlebar_transparent(true)
            .with_title_hidden(true)
            .with_fullsize_content_view(true)
            .with_traffic_light_inset(LogicalPosition::new(LIGHTS_LEFT, LIGHTS_INSET))
            .with_automatic_window_tabbing(false);
        match frame {
            Some(f) => {
                builder = builder
                    .with_inner_size(LogicalSize::new(f.w.max(420.0), f.h.max(300.0)))
                    .with_position(LogicalPosition::new(f.x, f.y));
            }
            None => {
                builder = builder.with_inner_size(LogicalSize::new(1400.0, 860.0));
                // Neue Fenster leicht versetzt zum aktuellen öffnen.
                if let Some(current) = self.focused().and_then(|id| self.windows.get(&id)) {
                    if let Ok(position) = current.window.outer_position() {
                        let p = position.to_logical::<f64>(current.window.scale_factor());
                        builder = builder.with_position(LogicalPosition::new(p.x + 26.0, p.y + 26.0));
                    }
                }
            }
        }
        let window = Rc::new(builder.build(target).ok()?);
        let id = window.id();
        debug_timing("fenster_gebaut");
        let splash = self.starting && !private;
        let stealth = std::mem::take(&mut self.stealth_next) && private;
        // Läuft Tor auf diesem Mac, geht das ganze Fenster dort hindurch.
        let route = if stealth { crate::stealth::find_route() } else { None };
        let tor = route.is_some();

        let shared = Rc::new(Shared {
            proxy: self.proxy.clone(),
            window: window.clone(),
            window_id: id,
            private,
            stealth,
            route,
            store: private.then(native::private_store),
            popups: self.popups.clone(),
            ids: self.ids.clone(),
            newtab: self.newtab.clone(),
            ua_suffix: self.ua_suffix.clone(),
            sources: self.sources.clone(),
            thumbs: self.thumbs.clone(),
            splash,
        });

        let size = window.inner_size().to_logical::<f64>(window.scale_factor());
        // Mit Ladebild deckt der Rahmen von Anfang an das ganze Fenster ab.
        let chrome_height = if splash { size.height } else { CHROME_HEIGHT };
        let chrome = build_chrome(&shared, rect(0.0, 0.0, size.width, chrome_height)).ok()?;
        // Erst jetzt gibt es die Delegate-Klassen von wry – vor dem ersten Tab nachrüsten.
        native::install_hooks(&chrome);
        align_traffic_lights(&window, &chrome);

        let mut tabs: Vec<Tab> = seeds
            .into_iter()
            .map(|(url, title)| {
                let mut tab = Tab::new(shared.next_id(), loadable(&url, private));
                // Aus der Sitzungsdatei – so lang, wie ein Titel eben ist.
                tab.title = title.chars().take(300).collect();
                tab
            })
            .collect();
        if tabs.is_empty() {
            let blank = if private { NEWTAB_PRIVATE } else { NEWTAB };
            tabs.push(Tab::new(shared.next_id(), blank.to_string()));
        }
        let active = active.min(tabs.len() - 1);

        debug_timing("fenster_angelegt");
        self.windows.insert(
            id,
            BrowserWindow {
                groups: Vec::new(),
                tabs,
                active,
                chrome,
                assistant: None,
                assistant_open: false,
                assistant_ready: false,
                assistant_queue: Vec::new(),
                assistant_page: String::new(),
                ai_job: None,
                bubble: None,
                bubble_ready: false,
                bubble_queue: Vec::new(),
                bubble_state: None,
                bubble_height: 120.0,
                bubble_seq: 0,
                split: None,
                shared,
                private,
                stealth,
                tor,
                // Unter dem Ladebild bleibt die Seite unsichtbar, bis es wirklich
                // steht – sonst blitzte sie davor kurz auf.
                hold_content: splash,
                ampel_pending: false,
                blocked_shown: 0,
                overlay: splash,
                chrome_height: CHROME_HEIGHT,
                fullscreen: false,
                ui_ready: false,
                dirty: true,
                last_progress: 0.0,
                window_title: String::new(),
                queue: Vec::new(),
                window,
            },
        );
        self.focus_order.push(id);
        self.activate_tab(id, active);
        self.session_changed();
        Some(id)
    }

    pub fn close_window(&mut self, id: WindowId) {
        // Vorher sichern: Ist es das letzte Fenster, bleibt seine Sitzung erhalten.
        self.save_session();
        self.popups.borrow_mut().retain(|p| p.window != id);
        if let Some(mut win) = self.windows.remove(&id) {
            // Eine laufende Anfrage an den Assistenten endet mit dem Fenster.
            if let Some(job) = win.ai_job.take() {
                job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            if !win.private {
                for tab in &win.tabs {
                    Self::remember_closed(&mut self.closed, tab);
                }
            }
            win.tabs.clear();
        }
        self.focus_order.retain(|w| *w != id);
        self.save_session();
    }

    fn remember_closed(closed: &mut Vec<(String, String)>, tab: &Tab) {
        if tab.url.starts_with("http://") || tab.url.starts_with("https://") {
            closed.push((tab.url.clone(), tab.title.clone()));
            if closed.len() > 30 {
                closed.remove(0);
            }
        }
    }

    /// Passt Rahmen und sichtbaren Tab an die Fenstergröße an.
    pub fn layout(&mut self, id: WindowId) {
        let Some(win) = self.windows.get_mut(&id) else {
            return;
        };
        let (w, h) = win.size();
        let chrome_height = if win.overlay { h } else { win.chrome_height };
        let _ = win.chrome.set_bounds(rect(0.0, 0.0, w, chrome_height));
        for tab in &win.tabs {
            let shown = win.in_split(tab.id) || win.active_tab().is_some_and(|a| a.id == tab.id);
            if let (true, Some(webview)) = (shown, tab.webview.as_ref()) {
                let _ = webview.set_bounds(win.pane_rect(tab.id));
            }
        }
        if let (true, Some(assistant)) = (win.assistant_open, win.assistant.as_ref()) {
            let _ = assistant.set_bounds(win.assistant_rect());
        }
        self.place_bubble(id);
    }

    /// Zeigt einen zweiten Tab neben dem aktiven – oder beendet das Nebeneinander.
    pub fn set_split(&mut self, id: WindowId, companion: Option<TabId>) {
        let Some(win) = self.windows.get_mut(&id) else {
            return;
        };
        let active_id = win.active_tab().map(|t| t.id);
        // Ein bestehendes Paar löst sich zuerst auf.
        if let Some((left, right)) = win.split.take() {
            for tab in &win.tabs {
                let hide = (tab.id == left || tab.id == right) && Some(tab.id) != active_id;
                if let (true, Some(webview)) = (hide, tab.webview.as_ref()) {
                    let _ = webview.set_visible(false);
                }
            }
        }
        let mut shown = None;
        if let (Some(companion), Some(active_id)) = (companion, active_id) {
            if companion != active_id {
                if let Some(index) = win.tabs.iter().position(|t| t.id == companion) {
                    win.split = Some((active_id, companion));
                    shown = Some(index);
                }
            }
        }
        win.dirty = true;
        if let Some(index) = shown {
            self.ensure_webview(id, index);
            if let Some(win) = self.windows.get_mut(&id) {
                if let Some(tab) = win.tabs.get_mut(index) {
                    tab.last_used = Instant::now();
                    if let Some(webview) = &tab.webview {
                        let _ = webview.set_visible(true);
                    }
                }
                if win.overlay {
                    native::bring_to_front(&win.chrome);
                }
            }
        }
        self.layout(id);
    }

    // -----------------------------------------------------------------------
    // Tabs
    // -----------------------------------------------------------------------

    /// Erzeugt die WebView eines Tabs, falls er noch keine hat.
    pub(crate) fn ensure_webview(&mut self, id: WindowId, index: usize) {
        let Some(win) = self.windows.get_mut(&id) else {
            return;
        };
        let Some(bounds) = win.tabs.get(index).map(|t| win.pane_rect(t.id)) else {
            return;
        };
        let private = win.private;
        let Some(tab) = win.tabs.get_mut(index) else {
            return;
        };
        if tab.webview.is_some() {
            return;
        }
        // Solange WebKit die Schutzlisten noch übersetzt (Bruchteile einer
        // Sekunde nach dem Start), entsteht kein Tab: Sonst lüde die erste Seite
        // ganz ohne sie. `blocker_ready` holt das nach.
        if self.blocker.pending() {
            return;
        }
        // Ohne die Navigationsentscheidung gäbe es keinen der Schutzmechanismen:
        // Dann lädt Zunder lieber gar nichts.
        if !native::policy_hooked() {
            eprintln!("zunder: Schutzfunktionen fehlen – es wird keine Seite geladen.");
            return;
        }
        let mut url = loadable(&tab.url, private);
        if !native::url_ok(&url) {
            url = if private { NEWTAB_PRIVATE } else { NEWTAB }.to_string();
        }
        // Der Browser selbst lädt diese Adresse – das gilt für ein Gerät im
        // eigenen Netz wie (gleich unten, sobald es die WebView gibt) für
        // interne Seiten.
        native::user_navigation(None, &url);
        // Dateien brauchen Lesezugriff auf ihren Ordner – das geht erst nach dem Anlegen.
        let is_file = url.starts_with("file://");
        let initial = if is_file { None } else { Some(url.as_str()) };
        // Ein Container-Tab eines gewöhnlichen Fensters bekommt den dauerhaften
        // Datenspeicher seines Containers. In privaten und Stealth-Fenstern gilt
        // das nicht – die haben ohnehin eigene, flüchtige Speicher.
        let container_store = if private || win.stealth {
            None
        } else {
            tab.container
                .and_then(|id| self.containers.iter().find(|container| container.id == id))
                .and_then(|container| native::container_store(&container.uuid))
        };
        match build_content(&win.shared, tab.id, bounds, initial, None, container_store) {
            Ok(webview) => {
                // Die Anmeldung gilt genau dieser WebView. WebKit fragt erst
                // nach dieser Zeile, ob sie die Adresse laden darf.
                native::expect_internal(native::key(&webview), &url);
                if is_file && !native::load_file(&webview, &url) {
                    let _ = webview.load_url(&url);
                }
                if (tab.zoom - 1.0).abs() > 0.001 {
                    let _ = webview.zoom(tab.zoom);
                }
                if tab.mobile {
                    native::set_mobile(&webview, true);
                }
                self.blocker.apply_new(&webview, &url);
                if !omnibox::is_newtab(&url) {
                    tab.loading = true;
                    tab.progress = 0.1;
                    tab.pending = Some(url.clone());
                    tab.pending_shown = true;
                }
                tab.url = url;
                tab.webview = Some(webview);
                if win.overlay {
                    native::bring_to_front(&win.chrome);
                }
            }
            Err(error) => eprintln!("zunder: Tab konnte nicht angelegt werden: {error}"),
        }
    }

    pub fn activate_tab(&mut self, id: WindowId, index: usize) {
        // Vom Tab, der gleich in den Hintergrund geht, noch ein Vorschaubild –
        // für die Übersicht aller Tabs.
        if let Some(previous) = self.windows.get(&id).map(|w| w.active).filter(|previous| *previous != index) {
            self.capture_thumb(id, previous);
        }
        {
            let Some(win) = self.windows.get_mut(&id) else {
                return;
            };
            if index >= win.tabs.len() {
                return;
            }
            let target = win.tabs[index].id;
            let within_pair = win.in_split(target);
            if let (Some((left, right)), false) = (win.split, within_pair) {
                // Ein dritter Tab kommt nach vorn: Das Nebeneinander endet.
                for tab in &win.tabs {
                    if let (true, Some(webview)) = (tab.id == left || tab.id == right, tab.webview.as_ref()) {
                        let _ = webview.set_visible(false);
                    }
                }
                win.split = None;
            }
            if win.active != index {
                // Innerhalb des Paars bleiben beide Seiten stehen – nur die
                // Symbolleiste wechselt die Seite.
                if !within_pair {
                    if let Some(webview) = win.active_webview() {
                        let _ = webview.set_visible(false);
                    }
                }
                let previous = win.active;
                if let Some(tab) = win.tabs.get_mut(previous) {
                    tab.last_used = Instant::now();
                }
            }
            win.active = index;
            // Ein Tab aus einer zugeklappten Gruppe kommt nach vorn: Sie klappt auf.
            if let Some(group) = win.tabs[index].group {
                if let Some(group) = win.groups.iter_mut().find(|g| g.id == group) {
                    group.collapsed = false;
                }
            }
        }
        self.ensure_webview(id, index);
        let Some(win) = self.windows.get_mut(&id) else {
            return;
        };
        let Some(bounds) = win.tabs.get(index).map(|t| win.pane_rect(t.id)) else {
            return;
        };
        let mut blank = true;
        if let Some(tab) = win.tabs.get(index) {
            blank = omnibox::is_newtab(tab.shown_url());
            if let Some(webview) = &tab.webview {
                let _ = webview.set_bounds(bounds);
                if !win.hold_content {
                    let _ = webview.set_visible(true);
                }
                if !blank && !win.overlay {
                    let _ = webview.focus();
                }
            }
        }
        if blank {
            // Auf einer leeren Seite soll man sofort lostippen können.
            let _ = win.chrome.focus();
            win.send(json!({ "t": "focusAddress" }));
        }
        // Ein Hinweis fragt für die Seite, die man vor sich hat – nicht für eine im Hintergrund.
        self.notices_end_hidden(id);
        let Some(win) = self.windows.get_mut(&id) else {
            return;
        };
        win.last_progress = 0.0;
        win.dirty = true;
        self.session_changed();
        self.bubble_context_changed(id);
    }

    /// Legt einen Tab an. Ohne Adresse entsteht die Startseite.
    pub fn new_tab(
        &mut self,
        id: WindowId,
        url: Option<String>,
        foreground: bool,
        opener: Option<TabId>,
    ) -> Option<usize> {
        if url.is_none() {
            self.refresh_newtab();
        }
        let win = self.windows.get_mut(&id)?;
        let url = loadable(url.as_deref().unwrap_or(""), win.private);
        let mut tab = Tab::new(win.shared.next_id(), url);
        tab.opener = opener;
        let new_id = tab.id;
        let from = opener.and_then(|o| win.tabs.iter().position(|t| t.id == o));
        // Was aus einem Tab einer Gruppe heraus aufgeht, gehört dazu.
        tab.group = from.and_then(|i| win.tabs[i].group);
        let position = from.map(|i| i + 1).unwrap_or(win.tabs.len());
        win.tabs.insert(position, tab);
        if position <= win.active {
            win.active += 1;
        }
        // Neben einem angehefteten Tab entsteht nichts – der neue rückt hinter sie.
        win.tidy_tabs();
        let position = win.tabs.iter().position(|t| t.id == new_id).unwrap_or(position);
        win.dirty = true;
        if foreground {
            self.activate_tab(id, position);
        }
        self.session_changed();
        Some(position)
    }

    pub fn close_tab(&mut self, id: WindowId, index: usize) {
        let Some(win) = self.windows.get_mut(&id) else {
            return;
        };
        if index >= win.tabs.len() {
            return;
        }
        if win.tabs.len() == 1 {
            self.close_window(id);
            return;
        }
        // Schließt eine Hälfte des Paars, bleibt die andere allein stehen.
        let closing = win.tabs[index].id;
        let mut partner = None;
        if let Some((left, right)) = win.split {
            if closing == left || closing == right {
                win.split = None;
                partner = Some(if closing == left { right } else { left });
            }
        }
        let tab = win.tabs.remove(index);
        if !win.private {
            Self::remember_closed(&mut self.closed, &tab);
        }
        let closed_id = tab.id;
        drop(tab);
        win.dirty = true;
        // Was der geschlossene Tab noch gefragt hatte, hat sich erledigt.
        self.notices_end(id, closed_id, None);
        let Some(win) = self.windows.get_mut(&id) else {
            return;
        };
        if index < win.active {
            win.active -= 1;
        } else if index == win.active {
            // Der rechte Nachbar rückt nach; war es der letzte Tab, der linke.
            let next = index.min(win.tabs.len() - 1);
            self.activate_tab(id, next);
        }
        if let Some(position) = partner.and_then(|p| self.tab_index(id, p)) {
            self.activate_tab(id, position);
        }
        self.ai_tab_closed(id, closed_id);
        if self
            .windows
            .get(&id)
            .and_then(|w| w.bubble_state.as_ref())
            .is_some_and(|b| b.tab == closed_id)
        {
            self.hide_bubble(id);
        }
        self.session_changed();
    }

    pub fn tab_index(&self, id: WindowId, tab: TabId) -> Option<usize> {
        self.windows.get(&id)?.tabs.iter().position(|t| t.id == tab)
    }

    /// Lädt eine Adresse in einem Tab.
    pub fn navigate(&mut self, id: WindowId, index: usize, url: &str, typed: bool) {
        let Ok(parsed) = url::Url::parse(url) else {
            return;
        };
        let url = parsed.to_string();
        if !native::url_ok(&url) {
            return;
        }
        let Some(win) = self.windows.get_mut(&id) else {
            return;
        };
        let is_active = win.active == index;
        let overlay = win.overlay;
        let Some(tab) = win.tabs.get_mut(index) else {
            return;
        };
        tab.typed = typed;
        let mut blocked_file = false;
        // Geht es woandershin, ist ein laufender Verschlüsselungsversuch überholt.
        if tab.upgrade.as_ref().is_some_and(|u| crate::shield::host_of(&url) != u.host) {
            tab.upgrade = None;
        }
        match &tab.webview {
            Some(webview) => {
                native::expect_internal(native::key(webview), &url);
                native::user_navigation(Some(native::key(webview)), &url);
                // Der Mensch am Browser schickt den Tab woandershin: Für die
                // nächste Seite beginnen Dialogzähler und Stummschaltung neu.
                native::reset_dialogs(native::key(webview));
                tab.pending = Some(url.clone());
                tab.pending_shown = true;
                tab.loading = true;
                tab.progress = 0.1;
                tab.idle_ticks = 0;
                if !(url.starts_with("file://") && native::load_file(webview, &url)) {
                    let _ = webview.load_url(&url);
                }
                // In der Sandbox kommt der Browser nur an Dateien, die man ihm
                // gibt – eine eingetippte Dateiadresse gehört nicht dazu.
                blocked_file = url.starts_with("file://")
                    && native::in_sandbox()
                    && url::Url::parse(&url)
                        .ok()
                        .and_then(|u| u.to_file_path().ok())
                        .is_some_and(|path| std::fs::File::open(path).is_err());
                if is_active && !overlay {
                    let _ = webview.focus();
                }
            }
            None => tab.url = url,
        }
        win.dirty = true;
        if blocked_file {
            if let Some(tab) = self.windows.get_mut(&id).and_then(|w| w.tabs.get_mut(index)) {
                tab.loading = false;
                tab.pending = None;
            }
            self.notice(id, t("n_file_sandbox").to_string(), None, 9000);
        }
        if is_active {
            self.ensure_webview(id, index);
        }
    }

    /// Lädt eine Adresse, zu der eine SEITE wollte (nicht der Mensch am Browser):
    /// Die Adressleiste zeigt sie erst, wenn die neue Seite wirklich da ist.
    pub fn load_quietly(&mut self, id: WindowId, index: usize, url: &str) {
        if !native::url_ok(url) || url::Url::parse(url).is_err() {
            return;
        }
        let Some(win) = self.windows.get_mut(&id) else {
            return;
        };
        let Some(tab) = win.tabs.get_mut(index) else {
            return;
        };
        if let Some(webview) = &tab.webview {
            tab.pending = Some(url.to_string());
            tab.pending_shown = false;
            tab.loading = true;
            tab.progress = 0.1;
            tab.idle_ticks = 0;
            let _ = webview.load_url(url);
        }
        win.dirty = true;
    }

    /// Findet Fenster und Tab zu einer WebView (für Ereignisse aus WebKit).
    pub fn find_webview(&self, key: usize) -> Option<(WindowId, usize)> {
        for (id, win) in &self.windows {
            for (index, tab) in win.tabs.iter().enumerate() {
                if tab.webview.as_ref().is_some_and(|w| native::key(w) == key) {
                    return Some((*id, index));
                }
            }
        }
        None
    }

    pub fn apply_blocker(&mut self) {
        for win in self.windows.values() {
            for tab in &win.tabs {
                if let Some(webview) = &tab.webview {
                    self.blocker.apply(webview, tab.pending.as_deref().unwrap_or(&tab.url));
                }
            }
        }
    }

    /// Die Tarn-Ampel für den aktiven Tab eines Stealth-Fensters: wie gut man
    /// auf dieser Seite getarnt ist – und was die Seite abgefragt hat. `null`
    /// in anderen Fenstern und auf Zunders eigenen Seiten.
    ///
    /// Rot: Die Website weiß, wer man ist (Passwort eingegeben), oder die
    /// Verbindung ist unverschlüsselt. Gelb: Die IP-Adresse ist sichtbar (kein
    /// Tor) oder der Blocker ist für die Website aus. Sonst grün.
    pub fn ampel(&self, id: WindowId) -> Value {
        let Some(win) = self.windows.get(&id) else {
            return Value::Null;
        };
        let Some(tab) = win.active_tab() else {
            return Value::Null;
        };
        let bar = omnibox::bar_url(tab.shown_url());
        let secure = bar.starts_with("https://");
        if !win.stealth || !(secure || bar.starts_with("http://")) {
            return Value::Null;
        }
        let site = crate::sites::domain_of(&bar);
        let blocker = !crate::blocker::site_off(&bar);
        let signed_in = site.as_ref().is_some_and(|site| tab.signed_in.contains(site));
        let level = if !secure || signed_in {
            0
        } else if !win.tor || !blocker {
            1
        } else {
            2
        };
        let asked: Vec<Value> = tab
            .asked
            .get("")
            .map(|kinds| kinds.iter().map(|(kind, count)| json!({ "k": kind, "n": count })).collect())
            .unwrap_or_default();
        let mut embedded: Vec<(&String, Vec<&'static str>)> = tab
            .asked
            .iter()
            .filter(|(host, kinds)| !host.is_empty() && !kinds.is_empty())
            .map(|(host, kinds)| (host, kinds.keys().copied().collect()))
            .collect();
        embedded.sort();
        let embedded: Vec<Value> =
            embedded.into_iter().take(12).map(|(host, kinds)| json!({ "host": host, "kinds": kinds })).collect();
        json!({
            "level": level,
            "tor": win.tor,
            "secure": secure,
            "blocker": blocker,
            "signedIn": signed_in,
            "site": site,
            "blocked": tab.webview.as_ref().map(|webview| crate::blocker::blocked(native::key(webview))).unwrap_or(0),
            "asked": asked,
            "embedded": embedded,
        })
    }

    // --- Angeheftete Tabs und Tab-Gruppen -----------------------------------

    /// Nach jeder Änderung an Anheftung oder Gruppen: ordnen, zeigen, sichern.
    fn tabs_changed(&mut self, id: WindowId) {
        if let Some(win) = self.windows.get_mut(&id) {
            win.tidy_tabs();
            win.dirty = true;
        }
        self.session_changed();
    }

    pub fn pin_tab(&mut self, id: WindowId, index: usize, pinned: bool) {
        let Some(tab) = self.windows.get_mut(&id).and_then(|w| w.tabs.get_mut(index)) else {
            return;
        };
        tab.pinned = pinned;
        tab.group = None;
        self.tabs_changed(id);
    }

    /// Legt eine neue Gruppe an und steckt den Tab hinein.
    pub fn group_new(&mut self, id: WindowId, index: usize) {
        let Some(win) = self.windows.get_mut(&id) else {
            return;
        };
        if index >= win.tabs.len() || win.groups.len() >= 40 {
            return;
        }
        // Auch eine verbogene Sitzungsdatei mit der größten denkbaren Nummer bringt hier nichts zum Überlaufen.
        let Some(group) = win.groups.iter().map(|g| g.id).max().unwrap_or(0).checked_add(1) else {
            return;
        };
        let name = t("tg_default").replace("{n}", &group.to_string());
        win.groups.push(TabGroup { id: group, name, color: ((group - 1) % 8) as u8, collapsed: false });
        win.tabs[index].pinned = false;
        win.tabs[index].group = Some(group);
        self.tabs_changed(id);
    }

    /// Steckt den Tab in eine bestehende Gruppe – oder nimmt ihn heraus (`None`).
    pub fn group_set(&mut self, id: WindowId, index: usize, group: Option<u32>) {
        let Some(win) = self.windows.get_mut(&id) else {
            return;
        };
        if index >= win.tabs.len() || group.is_some_and(|group| !win.groups.iter().any(|g| g.id == group)) {
            return;
        }
        let moved_id = win.tabs[index].id;
        let active_id = win.tabs[win.active].id;
        let mut tab = win.tabs.remove(index);
        tab.pinned = false;
        tab.group = group;
        // Hinter den letzten Tab der Gruppe – beim Herausnehmen ans Ende der Leiste.
        let position = group
            .and_then(|group| win.tabs.iter().rposition(|t| t.group == Some(group)))
            .map(|last| last + 1)
            .unwrap_or(win.tabs.len());
        win.tabs.insert(position, tab);
        win.active = win.tabs.iter().position(|t| t.id == active_id).unwrap_or(0);
        let _ = moved_id;
        self.tabs_changed(id);
    }

    /// Klappt eine Gruppe zu oder auf. Liegt der aktive Tab darin, kommt
    /// vorher ein anderer nach vorn – gibt es keinen, bleibt sie offen.
    pub fn group_toggle(&mut self, id: WindowId, group: u32) {
        let Some(win) = self.windows.get(&id) else {
            return;
        };
        let Some(collapsed) = win.groups.iter().find(|g| g.id == group).map(|g| g.collapsed) else {
            return;
        };
        if !collapsed && win.tabs[win.active].group == Some(group) {
            let visible = |tab: &Tab| {
                tab.group != Some(group) && !tab.group.is_some_and(|g| win.groups.iter().any(|other| other.id == g && other.collapsed))
            };
            let after = win.tabs.iter().enumerate().skip(win.active).find(|(_, tab)| visible(tab)).map(|(i, _)| i);
            let before = win.tabs.iter().enumerate().take(win.active).rev().find(|(_, tab)| visible(tab)).map(|(i, _)| i);
            match after.or(before) {
                Some(other) => self.activate_tab(id, other),
                None => return,
            }
        }
        if let Some(entry) = self.windows.get_mut(&id).and_then(|w| w.groups.iter_mut().find(|g| g.id == group)) {
            entry.collapsed = !collapsed;
        }
        // Der aktive Tab liegt jetzt außerhalb – `tidy_tabs` klappt nichts wieder auf.
        self.tabs_changed(id);
    }

    pub fn group_edit(&mut self, id: WindowId, group: u32, name: Option<&str>, color: Option<u8>) {
        let Some(entry) = self.windows.get_mut(&id).and_then(|w| w.groups.iter_mut().find(|g| g.id == group)) else {
            return;
        };
        if let Some(name) = name {
            let name: String = name.trim().chars().filter(|c| !c.is_control()).take(40).collect();
            if !name.is_empty() {
                entry.name = name;
            }
        }
        if let Some(color) = color {
            entry.color = color % 8;
        }
        self.tabs_changed(id);
    }

    /// Löst die Gruppe auf – die Tabs bleiben.
    pub fn group_dissolve(&mut self, id: WindowId, group: u32) {
        if let Some(win) = self.windows.get_mut(&id) {
            for tab in &mut win.tabs {
                if tab.group == Some(group) {
                    tab.group = None;
                }
            }
        }
        self.tabs_changed(id);
    }

    /// Schließt die Gruppe samt ihren Tabs.
    pub fn group_close(&mut self, id: WindowId, group: u32) {
        loop {
            let Some(win) = self.windows.get(&id) else {
                return;
            };
            // Der letzte Tab des Fensters bleibt – sonst ginge das Fenster mit zu.
            if win.tabs.len() == 1 {
                break;
            }
            let Some(victim) = win.tabs.iter().position(|tab| tab.group == Some(group)) else {
                break;
            };
            self.close_tab(id, victim);
        }
        self.group_dissolve(id, group);
    }

    // -----------------------------------------------------------------------
    // Container
    // -----------------------------------------------------------------------

    /// Sichert einen Container dauerhaft. `true`, wenn es geklappt hat.
    fn store_container(&self, container: &Container) -> bool {
        let Ok(data) = serde_json::to_string(container) else {
            return false;
        };
        matches!(
            self.store.put_app_record("container", &container.id.to_string(), &data, 64),
            crate::store::RecordPut::Stored
        )
    }

    /// Container sind für alle Fenster gleich – nach einer Änderung bekommen alle
    /// den neuen Stand.
    fn containers_changed(&mut self) {
        for win in self.windows.values_mut() {
            win.dirty = true;
        }
    }

    /// Legt einen neuen Container an (Name, nächste Farbe, eigene Kennung).
    pub fn container_new(&mut self, name: &str) {
        if self.containers.len() >= 24 {
            return;
        }
        let name: String = name.chars().filter(|c| !c.is_control()).take(40).collect();
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        let id = self.containers.iter().map(|container| container.id).max().unwrap_or(0) + 1;
        let color = (self.containers.len() as u8) % 8;
        let uuid = native::new_uuid();
        if uuid.is_empty() {
            return;
        }
        let container = Container { id, name, color, uuid };
        if self.store_container(&container) {
            self.containers.push(container);
            self.containers_changed();
        }
    }

    /// Benennt einen Container um oder färbt ihn neu.
    pub fn container_edit(&mut self, id: u32, name: Option<&str>, color: Option<u8>) {
        let Some(container) = self.containers.iter_mut().find(|container| container.id == id) else {
            return;
        };
        if let Some(name) = name {
            let name: String = name.chars().filter(|c| !c.is_control()).take(40).collect();
            let name = name.trim().to_string();
            if !name.is_empty() {
                container.name = name;
            }
        }
        if let Some(color) = color {
            container.color = color % 8;
        }
        let snapshot = container.clone();
        self.store_container(&snapshot);
        self.containers_changed();
    }

    /// Löscht einen Container und leert seinen Datenspeicher. Tabs, die darin
    /// liefen, kehren ins normale Profil zurück (ihre offenen Seiten erst beim
    /// nächsten Laden).
    pub fn container_delete(&mut self, id: u32) {
        let Some(position) = self.containers.iter().position(|container| container.id == id) else {
            return;
        };
        let container = self.containers.remove(position);
        self.store.remove_app_record("container", &id.to_string());
        native::clear_container_data(&container.uuid);
        for win in self.windows.values_mut() {
            for tab in &mut win.tabs {
                if tab.container == Some(id) {
                    tab.container = None;
                }
            }
        }
        self.session_changed();
        self.containers_changed();
    }

    /// Öffnet einen neuen Tab in diesem Container. Gilt nur in gewöhnlichen
    /// Fenstern – private und Stealth-Fenster kennen keine Container.
    pub fn new_container_tab(&mut self, id: WindowId, container: u32) {
        let ordinary = self.windows.get(&id).is_some_and(|win| !win.private && !win.stealth);
        if !ordinary || !self.containers.iter().any(|entry| entry.id == container) {
            return;
        }
        self.refresh_newtab();
        let Some(win) = self.windows.get_mut(&id) else {
            return;
        };
        let url = loadable("", win.private);
        let mut tab = Tab::new(win.shared.next_id(), url);
        tab.container = Some(container);
        let new_id = tab.id;
        let position = win.tabs.len();
        win.tabs.insert(position, tab);
        win.tidy_tabs();
        let position = win.tabs.iter().position(|tab| tab.id == new_id).unwrap_or(position);
        win.dirty = true;
        self.activate_tab(id, position);
        self.session_changed();
    }

    // -----------------------------------------------------------------------
    // Timer – die Uhr der Zeiterfassung, von der Symbolleiste aus steuerbar.
    // Der Zustand lebt im `run`-Feld des Zettels der Zeit-App; die Zeit-App
    // liest dasselbe Feld, darum bleiben beide ohne Umbau synchron.
    // -----------------------------------------------------------------------

    fn timer_now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    /// Der Zettel der Zeit-App als JSON-Objekt (sonst ein leeres).
    fn time_zettel(&self) -> Value {
        self.store
            .setting(&crate::apps::data_key("time"))
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}))
    }

    /// Was die Symbolleiste über die laufende Uhr wissen muss.
    fn timer_state(&self) -> Value {
        let zettel = self.time_zettel();
        let run = zettel.get("run").cloned().filter(Value::is_object);
        let step = zettel.get("step").and_then(Value::as_i64).filter(|s| *s > 0).unwrap_or(5);
        json!({ "run": run, "step": step })
    }

    /// Schreibt das `run`-Feld in den Zettel der Zeit-App (oder entfernt es) und
    /// bringt alle Fenster auf den neuen Stand.
    fn timer_set_run(&mut self, run: Option<Value>) {
        let mut zettel = self.time_zettel();
        if let Some(object) = zettel.as_object_mut() {
            match run {
                Some(run) => {
                    object.insert("run".into(), run);
                }
                None => {
                    object.remove("run");
                }
            }
        }
        if let Ok(text) = serde_json::to_string(&zettel) {
            self.store.set_setting(&crate::apps::data_key("time"), &text);
        }
        for win in self.windows.values_mut() {
            win.dirty = true;
        }
    }

    /// Notiz und Projekt des zuletzt gebuchten Eintrags – für die Schnellbuchung.
    fn timer_recent(&self) -> (String, String) {
        let mut best: Option<(String, u64, String, String)> = None;
        if let Some((records, _)) = self.store.app_records("time", "", 20_000, 4_000_000) {
            for (_id, data) in records {
                let Ok(value) = serde_json::from_str::<Value>(&data) else {
                    continue;
                };
                if value.get("k").and_then(Value::as_str) != Some("e") {
                    continue;
                }
                let day = value.get("d").and_then(Value::as_str).unwrap_or("").to_string();
                let end = value.get("e").and_then(Value::as_u64).unwrap_or(0);
                if best.as_ref().map_or(true, |(bd, be, _, _)| (day.as_str(), end) > (bd.as_str(), *be)) {
                    let note = value.get("n").and_then(Value::as_str).unwrap_or("").to_string();
                    let project = value.get("p").and_then(Value::as_str).unwrap_or("").to_string();
                    best = Some((day, end, note, project));
                }
            }
        }
        best.map(|(_, _, note, project)| (note, project)).unwrap_or_default()
    }

    /// Startet die Uhr mit Notiz und Projekt des letzten Eintrags. Läuft schon
    /// eine, bleibt sie.
    pub fn timer_start(&mut self) {
        if self.time_zettel().get("run").filter(|value| value.is_object()).is_some() {
            return;
        }
        let (note, project) = self.timer_recent();
        let run = json!({ "p": project, "n": note, "t0": Self::timer_now_ms(), "id": "", "si": 0, "b": [], "pa": 0 });
        self.timer_set_run(Some(run));
    }

    pub fn timer_pause(&mut self) {
        let mut zettel = self.time_zettel();
        let Some(run) = zettel.get_mut("run").filter(|value| value.is_object()) else {
            return;
        };
        if run.get("pa").and_then(Value::as_i64).unwrap_or(0) > 0 {
            return;
        }
        if let Some(object) = run.as_object_mut() {
            object.insert("pa".into(), json!(Self::timer_now_ms()));
        }
        let run = run.clone();
        self.timer_set_run(Some(run));
    }

    pub fn timer_resume(&mut self) {
        let mut zettel = self.time_zettel();
        let Some(run) = zettel.get_mut("run").filter(|value| value.is_object()) else {
            return;
        };
        let paused = run.get("pa").and_then(Value::as_i64).unwrap_or(0);
        if paused <= 0 {
            return;
        }
        let now = Self::timer_now_ms();
        if let Some(breaks) = run.get_mut("b").and_then(Value::as_array_mut) {
            if now > paused {
                breaks.push(json!([paused, now]));
            }
        }
        if let Some(object) = run.as_object_mut() {
            object.insert("pa".into(), json!(0));
        }
        let run = run.clone();
        self.timer_set_run(Some(run));
    }

    /// Bucht den in der Leiste berechneten Eintrag (Zeit-App-Mathe) und stoppt
    /// die Uhr.
    pub fn timer_book(&mut self, entry: Value) {
        let day = entry.get("d").and_then(Value::as_str).unwrap_or("").to_string();
        if !day.is_empty() {
            static TIMER_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let seq = TIMER_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let id = format!("e{}t{:x}{:x}", day.replace('-', ""), Self::timer_now_ms(), seq);
            if let Ok(text) = serde_json::to_string(&entry) {
                let _ = self.store.put_app_record("time", &id, &text, 20_000);
            }
        }
        self.timer_set_run(None);
    }

    /// Sagt allen Fenstern, wie es um die Filterlisten steht (für die Einstellungen).
    pub fn send_filter_info(&mut self) {
        let info = self.blocker.info();
        for win in self.windows.values_mut() {
            win.send(json!({ "t": "filters", "info": info }));
        }
    }

    /// Schaltet den Werbeblocker für die Website des aktiven Tabs ab oder an
    /// und lädt die Seite neu, damit es sofort gilt.
    pub fn set_blocker_for_site(&mut self, id: WindowId, off: bool, expected: &str) {
        let Some(win) = self.windows.get(&id) else {
            return;
        };
        let Some(tab) = win.active_tab() else {
            return;
        };
        let shown = omnibox::bar_url(tab.shown_url());
        if !(shown.starts_with("http://") || shown.starts_with("https://")) {
            return;
        }
        let Some(site) = crate::sites::domain_of(&shown) else {
            return;
        };
        // Der Schalter gilt der Website, die beim Klick dastand. Zeigt der Tab
        // inzwischen eine andere, passiert nichts.
        if site != expected {
            return;
        }
        let (private, index) = (win.private, win.active);
        self.set_site_blocker(&site, off, private);
        self.reload(id, index, false);
    }

    /// Schaltet den Werbeblocker für eine Website ab oder an – in allen Tabs.
    /// Gemerkt wird es nur, wenn es nicht aus einem privaten Fenster kommt.
    pub fn set_site_blocker(&mut self, site: &str, off: bool, private: bool) {
        self.blocker.set_site_off(site, off);
        if !private {
            self.blocker_off.retain(|known| known != site);
            if off {
                self.blocker_off.push(site.to_string());
            }
            self.blocker_off.sort();
            self.blocker_off.truncate(500);
            self.store.set_setting("blocker_off", &serde_json::to_string(&self.blocker_off).unwrap_or_default());
        }
        self.apply_blocker();
        for win in self.windows.values_mut() {
            win.dirty = true;
        }
    }

    /// Das Ladebild steht (oder ist schon wieder weg): Was darunter lag, darf
    /// jetzt erscheinen.
    pub fn release_content(&mut self, id: WindowId) {
        let Some(win) = self.windows.get_mut(&id) else {
            return;
        };
        if !std::mem::take(&mut win.hold_content) {
            return;
        }
        for tab in &win.tabs {
            let shown = win.in_split(tab.id) || win.active_tab().is_some_and(|active| active.id == tab.id);
            if let (true, Some(webview)) = (shown, tab.webview.as_ref()) {
                let _ = webview.set_visible(true);
            }
        }
        native::bring_to_front(&win.chrome);
        self.layout(id);
    }

    /// WebKit hat eine Schutzliste fertig übersetzt (oder es wird nicht länger
    /// gewartet): offene Tabs nachrüsten und die Tabs anlegen, die gewartet haben.
    fn blocker_ready(&mut self) {
        // Spätestens ein paar Sekunden nach dem Start zeigt sich jede Seite –
        // auch wenn sich das Ladebild nie gemeldet haben sollte.
        if STARTED.get().is_some_and(|at| at.elapsed() >= Duration::from_millis(2500)) {
            let held: Vec<WindowId> = self.windows.iter().filter(|(_, w)| w.hold_content).map(|(id, _)| *id).collect();
            for id in held {
                self.release_content(id);
            }
        }
        if self.blocker.pending() && STARTED.get().is_some_and(|at| at.elapsed() >= Duration::from_millis(2500)) {
            self.blocker.give_up_waiting();
        }
        self.blocker.publish();
        self.apply_blocker();
        self.send_filter_info();
        if self.blocker.pending() {
            return;
        }
        let waiting: Vec<(WindowId, usize)> = self
            .windows
            .iter()
            .filter(|(_, win)| win.active_tab().is_some_and(|tab| tab.webview.is_none()))
            .map(|(id, win)| (*id, win.active))
            .collect();
        for (id, index) in waiting {
            self.activate_tab(id, index);
        }
        for (id, tab) in std::mem::take(&mut self.reload_when_ready) {
            if let Some(index) = self.tab_index(id, tab) {
                self.reload(id, index, false);
            }
        }
    }

    /// An den Lesezeichen hat sich etwas geändert: alles nachziehen, was sie
    /// zeigt – Startseite, Leiste, Seitenleiste – und was aus ihnen folgt.
    pub fn bookmarks_changed(&mut self, panel: Option<WindowId>, query: &str) {
        self.sites_stale = true;
        self.refresh_newtab();
        if let Some(window) = panel {
            self.send_panel(window, "bookmarks", query);
        }
        let windows: Vec<WindowId> = self.windows.keys().copied().collect();
        for window in windows {
            self.send_bookmark_bar(window);
        }
        self.mark_all_dirty();
    }

    /// Schickt einem Fenster, was seine Lesezeichenleiste zeigt.
    pub fn send_bookmark_bar(&mut self, window: WindowId) {
        let on = self.settings.bm_bar;
        let (items, folders) = if on {
            let all = self.store.bookmarks("", 400);
            let loose: Vec<&crate::store::Entry> = all.iter().filter(|b| b.folder.is_empty()).take(40).collect();
            let folders: Vec<Value> = self
                .store
                .bookmark_folders()
                .into_iter()
                .map(|name| {
                    let inside: Vec<&crate::store::Entry> = all.iter().filter(|b| b.folder == name).take(60).collect();
                    json!({ "name": name, "items": inside })
                })
                .collect();
            (json!(loose), json!(folders))
        } else {
            (json!([]), json!([]))
        };
        if let Some(win) = self.windows.get_mut(&window) {
            win.send(json!({ "t": "bmbar", "on": on, "items": items, "folders": folders }));
        }
    }

    /// Macht Vorschaubilder der geladenen Tabs eines Fensters – für die
    /// Übersicht aller Tabs. Sie bleiben im Arbeitsspeicher.
    pub fn capture_thumbs(&mut self, window: WindowId) {
        // Nur was gerade zu sehen ist, lässt sich verlässlich abbilden; von den
        // anderen Tabs gibt es das Bild vom letzten Mal, als sie vorn waren.
        let Some(win) = self.windows.get(&window) else {
            return;
        };
        let shown: Vec<usize> = win
            .tabs
            .iter()
            .enumerate()
            .filter(|(index, tab)| *index == win.active || win.in_split(tab.id))
            .map(|(index, _)| index)
            .collect();
        for index in shown {
            self.capture_thumb(window, index);
        }
    }

    /// Macht ein Vorschaubild eines Tabs (er muss gerade zu sehen sein).
    pub fn capture_thumb(&self, window: WindowId, index: usize) {
        let Some(tab) = self.windows.get(&window).and_then(|w| w.tabs.get(index)) else {
            return;
        };
        let Some(webview) = &tab.webview else {
            return;
        };
        let proxy = self.proxy.clone();
        let id = tab.id;
        native::snapshot_jpeg(webview, 560.0, move |bytes| {
            if let Some(bytes) = bytes {
                let _ = proxy.send_event(UserEvent::Thumb { window, tab: id, bytes });
            }
        });
    }

    /// Frischt auf, welche Seiten der Browser kennt – Grundlage der Warnung
    /// vor täuschend ähnlichen Adressen.
    pub fn refresh_sites(&mut self) {
        self.sites_stale = false;
        let (known, seen) = self.store.known_hosts();
        let enabled = self.settings.lookalike;
        native::with_sites(|sites| {
            sites.enabled = enabled;
            sites.rebuild(&known, &seen);
        });
    }

    /// Erlaubt einer Website, Geräte im eigenen Netz anzusprechen – dauerhaft,
    /// oder (`private`, aus einem privaten oder Stealth-Fenster) nur bis zum
    /// Beenden: Von dort landet nichts in den Einstellungen.
    pub fn allow_local_site(&mut self, site: &str, private: bool) {
        let site = crate::sites::split_host(site).1;
        if !native::allow_local_site(&site) || self.local_sites.contains(&site) {
            return;
        }
        if private {
            if !self.local_sites_session.contains(&site) && self.local_sites_session.len() < 200 {
                self.local_sites_session.push(site);
            }
        } else {
            self.local_sites.push(site);
        }
        self.local_sites_changed();
    }

    /// Nimmt die Erlaubnis wieder zurück.
    pub fn forbid_local_site(&mut self, site: &str) {
        self.local_sites.retain(|s| s != site);
        self.local_sites_changed();
    }

    /// Wie viele Websites eine eigene Regel haben (für die Einstellungen).
    pub fn rights_count(&self) -> usize {
        let mut sites: HashSet<&str> = self.media_sites.keys().map(String::as_str).collect();
        sites.extend(self.local_sites.iter().map(String::as_str));
        sites.extend(self.blocker_off.iter().map(String::as_str));
        sites.len()
    }

    /// Eine Antwort mit „merken“ aus dem Dialog zu Kamera und Mikrofon.
    pub fn media_remembered(&mut self, site: &str, rule: crate::media::Rule) {
        let known = self.media_sites.contains_key(site);
        if !crate::media::site_ok(site) || rule.is_empty() || (!known && self.media_sites.len() >= crate::media::MAX_SITES) {
            return;
        }
        self.media_sites.insert(site.to_string(), rule);
        self.media_sites_changed();
        // Wer die Übersicht gerade offen hat, sieht den neuen Stand (die
        // Oberfläche verwirft die Nachricht, wenn etwas anderes offen ist).
        let windows: Vec<WindowId> = self.windows.keys().copied().collect();
        for id in windows {
            self.send_panel(id, "rights", "");
        }
    }

    /// Nimmt eine gemerkte Entscheidung zu Kamera (`camera`) oder Mikrofon
    /// (`mic`) zurück: Die Website wird beim nächsten Mal wieder gefragt.
    pub fn media_forget(&mut self, site: &str, kind: &str) {
        let was_allowed = self.media_sites.get(site).is_some_and(|rule| match kind {
            "camera" => rule.camera == Some(true),
            _ => rule.microphone == Some(true),
        });
        if !crate::media::forget(&mut self.media_sites, site, kind) {
            return;
        }
        self.media_sites_changed();
        if was_allowed {
            // Wer einer Website ein Recht nimmt, meint: ab sofort. Was sie in
            // einem offenen Tab gerade aufnimmt, endet deshalb gleich mit.
            for tab in self.windows.values().flat_map(|win| win.tabs.iter()) {
                let on_site = native::media_site_of(&tab.url).as_deref() == Some(site);
                if let (true, Some(webview)) = (on_site, tab.webview.as_ref()) {
                    native::stop_capture(webview, kind == "camera", kind == "mic");
                }
            }
        }
    }

    fn media_sites_changed(&mut self) {
        native::set_media_rules(self.media_sites.clone());
        self.store.set_setting(
            "media_sites",
            &serde_json::to_string(&self.media_sites).unwrap_or_else(|_| "{}".into()),
        );
    }

    fn local_sites_changed(&mut self) {
        // Gemerkt wird nur das Dauerhafte; es gilt beides.
        let all: Vec<String> = self.local_sites.iter().chain(&self.local_sites_session).cloned().collect();
        native::set_local_sites(all.iter().cloned().collect());
        self.store.set_setting(
            "local_sites",
            &serde_json::to_string(&self.local_sites).unwrap_or_else(|_| "[]".into()),
        );
        self.blocker.compile_local(&all);
    }

    // -----------------------------------------------------------------------
    // Oberfläche
    // -----------------------------------------------------------------------

    fn sync_ui(&mut self, id: WindowId) {
        let Some(win) = self.windows.get(&id) else {
            return;
        };
        let tabs: Vec<Value> = win
            .tabs
            .iter()
            .map(|tab| {
                json!({
                    "id": tab.id,
                    "title": tab.display_title(),
                    "favicon": tab.favicon,
                    "loading": tab.loading,
                    "paired": win.in_split(tab.id),
                    "url": omnibox::pretty_url(tab.shown_url()),
                    "pinned": tab.pinned,
                    "group": tab.group,
                    "container": tab.container,
                })
            })
            .collect();
        let Some(active) = win.active_tab() else {
            return;
        };
        // Ein Fenster, das eine Seite per Skript geöffnet und selbst befüllt hat,
        // hat keine Adresse. Es darf nicht aussehen wie die Startseite (leere
        // Adresszeile) – dort steht dann, was es ist.
        let shown = match active.shown_url() {
            "" if active.opener.is_some() => "about:blank",
            other => other,
        };
        let (can_back, can_forward) = match &active.webview {
            Some(webview) => (
                webview.can_go_back().unwrap_or(false),
                webview.can_go_forward().unwrap_or(false),
            ),
            None => (false, false),
        };
        let bar = omnibox::bar_url(shown);
        let blocked = active.webview.as_ref().map(|webview| crate::blocker::blocked(native::key(webview))).unwrap_or(0);
        // Der Bericht am Schild: die am häufigsten gesperrten Hosts dieser Seite.
        let blocked_hosts: Vec<Value> = if blocked > 0 {
            active
                .webview
                .as_ref()
                .map(|webview| crate::blocker::blocked_hosts(native::key(webview)))
                .unwrap_or_default()
                .into_iter()
                .take(16)
                .map(|(host, n)| json!({ "host": host, "n": n }))
                .collect()
        } else {
            Vec::new()
        };
        // Die Adresse in drei Teilen: Die Adressleiste hebt den Teil hervor,
        // der sagt, wem die Seite gehört.
        let parts = crate::sites::display_parts(&bar);
        // Stehen zwei Tabs nebeneinander, bekommt jede Hälfte ihre eigene
        // Adresszeile – so ist immer zu sehen, welche Seite wo steht, gleich,
        // in welcher gerade gearbeitet wird.
        let panes: Option<Vec<Value>> = win.split.map(|(left, right)| {
            [left, right]
                .iter()
                .filter_map(|id| win.tabs.iter().find(|tab| tab.id == *id))
                .map(|tab| {
                    let shown = tab.shown_url();
                    let address = omnibox::bar_url(shown);
                    json!({
                        "id": tab.id,
                        "pretty": omnibox::pretty_url(shown),
                        "parts": crate::sites::display_parts(&address),
                        "security": omnibox::security(shown),
                    })
                })
                .collect()
        });
        let assistant_width = win.assistant_width();
        // Die eingerichteten Container – damit die Oberfläche sie anbietet und
        // den Tabs ihre Farbe gibt.
        let containers: Vec<Value> = self
            .containers
            .iter()
            .map(|container| json!({ "id": container.id, "name": container.name, "color": container.color }))
            .collect();
        let state = json!({
            "t": "state",
            "tabs": tabs,
            "active": active.id,
            "groups": win.groups,
            "containers": containers,
            "private": win.private,
            "stealth": win.stealth,
            "tor": win.tor,
            "fullscreen": win.fullscreen,
            "assistant": win.assistant_open,
            "split": win.split.is_some(),
            "panes": panes,
            "assistantWidth": assistant_width,
            "nav": {
                "url": bar,
                "pretty": omnibox::pretty_url(shown),
                "parts": parts,
                "security": omnibox::security(shown),
                "loading": active.loading,
                "progress": active.progress,
                "canBack": can_back,
                "canForward": can_forward,
                "bookmarked": !bar.is_empty() && self.store.is_bookmarked(&bar),
                "canBookmark": bar.starts_with("http://") || bar.starts_with("https://"),
                "zoom": (active.zoom * 100.0).round(),
            },
            "downloads": {
                "active": self.active_downloads.len(),
                "any": !self.active_downloads.is_empty() || !self.store.downloads(1).is_empty(),
            },
            "logins": self.logins_state(id),
            "reader": active.reader,
            "canRead": active.readable || active.reader,
            "check": active.check.view,
            "translated": active.translation.is_some(),
            "canTranslate": active.translation.is_some() || crate::translate::foreign(&active.lang),
            "mobile": active.mobile,
            "ampel": self.ampel(id),
            "timer": self.timer_state(),
            "shield": {
                "on": self.settings.blocker || win.stealth,
                "site": Some(&bar)
                    .filter(|bar| bar.starts_with("http://") || bar.starts_with("https://"))
                    .and_then(|bar| crate::sites::domain_of(bar)),
                "off": crate::blocker::site_off(&bar),
                "count": blocked,
                "hosts": blocked_hosts,
            },
        });
        let title = active.display_title();
        if let Some(win) = self.windows.get_mut(&id) {
            win.dirty = false;
            win.blocked_shown = blocked;
            win.send(state);
            if win.window_title != title {
                win.window.set_title(&title);
                win.window_title = title;
            }
        }
        // Der Assistent erfährt, welche Seite jetzt vor ihm liegt.
        self.assistant_page(id);
    }

    /// Zeigt einen Hinweis unter der Symbolleiste, optional mit Knopf.
    pub fn notice(
        &mut self,
        id: WindowId,
        text: String,
        action: Option<(&str, NoticeAction)>,
        timeout_ms: u64,
    ) {
        self.notice_with(id, text, action, timeout_ms, 0);
    }

    /// Wie `notice`, nennt aber die Kennung des Hinweises – wer sie hat, kann
    /// seinen Text später ändern (`noticeText`) oder ihn schließen (`noticeHide`).
    pub fn notice_id(
        &mut self,
        id: WindowId,
        text: String,
        action: Option<(&str, NoticeAction)>,
        timeout_ms: u64,
    ) -> u64 {
        self.notice_with(id, text, action, timeout_ms, 0)
    }

    /// Ein Hinweis, der auffallen soll: eine Warnung.
    pub fn warn(&mut self, id: WindowId, text: String, timeout_ms: u64) {
        self.notice_with(id, text, None, timeout_ms, 1);
    }

    /// Eine Warnung, die stehen bleiben muss: Nichts, was eine Seite auslösen
    /// kann, schiebt sie in ihren ersten Sekunden beiseite (Passwort am falschen Ort).
    pub fn warn_keep(&mut self, id: WindowId, text: String, timeout_ms: u64) {
        self.notice_with(id, text, None, timeout_ms, 2);
    }

    /// Ein Hinweis, den die Seite eines Tabs ausgelöst hat. Er erscheint nur,
    /// solange dieser Tab zu sehen ist, und gehört zu der Website, die der Tab
    /// in diesem Moment zeigt. `false`: nicht gezeigt.
    pub fn notice_from(
        &mut self,
        id: WindowId,
        tab: TabId,
        text: String,
        action: Option<(&str, NoticeAction)>,
        timeout_ms: u64,
    ) -> bool {
        self.notice_from_id(id, tab, text, action, timeout_ms).is_some()
    }

    /// Wie `notice_from`, nennt aber die Kennung des Hinweises (`None`: nicht
    /// gezeigt) – wer sie hat, erfährt über `noticeGone`, wann er wieder verschwand.
    pub fn notice_from_id(
        &mut self,
        id: WindowId,
        tab: TabId,
        text: String,
        action: Option<(&str, NoticeAction)>,
        timeout_ms: u64,
    ) -> Option<u64> {
        let win = self.windows.get(&id)?;
        let site = win.tabs.iter().find(|candidate| candidate.id == tab).map(|tab| crate::shield::host_of(&tab.url))?;
        if !win.shows(tab) {
            return None;
        }
        let owned = action.is_some();
        let notice = self.notice_with(id, text, action, timeout_ms, 0);
        if owned {
            self.notice_owners.insert(notice, (id, tab, site));
        }
        Some(notice)
    }

    /// Hinweise, die zu einem Tab gehören, haben sich erledigt. Mit `site`
    /// bleiben die stehen, die zu genau dieser Website gehören (der Tab ist
    /// innerhalb derselben Website weitergegangen).
    pub fn notices_end(&mut self, id: WindowId, tab: TabId, site: Option<&str>) {
        let gone: Vec<u64> = self
            .notice_owners
            .iter()
            .filter(|(_, (window, owner, from))| *window == id && *owner == tab && site != Some(from.as_str()))
            .map(|(notice, _)| *notice)
            .collect();
        for notice in gone {
            self.notice_owners.remove(&notice);
            // Stand dort die Frage einer App nach dem Standort, bleibt sie damit
            // unbeantwortet – das erfährt die App (der Tab kann sie noch zeigen).
            if let Some(NoticeAction::AppLocation(tab_window, tab, app, n)) = self.notices.remove(&notice) {
                self.app_location_unanswered(tab_window, tab, app, n);
            }
            if let Some(win) = self.windows.get_mut(&id) {
                win.send(json!({ "t": "noticeHide", "id": notice }));
            }
        }
    }

    /// Nach einem Wechsel des Tabs: Hinweise von Tabs, die nicht mehr zu sehen sind, verschwinden.
    pub fn notices_end_hidden(&mut self, id: WindowId) {
        let Some(win) = self.windows.get(&id) else {
            return;
        };
        let hidden: Vec<TabId> = self
            .notice_owners
            .values()
            .filter(|(window, tab, _)| *window == id && !win.shows(*tab))
            .map(|(_, tab, _)| *tab)
            .collect();
        for tab in hidden {
            self.notices_end(id, tab, None);
        }
    }

    /// Gilt der Knopf eines Hinweises noch der Seite, die ihn ausgelöst hat?
    /// Hinweise ohne Besitzer (Download fertig, Einstellungen) gelten immer.
    pub fn notice_still_valid(&mut self, notice: u64) -> bool {
        let Some((window, tab, site)) = self.notice_owners.remove(&notice) else {
            return true;
        };
        self.windows.get(&window).is_some_and(|win| {
            win.shows(tab) && win.tabs.iter().any(|candidate| candidate.id == tab && crate::shield::host_of(&candidate.url) == site)
        })
    }

    /// `level`: 0 gewöhnlich, 1 Warnung, 2 Warnung, die stehen bleiben muss.
    fn notice_with(
        &mut self,
        id: WindowId,
        text: String,
        action: Option<(&str, NoticeAction)>,
        timeout_ms: u64,
        level: u8,
    ) -> u64 {
        self.next_notice += 1;
        let notice_id = self.next_notice;
        let label = action.as_ref().map(|(label, _)| label.to_string());
        if let Some((_, action)) = action {
            self.notices.insert(notice_id, action);
            if self.notices.len() > 50 {
                if let Some(oldest) = self.notices.keys().min().copied() {
                    self.notices.remove(&oldest);
                    self.notice_owners.remove(&oldest);
                }
            }
        }
        if let Some(win) = self.windows.get_mut(&id) {
            win.send(json!({
                "t": "notice",
                "id": notice_id,
                "text": text,
                "action": label,
                "timeout": timeout_ms,
                "warn": level > 0,
                "keep": level > 1,
            }));
        }
        notice_id
    }

    /// Lädt offene Startseiten neu – nachdem sich geändert hat, was sie zeigen.
    /// (Private Fenster haben ihre eigene Startseite ohne Lesezeichen.)
    pub fn reload_start_pages(&mut self) {
        for tab in self.windows.values().flat_map(|win| win.tabs.iter()) {
            if let (true, Some(webview)) = (tab.url == NEWTAB, tab.webview.as_ref()) {
                let _ = webview.reload();
            }
        }
    }

    /// Bereitet die Daten der Startseite vor (Lesezeichen und oft Besuchtes).
    pub fn refresh_newtab(&mut self) {
        let wallpaper = self
            .store
            .setting("wallpaper")
            .filter(|id| crate::wallpapers::exists(id))
            .unwrap_or_default();
        // Lesezeichen liegen nur auf der Startseite, wenn das eingeschaltet ist
        // (der „Schreibtisch“, auf dem sie sich anordnen lassen). Sonst bekommt
        // die Seite sie gar nicht erst.
        let (bookmarks, folders) = if self.settings.start_bookmarks {
            (self.store.bookmarks("", 300), self.store.bookmark_folders())
        } else {
            (Vec::new(), Vec::new())
        };
        let data = json!({
            "bookmarks": bookmarks,
            "folders": folders,
            "top": self.store.top_sites(12),
            "wallpaper": wallpaper,
            "wallpapers": crate::wallpapers::list(),
        });
        if let Ok(mut slot) = self.newtab.lock() {
            *slot = data.to_string();
        }
    }

    // -----------------------------------------------------------------------
    // Sitzung
    // -----------------------------------------------------------------------

    pub fn session_changed(&mut self) {
        if self.session_dirty.is_none() {
            self.session_dirty = Some(Instant::now());
        }
    }

    fn session_file(&self) -> PathBuf {
        self.data_dir.join("session.json")
    }

    /// Schreibt die offenen (nicht privaten) Fenster auf die Platte. Gibt es
    /// keines mehr, bleibt die zuletzt gesicherte Sitzung stehen.
    pub fn save_session(&mut self) {
        self.session_dirty = None;
        let mut session = Session::default();
        for id in &self.focus_order {
            let Some(win) = self.windows.get(id) else {
                continue;
            };
            if win.private {
                continue;
            }
            let tabs: Vec<SessionTab> = win
                .tabs
                .iter()
                .map(|tab| SessionTab {
                    url: omnibox::error_retry(&tab.url).unwrap_or_else(|| tab.url.clone()),
                    title: tab.title.clone(),
                    pinned: tab.pinned,
                    group: tab.group,
                    container: tab.container,
                })
                .collect();
            let scale = win.window.scale_factor();
            let position = win
                .window
                .outer_position()
                .map(|p| p.to_logical::<f64>(scale))
                .unwrap_or(LogicalPosition::new(80.0, 80.0));
            let (w, h) = win.size();
            session.windows.push(SessionWindow {
                tabs,
                groups: win.groups.clone(),
                active: win.active,
                x: position.x,
                y: position.y,
                w,
                h,
            });
        }
        if session.windows.is_empty() {
            return;
        }
        let Ok(text) = serde_json::to_string(&session) else {
            return;
        };
        let file = self.session_file();
        let temp = file.with_extension("json.tmp");
        if std::fs::write(&temp, text).is_ok() {
            // Die offenen Seiten gehen nur den eigenen Nutzer etwas an.
            crate::store::private_file(&temp);
            let _ = std::fs::rename(&temp, &file);
        }
    }

    fn restore_session(&mut self, target: &Target, pinned_only: bool) {
        let Ok(text) = std::fs::read_to_string(self.session_file()) else {
            return;
        };
        let Ok(session) = serde_json::from_str::<Session>(&text) else {
            return;
        };
        for mut window in session.windows.into_iter().take(12) {
            window.tabs.truncate(400);
            if pinned_only {
                window.tabs.retain(|tab| tab.pinned);
                window.groups.clear();
                window.active = 0;
            }
            let saved: Vec<(bool, Option<u32>, Option<u32>)> =
                window.tabs.iter().map(|tab| (tab.pinned, tab.group, tab.container)).collect();
            let groups: Vec<TabGroup> = window
                .groups
                .iter()
                .take(40)
                .map(|group| TabGroup {
                    id: group.id,
                    name: group.name.chars().filter(|c| !c.is_control()).take(40).collect(),
                    color: group.color % 8,
                    collapsed: group.collapsed,
                })
                .collect();
            let seeds: Vec<(String, String)> = window.tabs.into_iter().map(|t| (t.url, t.title)).collect();
            if seeds.is_empty() {
                continue;
            }
            // Auch die gesicherte Sitzung ist nur eine Datei: Unsinnige Maße
            // sollen kein Fenster ins Nirgendwo oder ins Riesenhafte schicken.
            let sane = |value: f64, low: f64, high: f64| if value.is_finite() { value.clamp(low, high) } else { low };
            let frame = Frame {
                x: sane(window.x, -4000.0, 12000.0),
                y: sane(window.y, 0.0, 8000.0),
                w: sane(window.w, 420.0, 10000.0),
                h: sane(window.h, 300.0, 8000.0),
            };
            let Some(id) = self.create_window(target, false, seeds, window.active, Some(frame)) else {
                continue;
            };
            if let Some(win) = self.windows.get_mut(&id) {
                for (tab, (pinned, group, container)) in win.tabs.iter_mut().zip(saved) {
                    tab.pinned = pinned;
                    tab.group = group;
                    tab.container = container;
                }
                win.groups = groups;
                win.tidy_tabs();
            }
        }
    }

    // -----------------------------------------------------------------------
    // Prüfhilfe (nur mit ZUNDER_DEBUG): Befehle über die Standardeingabe
    // -----------------------------------------------------------------------

    fn handle_debug(&mut self, line: &str, target: &Target) {
        let (command, rest) = line.split_once(' ').unwrap_or((line, ""));
        let window = self.focused();
        match command {
            "ui" => {
                if let Some(window) = window {
                    self.handle_ui(window, rest, target);
                }
            }
            "menu" => self.command(window, rest.trim(), target),
            "lights" => {
                if let Some(win) = window.and_then(|id| self.windows.get(&id)) {
                    println!("LIGHTS {}", native::traffic_lights(&win.chrome));
                    let _ = std::io::stdout().flush();
                }
            }
            "resize" => {
                let mut parts = rest.split_whitespace().filter_map(|p| p.parse::<f64>().ok());
                if let (Some(w), Some(h), Some(win)) =
                    (parts.next(), parts.next(), window.and_then(|id| self.windows.get(&id)))
                {
                    win.window.set_inner_size(LogicalSize::new(w, h));
                }
            }
            "hooks" => {
                println!("HOOKS {}", native::installed_hooks().join(" "));
                let _ = std::io::stdout().flush();
            }
            // geofake <breite> <länge>: als käme dieser Standort von macOS –
            // durch dieselbe Stelle, ohne die Ortung zu berühren.
            "geofake" => {
                let mut parts = rest.split_whitespace().map(|part| part.parse::<f64>());
                if let (Some(Ok(latitude)), Some(Ok(longitude))) = (parts.next(), parts.next()) {
                    crate::geo::deliver_for_test(latitude, longitude);
                }
            }
            "abortmodal" => native::abort_modal_after(rest.trim().parse().unwrap_or(1.0)),
            // Verwirft, was für einen Dialog geplant, aber nicht geschehen ist.
            "modalclear" => native::forget_modal_plans(),
            // „modalclick 2.5 Erlauben“: nach 2,5 Sekunden den Knopf mit dieser Aufschrift drücken.
            "modalclick" => {
                let (seconds, title) = rest.trim().split_once(' ').unwrap_or((rest.trim(), ""));
                native::click_modal_after(seconds.parse().unwrap_or(1.0), title.trim());
            }
            "appearance" => native::force_appearance(rest.trim() == "dark"),
            "echo" => {
                println!("ECHO {rest}");
                let _ = std::io::stdout().flush();
            }
            "dump" => {
                let windows: Vec<Value> = self
                    .focus_order
                    .iter()
                    .filter_map(|id| self.windows.get(id))
                    .map(|win| {
                        let tabs: Vec<Value> = win
                            .tabs
                            .iter()
                            .map(|t| {
                                json!({
                                    "id": t.id, "url": t.url, "pending": t.pending,
                                    "title": t.title, "loading": t.loading,
                                    "favicon": t.favicon, "live": t.webview.is_some(),
                                    "zoom": t.zoom, "reader": t.reader,
                                    "lang": t.lang, "translation": t.translation.as_ref().map(|translation| translation.dump()),
                                    "check": t.check.view,
                                    "blocked": t.webview.as_ref().map(|w| crate::blocker::blocked(native::key(w))),
                                    "adsOff": crate::blocker::site_off(&t.url),
                                    "login": {
                                        "form": t.login.form, "signup": t.login.signup,
                                        "userStep": t.login.user_step,
                                        "candidate": t.login.candidate.is_some(),
                                        "pending": t.login.pending.is_some(),
                                    },
                                })
                            })
                            .collect();
                        json!({
                            "private": win.private, "stealth": win.stealth, "tor": win.tor, "active": win.active, "tabs": tabs,
                            "overlay": win.overlay, "chromeHeight": win.chrome_height,
                            "uiReady": win.ui_ready, "size": win.size(),
                            "assistant": win.assistant_open,
                            "assistantWidth": win.assistant_width(),
                            "aiBusy": win.ai_job.is_some(),
                            "split": win.split.map(|(left, right)| vec![left, right]),
                            "bubble": win.bubble_state.as_ref().map(|b| json!({
                                "kind": format!("{:?}", b.kind), "tab": b.tab,
                                "visible": b.visible_since.is_some(), "anchored": b.anchor.is_some(),
                                "height": win.bubble_height,
                                "frame": win.bubble.as_ref().and_then(|v| v.bounds().ok()).map(|r| {
                                    let scale = win.window.scale_factor();
                                    let p = r.position.to_logical::<f64>(scale);
                                    let s = r.size.to_logical::<f64>(scale);
                                    vec![p.x, p.y, s.width, s.height]
                                }),
                            })),
                        })
                    })
                    .collect();
                println!(
                    "STATE {}",
                    json!({
                        "windows": windows,
                        "downloads": self.store.downloads(20),
                        "logins": self.store.login_count(),
                        "localSites": self.local_sites,
                        "localRules": self.blocker.local_ready(),
                        "filters": self.blocker.info(),
                        "blockerOff": self.blocker_off,
                        "authAsked": self.next_auth,
                        "unlocked": {
                            "fill": self.fill_unlocked_until.is_some_and(|until| Instant::now() < until),
                            "panel": self.panel_unlocked_until.is_some_and(|until| Instant::now() < until),
                        },
                    })
                );
                let _ = std::io::stdout().flush();
            }
            "snap" => {
                // Bilder des Rahmens und der aktiven Seite in einen Ordner schreiben.
                let dir = PathBuf::from(rest.trim());
                if let Some(win) = window.and_then(|id| self.windows.get(&id)) {
                    native::snapshot(&win.chrome, dir.join("chrome.png"));
                    if let Some(webview) = win.active_webview() {
                        native::snapshot(webview, dir.join("content.png"));
                    }
                    // Stehen zwei Tabs nebeneinander: beide Hälften einzeln.
                    if let Some((left, right)) = win.split {
                        for (id, name) in [(left, "left.png"), (right, "right.png")] {
                            let webview = win.tabs.iter().find(|t| t.id == id).and_then(|t| t.webview.as_ref());
                            if let Some(webview) = webview {
                                native::snapshot(webview, dir.join(name));
                            }
                        }
                    }
                    if let (true, Some(assistant)) = (win.assistant_open, win.assistant.as_ref()) {
                        native::snapshot(assistant, dir.join("assistant.png"));
                    }
                    if let (true, Some(bubble)) = (win.bubble_state.is_some(), win.bubble.as_ref()) {
                        native::snapshot(bubble, dir.join("bubble.png"));
                    }
                }
            }
            "filechosen" => {
                // filechosen <zweck> <pfad>: als wäre im Dateidialog gewählt worden.
                let (purpose, path) = rest.split_once(' ').unwrap_or((rest, ""));
                let purpose = match purpose {
                    "pw-import" => "pw-import",
                    "pw-export" => "pw-export",
                    "bm-import" => "bm-import",
                    "bm-export" => "bm-export",
                    _ => return,
                };
                if let Some(window) = window {
                    self.file_chosen(window, purpose, Some(path.trim().to_string()));
                }
            }
            "tap" => {
                // tap <page|ui|bubble|ai> <x> <y>: ein echter Mausklick (gilt der Seite als vom Menschen).
                let mut parts = rest.split_whitespace();
                let target = parts.next().unwrap_or("");
                let x = parts.next().and_then(|p| p.parse::<f64>().ok());
                let y = parts.next().and_then(|p| p.parse::<f64>().ok());
                let Some(win) = window.and_then(|id| self.windows.get(&id)) else {
                    return;
                };
                let webview = match target {
                    "ui" => Some(&win.chrome),
                    "bubble" => win.bubble.as_ref(),
                    "ai" => win.assistant.as_ref(),
                    _ => win.active_webview(),
                };
                if let (Some(webview), Some(x), Some(y)) = (webview, x, y) {
                    native::click(webview, x, y);
                }
            }
            "type" => {
                // type <text>: echte Eingabe in das Feld, das in der aktiven Seite den Fokus hat.
                if let Some(webview) = window.and_then(|id| self.windows.get(&id)).and_then(|w| w.active_webview()) {
                    let _ = webview.focus();
                    native::type_text(webview, rest);
                }
            }
            "isojs" => {
                // Skript in Zunders abgeschotteter Welt der aktiven Seite ausführen.
                if let Some(webview) = window.and_then(|id| self.windows.get(&id)).and_then(|w| w.active_webview()) {
                    let proxy = self.proxy.clone();
                    native::eval_isolated(webview, rest, move |result| {
                        let _ = proxy.send_event(UserEvent::DebugOutput(format!("ISOJS {}", result.unwrap_or_default())));
                    });
                }
            }
            "jsin" => {
                // jsin <tab-nr> <code>: Skript in einem bestimmten Tab ausführen.
                let (index, code) = rest.split_once(' ').unwrap_or((rest, ""));
                let webview = window
                    .and_then(|id| self.windows.get(&id))
                    .and_then(|w| w.tabs.get(index.parse::<usize>().unwrap_or(0)))
                    .and_then(|t| t.webview.as_ref());
                if let Some(webview) = webview {
                    let proxy = self.proxy.clone();
                    let _ = webview.evaluate_script_with_callback(code, move |result| {
                        let _ = proxy.send_event(UserEvent::DebugOutput(format!("JSIN {result}")));
                    });
                }
            }
            "js" | "uijs" | "aijs" | "bubjs" => {
                let Some(win) = window.and_then(|id| self.windows.get(&id)) else {
                    return;
                };
                let webview = match command {
                    "uijs" => Some(&win.chrome),
                    "aijs" => win.assistant.as_ref(),
                    "bubjs" => win.bubble.as_ref(),
                    _ => win.active_webview(),
                };
                if let Some(webview) = webview {
                    let proxy = self.proxy.clone();
                    let tag = command.to_uppercase();
                    let _ = webview.evaluate_script_with_callback(rest, move |result| {
                        let _ = proxy.send_event(UserEvent::DebugOutput(format!("{tag} {result}")));
                    });
                }
            }
            _ => {}
        }
    }
}
