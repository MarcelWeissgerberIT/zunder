//! Passwortmanager im Browser: was mit den Meldungen aus den Seiten geschieht,
//! die Sprechblase am Anmeldefeld, Sichern, Ausfüllen und die Verwaltung.
//!
//! Grundsätze:
//! - Ausgefüllt wird nur auf einen Klick des Menschen, nie von selbst.
//! - Ein Passwort geht nur an die Website, für die es gesichert wurde. Das
//!   entscheidet das Skript in der Seite selbst, unmittelbar bevor es das Feld
//!   füllt – an der Herkunft des Dokuments, in dem es läuft. Der Browser prüft
//!   vorher gegen die Adresse des Tabs, verlässt sich darauf aber nicht.
//! - Ein Passwort steht nie im Quelltext eines Skripts: Es reist als Argument
//!   in Zunders abgeschottete Welt der Seite.
//! - Nur der Hauptrahmen einer Seite zählt; eingebettete Rahmen bekommen nichts.
//! - Touch ID bzw. das Passwort des Macs: vor dem Ausfüllen (abschaltbar), in
//!   der Verwaltung vor dem Anzeigen, Kopieren und Ändern, und jedes Mal eigens
//!   vor dem Exportieren und vor dem Abschalten der Abfrage. Eine Freigabe zum
//!   Ausfüllen gilt nicht für die Verwaltung und umgekehrt.
//! - In privaten Fenstern wird ausgefüllt, aber nie gesichert.

use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tao::window::WindowId;

use crate::app::{App, NoticeAction, TabId, UserEvent};
use crate::i18n::t;
use crate::native::{self, BridgeMessage};
use crate::store::LoginRow;
use crate::vault::{self, Site, VaultError, MAX_PASSWORD, MAX_USERNAME};
use crate::webviews::{build_bubble, rect};

/// Breite der Sprechblase samt dem Rand, den ihr Schatten braucht.
const BUBBLE_WIDTH: f64 = 340.0;
const BUBBLE_MARGIN: f64 = 10.0;
/// Erst so lange nach dem Erscheinen nimmt die Sprechblase Klicks an. So lässt
/// sich niemand dazu bringen, auf eine Stelle zu klicken, an der sie dann
/// unter dem Mauszeiger auftaucht.
const BUBBLE_ARMED_AFTER: Duration = Duration::from_millis(400);
/// So lange gilt eine abgeschickte Anmeldung als „gerade eben“.
const CANDIDATE_SURE: Duration = Duration::from_secs(120);
/// Ein bloßer Klick im Formular zählt nur, wenn die Seite gleich darauf wechselt.
const CANDIDATE_WEAK: Duration = Duration::from_secs(15);
/// So lange bleibt eine unbeantwortete Frage „sichern?“ am Schlüssel-Knopf stehen.
const PENDING_KEEP: Duration = Duration::from_secs(300);
/// Nach Touch ID: so lange füllt Zunder ohne erneute Abfrage aus (für Anmeldungen in zwei Schritten).
const UNLOCK_FILL: Duration = Duration::from_secs(45);
/// Nach Touch ID in der Verwaltung: so lange bleiben die Passwörter dort zugänglich.
const UNLOCK_PANEL: Duration = Duration::from_secs(300);
/// So lange darf nach der Freigabe einer Ausfuhr deren Ziel gewählt werden.
const EXPORT_WINDOW: Duration = Duration::from_secs(300);
/// Ein kopiertes Passwort verschwindet nach dieser Zeit aus der Zwischenablage.
const CLIPBOARD_SECONDS: u64 = 90;
/// Mehr Zeilen nimmt eine Einfuhr nicht an.
const IMPORT_LIMIT: usize = 20_000;

/// Rumpf der Funktion, die in der Seite ausfüllt. Die Anmeldung kommt als
/// Argument `payload` an. Geprüft wird, dass wirklich Zunders eigenes Objekt
/// antwortet – nicht ein Element der Seite, das zufällig oder absichtlich so heißt.
const FILL_BODY: &str = "try {\
  if (window.top !== window) return 'origin';\
  var own = Object.prototype.hasOwnProperty.call(globalThis, '__zv') ? globalThis.__zv : null;\
  if (!own || typeof own.fill !== 'function') return 'nofield';\
  return String(own.fill(JSON.parse(payload)));\
} catch (error) { return 'error'; }";

/// Welcher Name steht im Formular? (für ein neu vorgeschlagenes Passwort)
const USER_BODY: &str = "try {\
  var own = Object.prototype.hasOwnProperty.call(globalThis, '__zv') ? globalThis.__zv : null;\
  return own && typeof own.user === 'function' ? String(own.user()) : '';\
} catch (error) { return ''; }";

/// Die Vorschläge stehen am Schlüssel-Knopf: Ein Klick in die Seite soll sie schließen.
const SHOWN_BODY: &str = "try {\
  var own = Object.prototype.hasOwnProperty.call(globalThis, '__zv') ? globalThis.__zv : null;\
  if (own && typeof own.shown === 'function') own.shown();\
} catch (error) {}\
return '';";

/// Was ein Tab über Anmeldungen auf seiner Seite weiß.
#[derive(Default)]
pub struct TabLogin {
    /// Die Seite hat ein Anmeldeformular / eines zum Registrieren / nur ein Namensfeld.
    pub form: bool,
    pub signup: bool,
    pub user_step: bool,
    /// Zuletzt abgeschickte Zugangsdaten – wird nach dem Seitenwechsel zur Frage.
    pub candidate: Option<Candidate>,
    /// Die offene Frage „sichern?“.
    pub pending: Option<PendingSave>,
    /// Name aus dem ersten Schritt einer Anmeldung in zwei Schritten (Website, Name, wann).
    pub last_user: Option<(String, String, Instant)>,
}

pub struct Candidate {
    pub site: Site,
    pub username: String,
    pub password: String,
    /// Beim Ändern eines Passworts: das bisherige, wie es im Formular stand.
    pub old: Option<String>,
    /// Wirklich abgeschickt (Formular, Eingabetaste) – nicht bloß ein Klick im Formular.
    pub sure: bool,
    pub at: Instant,
}

pub struct PendingSave {
    pub site: Site,
    pub username: String,
    pub password: String,
    /// Nummer der Anmeldung, deren Passwort ersetzt würde.
    pub update: Option<i64>,
    pub at: Instant,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum BubbleKind {
    Fill,
    Save,
}

/// Was die Sprechblase gerade zeigt.
pub struct BubbleState {
    pub kind: BubbleKind,
    pub tab: TabId,
    /// Für welche Herkunft sie etwas anbietet.
    pub origin: String,
    /// Lage des Feldes in der Seite (x, y, Breite, Höhe) – laut Seite, also nur ein Hinweis.
    pub anchor: Option<[f64; 4]>,
    pub seq: u64,
    /// Das vorgeschlagene neue Passwort, solange die Blase es anbietet.
    pub suggestion: Option<String>,
    /// Seit wann sie zu sehen ist.
    pub visible_since: Option<Instant>,
}

/// Wofür eine bestandene Abfrage (Touch ID) gilt.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Gate {
    /// Ausfüllen – gilt kurz weiter, aber nur dafür.
    Fill,
    /// Anzeigen, Kopieren und Ändern in der Verwaltung, solange sie offen ist.
    Panel,
    /// Jedes Mal eigens: Exportieren, Abschalten der Abfrage.
    Always,
}

/// Was nach bestandener Abfrage geschehen soll.
pub enum AuthThen {
    Fill { window: WindowId, tab: TabId, login: i64 },
    Reveal { window: WindowId, login: i64 },
    CopyPassword { window: WindowId, login: i64 },
    Edit { window: WindowId, login: i64, username: String, password: Option<String>, query: String },
    Export { window: WindowId },
    DisableAuth { window: WindowId },
}

fn error_text(error: &VaultError) -> String {
    match error {
        VaultError::NoKey | VaultError::Damaged => t("pw_err_unreadable").to_string(),
        VaultError::KeyLost => t("pw_err_keylost").to_string(),
        VaultError::Denied(_) => t("pw_err_keychain").to_string(),
        VaultError::Other(detail) => format!("{} ({detail})", t("pw_err_other")),
    }
}

/// Läuft die Seite auf diesem Mac selbst? Dann liest unterwegs niemand mit.
fn same_machine(host: &str) -> bool {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    host == "localhost"
        || host.ends_with(".localhost")
        || bare.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

fn text_of(data: &Value, key: &str, limit: usize) -> Option<String> {
    let text = data.get(key)?.as_str()?;
    (text.len() <= limit).then(|| text.to_string())
}

fn still(until: Option<Instant>) -> bool {
    until.is_some_and(|until| Instant::now() < until)
}

impl crate::app::BrowserWindow {
    /// Schickt eine Nachricht an die Sprechblase dieses Fensters.
    fn bubble_send(&mut self, message: Value) {
        let script = format!("window.__zunderBubble&&window.__zunderBubble({message})");
        match (&self.bubble, self.bubble_ready) {
            (Some(bubble), true) => {
                let _ = bubble.evaluate_script(&script);
            }
            _ => self.bubble_queue.push(script),
        }
    }

    /// Ist dieser Tab gerade zu sehen (aktiv oder Teil des nebeneinander gezeigten Paars)?
    pub(crate) fn shows(&self, tab: TabId) -> bool {
        self.active_tab().is_some_and(|t| t.id == tab) || self.in_split(tab)
    }
}

impl App {
    /// Anmeldungen, die auf dieser Website angeboten werden dürfen.
    fn offered_logins(&self, site: &Site) -> Vec<LoginRow> {
        self.store
            .logins_for_site(&site.key)
            .into_iter()
            .filter(|row| vault::offered(&row.origin, site))
            .collect()
    }

    fn tab_site(&self, window: WindowId, tab: TabId) -> Option<Site> {
        let win = self.windows.get(&window)?;
        let tab = win.tabs.iter().find(|t| t.id == tab)?;
        vault::site_of(&tab.url)
    }

    /// Verschlüsselt ein Passwort. Ein neuer Schlüssel entsteht dabei nur,
    /// solange noch nichts gesichert ist.
    fn seal_login(&mut self, origin: &str, username: &str, password: &str) -> Result<Vec<u8>, VaultError> {
        // Eine Website mit gesichertem Passwort zählt zu den bekannten Seiten.
        self.sites_stale = true;
        let fresh = self.store.login_count() == 0;
        self.vault.seal(origin, username, password, fresh)
    }

    /// Wird hier gerade das Passwort einer ANDEREN Website eingegeben? Dann
    /// warnt Zunder sofort – das ist das sicherste Zeichen einer Betrugsseite,
    /// und auch sonst ein Grund, kurz innezuhalten. Verglichen wird im Browser
    /// mit den gesicherten Passwörtern; nichts davon verlässt den Mac, und die
    /// Seite erfährt nichts.
    fn check_reuse(&mut self, window: WindowId, site: &Site, password: &str) {
        if password.chars().count() < 8 || self.store.login_count() == 0 {
            return;
        }
        let here = crate::sites::split_host(&site.host).1;
        // Ein gesichertes Passwort ist verschlüsselt genau 29 Bytes länger als
        // im Klartext – alles andere braucht gar nicht erst geöffnet zu werden.
        let sealed_len = password.len() + 29;
        let mut foreign: Option<(i64, String)> = None;
        for row in self.store.logins("", 20000) {
            if row.secret.len() != sealed_len {
                continue;
            }
            let Some(saved) = vault::site_of(&row.origin) else {
                continue;
            };
            match self.vault.open(&row.origin, &row.username, &row.secret) {
                Ok(saved_password) if saved_password == password => {}
                Ok(_) => continue,
                // Ohne Schlüssel gibt es nichts zu vergleichen.
                Err(_) => return,
            }
            let there = crate::sites::split_host(&saved.host).1;
            if there == here {
                // Dasselbe Passwort ist auch für diese Website gesichert: alles in Ordnung.
                return;
            }
            foreign.get_or_insert((row.id, there));
        }
        let Some((login, known)) = foreign else {
            return;
        };
        if !self.reuse_warned.insert((here.clone(), login)) {
            return;
        }
        let text = t("pw_reuse_warn").replace("{known}", &known).replace("{here}", &here);
        self.warn(window, text, 25000);
    }

    /// Verlangt das Ausfüllen Touch ID? Der Schalter dafür liegt beim Schlüssel
    /// im Schlüsselbund; ohne ihn gilt: ja.
    fn fill_needs_auth(&mut self) -> bool {
        !self.vault.fill_without_auth()
    }

    // -----------------------------------------------------------------------
    // Meldungen aus den Seiten
    // -----------------------------------------------------------------------

    /// Nachricht von Zunders Skript in einer Seite. Der Kanal ist für die Seite
    /// selbst nicht erreichbar; Rahmen und Herkunft nennt WebKit.
    pub fn handle_bridge(&mut self, message: BridgeMessage) {
        if !message.main_frame {
            return;
        }
        let Some((window, index)) = self.find_webview(message.webview) else {
            return;
        };
        let Some(site) = vault::site_of(&message.origin) else {
            return;
        };
        let Ok(data) = serde_json::from_str::<Value>(&message.body) else {
            return;
        };
        let Some(win) = self.windows.get_mut(&window) else {
            return;
        };
        let private = win.private;
        let Some(tab) = win.tabs.get(index) else {
            return;
        };
        let tab_id = tab.id;
        let visible = win.shows(tab_id);
        // Die Meldung muss zu der Seite gehören, die der Tab gerade zeigt.
        if vault::site_of(&tab.url).map(|s| s.origin) != Some(site.origin.clone()) {
            return;
        }
        let tab = &mut win.tabs[index];
        let flag = |key: &str| data.get(key).and_then(Value::as_bool).unwrap_or(false);
        match data.get("t").and_then(Value::as_str).unwrap_or("") {
            "forms" => {
                let (form, signup, user_step) = (flag("login"), flag("signup"), flag("user"));
                if (tab.login.form, tab.login.signup, tab.login.user_step) != (form, signup, user_step) {
                    tab.login.form = form;
                    tab.login.signup = signup;
                    tab.login.user_step = user_step;
                    win.dirty = true;
                }
            }
            "focus" => {
                // Vorschläge gibt es nur zu einem Tab, der gerade zu sehen ist –
                // sonst stünden sie über einer fremden Seite.
                if !visible {
                    return;
                }
                let number = |key: &str| {
                    data.get(key)
                        .and_then(Value::as_f64)
                        .filter(|v| v.is_finite())
                        .map(|v| v.clamp(-20000.0, 20000.0))
                };
                let (Some(x), Some(y), Some(w), Some(h)) = (number("x"), number("y"), number("w"), number("h")) else {
                    return;
                };
                let fresh = data.get("kind").and_then(Value::as_str) == Some("new");
                self.show_fill_bubble(window, tab_id, site, Some([x, y, w, h]), fresh, false);
            }
            "blur" => {
                let showing = win
                    .bubble_state
                    .as_ref()
                    .is_some_and(|b| b.kind == BubbleKind::Fill && b.tab == tab_id);
                if showing {
                    self.hide_bubble(window);
                }
            }
            "move" => {
                // Das Feld hat sich in der Seite verschoben: Die Blase rückt nach.
                let number = |key: &str| {
                    data.get(key)
                        .and_then(Value::as_f64)
                        .filter(|v| v.is_finite())
                        .map(|v| v.clamp(-20000.0, 20000.0))
                };
                let (Some(x), Some(y), Some(w), Some(h)) = (number("x"), number("y"), number("w"), number("h")) else {
                    return;
                };
                let anchored = win
                    .bubble_state
                    .as_mut()
                    .filter(|b| b.kind == BubbleKind::Fill && b.tab == tab_id && b.anchor.is_some());
                if let Some(state) = anchored {
                    state.anchor = Some([x, y, w, h]);
                    self.place_bubble(window);
                }
            }
            "user" => {
                if let Some(user) = text_of(&data, "u", MAX_USERNAME).filter(|u| !u.is_empty()) {
                    tab.login.last_user = Some((site.key, user, Instant::now()));
                }
            }
            "icon" => {
                // Das Icon der Seite, von Zunders Skript in der Seite frisch auf
                // 32 × 32 Punkte gezeichnet. Angenommen wird nur genau das: ein
                // kleines PNG als Text – nie eine Adresse zum Nachladen.
                let Some(data) = text_of(&data, "d", 12000) else {
                    return;
                };
                if !crate::store::is_icon(&data) {
                    return;
                }
                if tab.favicon.as_deref() != Some(data.as_str()) {
                    let url = tab.url.clone();
                    tab.favicon = Some(data.clone());
                    win.dirty = true;
                    if !private {
                        self.store.set_favicon(&url, &data);
                    }
                }
            }
            "typed" => {
                // Kurz nach der letzten Taste in einem Passwortfeld: Gehört das
                // Getippte zu einer anderen Website?
                if let Some(password) = text_of(&data, "p", MAX_PASSWORD) {
                    self.check_reuse(window, &site, &password);
                }
            }
            "submit" => {
                // Auch wer schneller abschickt, als die Prüfung beim Tippen
                // greift, erfährt es – dann eben einen Augenblick danach.
                if let Some(password) = text_of(&data, "p", MAX_PASSWORD) {
                    self.check_reuse(window, &site, &password);
                }
                let Some(win) = self.windows.get_mut(&window) else {
                    return;
                };
                let Some(tab) = win.tabs.get_mut(index) else {
                    return;
                };
                if private {
                    return;
                }
                let Some(password) = text_of(&data, "p", MAX_PASSWORD).filter(|p| !p.is_empty()) else {
                    return;
                };
                let mut username = text_of(&data, "u", MAX_USERNAME).unwrap_or_default();
                if username.is_empty() {
                    // Zweiter Schritt einer Anmeldung in zwei Schritten: der Name kam vorher.
                    if let Some((key, user, at)) = &tab.login.last_user {
                        if *key == site.key && at.elapsed() < Duration::from_secs(600) {
                            username = user.clone();
                        }
                    }
                }
                let mut sure = flag("sure");
                if let Some(old) = &tab.login.candidate {
                    if old.site == site && old.username == username && old.password == password {
                        sure |= old.sure;
                    }
                }
                let old = text_of(&data, "o", MAX_PASSWORD).filter(|o| !o.is_empty());
                tab.login.candidate = Some(Candidate { site, username, password, old, sure, at: Instant::now() });
            }
            "reader" => self.reader_message(window, index, &data),
            "readable" => {
                if tab.readable != flag("on") {
                    tab.readable = flag("on");
                    win.dirty = true;
                }
            }
            "gone" => {
                // Das Passwortfeld ist nach dem Abschicken verschwunden, ohne dass die Seite neu lud.
                let recent = tab.login.candidate.as_ref().is_some_and(|c| c.at.elapsed() < CANDIDATE_SURE);
                if recent {
                    self.offer_save(window, tab_id);
                } else {
                    tab.login.candidate = None;
                }
            }
            _ => {}
        }
    }

    /// Ein Tab hat eine neue Seite übernommen.
    pub fn login_committed(&mut self, window: WindowId, index: usize) {
        let Some(win) = self.windows.get_mut(&window) else {
            return;
        };
        let Some(tab) = win.tabs.get_mut(index) else {
            return;
        };
        let tab_id = tab.id;
        tab.login.form = false;
        tab.login.signup = false;
        tab.login.user_step = false;
        if tab.login.pending.as_ref().is_some_and(|p| p.at.elapsed() > PENDING_KEEP) {
            tab.login.pending = None;
        }
        // Wer selbst eine Adresse eingibt, hat sich nicht gerade angemeldet.
        let typed = tab.typed;
        let offer = match &tab.login.candidate {
            Some(candidate) => {
                let limit = if candidate.sure { CANDIDATE_SURE } else { CANDIDATE_WEAK };
                !typed && candidate.at.elapsed() < limit
            }
            None => false,
        };
        if !offer {
            tab.login.candidate = None;
        }
        // Vorschläge der alten Seite verschwinden mit ihr.
        if win.bubble_state.as_ref().is_some_and(|b| b.tab == tab_id && b.kind == BubbleKind::Fill) {
            self.hide_bubble(window);
        }
        if offer {
            self.offer_save(window, tab_id);
        }
    }

    /// Prüft die zuletzt abgeschickten Zugangsdaten und fragt, ob sie gesichert
    /// werden sollen – außer es gibt nichts Neues daran.
    fn offer_save(&mut self, window: WindowId, tab_id: TabId) {
        let Some(win) = self.windows.get_mut(&window) else {
            return;
        };
        let private = win.private;
        let Some(tab) = win.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return;
        };
        let Some(candidate) = tab.login.candidate.take() else {
            return;
        };
        if private || !self.settings.pw_offer || self.store.is_never_save(&candidate.site.key) {
            return;
        }
        let existing = self.offered_logins(&candidate.site);
        // Wurde ein Passwort geändert, zeigt das bisherige, welches Konto gemeint ist.
        let mut changed_account = None;
        for row in &existing {
            let opened = self.vault.open(&row.origin, &row.username, &row.secret);
            if let (Ok(saved), Some(old)) = (&opened, &candidate.old) {
                if saved == old && changed_account.is_none() {
                    changed_account = Some(row.id);
                }
            }
            match opened {
                Ok(saved) if saved == candidate.password => {
                    if row.username.is_empty() && !candidate.username.is_empty() {
                        // Ein vorgeschlagenes Passwort, gesichert, bevor der Name feststand: Namen nachtragen.
                        if let Ok(secret) = self.seal_login(&row.origin, &candidate.username, &candidate.password) {
                            self.store.update_login(row.id, &candidate.username, &secret);
                        }
                        return;
                    }
                    if row.username == candidate.username || candidate.username.is_empty() {
                        // Genau so schon gesichert.
                        self.store.touch_login(row.id);
                        return;
                    }
                }
                // Ohne Zugang zum Schlüsselbund lässt sich nicht vergleichen – dann nicht fragen.
                Err(VaultError::Denied(_)) => return,
                _ => {}
            }
        }
        let update = changed_account.or_else(|| {
            if candidate.username.is_empty() {
                // Die Seite zeigt keinen Namen: am ehesten die zuletzt benutzte Anmeldung.
                existing.first().map(|row| row.id)
            } else {
                existing.iter().find(|row| row.username == candidate.username).map(|row| row.id)
            }
        });
        let username = match update.and_then(|id| existing.iter().find(|row| row.id == id)) {
            Some(row) => row.username.clone(),
            None => candidate.username.clone(),
        };
        let Some(win) = self.windows.get_mut(&window) else {
            return;
        };
        if let Some(tab) = win.tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.login.pending = Some(PendingSave {
                site: candidate.site,
                username,
                password: candidate.password,
                update,
                at: Instant::now(),
            });
        }
        win.dirty = true;
        // Die Frage zeigt sich nur am Tab, der gerade zu sehen ist; sonst wartet sie am Schlüssel-Knopf.
        if win.active_tab().is_some_and(|t| t.id == tab_id) {
            self.show_save_bubble(window, tab_id);
        }
    }

    // -----------------------------------------------------------------------
    // Sprechblase
    // -----------------------------------------------------------------------

    fn show_bubble(&mut self, window: WindowId, mut state: BubbleState, mut content: Value) {
        let Some(win) = self.windows.get_mut(&window) else {
            return;
        };
        if win.bubble.is_none() {
            match build_bubble(&win.shared, rect(0.0, 0.0, BUBBLE_WIDTH, 120.0)) {
                Ok(bubble) => win.bubble = Some(bubble),
                Err(error) => {
                    eprintln!("zunder: Sprechblase konnte nicht angelegt werden: {error}");
                    return;
                }
            }
        }
        win.bubble_seq += 1;
        state.seq = win.bubble_seq;
        // Neuer Inhalt, neue Frist: Bis die Blase ihre Höhe gemeldet hat, gilt sie
        // als nicht zu sehen und nimmt nichts an.
        state.visible_since = None;
        if let Some(bubble) = &win.bubble {
            let _ = bubble.set_visible(false);
        }
        content["seq"] = json!(state.seq);
        win.bubble_state = Some(state);
        win.bubble_send(content);
    }

    pub fn hide_bubble(&mut self, window: WindowId) {
        let Some(win) = self.windows.get_mut(&window) else {
            return;
        };
        if win.bubble_state.take().is_none() {
            return;
        }
        if let Some(bubble) = &win.bubble {
            let _ = bubble.set_visible(false);
        }
        // Auch der Inhalt verschwindet – nichts bleibt in der Blase stehen.
        win.bubble_send(json!({ "t": "hide" }));
    }

    /// Rückt die Sprechblase an ihre Stelle: unter das Feld – oder oben rechts
    /// in die Seite, unter den Schlüssel-Knopf.
    pub fn place_bubble(&mut self, window: WindowId) {
        let Some(win) = self.windows.get(&window) else {
            return;
        };
        let (Some(state), Some(bubble)) = (&win.bubble_state, &win.bubble) else {
            return;
        };
        let Some(tab) = win.tabs.iter().find(|t| t.id == state.tab) else {
            return;
        };
        let (px, py, pw, ph) = win.pane_frame(tab.id);
        let (width, height) = (BUBBLE_WIDTH, win.bubble_height);
        let (mut x, mut y) = match state.anchor {
            Some([ax, ay, _aw, ah]) => {
                let zoom = tab.zoom;
                let below = py + (ay + ah) * zoom + 2.0 - BUBBLE_MARGIN;
                let above = py + ay * zoom - 2.0 - height + BUBBLE_MARGIN;
                let y = if below + height > py + ph && above >= py { above } else { below };
                (px + ax * zoom - BUBBLE_MARGIN, y)
            }
            None => (px + pw - width - 2.0, py - 2.0),
        };
        x = x.min(px + pw - width).max(px);
        y = y.min(py + ph - height).max(py - BUBBLE_MARGIN);
        let _ = bubble.set_bounds(rect(x, y, width, height));
    }

    /// Hat die Sprechblase lange genug sichtbar gestanden, um einen Klick anzunehmen?
    fn bubble_armed(&self, window: WindowId) -> bool {
        self.windows
            .get(&window)
            .and_then(|w| w.bubble_state.as_ref())
            .and_then(|b| b.visible_since)
            .is_some_and(|since| since.elapsed() >= BUBBLE_ARMED_AFTER)
    }

    /// Vorschläge zu einem Anmeldefeld (oder, ohne Feld, zum Schlüssel-Knopf)
    /// für die Website `site`.
    fn show_fill_bubble(
        &mut self,
        window: WindowId,
        tab_id: TabId,
        site: Site,
        anchor: Option<[f64; 4]>,
        fresh: bool,
        always: bool,
    ) {
        let private = self.windows.get(&window).is_none_or(|w| w.private);
        let accounts: Vec<Value> = self
            .offered_logins(&site)
            .iter()
            .take(8)
            .map(|row| json!({ "id": row.id, "user": row.username }))
            .collect();
        // Ein neues Passwort vorschlagen heißt auch, es zu sichern – also nicht in privaten Fenstern.
        let suggestion = (fresh && !private && self.settings.pw_offer).then(vault::generate_password);
        if accounts.is_empty() && suggestion.is_none() && !always {
            // Nichts anzubieten: keine Blase.
            let stale = self
                .windows
                .get(&window)
                .and_then(|w| w.bubble_state.as_ref())
                .is_some_and(|b| b.kind == BubbleKind::Fill);
            if stale {
                self.hide_bubble(window);
            }
            return;
        }
        // Der Hinweis „mit Touch ID“ – nur nachsehen, wenn es etwas auszufüllen gibt.
        let auth = !accounts.is_empty() && self.fill_needs_auth() && !still(self.fill_unlocked_until);
        let content = json!({
            "t": "fill",
            "host": site.key,
            "accounts": accounts,
            "suggest": suggestion,
            "auth": auth,
        });
        let state = BubbleState {
            kind: BubbleKind::Fill,
            tab: tab_id,
            origin: site.origin,
            anchor,
            seq: 0,
            suggestion,
            visible_since: None,
        };
        self.show_bubble(window, state, content);
    }

    fn show_save_bubble(&mut self, window: WindowId, tab_id: TabId) {
        let Some(win) = self.windows.get(&window) else {
            return;
        };
        let Some(pending) = win.tabs.iter().find(|t| t.id == tab_id).and_then(|t| t.login.pending.as_ref()) else {
            return;
        };
        let content = json!({
            "t": "save",
            "host": pending.site.key,
            "user": pending.username,
            "update": pending.update.is_some(),
            "dots": pending.password.chars().count().min(14),
            "insecure": !pending.site.secure && !same_machine(&pending.site.host),
        });
        let state = BubbleState {
            kind: BubbleKind::Save,
            tab: tab_id,
            origin: pending.site.origin.clone(),
            anchor: None,
            seq: 0,
            suggestion: None,
            visible_since: None,
        };
        self.show_bubble(window, state, content);
    }

    /// Der Tab im Vordergrund hat gewechselt (oder das Fenster sich verändert):
    /// Vorschläge zum alten Feld gelten nicht mehr.
    pub fn bubble_context_changed(&mut self, window: WindowId) {
        let Some(win) = self.windows.get(&window) else {
            return;
        };
        let active = win.active_tab().map(|t| t.id);
        if win.bubble_state.as_ref().is_some_and(|b| Some(b.tab) != active || b.kind == BubbleKind::Fill) {
            self.hide_bubble(window);
        } else {
            self.place_bubble(window);
        }
    }

    // -----------------------------------------------------------------------
    // Zustand für die Symbolleiste
    // -----------------------------------------------------------------------

    /// Was der Schlüssel-Knopf in der Adressleiste zeigen soll.
    pub fn logins_state(&self, window: WindowId) -> Value {
        let Some(tab) = self.windows.get(&window).and_then(|w| w.active_tab()) else {
            return json!({ "show": false });
        };
        let Some(site) = vault::site_of(&tab.url) else {
            return json!({ "show": false });
        };
        let saved = self.offered_logins(&site).len();
        let pending = tab.login.pending.is_some();
        let form = tab.login.form || tab.login.signup || tab.login.user_step;
        json!({
            // Der Knopf erscheint, wo es etwas zum Anmelden gibt – oder eine Frage offen ist.
            "show": pending || form,
            "saved": saved,
            "pending": pending,
            "form": form,
        })
    }

    // -----------------------------------------------------------------------
    // Befehle aus Sprechblase und Oberfläche
    // -----------------------------------------------------------------------

    pub fn handle_logins(&mut self, window: WindowId, command: &str, message: &Value) {
        let number = |key: &str| message.get(key).and_then(Value::as_i64).unwrap_or(0);
        let text = |key: &str| message.get(key).and_then(Value::as_str).unwrap_or("").to_string();
        // Befehle der Sprechblase gelten nur für das, was sie gerade zeigt.
        let bubble_tab = |app: &App| -> Option<(TabId, BubbleKind)> {
            let state = app.windows.get(&window)?.bubble_state.as_ref()?;
            (state.seq == number("seq") as u64).then_some((state.tab, state.kind))
        };

        match command {
            "pwBubbleReady" => {
                if let Some(win) = self.windows.get_mut(&window) {
                    win.bubble_ready = true;
                    if let Some(bubble) = &win.bubble {
                        for script in std::mem::take(&mut win.bubble_queue) {
                            let _ = bubble.evaluate_script(&script);
                        }
                    }
                }
            }
            "pwBubbleSize" => {
                if bubble_tab(self).is_none() {
                    return;
                }
                let height = message.get("h").and_then(Value::as_f64).unwrap_or(120.0).clamp(40.0, 460.0);
                if let Some(win) = self.windows.get_mut(&window) {
                    win.bubble_height = height;
                }
                self.place_bubble(window);
                if let Some(win) = self.windows.get_mut(&window) {
                    if let Some(bubble) = &win.bubble {
                        let _ = bubble.set_visible(true);
                        native::bring_to_front(bubble);
                    }
                    // Menüs und Seitenleisten des Rahmens liegen darüber.
                    if win.overlay {
                        native::bring_to_front(&win.chrome);
                    }
                    if let Some(state) = win.bubble_state.as_mut() {
                        if state.visible_since.is_none() {
                            state.visible_since = Some(Instant::now());
                        }
                    }
                }
            }
            "pwFill" => {
                let Some((tab, BubbleKind::Fill)) = bubble_tab(self) else {
                    return;
                };
                if self.bubble_armed(window) {
                    self.request_fill(window, tab, number("id"));
                }
            }
            "pwUse" => {
                let Some((tab, BubbleKind::Fill)) = bubble_tab(self) else {
                    return;
                };
                if self.bubble_armed(window) {
                    self.use_suggestion(window, tab);
                }
            }
            "pwSave" | "pwNever" | "pwDismiss" => {
                let Some((tab, BubbleKind::Save)) = bubble_tab(self) else {
                    return;
                };
                if self.bubble_armed(window) {
                    self.answer_save(window, tab, command);
                }
            }
            "pwTimeout" => {
                // Unbeantwortet: Die Frage bleibt am Schlüssel-Knopf stehen.
                if let Some((_, BubbleKind::Save)) = bubble_tab(self) {
                    self.hide_bubble(window);
                }
            }
            "pwManage" => {
                self.hide_bubble(window);
                if let Some(win) = self.windows.get_mut(&window) {
                    let _ = win.chrome.focus();
                    win.send(json!({ "t": "show", "what": "passwords" }));
                }
            }
            "pwBubbleClose" => {
                self.hide_bubble(window);
                self.focus_content(window);
            }
            // Der Schlüssel-Knopf in der Adressleiste.
            "pwKey" => {
                let Some(win) = self.windows.get(&window) else {
                    return;
                };
                let Some(tab) = win.active_tab() else {
                    return;
                };
                let tab_id = tab.id;
                let open_here = win.bubble_state.as_ref().is_some_and(|b| b.tab == tab_id && b.anchor.is_none());
                if open_here {
                    self.hide_bubble(window);
                } else if tab.login.pending.is_some() {
                    self.show_save_bubble(window, tab_id);
                } else if let Some(site) = vault::site_of(&tab.url) {
                    let fresh = tab.login.signup && !tab.login.form;
                    if let Some(webview) = &tab.webview {
                        native::call_isolated(webview, SHOWN_BODY, "", |_| {});
                    }
                    self.show_fill_bubble(window, tab_id, site, None, fresh, true);
                }
            }
            // ----- Verwaltung -------------------------------------------------
            "pwReveal" => {
                let login = number("id");
                self.with_auth(AuthThen::Reveal { window, login }, "pw_reason_show", Gate::Panel);
            }
            "pwCopy" => {
                let login = number("id");
                if text("what") == "username" {
                    if let Some(row) = self.store.login(login) {
                        native::copy_text(&row.username, false);
                        self.notice(window, t("pw_copied_user").to_string(), None, 2500);
                    }
                } else {
                    self.with_auth(AuthThen::CopyPassword { window, login }, "pw_reason_show", Gate::Panel);
                }
            }
            "pwDelete" => {
                self.sites_stale = true;
                self.store.delete_login(number("id"));
                self.send_panel(window, "passwords", &text("q"));
                self.mark_all_dirty();
            }
            "pwEdit" => {
                let username = text("username").trim().to_string();
                let password = Some(text("password")).filter(|p| !p.is_empty());
                if username.len() > MAX_USERNAME || password.as_ref().is_some_and(|p| p.len() > MAX_PASSWORD) {
                    return;
                }
                let then = AuthThen::Edit { window, login: number("id"), username, password, query: text("q") };
                self.with_auth(then, "pw_reason_change", Gate::Panel);
            }
            "pwAdd" => self.add_login(window, &text("url"), text("username").trim(), &text("password"), &text("q")),
            "pwAllow" => {
                self.store.allow_save(&text("site"));
                self.send_panel(window, "passwords", &text("q"));
            }
            "pwImport" => {
                if let Some(win) = self.windows.get(&window) {
                    native::choose_file(&win.chrome, window, "pw-import", None);
                }
            }
            "pwExport" => self.with_auth(AuthThen::Export { window }, "pw_reason_export", Gate::Always),
            // Die Verwaltung wurde geschlossen: wieder zuschließen. Auch eine
            // Abfrage, die noch läuft, öffnet danach nichts mehr.
            "pwLock" => {
                self.panel_unlocked_until = None;
                self.export_until = None;
                let stale: Vec<u64> = self
                    .auth_waiting
                    .iter()
                    .filter(|(_, (_, gate))| *gate != Gate::Fill)
                    .map(|(token, _)| *token)
                    .collect();
                for token in stale {
                    self.auth_waiting.remove(&token);
                    native::auth_finished(token);
                }
            }
            _ => {}
        }
    }

    /// Führt etwas aus, das Touch ID verlangt – ohne erneute Abfrage nur, wenn
    /// für genau diesen Zweck gerade erst gefragt wurde.
    fn with_auth(&mut self, then: AuthThen, reason_key: &str, gate: Gate) {
        let open = match gate {
            Gate::Fill => still(self.fill_unlocked_until),
            Gate::Panel => still(self.panel_unlocked_until),
            Gate::Always => false,
        };
        if open {
            self.run_authorized(then);
            return;
        }
        self.next_auth += 1;
        let token = self.next_auth;
        self.auth_waiting.insert(token, (then, gate));
        native::authenticate(t(reason_key), token);
    }

    /// Antwort auf eine Abfrage per Touch ID bzw. Passwort des Macs.
    pub fn auth_result(&mut self, token: u64, ok: bool) {
        native::auth_finished(token);
        let Some((then, gate)) = self.auth_waiting.remove(&token) else {
            return;
        };
        if ok {
            match gate {
                Gate::Fill => self.fill_unlocked_until = Some(Instant::now() + UNLOCK_FILL),
                Gate::Panel => self.panel_unlocked_until = Some(Instant::now() + UNLOCK_PANEL),
                Gate::Always => {}
            }
            self.run_authorized(then);
        } else if let AuthThen::Reveal { window, login } = then {
            // Die Oberfläche wartet auf eine Antwort.
            if let Some(win) = self.windows.get_mut(&window) {
                win.send(json!({ "t": "pwSecret", "id": login, "password": Value::Null }));
            }
        }
    }

    fn run_authorized(&mut self, then: AuthThen) {
        match then {
            AuthThen::Fill { window, tab, login } => self.fill_now(window, tab, login),
            AuthThen::Reveal { window, login } => {
                let password = self.open_login(window, login);
                if let Some(win) = self.windows.get_mut(&window) {
                    win.send(json!({ "t": "pwSecret", "id": login, "password": password }));
                }
            }
            AuthThen::CopyPassword { window, login } => {
                if let Some(password) = self.open_login(window, login) {
                    let count = native::copy_text(&password, true);
                    self.clipboard_count = Some(count);
                    let proxy = self.proxy.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_secs(CLIPBOARD_SECONDS));
                        let _ = proxy.send_event(UserEvent::ClipboardExpire(count));
                    });
                    self.notice(window, t("pw_copied").to_string(), None, 4000);
                }
            }
            AuthThen::Edit { window, login, username, password, query } => {
                self.edit_login(window, login, &username, password, &query)
            }
            AuthThen::Export { window } => {
                // Die Freigabe gilt für genau eine Ausfuhr, deren Ziel jetzt gewählt wird.
                self.export_until = Some(Instant::now() + EXPORT_WINDOW);
                if let Some(win) = self.windows.get(&window) {
                    native::choose_file(&win.chrome, window, "pw-export", Some(t("pw_export_name")));
                }
            }
            AuthThen::DisableAuth { window } => {
                let fresh = self.store.login_count() == 0;
                if let Err(error) = self.vault.set_fill_without_auth(true, fresh) {
                    self.notice(window, error_text(&error), None, 6000);
                }
                self.send_panel(window, "passwords", "");
            }
        }
    }

    /// Schaltet „Touch ID vor dem Ausfüllen“ um. Abschalten verlangt selbst
    /// Touch ID – jedes Mal.
    pub fn set_fill_auth(&mut self, window: WindowId, on: bool) {
        if on {
            let fresh = self.store.login_count() == 0;
            if let Err(error) = self.vault.set_fill_without_auth(false, fresh) {
                self.notice(window, error_text(&error), None, 6000);
            }
            self.send_panel(window, "passwords", "");
        } else {
            self.with_auth(AuthThen::DisableAuth { window }, "pw_reason_change", Gate::Always);
        }
    }

    /// Entschlüsselt das Passwort einer Anmeldung; meldet Fehler als Hinweis.
    fn open_login(&mut self, window: WindowId, login: i64) -> Option<String> {
        let row = self.store.login(login)?;
        match self.vault.open(&row.origin, &row.username, &row.secret) {
            Ok(password) => Some(password),
            Err(error) => {
                self.notice(window, error_text(&error), None, 6000);
                None
            }
        }
    }

    // -----------------------------------------------------------------------
    // Ausfüllen
    // -----------------------------------------------------------------------

    /// Befehl „Anmeldung ausfüllen“ aus der Menüleiste: die zuletzt benutzte dieser Website.
    pub fn fill_command(&mut self, window: WindowId) {
        let Some(tab) = self.windows.get(&window).and_then(|w| w.active_tab()) else {
            return;
        };
        let tab_id = tab.id;
        let first = vault::site_of(&tab.url).and_then(|site| self.offered_logins(&site).first().map(|row| row.id));
        match first {
            Some(login) => self.request_fill(window, tab_id, login),
            None => self.notice(window, t("pw_none_here").to_string(), None, 3500),
        }
    }

    fn request_fill(&mut self, window: WindowId, tab: TabId, login: i64) {
        self.hide_bubble(window);
        if self.fill_needs_auth() {
            self.with_auth(AuthThen::Fill { window, tab, login }, "pw_reason_fill", Gate::Fill);
        } else {
            self.fill_now(window, tab, login);
        }
    }

    fn fill_now(&mut self, window: WindowId, tab_id: TabId, login: i64) {
        let Some(row) = self.store.login(login) else {
            return;
        };
        // Wo steht der Tab jetzt? Nur dorthin, wofür die Anmeldung gesichert wurde.
        // (Das Skript prüft die Herkunft seines Dokuments gleich noch einmal selbst.)
        let Some(site) = self.tab_site(window, tab_id) else {
            return;
        };
        if !vault::offered(&row.origin, &site) {
            return;
        }
        let password = match self.vault.open(&row.origin, &row.username, &row.secret) {
            Ok(password) => password,
            Err(error) => {
                self.notice(window, error_text(&error), None, 6000);
                return;
            }
        };
        let request = json!({ "origin": site.origin, "u": row.username, "p": password, "mode": "login" });
        self.run_fill(window, tab_id, login, &request);
    }

    /// Lässt Zunders Skript in der Seite ausfüllen. Die Anmeldung geht als
    /// Argument hinein, nicht als Quelltext.
    fn run_fill(&mut self, window: WindowId, tab_id: TabId, login: i64, request: &Value) {
        let Some(webview) = self
            .windows
            .get(&window)
            .and_then(|w| w.tabs.iter().find(|t| t.id == tab_id))
            .and_then(|t| t.webview.as_ref())
        else {
            return;
        };
        let proxy = self.proxy.clone();
        native::call_isolated(webview, FILL_BODY, &request.to_string(), move |result| {
            let _ = proxy.send_event(UserEvent::Filled { window, tab: tab_id, login, result });
        });
    }

    /// Das Skript meldet, was aus dem Ausfüllen wurde.
    pub fn filled(&mut self, window: WindowId, tab_id: TabId, login: i64, result: Option<String>) {
        let private = self.windows.get(&window).is_none_or(|w| w.private);
        match result.as_deref() {
            Some("ok") | Some("user") => {
                if !private && login > 0 {
                    self.store.touch_login(login);
                }
                if let Some(win) = self.windows.get(&window) {
                    let webview = win.tabs.iter().find(|t| t.id == tab_id).and_then(|t| t.webview.as_ref());
                    if let (Some(webview), false) = (webview, win.overlay) {
                        let _ = webview.focus();
                    }
                }
            }
            // Die Seite ist inzwischen eine andere – dann passiert nichts.
            Some("origin") => {}
            _ => self.notice(window, t("pw_no_field").to_string(), None, 4000),
        }
    }

    // -----------------------------------------------------------------------
    // Neues Passwort vorschlagen
    // -----------------------------------------------------------------------

    /// Das vorgeschlagene Passwort wurde angenommen: erst nachsehen, welcher
    /// Name im Formular steht, dann sichern und einsetzen.
    fn use_suggestion(&mut self, window: WindowId, tab_id: TabId) {
        let Some(win) = self.windows.get_mut(&window) else {
            return;
        };
        let Some(state) = win.bubble_state.as_mut() else {
            return;
        };
        let Some(password) = state.suggestion.take() else {
            return;
        };
        // Für diese Herkunft wurde der Vorschlag gemacht – dabei bleibt es.
        let origin = state.origin.clone();
        let webview = win.tabs.iter().find(|t| t.id == tab_id).and_then(|t| t.webview.as_ref());
        let Some(webview) = webview else {
            return;
        };
        let proxy = self.proxy.clone();
        native::call_isolated(webview, USER_BODY, "", move |user| {
            let _ = proxy.send_event(UserEvent::Suggested {
                window,
                tab: tab_id,
                origin: origin.clone(),
                user: user.unwrap_or_default(),
                password: password.clone(),
            });
        });
        self.hide_bubble(window);
    }

    pub fn suggestion_accepted(&mut self, window: WindowId, tab_id: TabId, origin: String, user: String, password: String) {
        let Some(site) = vault::site_of(&origin) else {
            return;
        };
        // Steht der Tab inzwischen woanders, verfällt der Vorschlag.
        if self.tab_site(window, tab_id).map(|s| s.origin) != Some(site.origin.clone()) {
            return;
        }
        let private = self.windows.get(&window).is_none_or(|w| w.private);
        if private {
            return;
        }
        let user: String = user.chars().take(MAX_USERNAME).collect();
        // Gibt es diese Anmeldung schon, wird ihr Passwort nicht still ersetzt –
        // dann fragt Zunder nach dem Abschicken wie bei jedem geänderten Passwort.
        if self.store.login_exists(&site.origin, &user) {
            self.notice(window, t("pw_suggest_pending").to_string(), None, 6000);
        } else {
            match self.seal_login(&site.origin, &user, &password) {
                Ok(secret) => {
                    self.store.save_login(&site.origin, &site.key, &user, &secret);
                    self.notice(window, t("pw_suggest_saved").to_string(), None, 5000);
                }
                Err(error) => {
                    // Ohne Tresor kein Vorschlag: sonst stünde ein Passwort im Formular, das niemand kennt.
                    self.notice(window, error_text(&error), None, 6000);
                    return;
                }
            }
        }
        let request = json!({ "origin": site.origin, "u": "", "p": password, "mode": "new" });
        self.run_fill(window, tab_id, 0, &request);
        self.mark_all_dirty();
    }

    // -----------------------------------------------------------------------
    // Sichern
    // -----------------------------------------------------------------------

    fn answer_save(&mut self, window: WindowId, tab_id: TabId, answer: &str) {
        self.hide_bubble(window);
        let Some(win) = self.windows.get_mut(&window) else {
            return;
        };
        win.dirty = true;
        let Some(pending) = win.tabs.iter_mut().find(|t| t.id == tab_id).and_then(|t| t.login.pending.take()) else {
            return;
        };
        match answer {
            "pwNever" => {
                self.store.never_save(&pending.site.key);
                let text = t("pw_never_set").replace("{host}", &pending.site.key);
                self.notice(window, text, None, 4000);
            }
            "pwSave" => {
                let saved = self.save_pending(&pending);
                match saved {
                    Ok(true) => self.notice(window, t("pw_saved").to_string(), None, 3000),
                    Ok(false) => self.notice(window, t("pw_err_other").to_string(), None, 5000),
                    Err(error) => self.notice(window, error_text(&error), None, 6000),
                }
            }
            _ => {}
        }
        self.focus_content(window);
    }

    /// Schreibt die Antwort „sichern“ fest: ersetzt das Passwort der gemeinten
    /// Anmeldung oder legt eine neue an.
    fn save_pending(&mut self, pending: &PendingSave) -> Result<bool, VaultError> {
        // Die Anmeldung, die ersetzt werden soll, muss noch dieselbe sein wie
        // beim Fragen – Nummern allein sagen das nicht.
        let target = pending
            .update
            .and_then(|id| self.store.login(id))
            .filter(|row| row.username == pending.username && vault::offered(&row.origin, &pending.site));
        match target {
            Some(row) => {
                // Unverschlüsselt gesichert, jetzt verschlüsselt benutzt: Die Anmeldung
                // zieht mit um und wird auf der unverschlüsselten Seite nicht mehr angeboten.
                let upgrade = pending.site.secure
                    && row.origin != pending.site.origin
                    && !self.store.login_exists(&pending.site.origin, &row.username);
                if upgrade {
                    let secret = self.seal_login(&pending.site.origin, &row.username, &pending.password)?;
                    Ok(self.store.rebind_login(row.id, &pending.site.origin, &pending.site.key, &secret))
                } else {
                    let secret = self.seal_login(&row.origin, &row.username, &pending.password)?;
                    Ok(self.store.update_login(row.id, &row.username, &secret))
                }
            }
            None => {
                let secret = self.seal_login(&pending.site.origin, &pending.username, &pending.password)?;
                Ok(self
                    .store
                    .save_login(&pending.site.origin, &pending.site.key, &pending.username, &secret)
                    .is_some())
            }
        }
    }

    // -----------------------------------------------------------------------
    // Verwaltung
    // -----------------------------------------------------------------------

    /// Daten für die Seitenleiste „Passwörter“. Passwörter selbst sind nie dabei.
    pub fn passwords_panel(&mut self, query: &str) -> Value {
        let items: Vec<Value> = self
            .store
            .logins(query, 2000)
            .iter()
            .map(|row| {
                let site = vault::site_of(&row.origin);
                json!({
                    "id": row.id,
                    "host": site.as_ref().map(|s| s.key.clone()).unwrap_or_else(|| row.origin.clone()),
                    "origin": row.origin,
                    "insecure": site.as_ref().is_some_and(|s| !s.secure),
                    "username": row.username,
                    "changed": row.changed,
                    "used": row.used,
                    "icon": self.store.favicon_for(&row.origin),
                })
            })
            .collect();
        // Der Schalter liegt beim Schlüssel; den braucht es nur, wenn es etwas zu schützen gibt.
        let total = self.store.login_count();
        let auth = total == 0 || self.fill_needs_auth();
        json!({
            "items": items,
            "total": total,
            "never": self.store.never_save_list(),
            "offer": self.settings.pw_offer,
            "auth": auth,
            "touch": native::biometrics_available(),
        })
    }

    fn add_login(&mut self, window: WindowId, url: &str, username: &str, password: &str, query: &str) {
        let url = url.trim();
        let url = if url.contains("://") { url.to_string() } else { format!("https://{url}") };
        let Some(site) = vault::site_of(&url) else {
            self.notice(window, t("pw_add_bad_url").to_string(), None, 4000);
            return;
        };
        if password.is_empty() || password.len() > MAX_PASSWORD || username.len() > MAX_USERNAME {
            self.notice(window, t("pw_add_bad_password").to_string(), None, 4000);
            return;
        }
        // Hinzufügen ersetzt nichts: Ein vorhandenes Passwort ändert nur, wer sich ausweist.
        if self.store.login_exists(&site.origin, username) {
            self.notice(window, t("pw_add_exists").to_string(), None, 5000);
            return;
        }
        match self.seal_login(&site.origin, username, password) {
            Ok(secret) => {
                self.store.save_login(&site.origin, &site.key, username, &secret);
                self.notice(window, t("pw_saved").to_string(), None, 3000);
            }
            Err(error) => self.notice(window, error_text(&error), None, 6000),
        }
        self.send_panel(window, "passwords", query);
        self.mark_all_dirty();
    }

    fn edit_login(&mut self, window: WindowId, login: i64, username: &str, password: Option<String>, query: &str) {
        let Some(row) = self.store.login(login) else {
            return;
        };
        let password = match password {
            Some(password) => password,
            // Nur der Name ändert sich: Das Passwort muss trotzdem neu an ihn gebunden werden.
            None => match self.vault.open(&row.origin, &row.username, &row.secret) {
                Ok(password) => password,
                Err(error) => {
                    self.notice(window, error_text(&error), None, 6000);
                    return;
                }
            },
        };
        match self.seal_login(&row.origin, username, &password) {
            Ok(secret) => {
                if self.store.update_login(login, username, &secret) {
                    self.notice(window, t("pw_saved").to_string(), None, 3000);
                } else {
                    self.notice(window, t("pw_edit_conflict").to_string(), None, 5000);
                }
            }
            Err(error) => self.notice(window, error_text(&error), None, 6000),
        }
        self.send_panel(window, "passwords", query);
        self.mark_all_dirty();
    }

    /// Eine Datei wurde gewählt (Einfuhr) bzw. ein Ziel (Ausfuhr).
    pub fn file_chosen(&mut self, window: WindowId, purpose: &str, path: Option<String>) {
        let Some(path) = path else {
            if purpose == "pw-export" {
                self.export_until = None;
            }
            return;
        };
        match purpose {
            "pw-import" => self.import_logins(window, &path),
            "pw-export" => self.export_logins(window, &path),
            "bm-import" => self.import_bookmarks(window, &path),
            "bm-export" => self.export_bookmarks(window, &path),
            _ => {}
        }
    }

    fn import_logins(&mut self, window: WindowId, path: &str) {
        self.sites_stale = true;
        let too_big = std::fs::metadata(path).map(|m| m.len() > 20_000_000).unwrap_or(true);
        let bytes = if too_big { None } else { std::fs::read(path).ok() };
        let Some(bytes) = bytes else {
            self.notice(window, t("pw_import_unreadable").to_string(), None, 5000);
            return;
        };
        let items = match vault::read_export(&String::from_utf8_lossy(&bytes)) {
            Ok(items) => items,
            Err(_) => {
                self.notice(window, t("pw_import_format").to_string(), None, 7000);
                return;
            }
        };
        let (mut added, mut known, mut skipped) = (0, 0, items.len().saturating_sub(IMPORT_LIMIT));
        let mut failure = None;
        // Alles in einem Zug festschreiben – auch bei sehr vielen Zeilen bleibt der Browser bedienbar.
        self.store.begin();
        for item in items.into_iter().take(IMPORT_LIMIT) {
            let url = if item.url.contains("://") { item.url.clone() } else { format!("https://{}", item.url) };
            let Some(site) = vault::site_of(&url) else {
                skipped += 1;
                continue;
            };
            if item.password.len() > MAX_PASSWORD || item.username.len() > MAX_USERNAME {
                skipped += 1;
                continue;
            }
            // Was es schon gibt, bleibt unangetastet.
            if self.store.login_exists(&site.origin, &item.username) {
                known += 1;
                continue;
            }
            // Solange diese Einfuhr die ersten Einträge überhaupt anlegt, darf dabei der Schlüssel entstehen.
            match self.vault.seal(&site.origin, &item.username, &item.password, added == 0 && self.store.login_count() == 0) {
                Ok(secret) => {
                    if self.store.save_login(&site.origin, &site.key, &item.username, &secret).is_some() {
                        added += 1;
                    } else {
                        skipped += 1;
                    }
                }
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
        self.store.commit();
        if let Some(error) = failure {
            self.notice(window, error_text(&error), None, 6000);
        } else {
            let text = t("pw_imported")
                .replace("{n}", &added.to_string())
                .replace("{k}", &known.to_string())
                .replace("{s}", &skipped.to_string());
            self.notice(window, text, None, 8000);
        }
        self.send_panel(window, "passwords", "");
        self.mark_all_dirty();
    }

    fn export_logins(&mut self, window: WindowId, path: &str) {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        // Die Freigabe gilt für genau eine Ausfuhr und nur kurze Zeit.
        if !still(self.export_until.take()) {
            self.notice(window, t("pw_export_again").to_string(), None, 5000);
            return;
        }
        let mut rows = Vec::new();
        for row in self.store.logins("", 100_000) {
            match self.vault.open(&row.origin, &row.username, &row.secret) {
                Ok(password) => rows.push((row.origin, row.username, password)),
                Err(VaultError::Denied(_)) => {
                    self.notice(window, t("pw_err_keychain").to_string(), None, 6000);
                    return;
                }
                Err(_) => {}
            }
        }
        let text = vault::write_export(&rows);
        // Nur für den eigenen Benutzer lesbar – in der Datei stehen die Passwörter
        // im Klartext. Das gilt auch, wenn eine vorhandene Datei ersetzt wird.
        let written = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .and_then(|mut file| {
                file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
                file.write_all(text.as_bytes())
            });
        match written {
            Ok(()) => {
                let text = t("pw_exported").replace("{n}", &rows.len().to_string());
                let action = NoticeAction::Reveal(std::path::PathBuf::from(path));
                self.notice(window, text, Some((t("d_show"), action)), 12000);
            }
            Err(_) => self.notice(window, t("pw_export_failed").to_string(), None, 6000),
        }
    }

    /// Ein kopiertes Passwort verschwindet wieder aus der Zwischenablage –
    /// außer es liegt dort inzwischen etwas anderes.
    pub fn clipboard_expire(&mut self, count: isize) {
        if self.clipboard_count == Some(count) {
            native::clear_clipboard_if(count);
            self.clipboard_count = None;
        }
    }
}
