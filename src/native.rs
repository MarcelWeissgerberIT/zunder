//! Direkter Draht zu AppKit/WebKit für alles, was wry nicht selbst anbietet:
//! Ladezustand, Fortschritt, Fehlermeldungen beim Laden, JavaScript-Dialoge,
//! gemeinsame Datenhaltung privater Fenster.
//!
//! Alle Funktionen hier laufen ausschließlich auf dem Hauptthread.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::CStr;
use std::time::{Duration, Instant};

use block2::Block;
use objc2::ffi;
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
use objc2::{class, msg_send, sel, MainThreadMarker};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use objc2_web_kit::{WKWebViewConfiguration, WKWebsiteDataStore};
use tao::event_loop::EventLoopProxy;
use wry::{WebView, WebViewExtMacOS};

use crate::app::UserEvent;
use crate::i18n::t;

/// Was beim Laden einer Seite passiert ist – gemeldet von den nachgerüsteten
/// Delegate-Methoden (siehe [`install_hooks`]).
#[derive(Debug)]
pub enum NavKind {
    Started,
    Redirected,
    Failed {
        /// Fehler vor der Übernahme der Seite (die alte Seite steht noch).
        provisional: bool,
        code: i64,
        domain: String,
        description: String,
        url: String,
    },
    /// Die Seite hat `window.close()` aufgerufen.
    Closed,
    /// Link mit gedrückter Befehlstaste oder mittlerer Maustaste angeklickt.
    OpenInTab { url: String, foreground: bool },
    /// Antwort auf die Frage, ob in der Seite gerade etwas abgespielt wird.
    Media { playing: bool },
    /// Der Tab wollte eine unverschlüsselte Seite laden; das wurde angehalten.
    Insecure { url: String },
    /// Ein Formular sollte unverschlüsselt abgeschickt werden; das wurde angehalten.
    InsecureForm { url: String },
    /// Statt der gewünschten Adresse (`original`) soll diese bereinigte geladen werden.
    Rewritten { url: String, original: String },
    /// Die Adresse sieht einer bekannten Seite zum Verwechseln ähnlich; das
    /// Laden wurde angehalten.
    Lookalike { url: String, known: crate::sites::Match },
    /// Eine Website wollte den Tab zu einem Gerät im eigenen Netz schicken.
    /// `in_place`: Es ging um diesen Tab selbst (sonst um ein neues Fenster);
    /// `replay`: ein gewöhnlicher Abruf, der sich nach Zustimmung wiederholen lässt.
    LocalNet { url: String, from: String, in_place: bool, replay: bool },
    /// Eine Seite wollte im Hintergrund ein Gerät im eigenen Netz ansprechen;
    /// WebKit hat das nach Zunders Regeln verhindert.
    LocalBlocked { url: String },
    /// Rücksprung von der Anmeldung bei OpenRouter – mit dem Einmal-Code in der Adresse.
    OAuthCode { url: String },
    /// Die Adresse steht auf der Liste gemeldeter Betrugs- und Schadseiten.
    /// `in_place`: Der Tab selbst wollte dorthin – sonst ein neues Fenster.
    Guard { url: String, in_place: bool },
    /// WebKit wollte den Inhalt eines eingebetteten Rahmens von sich aus als
    /// Datei herunterladen (mit der Härtung und im Stealth-Fenster: PDFs);
    /// Zunder hat den Download abgesagt.
    FrameFile { pdf: bool },
}

#[derive(Debug)]
pub struct NavEvent {
    /// Adresse der WKWebView – dient nur als Schlüssel zum Wiederfinden des Tabs.
    pub webview: usize,
    pub kind: NavKind,
}

thread_local! {
    static PROXY: RefCell<Option<EventLoopProxy<UserEvent>>> = const { RefCell::new(None) };
    /// Dialoge je WebView: von welcher Website, wie viele, stummgeschaltet?
    /// Der Stand überdauert Seitenwechsel, die die Seite selbst auslöst – sonst
    /// setzte ein Neuladen in der Schleife die Stummschaltung zurück.
    static DIALOGS: RefCell<HashMap<usize, DialogState>> = RefCell::new(HashMap::new());
    /// Anmelde-Abfragen je WebView: wie viele seit wann.
    static AUTH_PROMPTS: RefCell<HashMap<usize, (u32, Instant)>> = RefCell::new(HashMap::new());
    /// Adressen, die einmal unverändert geladen werden – samt Tracking-Anhängsel
    /// (wenn der Server sie immer wieder anhängt).
    static AS_IS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// Sicherheits-Schalter, die beim Navigieren und beim Anlegen neuer Tabs gelten.
    static GUARDS: std::cell::Cell<Guards> = const { std::cell::Cell::new(Guards { https_only: false, strip_tracking: false, hardened: false, lookalike: false, local_net: false, devtools: false }) };
    /// Rechner, die in dieser Sitzung ausdrücklich unverschlüsselt geladen werden dürfen.
    static HTTP_ALLOWED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    /// Interne Seiten (`zunder://…`), die der Browser gleich selbst lädt.
    static INTERNAL: RefCell<Vec<(usize, String, Instant)>> = const { RefCell::new(Vec::new()) };
    /// Dateinamen, die eine eingebaute App für die Datei angekündigt hat, die
    /// sie gleich sichert: (WebView, Name, seit wann).
    static FILE_NAMES: RefCell<Vec<(usize, String, Instant)>> = const { RefCell::new(Vec::new()) };
    /// Geräte im eigenen Netz, die der Mensch am Browser gerade selbst aufruft:
    /// (Rechner, WebView oder jede, seit wann).
    static USER_LOCAL: RefCell<Vec<(String, Option<usize>, Instant)>> = const { RefCell::new(Vec::new()) };
    /// Websites, die Geräte im eigenen Netz ansprechen dürfen.
    static LOCAL_SITES: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    /// Was der Browser über bekannte Seiten weiß – für die Warnung vor
    /// täuschend ähnlichen Adressen.
    static SITES: RefCell<crate::sites::SiteGuard> = RefCell::new(crate::sites::SiteGuard::default());
    /// Wann zuletzt ein verhinderter Zugriff ins eigene Netz gemeldet wurde (je WebView).
    static LOCAL_REPORTED: RefCell<HashMap<usize, Instant>> = RefCell::new(HashMap::new());
}

/// So lange gilt die Anmeldung einer internen Seite bzw. eines selbst
/// aufgerufenen Geräts – reichlich für den Weg bis zur Navigationsentscheidung.
const EXPECT_WINDOW: Duration = Duration::from_secs(20);
/// So lange gilt der Name, den eine App für ihre Datei angekündigt hat.
const FILE_NAME_WINDOW: Duration = Duration::from_secs(15);

#[derive(Default)]
struct DialogState {
    host: String,
    count: u32,
    muted: bool,
}

/// Die Sicherheits-Schalter aus den Einstellungen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Guards {
    /// Unverschlüsselte Seiten erst verschlüsselt versuchen, sonst nachfragen.
    pub https_only: bool,
    /// Tracking-Anhängsel aus Adressen entfernen.
    pub strip_tracking: bool,
    /// Angreifbare Web-Techniken abschalten (Lockdown-Modus von WebKit).
    pub hardened: bool,
    /// Vor Adressen warnen, die einer bekannten Seite zum Verwechseln ähneln.
    pub lookalike: bool,
    /// Websites nicht ungefragt auf Geräte im eigenen Netz zugreifen lassen.
    pub local_net: bool,
    /// „Element untersuchen“ und der Zugriff von Safaris Entwicklerwerkzeugen.
    pub devtools: bool,
}

pub fn set_guards(guards: Guards) {
    GUARDS.with(|g| g.set(guards));
}

pub fn guards() -> Guards {
    GUARDS.with(|g| g.get())
}

thread_local! {
    /// Läuft gerade eine Anmeldung bei OpenRouter? Nur dann wird ihr Rücksprung abgefangen.
    static OAUTH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Schaltet das Abfangen des Rücksprungs der Anmeldung ein oder aus.
pub fn expect_oauth(on: bool) {
    OAUTH.with(|flag| flag.set(on));
}

thread_local! {
    /// Tabs von Stealth-Fenstern: Für sie gelten die Schutzschalter immer.
    static STEALTH: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
}

/// Dieser Tab gehört zu einem Stealth-Fenster.
pub fn mark_stealth(key: usize) {
    STEALTH.with(|tabs| tabs.borrow_mut().insert(key));
}

thread_local! {
    /// Stealth-Tabs, deren Tunnel sich nicht einrichten ließ: Sie laden nichts.
    static UNROUTED: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
}

/// Dieser Stealth-Tab hat keinen Tunnel bekommen, obwohl einer da wäre.
pub fn mark_unrouted(key: usize) {
    UNROUTED.with(|set| set.borrow_mut().insert(key));
}

fn unrouted(key: usize) -> bool {
    UNROUTED.with(|set| set.borrow().contains(&key))
}

/// Räumt die Vermerke zu Tabs weg, die es nicht mehr gibt.
pub fn keep_stealth(live: &HashSet<usize>) {
    UNROUTED.with(|set| set.borrow_mut().retain(|key| live.contains(key)));
    STEALTH.with(|tabs| tabs.borrow_mut().retain(|key| live.contains(key)));
    PRIVATE_VIEWS.with(|set| set.borrow_mut().retain(|key| live.contains(key)));
    MEDIA_REFUSED.with(|refused| refused.borrow_mut().retain(|key, _| live.contains(key)));
    PDF_SHELF.with(|shelf| shelf.borrow_mut().keep(live));
    FRAME_CLICKS.with(|clicks| clicks.borrow_mut().keep(live));
}

/// Die Schutzschalter, wie sie für diese WebView gelten: In einem
/// Stealth-Fenster sind sie an, gleich was sonst eingestellt ist.
fn guards_for(webview: Obj) -> Guards {
    let mut guards = guards();
    if STEALTH.with(|tabs| tabs.borrow().contains(&(webview as usize))) {
        guards.https_only = true;
        guards.strip_tracking = true;
        guards.lookalike = true;
        guards.local_net = true;
    }
    guards
}

/// Was Apples Netzwerk-Bibliothek für einen Tunnel anbietet (ab macOS 14).
/// Die Funktionen werden erst zur Laufzeit gesucht – auf älteren Systemen
/// fehlen sie, und dann gibt es eben keinen Tunnel.
struct ProxyApi {
    endpoint: unsafe extern "C" fn(*const std::ffi::c_char, *const std::ffi::c_char) -> *mut AnyObject,
    socks: unsafe extern "C" fn(*mut AnyObject) -> *mut AnyObject,
    login: unsafe extern "C" fn(*mut AnyObject, *const std::ffi::c_char, *const std::ffi::c_char),
    failover: unsafe extern "C" fn(*mut AnyObject, bool),
}

extern "C" {
    fn dlsym(handle: *mut std::ffi::c_void, symbol: *const std::ffi::c_char) -> *mut std::ffi::c_void;
}

fn proxy_api() -> Option<&'static ProxyApi> {
    static API: std::sync::OnceLock<Option<ProxyApi>> = std::sync::OnceLock::new();
    API.get_or_init(|| unsafe {
        // RTLD_DEFAULT: in allem suchen, was das Programm geladen hat.
        let everywhere = -2isize as *mut std::ffi::c_void;
        let find = |name: &std::ffi::CStr| {
            let found = dlsym(everywhere, name.as_ptr());
            (!found.is_null()).then_some(found)
        };
        Some(ProxyApi {
            endpoint: std::mem::transmute::<*mut std::ffi::c_void, _>(find(c"nw_endpoint_create_host")?),
            socks: std::mem::transmute::<*mut std::ffi::c_void, _>(find(c"nw_proxy_config_create_socksv5")?),
            login: std::mem::transmute::<*mut std::ffi::c_void, _>(find(c"nw_proxy_config_set_username_and_password")?),
            failover: std::mem::transmute::<*mut std::ffi::c_void, _>(find(c"nw_proxy_config_set_failover_allowed")?),
        })
    })
    .as_ref()
}

/// Kann dieses System den Verkehr eines Datenspeichers durch einen Tunnel schicken?
pub fn can_route() -> bool {
    let known: bool = unsafe { msg_send![class!(WKWebsiteDataStore), instancesRespondToSelector: sel!(setProxyConfigurations:)] };
    known && proxy_api().is_some()
}

/// Schickt den gesamten Verkehr eines Datenspeichers durch einen
/// SOCKS5-Zugang (Tor). `name` unterscheidet die Tabs: Tor legt für jeden
/// Namen einen eigenen Weg durchs Netz. Am Tunnel vorbei wird nie geladen.
pub fn route_store(store: &WKWebsiteDataStore, host: &str, port: u16, name: &str) -> bool {
    let Some(api) = proxy_api() else {
        return false;
    };
    let (Ok(host), Ok(port), Ok(name)) =
        (std::ffi::CString::new(host), std::ffi::CString::new(port.to_string()), std::ffi::CString::new(name))
    else {
        return false;
    };
    unsafe {
        let known: bool = msg_send![store, respondsToSelector: sel!(setProxyConfigurations:)];
        if !known {
            return false;
        }
        // Beide Funktionen geben ein Objekt zurück, das dem Aufrufer gehört.
        let Some(endpoint) = Retained::from_raw((api.endpoint)(host.as_ptr(), port.as_ptr())) else {
            return false;
        };
        let Some(config) = Retained::from_raw((api.socks)(Retained::as_ptr(&endpoint) as *mut AnyObject)) else {
            return false;
        };
        let config_ptr = Retained::as_ptr(&config) as *mut AnyObject;
        (api.login)(config_ptr, name.as_ptr(), c"zunder".as_ptr());
        (api.failover)(config_ptr, false);
        let list: Retained<AnyObject> = msg_send![class!(NSArray), arrayWithObject: &*config];
        let _: () = msg_send![store, setProxyConfigurations: &*list];
    }
    true
}

/// Erlaubt für diese Sitzung, einen Rechner unverschlüsselt zu laden.
pub fn allow_http(host: &str) {
    let host = host.trim().to_ascii_lowercase();
    let plausible = !host.is_empty()
        && host.len() <= 253
        && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'));
    if plausible {
        HTTP_ALLOWED.with(|allowed| allowed.borrow_mut().insert(host));
    }
}

/// Meldet eine interne Seite an, die der Browser gleich selbst in dieser
/// WebView lädt. Nur angemeldete Adressen kommen durch die
/// Navigationsentscheidung – eine Website kann einen Tab deshalb nie auf eine
/// interne Seite schicken. Die Anmeldung gilt genau der einen WebView: Eine
/// Seite in einem anderen Tab, die es im selben Augenblick mit derselben
/// Adresse versucht, kann sie nicht für sich verbrauchen.
pub fn expect_internal(webview: usize, url: &str) {
    // Auch eine Adresse im Netz zählt: Mit ihr ist eine ältere Anmeldung dieser
    // WebView überholt.
    let wanted = internal_address(url);
    INTERNAL.with(|list| expect_in(&mut list.borrow_mut(), webview, wanted, Instant::now()));
}

/// Wurde genau diese interne Adresse für diese WebView angemeldet? Die
/// Anmeldung gilt einmal.
fn internal_expected(webview: usize, url: &str) -> bool {
    let Some(wanted) = internal_address(url) else {
        return false;
    };
    INTERNAL.with(|list| expected_in(&mut list.borrow_mut(), webview, &wanted, Instant::now()))
}

fn internal_address(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    (parsed.scheme() == "zunder").then(|| parsed.to_string())
}

/// Eine WebView lädt eines nach dem anderen: Was der Browser als Nächstes in
/// ihr lädt (`wanted`: eine interne Seite, sonst nichts), löst ihre ältere
/// Anmeldung ab – übrig bleibt nie eine, die später eine Seite einlösen könnte.
fn expect_in(list: &mut Vec<(usize, String, Instant)>, webview: usize, wanted: Option<String>, now: Instant) {
    list.retain(|(view, _, at)| *view != webview && now.saturating_duration_since(*at) < EXPECT_WINDOW);
    if let Some(wanted) = wanted {
        list.push((webview, wanted, now));
    }
}

fn expected_in(list: &mut Vec<(usize, String, Instant)>, webview: usize, wanted: &str, now: Instant) -> bool {
    list.retain(|(_, _, at)| now.saturating_duration_since(*at) < EXPECT_WINDOW);
    match list.iter().position(|(view, expected, _)| *view == webview && expected == wanted) {
        Some(position) => {
            list.remove(position);
            true
        }
        None => false,
    }
}

/// Eine eingebaute App sichert gleich eine Datei, die sie selbst erzeugt hat,
/// und kündigt an, wie sie heißen soll. In der Sandbox kennt WebKit den Namen
/// aus dem Link nicht (dort stößt der Browser den Download selbst an) – ohne
/// diese Ankündigung hieße die Datei „Unknown“.
pub fn expect_file_name(webview: usize, name: String) {
    FILE_NAMES.with(|list| name_in(&mut list.borrow_mut(), webview, name, Instant::now()));
}

/// Der angekündigte Name für die Datei, die diese WebView gerade sichert –
/// einmal, und nur kurz nach der Ankündigung.
pub fn take_file_name(webview: usize) -> Option<String> {
    FILE_NAMES.with(|list| name_out(&mut list.borrow_mut(), webview, Instant::now()))
}

/// Zwei Dateien kurz nacheinander (ein Doppelklick) bekommen ihre Namen in
/// der Reihenfolge der Ankündigung. Mehr als vier warten je WebView nicht.
fn name_in(list: &mut Vec<(usize, String, Instant)>, webview: usize, name: String, now: Instant) {
    list.retain(|(_, _, at)| now.saturating_duration_since(*at) < FILE_NAME_WINDOW);
    while list.iter().filter(|(view, _, _)| *view == webview).count() >= 4 {
        if let Some(oldest) = list.iter().position(|(view, _, _)| *view == webview) {
            list.remove(oldest);
        }
    }
    list.push((webview, name, now));
}

fn name_out(list: &mut Vec<(usize, String, Instant)>, webview: usize, now: Instant) -> Option<String> {
    list.retain(|(_, _, at)| now.saturating_duration_since(*at) < FILE_NAME_WINDOW);
    let position = list.iter().position(|(view, _, _)| *view == webview)?;
    Some(list.remove(position).1)
}

/// Der Mensch am Browser ruft diese Adresse gerade selbst auf (Eingabe,
/// Lesezeichen, neuer Tab). Liegt sie im eigenen Netz, gilt die Rückfrage für
/// sie nicht. `webview`: nur in dieser WebView – oder in jeder, wenn es sie
/// noch nicht gibt.
pub fn user_navigation(webview: Option<usize>, url: &str) {
    if let Some(webview) = webview {
        // Wer selbst eine Adresse aufruft, beginnt einen neuen Besuch.
        media_visit_over(webview);
    }
    let host = crate::shield::host_of(url);
    if host.is_empty() {
        return;
    }
    // Für jede Adresse gemerkt: Diese Navigation kommt vom Menschen am Browser
    // (siehe `started_by_user`).
    USER_STARTED.with(|list| {
        let mut list = list.borrow_mut();
        list.retain(|(_, _, at)| at.elapsed() < EXPECT_WINDOW);
        if list.len() < 200 {
            list.push((host.clone(), webview, Instant::now()));
        }
    });
    if !crate::shield::is_local_host(&host) {
        return;
    }
    USER_LOCAL.with(|list| {
        let mut list = list.borrow_mut();
        list.retain(|(_, _, at)| at.elapsed() < EXPECT_WINDOW);
        list.push((host, webview, Instant::now()));
    });
}

thread_local! {
    /// Navigationen, die der Browser selbst gerade anstößt (Rechner, WebView, wann).
    static USER_STARTED: RefCell<Vec<(String, Option<usize>, Instant)>> = const { RefCell::new(Vec::new()) };
}

/// Geht diese Navigation auf den Menschen am Browser zurück – Eingabe,
/// Lesezeichen, Klick, Formular – und nicht auf ein Skript der Seite?
unsafe fn started_by_user(action: Obj, webview: usize, url: &str, kind: isize) -> bool {
    let host = crate::shield::host_of(url);
    let by_browser = USER_STARTED.with(|list| {
        list.borrow().iter().any(|(h, w, at)| *h == host && w.is_none_or(|w| w == webview) && at.elapsed() < EXPECT_WINDOW)
    });
    if by_browser {
        return true;
    }
    // WebKit weiß, ob ein Klick oder Tastendruck dahintersteht.
    let knows: bool = msg_send![action, respondsToSelector: sel!(_isUserInitiated)];
    if knows {
        return msg_send![action, _isUserInitiated];
    }
    // Ohne diese Auskunft: Link und Formular gelten als gewollt, alles andere nicht.
    matches!(kind, 0 | 1)
}

fn user_asked_for(webview: usize, host: &str) -> bool {
    USER_LOCAL.with(|list| {
        list.borrow()
            .iter()
            .any(|(h, w, at)| h == host && w.is_none_or(|w| w == webview) && at.elapsed() < EXPECT_WINDOW)
    })
}

/// Die Websites, die Geräte im eigenen Netz ansprechen dürfen.
pub fn set_local_sites(sites: HashSet<String>) {
    LOCAL_SITES.with(|s| *s.borrow_mut() = sites);
}

/// Erlaubt einer Website den Zugriff ins eigene Netz – sofort wirksam, noch
/// bevor die App es dauerhaft vermerkt.
pub fn allow_local_site(site: &str) -> bool {
    let site = site.trim().to_ascii_lowercase();
    let plausible = !site.is_empty()
        && site.len() <= 253
        && site.contains('.')
        && site.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'));
    if plausible {
        LOCAL_SITES.with(|s| s.borrow_mut().insert(site));
    }
    plausible
}

fn local_site_allowed(host: &str) -> bool {
    let site = crate::sites::split_host(host).1;
    LOCAL_SITES.with(|s| s.borrow().contains(&site))
}

/// Zugriff auf das Wissen über bekannte Seiten (täuschend ähnliche Adressen).
pub fn with_sites<R>(f: impl FnOnce(&mut crate::sites::SiteGuard) -> R) -> R {
    SITES.with(|s| f(&mut s.borrow_mut()))
}

pub fn set_proxy(proxy: EventLoopProxy<UserEvent>) {
    PROXY.with(|p| *p.borrow_mut() = Some(proxy));
}

fn emit(event: UserEvent) {
    PROXY.with(|p| {
        if let Some(proxy) = p.borrow().as_ref() {
            let _ = proxy.send_event(event);
        }
    });
}

/// Ein Ereignis an die App – für die Nachbarn dieser Datei (`geo.rs`).
pub fn post(event: UserEvent) {
    emit(event);
}

pub fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("nur auf dem Hauptthread")
}

fn ns(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

/// Mehr Text nimmt Zunder von WebKit nie auf einmal entgegen (UTF-16-Einheiten).
const TEXT_LIMIT: usize = 1 << 20;

/// Text eines NSString. Gelesen werden die einzelnen UTF-16-Einheiten: Was
/// eine Seite liefert, muss kein gültiger Text sein – ein halbes Zeichenpaar
/// genügt, und dann liefert `UTF8String` nichts, worauf Verlass wäre. Halbe
/// Paare werden zum Ersatzzeichen, überlange Texte abgeschnitten.
unsafe fn string_of(obj: *mut AnyObject) -> String {
    if obj.is_null() {
        return String::new();
    }
    let length: usize = msg_send![obj, length];
    let length = length.min(TEXT_LIMIT);
    if length == 0 {
        return String::new();
    }
    let mut units = vec![0u16; length];
    let range = objc2_foundation::NSRange { location: 0, length };
    let _: () = msg_send![obj, getCharacters: units.as_mut_ptr(), range: range];
    String::from_utf16_lossy(&units)
}

/// Länge eines NSString in UTF-16-Einheiten – um vor dem Umwandeln zu prüfen.
unsafe fn length_of(obj: *mut AnyObject) -> usize {
    if obj.is_null() {
        0
    } else {
        msg_send![obj, length]
    }
}

/// Lässt sich diese Adresse überhaupt an WebKit übergeben? Ältere Systeme
/// lehnen Zeichen ab, die anderswo durchgehen – und wry bricht dann ab.
pub fn url_ok(url: &str) -> bool {
    unsafe {
        let parsed: Option<Retained<AnyObject>> = msg_send![class!(NSURL), URLWithString: &*ns(url)];
        parsed.is_some()
    }
}

/// Schlüssel einer WebView für die Zuordnung von Ereignissen zu Tabs.
pub fn key(webview: &WebView) -> usize {
    Retained::as_ptr(&webview.webview()) as usize
}

// ---------------------------------------------------------------------------
// Zustand einer WebView
// ---------------------------------------------------------------------------

pub fn is_loading(webview: &WebView) -> bool {
    let w = webview.webview();
    unsafe { msg_send![&*w, isLoading] }
}

pub fn progress(webview: &WebView) -> f64 {
    let w = webview.webview();
    unsafe { msg_send![&*w, estimatedProgress] }
}

pub fn stop(webview: &WebView) {
    let w = webview.webview();
    unsafe {
        let _: () = msg_send![&*w, stopLoading];
    }
}

pub fn reload_from_origin(webview: &WebView) {
    let w = webview.webview();
    unsafe {
        let _: Option<Retained<AnyObject>> = msg_send![&*w, reloadFromOrigin];
    }
}

pub fn title(webview: &WebView) -> String {
    let w = webview.webview();
    unsafe {
        let t: Option<Retained<NSString>> = msg_send![&*w, title];
        t.map(|s| s.to_string()).unwrap_or_default()
    }
}

/// Kennung, mit der sich ein Tab als iPhone ausgibt.
const MOBILE_AGENT: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_6 like Mac OS X) AppleWebKit/605.1.15 \
                            (KHTML, like Gecko) Version/26.0 Mobile/15E148 Safari/604.1";

/// Lässt einen Tab als iPhone auftreten (oder wieder als Mac): Websites
/// liefern dann ihre mobile Fassung. Wirkt ab dem nächsten Laden.
pub fn set_mobile(webview: &WebView, mobile: bool) {
    let w = webview.webview();
    unsafe {
        if mobile {
            let _: () = msg_send![&*w, setCustomUserAgent: &*ns(MOBILE_AGENT)];
        } else {
            let _: () = msg_send![&*w, setCustomUserAgent: Option::<&NSString>::None];
        }
    }
}

/// Bringt eine WebView innerhalb ihres Fensters ganz nach vorn.
pub fn bring_to_front(webview: &WebView) {
    let w = webview.webview();
    unsafe {
        let superview: Option<Retained<AnyObject>> = msg_send![&*w, superview];
        if let Some(superview) = superview {
            // NSWindowAbove = 1
            let _: () = msg_send![&*superview, addSubview: &*w, positioned: 1isize, relativeTo: Option::<&AnyObject>::None];
        }
    }
}

// ---------------------------------------------------------------------------
// Konfiguration neuer WebViews
// ---------------------------------------------------------------------------

/// Eigener, nicht dauerhafter Datenspeicher – einer je privatem Fenster,
/// damit dessen Tabs untereinander angemeldet bleiben, aber nichts auf der
/// Platte landet.
pub fn private_store() -> Retained<WKWebsiteDataStore> {
    unsafe { WKWebsiteDataStore::nonPersistentDataStore(mtm()) }
}

/// Konfiguration für einen Inhalts-Tab.
pub fn content_config(
    store: Option<&WKWebsiteDataStore>,
    ua_suffix: &str,
    stealth: bool,
) -> Retained<WKWebViewConfiguration> {
    unsafe {
        let config = WKWebViewConfiguration::new(mtm());
        if let Some(store) = store {
            config.setWebsiteDataStore(store);
        }
        // Wie Safari auftreten, damit Seiten ihre aktuelle Fassung ausliefern.
        let _: () = msg_send![&*config, setApplicationNameForUserAgent: &*ns(ua_suffix)];
        // Ton nur nach einer Nutzeraktion (WKAudiovisualMediaTypeAudio = 1).
        let _: () = msg_send![&*config, setMediaTypesRequiringUserActionForPlayback: 1usize];
        let prefs = config.preferences();
        // Prüfhilfe: gestellte Kamera und gestelltes Mikrofon. So lässt sich die
        // Abfrage prüfen, ohne dass macOS nach den echten Geräten fragt – und
        // ohne dass im Probelauf je etwas aufgenommen wird.
        if crate::dev::var_os("ZUNDER_MOCK_MEDIA").is_some() {
            let known: bool = msg_send![&*prefs, respondsToSelector: sel!(_setMockCaptureDevicesEnabled:)];
            if known {
                let _: () = msg_send![&*prefs, _setMockCaptureDevicesEnabled: true];
            }
        }
        // Pop-ups nur nach einem Klick, nicht ungefragt.
        let _: () = msg_send![&*prefs, setJavaScriptCanOpenWindowsAutomatically: false];
        let guards = guards();
        // WebKits Warnung vor Betrugsseiten bleibt an (das ist auch die Vorgabe).
        // Einen eigenen Schalter hat sie nicht: Mit der offiziellen Testseite
        // ließ sie sich in Zunder nicht auslösen – zugesagt wird sie deshalb nicht.
        let _: () = msg_send![&*prefs, setFraudulentWebsiteWarningEnabled: true];
        // Härtung: WebKits Lockdown-Modus für diesen Tab – ohne JIT,
        // WebAssembly, WebGL und Webfonts. Gilt für alles, was der Tab lädt.
        let page: Option<Retained<AnyObject>> = msg_send![&*config, defaultWebpagePreferences];
        if let (true, Some(page)) = (guards.hardened || stealth, page.as_ref()) {
            let known: bool = msg_send![&**page, respondsToSelector: sel!(setLockdownModeEnabled:)];
            if known {
                let _: () = msg_send![&**page, setLockdownModeEnabled: true];
            }
        }
        if stealth {
            // Kein WebRTC: Es spräche am Tunnel vorbei und verriete die echte Adresse.
            // (Der Lockdown-Modus nimmt es ebenfalls weg – doppelt hält besser.)
            let known: bool = msg_send![&*prefs, respondsToSelector: sel!(_setPeerConnectionEnabled:)];
            if known {
                let _: () = msg_send![&*prefs, _setPeerConnectionEnabled: false];
            }
            // WebKits eigener Schutz gegen Fingerabdrücke, wie ihn Safari in
            // privaten Fenstern nutzt: Was Seiten von einer Zeichenfläche
            // zurücklesen, ist leicht verrauscht (in jeder Sitzung anders), und
            // als Bildschirmgröße sehen sie die Größe des Fensters. Geprüft auf
            // macOS 26; ältere Systeme, die den Schalter nicht kennen, lassen ihn aus.
            if let Some(page) = page.as_ref() {
                let known: bool = msg_send![&**page, respondsToSelector: sel!(_setNetworkConnectionIntegrityPolicy:)];
                if known {
                    const ENABLED: usize = 1 << 0;
                    const FINGERPRINTING: usize = 1 << 6;
                    let _: () = msg_send![&**page, _setNetworkConnectionIntegrityPolicy: ENABLED | FINGERPRINTING];
                }
            }
        }
        // Zunders eigenes Skript (Passwort-Tresor) samt seinem Nachrichtenkanal.
        attach_bridge(&config);
        // Und die Hilfe für PDF-Rahmen, die WebKit leer lässt. Im gehärteten
        // Modus und im Stealth-Fenster bleibt sie weg: Dort zeigt WebKit
        // eingebettete PDFs ohnehin nicht an – und je weniger Zunder in eine
        // Seite hineinreicht, desto weniger gibt es an ihm zu erkennen.
        if !(guards.hardened || stealth) {
            attach_pdfs(&config);
        }
        if stealth {
            attach_watch(&config);
        }
        config
    }
}


/// Versionsnummer des installierten Safari, z. B. "26.1".
pub fn safari_version() -> String {
    unsafe {
        let bundle: Option<Retained<AnyObject>> =
            msg_send![class!(NSBundle), bundleWithPath: &*ns("/Applications/Safari.app")];
        if let Some(bundle) = bundle {
            let v: Option<Retained<NSString>> =
                msg_send![&*bundle, objectForInfoDictionaryKey: &*ns("CFBundleShortVersionString")];
            if let Some(v) = v {
                let v = v.to_string();
                if !v.is_empty() && v.chars().all(|c| c.is_ascii_digit() || c == '.') {
                    return v;
                }
            }
        }
    }
    "26.0".to_string()
}

/// Bevorzugte Sprache des Systems, z. B. "de-DE".
pub fn preferred_language() -> String {
    unsafe {
        let langs: Retained<AnyObject> = msg_send![class!(NSLocale), preferredLanguages];
        let first: Option<Retained<NSString>> = msg_send![&*langs, firstObject];
        first.map(|s| s.to_string()).unwrap_or_else(|| "en".into())
    }
}

// ---------------------------------------------------------------------------
// Nachgerüstete Delegate-Methoden
// ---------------------------------------------------------------------------
//
// wry meldet nur „Seite übernommen“ und „Seite fertig“. Für einen Browser
// fehlen: Ladebeginn, Weiterleitung, Ladefehler und die JavaScript-Dialoge.
// Die Delegate-Klassen von wry bekommen diese Methoden hier zur Laufzeit
// dazu. Das muss passieren, BEVOR die betreffende WebView erzeugt wird –
// WebKit merkt sich beim Setzen des Delegates, welche Methoden es gibt.

type Obj = *mut AnyObject;

extern "C-unwind" fn nav_started(_this: Obj, _cmd: Sel, webview: Obj, _nav: Obj) {
    emit(UserEvent::Nav(NavEvent {
        webview: webview as usize,
        kind: NavKind::Started,
    }));
}

extern "C-unwind" fn nav_redirected(_this: Obj, _cmd: Sel, webview: Obj, _nav: Obj) {
    emit(UserEvent::Nav(NavEvent {
        webview: webview as usize,
        kind: NavKind::Redirected,
    }));
}

extern "C-unwind" fn nav_failed_provisional(_this: Obj, _cmd: Sel, webview: Obj, _nav: Obj, error: Obj) {
    // Der Tab kam nicht an: Die Listen gelten wieder für die Seite, die er zeigt.
    if !webview.is_null() {
        unsafe { crate::blocker::revert(webview) };
    }
    report_failure(webview, error, true);
}

extern "C-unwind" fn nav_failed_committed(_this: Obj, _cmd: Sel, webview: Obj, _nav: Obj, error: Obj) {
    report_failure(webview, error, false);
}

fn report_failure(webview: Obj, error: Obj, provisional: bool) {
    if error.is_null() {
        return;
    }
    unsafe {
        let code: isize = msg_send![error, code];
        let domain: Obj = msg_send![error, domain];
        let description: Obj = msg_send![error, localizedDescription];
        let info: Obj = msg_send![error, userInfo];
        let mut url = String::new();
        if !info.is_null() {
            let failing: Obj = msg_send![info, objectForKey: &*ns("NSErrorFailingURLStringKey")];
            url = string_of(failing);
        }
        emit(UserEvent::Nav(NavEvent {
            webview: webview as usize,
            kind: NavKind::Failed {
                provisional,
                code: code as i64,
                domain: string_of(domain),
                description: string_of(description),
                url,
            },
        }));
    }
}

/// Die ursprüngliche Entscheidungsmethode von wry – sie wird weiter aufgerufen.
static ORIGINAL_POLICY: std::sync::OnceLock<Imp> = std::sync::OnceLock::new();

type PolicyFn = extern "C-unwind" fn(Obj, Sel, Obj, Obj, &Block<dyn Fn(isize)>);

/// Woher eine Navigation kommt – für die Frage, ob sie ins eigene Netz darf.
enum Source {
    /// Eine Website aus dem Internet (ihr Rechnername).
    Public(String),
    /// Eine Seite aus dem eigenen Netz, eine Datei oder eine interne Seite.
    Trusted,
    /// Nicht feststellbar: ein frischer Tab – oder ein abgeschottetes Dokument.
    Unknown,
}

unsafe fn source_of(action: Obj) -> Source {
    let frame: Obj = msg_send![action, sourceFrame];
    if frame.is_null() {
        return Source::Unknown;
    }
    let origin: Obj = msg_send![frame, securityOrigin];
    if origin.is_null() {
        return Source::Unknown;
    }
    let protocol: Obj = msg_send![origin, protocol];
    let host: Obj = msg_send![origin, host];
    let host = string_of(host).to_ascii_lowercase();
    match string_of(protocol).to_ascii_lowercase().as_str() {
        "http" | "https" if crate::shield::is_local_host(&host) => Source::Trusted,
        "http" | "https" => Source::Public(host),
        "file" | "zunder" => Source::Trusted,
        _ => Source::Unknown,
    }
}

/// Darf diese Navigation zu einem Gerät im eigenen Netz? Liefert sonst, wer
/// dorthin schicken wollte (leer, wenn es sich nicht feststellen lässt).
unsafe fn local_net_refused(action: Obj, webview: Obj, url: &str) -> Option<String> {
    let host = crate::shield::host_of(url);
    if host.is_empty() || !crate::shield::is_local_host(&host) {
        return None;
    }
    if user_asked_for(webview as usize, &host) {
        return None;
    }
    match source_of(action) {
        Source::Trusted => None,
        Source::Public(from) if local_site_allowed(&from) => None,
        Source::Public(from) => Some(from),
        Source::Unknown => Some(String::new()),
    }
}

/// Kommt diese Navigation von Zunders eigener Startseite – der Seite im Tab,
/// nicht aus einem Rahmen?
unsafe fn from_start_page(action: Obj) -> bool {
    let frame: Obj = msg_send![action, sourceFrame];
    if frame.is_null() {
        return false;
    }
    let main: bool = msg_send![frame, isMainFrame];
    let origin: Obj = msg_send![frame, securityOrigin];
    if !main || origin.is_null() {
        return false;
    }
    let protocol: Obj = msg_send![origin, protocol];
    let host: Obj = msg_send![origin, host];
    string_of(protocol).eq_ignore_ascii_case("zunder") && string_of(host).eq_ignore_ascii_case("newtab")
}

/// Antwortet hier ein Server mit einer Weiterleitung auf einen früheren Abruf?
unsafe fn is_redirect(action: Obj) -> bool {
    let knows: bool = msg_send![action, respondsToSelector: sel!(_isRedirect)];
    knows && msg_send![action, _isRedirect]
}

/// Die Adresse, mit der ein Abruf begann, bevor ein Server ihn weitergeleitet
/// hat – leer, wenn WebKit sie nicht nennt.
unsafe fn original_url(action: Obj) -> String {
    let knows: bool = msg_send![action, respondsToSelector: sel!(_originalURL)];
    if !knows {
        return String::new();
    }
    let address: Obj = msg_send![action, _originalURL];
    let text: Obj = if address.is_null() { std::ptr::null_mut() } else { msg_send![address, absoluteString] };
    string_of(text)
}

/// Bleibt diese Navigation im Dokument, das der Rahmen schon zeigt – wechselt
/// also nur der Anker? Dann folgt keine Antwort eines Servers.
unsafe fn same_document(frame: Obj, url: &str) -> bool {
    if frame.is_null() || !url.contains('#') {
        return false;
    }
    let request: Obj = msg_send![frame, request];
    let address: Obj = if request.is_null() { std::ptr::null_mut() } else { msg_send![request, URL] };
    let text: Obj = if address.is_null() { std::ptr::null_mut() } else { msg_send![address, absoluteString] };
    !text.is_null() && without_fragment(&string_of(text)) == without_fragment(url)
}

/// Führt diese Adresse wirklich zu einem Eintrag im Verlauf des Tabs? Beim
/// Neuladen muss es der aktuelle sein, beim Blättern irgendeiner.
unsafe fn history_target(webview: Obj, url: &str, kind: isize) -> bool {
    if webview.is_null() {
        return false;
    }
    let wanted = without_fragment(url);
    let same = |item: Obj| -> bool {
        if item.is_null() {
            return false;
        }
        let address: Obj = msg_send![item, URL];
        let text: Obj = if address.is_null() { std::ptr::null_mut() } else { msg_send![address, absoluteString] };
        !text.is_null() && length_of(text) <= 16_384 && without_fragment(&string_of(text)) == wanted
    };
    let list: Obj = msg_send![webview, backForwardList];
    if list.is_null() {
        return false;
    }
    let current: Obj = msg_send![list, currentItem];
    if same(current) {
        return true;
    }
    // Nur „Vor/Zurück“ darf zu einem anderen Eintrag führen.
    if kind != 2 {
        return false;
    }
    let back: Obj = msg_send![list, backList];
    let forward: Obj = msg_send![list, forwardList];
    for items in [back, forward] {
        if items.is_null() {
            continue;
        }
        let count: usize = msg_send![items, count];
        for index in 0..count.min(400) {
            let item: Obj = msg_send![items, objectAtIndex: index];
            if same(item) {
                return true;
            }
        }
    }
    false
}

/// Siegel für Zunders eigene Warn- und Fehlerseiten: eine Zufallszahl je Start,
/// die jede vom Browser erzeugte Adresse dieser Seiten trägt. Was auf ihnen
/// steht (welche Seite, welche Warnung, wohin der Knopf führt), kommt aus der
/// Adresse – also lädt Zunder eine solche Adresse nur, wenn es sie selbst
/// gebaut hat. Eine Website kennt das Siegel nicht.
pub fn seal() -> &'static str {
    static SEAL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SEAL.get_or_init(|| {
        let mut bytes = [0u8; 16];
        // Ohne Zufall gibt es kein Siegel, das sich erraten ließe – dann lädt gar keine solche Seite.
        if ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes).is_err() {
            return String::new();
        }
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    })
}

/// Trägt eine interne Warn- oder Fehlerseite das Siegel dieses Starts? Andere
/// interne Seiten brauchen keines – sie zeigen nichts, was in ihrer Adresse steht.
fn sealed(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    if parsed.host_str() != Some("error") {
        return true;
    }
    let seal = seal();
    !seal.is_empty() && parsed.query_pairs().any(|(key, value)| key == "z" && value == seal)
}

/// Sitzt vor der Navigationsentscheidung von wry. Hier entscheidet sich, was
/// ein Tab überhaupt laden darf:
///
///   - Interne Seiten (`zunder://…`) nur, wenn der Browser selbst sie lädt.
///   - Geräte im eigenen Netz nicht auf Geheiß einer Website aus dem Internet.
///   - Adressen, die einer bekannten Seite zum Verwechseln ähneln, erst nach Rückfrage.
///   - Unverschlüsseltes erst verschlüsselt versuchen; Tracking-Anhängsel entfernen.
///
/// Außerdem öffnet ein Klick mit gedrückter Befehlstaste (oder der mittleren
/// Maustaste) einen neuen Tab. Alles andere geht unverändert an wry.
extern "C-unwind" fn nav_policy(
    this: Obj,
    cmd: Sel,
    webview: Obj,
    action: Obj,
    decide: &Block<dyn Fn(isize)>,
) {
    const CANCEL: isize = 0; // WKNavigationActionPolicyCancel
    unsafe {
        // Ein echter Klick, der einen eingebetteten Rahmen woandershin führt
        // (Adresse; bei einer Weiterleitung dazu der Anfang der Kette). Vermerkt
        // wird er erst ganz unten – wenn feststeht, dass die Navigation
        // stattfindet und eine Antwort des Servers folgt.
        let mut frame_click: Option<(String, Option<String>)> = None;
        if !action.is_null() {
            // WKNavigationType: 0 Link, 1 Formular, 2 Vor/Zurück, 3 Neu laden,
            // 4 Formular erneut, -1 alles andere.
            let kind: isize = msg_send![action, navigationType];
            let frame: Obj = msg_send![action, targetFrame];
            let new_window = frame.is_null();
            let main: bool = !new_window && msg_send![frame, isMainFrame];
            let request: Obj = msg_send![action, request];
            let (url, method) = if request.is_null() {
                (String::new(), String::new())
            } else {
                let url: Obj = msg_send![request, URL];
                let text: Obj = if url.is_null() { std::ptr::null_mut() } else { msg_send![url, absoluteString] };
                let method: Obj = msg_send![request, HTTPMethod];
                (string_of(text), string_of(method))
            };
            let get = method.is_empty() || method.eq_ignore_ascii_case("GET");
            // „Im Verlauf geblättert“ oder „neu geladen“ gilt nur für den Abruf,
            // der wirklich zu einem Eintrag im Verlauf führt. WebKit nennt diese
            // Art nämlich auch bei jeder Weiterleitung, mit der ein Server auf
            // einen solchen Abruf antwortet – eine Seite, die sich selbst neu
            // lädt, käme sonst über ihren Server an den Prüfungen unten vorbei.
            let history = !new_window && matches!(kind, 2..=4) && !is_redirect(action) && history_target(webview, &url, kind);
            let lower = url.get(..8).unwrap_or(&url).to_ascii_lowercase();
            let web = lower.starts_with("http://") || lower.starts_with("https://");
            let guards = guards_for(webview);
            let emit_nav = |kind: NavKind| emit(UserEvent::Nav(NavEvent { webview: webview as usize, kind }));

            // Der Mensch klickt mit der Maus einen Link an oder schickt so ein
            // Formular ab, und das führt einen eingebetteten Rahmen woandershin:
            // Macht WebKit daraus einen Download, ist es seiner (siehe
            // `nav_response_download`). Was ein Skript der Seite in einen Rahmen
            // lädt, zählt nicht – so betten Seiten PDFs ein, auch auf einen Klick
            // hin (WebKit meldet selbst dann noch „vom Menschen“, wenn die Seite
            // nach dem Klick erst Daten abruft und den Rahmen später einhängt).
            // Eine Maustaste nennt WebKit dagegen nur für den echten Mausklick:
            // Klickt ein Skript den Link an oder schickt es das Formular ab,
            // bleibt sie 0 – auch mitten in einem echten Klick anderswo (gemessen
            // auf macOS 26). Weiterleitungen tragen Art, Klick und Taste weiter.
            if !main && !new_window {
                let button: isize = msg_send![action, buttonNumber];
                // Ein Sprung innerhalb desselben Dokuments (nur der Anker
                // wechselt) bringt keine Antwort – er braucht keinen Vermerk.
                if crate::pdfs::frame_click(kind, clicked_by_user(action), button) && !same_document(frame, &url) {
                    // Leitet ein Server weiter, wandert der Klick mit – wiedergefunden
                    // an der Adresse, mit der dieser Abruf begann.
                    let first = if is_redirect(action) { Some(original_url(action)) } else { None };
                    frame_click = Some((url.clone(), first));
                }
            }

            // Interne Seiten: nie in einem Unterrahmen, nie in einem neuen
            // Fenster – und im Tab nur, wenn der Browser selbst sie lädt oder
            // man im Verlauf zu einer zurückkehrt.
            if lower.starts_with("zunder:") {
                // Stealth-Tab ohne Tunnel: Es geht nichts hinaus – also lädt
                // dort auch keine eingebaute App, die von sich aus Daten aus
                // dem Netz holt (das sind keine Navigationen; hier käme es
                // nicht mehr vorbei).
                if crate::apps::by_url(&url).is_some() && unrouted(webview as usize) {
                    decide.call((CANCEL,));
                    return;
                }
                // Eine eingebaute App öffnet außer dem Browser selbst nur die
                // Startseite – mit einem Klick auf ihre Kachel, und nur unter
                // ihrer Adresse ohne jedes Anhängsel. Eine Webseite öffnet sie
                // nie; auch die Weiterleitung eines Servers führt nicht hin.
                // (Im Verlauf zu ihr zurückblättern kann ein Tab wie zu jeder
                // früheren Seite – auch auf Geheiß der Seite, die er gerade
                // zeigt. Die App spricht dann wie immer nur mit sich selbst.)
                let from_start =
                    main && kind == 0 && crate::apps::is_root(&url) && !is_redirect(action) && from_start_page(action);
                if !(main && sealed(&url) && (history || internal_expected(webview as usize, &url) || from_start)) {
                    decide.call((CANCEL,));
                    return;
                }
                // Eine eigene Seite des Browsers: Die Listen der Websites gelten dort nicht.
                crate::blocker::site_changed(webview, &url, false, true);
            } else if web && unrouted(webview as usize) {
                // Stealth-Tab ohne Tunnel: Es geht nichts hinaus.
                decide.call((CANCEL,));
                return;
            } else if web && (main || new_window) {
                let flags: usize = msg_send![action, modifierFlags];
                let button: isize = msg_send![action, buttonNumber];
                let command = flags & (1 << 20) != 0;
                let shift = flags & (1 << 17) != 0;
                let middle = button == 4 || button == 2;
                let in_new_tab = kind == 0 && (command || middle);

                // Anmeldung bei OpenRouter: Der Rücksprung trägt den Einmal-Code.
                // Er wird hier abgefangen und erreicht nie eine Seite – an der
                // Adresse lauscht ohnehin nichts.
                let callback = crate::ai::OAUTH_CALLBACK;
                let returns = url.strip_prefix(callback).is_some_and(|rest| rest.is_empty() || rest.starts_with('?'));
                if main && returns && OAUTH.with(std::cell::Cell::get) {
                    emit_nav(NavKind::OAuthCode { url });
                    decide.call((CANCEL,));
                    return;
                }
                // Gemeldete Betrugs- und Schadseiten: Davor steht eine Warnung –
                // gleich ob angeklickt, eingetippt, weitergeleitet oder im Verlauf.
                // (Was solche Seiten anderswo nachladen wollen, sperrt die
                // Regelliste; die Seite selbst prüft der Browser hier, weil
                // WebKit für gesperrte Seiten nicht meldet, welche Liste es war.)
                if crate::blocker::guard_hit(webview as usize, &url) {
                    emit_nav(NavKind::Guard { url, in_place: main });
                    decide.call((CANCEL,));
                    return;
                }
                // Eigenes Netz: Eine Website aus dem Internet schickt niemanden
                // ungefragt zum Router, zum Drucker oder zu einem Dienst auf
                // diesem Mac.
                if guards.local_net && !history {
                    if let Some(from) = local_net_refused(action, webview, &url) {
                        emit_nav(NavKind::LocalNet { url, from, in_place: main && !in_new_tab, replay: get });
                        decide.call((CANCEL,));
                        return;
                    }
                }
                if main {
                    // Ein Klick des Menschen: Die Geduld des Tabs mit Fragen
                    // nach Kamera und Mikrofon beginnt neu – auch wenn Zunder
                    // die Navigation unten anhält und selbst neu aufsetzt.
                    if clicked_by_user(action) {
                        media_user_moved(webview as usize);
                    }
                    if in_new_tab {
                        // Der neue Tab durchläuft diese Prüfungen dann selbst.
                        emit_nav(NavKind::OpenInTab { url, foreground: shift });
                        decide.call((CANCEL,));
                        return;
                    }
                    // Täuschend ähnliche Adressen – egal, ob angeklickt oder eingetippt.
                    if guards.lookalike {
                        if let Some(known) = SITES.with(|s| s.borrow().check(&url)) {
                            emit_nav(NavKind::Lookalike { url, known });
                            decide.call((CANCEL,));
                            return;
                        }
                    }
                    // Nur verschlüsselt: gilt für jede Art von Abruf. Ein Formular
                    // lässt sich nicht verschlüsselt wiederholen – es wird gar
                    // nicht erst unverschlüsselt abgeschickt.
                    // Ohne Tracking-Anhängsel: nur bei neuen Abrufen, nicht beim
                    // Blättern im Verlauf (das finge den Zurück-Knopf ein) und
                    // nicht, wenn der Server sie hartnäckig wieder anhängt.
                    let as_is = AS_IS.with(|list| {
                        let mut list = list.borrow_mut();
                        match list.iter().position(|kept| *kept == url) {
                            Some(position) => {
                                list.remove(position);
                                true
                            }
                            None => false,
                        }
                    });
                    let strip = guards.strip_tracking && get && !history && !as_is;
                    if guards.https_only || strip {
                        let verdict = HTTP_ALLOWED.with(|allowed| {
                            crate::shield::check(&url, guards.https_only, strip, &allowed.borrow())
                        });
                        if let Some(verdict) = verdict {
                            emit_nav(match verdict {
                                crate::shield::Verdict::Insecure(clean) if get => NavKind::Insecure { url: clean },
                                crate::shield::Verdict::Insecure(_) => NavKind::InsecureForm { url },
                                crate::shield::Verdict::Rewrite(clean) => NavKind::Rewritten { url: clean, original: url },
                            });
                            decide.call((CANCEL,));
                            return;
                        }
                    }
                    // Der Werbeblocker folgt der Website: Wo er abgeschaltet
                    // ist, hängen die Listen am Tab nicht.
                    let by_user = history || started_by_user(action, webview as usize, &url, kind);
                    crate::blocker::site_changed(webview, &url, kind == 3, by_user);
                }
            } else if has_scheme(&url, crate::pdfs::SCHEME) {
                // Zunders Adresse für eingebettete PDFs (`pdfs.rs`): nur in einem
                // Rahmen – nie als Seite im Tab, nie als neues Fenster.
                if main || new_window {
                    decide.call((CANCEL,));
                    return;
                }
            } else if !main && !new_window && !loads_in_frame(&lower) {
                // Ein eingebetteter Rahmen (Werbung, Widget) startet kein anderes
                // Programm – das darf nur die Seite selbst anstoßen.
                decide.call((CANCEL,));
                return;
            }
        }
        // In der Sandbox von macOS startet WebKit einen Download, den ein Link
        // mit „download“ verlangt, nicht von sich aus. Dann stößt ihn der
        // Browser selbst an – mit derselben Anfrage – und reicht ihn an die
        // übliche Download-Behandlung weiter.
        if !action.is_null() && in_sandbox() {
            let knows: bool = msg_send![action, respondsToSelector: sel!(shouldPerformDownload)];
            let wanted: bool = knows && msg_send![action, shouldPerformDownload];
            let request: Obj = msg_send![action, request];
            let can_start: bool = msg_send![webview, respondsToSelector: sel!(startDownloadUsingRequest:completionHandler:)];
            let delegate: Obj = msg_send![webview, navigationDelegate];
            if wanted && can_start && !request.is_null() && !delegate.is_null() {
                // Aus dem Abruf wird ein Download, keine Seite.
                crate::blocker::revert(webview);
                let view: Retained<AnyObject> = Retained::retain(webview).expect("WebView lebt");
                let kept: Retained<AnyObject> = Retained::retain(action).expect("Aktion lebt");
                let delegate: Retained<AnyObject> = Retained::retain(delegate).expect("Delegate lebt");
                let started = block2::RcBlock::new(move |download: *mut AnyObject| {
                    if download.is_null() {
                        return;
                    }
                    let _: () = msg_send![&*delegate, webView: &*view, navigationAction: &*kept, didBecomeDownload: download];
                });
                let _: () = msg_send![webview, startDownloadUsingRequest: request, completionHandler: &*started];
                decide.call((CANCEL,));
                return;
            }
        }
        if let Some((url, first)) = frame_click {
            // Bis hierher hat nichts die Navigation angehalten. Ein Link mit
            // „download“ wird ohne Antwort zum Download – auch er braucht
            // keinen Vermerk.
            let knows: bool = msg_send![action, respondsToSelector: sel!(shouldPerformDownload)];
            let wanted: bool = knows && msg_send![action, shouldPerformDownload];
            if !wanted {
                FRAME_CLICKS.with(|clicks| clicks.borrow_mut().note(webview as usize, &url, first.as_deref(), Instant::now()));
            }
        }
        match ORIGINAL_POLICY.get() {
            Some(original) => {
                let original: PolicyFn = std::mem::transmute(*original);
                original(this, cmd, webview, action, decide);
            }
            None => decide.call((1,)), // WKNavigationActionPolicyAllow
        }
    }
}

// ---------------------------------------------------------------------------
// Seitencheck: Antwortkopf, Verbindung, Merkmale der Cookies
// ---------------------------------------------------------------------------

thread_local! {
    /// Was die Server zu den zuletzt geladenen Seiten mitgeschickt haben, je
    /// WebView. Mehrere, weil beim Zurückgehen im Verlauf keine neue Antwort
    /// kommt – die Seite steht dann wieder da, ihr Antwortkopf soll es auch.
    static RESPONSES: RefCell<HashMap<usize, std::collections::VecDeque<crate::sitecheck::Response>>> =
        RefCell::new(HashMap::new());
}

static ORIGINAL_RESPONSE: std::sync::OnceLock<Imp> = std::sync::OnceLock::new();

fn without_fragment(url: &str) -> &str {
    url.split('#').next().unwrap_or(url)
}

/// Vorposten vor wrys Entscheidung über eine Antwort des Servers: Für den
/// Hauptrahmen merkt er sich die Zeilen des Antwortkopfs, die der Seitencheck
/// auswertet. An der Entscheidung selbst ändert er nichts.
extern "C-unwind" fn nav_response(this: Obj, cmd: Sel, webview: Obj, response: Obj, decide: &Block<dyn Fn(isize)>) {
    unsafe {
        if !response.is_null() && !webview.is_null() {
            let main: bool = msg_send![response, isForMainFrame];
            let inner: Obj = msg_send![response, response];
            let http: bool = main && !inner.is_null() && msg_send![inner, isKindOfClass: class!(NSHTTPURLResponse)];
            // Eine Antwort, aus der keine Seite wird (kein Inhalt, oder ein
            // Download): Der Tab bleibt, wo er ist – und mit ihm seine Listen.
            if main {
                let shows: bool = msg_send![response, canShowMIMEType];
                let status: isize = if http { msg_send![inner, statusCode] } else { 200 };
                if !shows || matches!(status, 204 | 205) {
                    crate::blocker::revert(webview);
                }
            } else if !inner.is_null() && !FRAME_CLICKS.with(|clicks| clicks.borrow().idle()) {
                // Die Antwort für einen eingebetteten Rahmen ist da. Hat der Mensch
                // sie mit einem Klick angefordert, deckt der Klick genau diese
                // Antwort – und keine spätere auf dieselbe Adresse (siehe
                // `nav_response_download`).
                let address: Obj = msg_send![inner, URL];
                let text: Obj = if address.is_null() { std::ptr::null_mut() } else { msg_send![address, absoluteString] };
                let url = string_of(text);
                FRAME_CLICKS.with(|clicks| clicks.borrow_mut().answer(webview as usize, &url, Instant::now()));
            }
            if http {
                let url: Obj = msg_send![inner, URL];
                let text: Obj = if url.is_null() { std::ptr::null_mut() } else { msg_send![url, absoluteString] };
                if !text.is_null() && length_of(text) <= 4096 {
                    let url = without_fragment(&string_of(text)).to_string();
                    let mut headers: Vec<(&'static str, String)> = Vec::new();
                    for name in crate::sitecheck::HEADERS {
                        let value: Obj = msg_send![inner, valueForHTTPHeaderField: &*ns(name)];
                        if value.is_null() {
                            continue;
                        }
                        // Mehr als das liest der Check von keiner Zeile: Schutzregeln
                        // können lang sein, alles andere ist kurz.
                        let limit = if name.starts_with("content-security-policy") { 16_384 } else { 1_024 };
                        if length_of(value) > 65_536 {
                            continue;
                        }
                        let mut value = string_of(value);
                        if value.len() > limit {
                            let mut end = limit;
                            while !value.is_char_boundary(end) {
                                end -= 1;
                            }
                            value.truncate(end);
                        }
                        headers.push((*name, value));
                    }
                    RESPONSES.with(|all| {
                        let mut all = all.borrow_mut();
                        let list = all.entry(webview as usize).or_default();
                        list.retain(|old| old.url != url);
                        list.push_back(crate::sitecheck::Response { url, headers });
                        while list.len() > 24 {
                            list.pop_front();
                        }
                    });
                }
            }
        }
        match ORIGINAL_RESPONSE.get() {
            Some(original) => {
                let original: PolicyFn = std::mem::transmute(*original);
                original(this, cmd, webview, response, decide);
            }
            None => decide.call((1,)), // WKNavigationResponsePolicyAllow
        }
    }
}

/// wrys Übernahme eines Downloads, der aus der Antwort eines Servers wurde –
/// sie wird weiter aufgerufen, wenn der Download bleiben darf.
static ORIGINAL_RESPONSE_DOWNLOAD: std::sync::OnceLock<Imp> = std::sync::OnceLock::new();

thread_local! {
    /// Links und Formulare, die der Mensch mit der Maus angeklickt hat und die
    /// einen eingebetteten Rahmen woandershin führen (siehe `nav_policy`,
    /// `nav_response` und `nav_response_download`).
    static FRAME_CLICKS: RefCell<crate::pdfs::FrameClicks> = RefCell::new(crate::pdfs::FrameClicks::default());
}

type DownloadFn = extern "C-unwind" fn(Obj, Sel, Obj, Obj, Obj);

/// Vorposten vor wrys Übernahme eines Downloads, der aus der Antwort eines
/// Servers wurde. Meist hat der Browser ihn selbst verlangt (der Inhalt lässt
/// sich nicht anzeigen, oder der Server weist ihn als Datei aus) – dann geht er
/// unverändert an wry. Mit der Härtung und im Stealth-Fenster macht WebKit aber
/// auch aus PDFs, die angezeigt werden sollten, von sich aus Downloads. Gilt
/// das einem Rahmen, den eine Seite nur einbettet, hat niemand danach gefragt:
/// Zunder sagt den Download ab, bevor es für ihn ein Ziel gibt (Regel:
/// `pdfs::unasked_download`). Es sei denn, der Mensch hat selbst einen Link
/// angeklickt oder ein Formular abgeschickt, das den Rahmen dorthin führt –
/// dann ist es sein Download (`pdfs::FrameClicks`).
extern "C-unwind" fn nav_response_download(this: Obj, cmd: Sel, webview: Obj, response: Obj, download: Obj) {
    unsafe {
        if !response.is_null() && !download.is_null() {
            let main: bool = msg_send![response, isForMainFrame];
            let shows: bool = msg_send![response, canShowMIMEType];
            let inner: Obj = msg_send![response, response];
            let mut attachment = false;
            let mut pdf = false;
            let mut url = String::new();
            if !inner.is_null() {
                let http: bool = msg_send![inner, isKindOfClass: class!(NSHTTPURLResponse)];
                if http {
                    let disposition: Obj = msg_send![inner, valueForHTTPHeaderField: &*ns("Content-Disposition")];
                    attachment = crate::pdfs::is_attachment(&string_of(disposition));
                }
                let mime: Obj = msg_send![inner, MIMEType];
                pdf = length_of(mime) <= 255 && crate::pdfs::is_pdf_type(&string_of(mime));
                let address: Obj = msg_send![inner, URL];
                let text: Obj = if address.is_null() { std::ptr::null_mut() } else { msg_send![address, absoluteString] };
                url = string_of(text);
            }
            if crate::pdfs::unasked_download(main, shows, attachment, pdf) {
                let clicked = FRAME_CLICKS.with(|clicks| clicks.borrow_mut().take(webview as usize, &url, Instant::now()));
                if !clicked {
                    let done = block2::RcBlock::new(|_resume: *mut AnyObject| {});
                    let _: () = msg_send![download, cancel: &*done];
                    if crate::dev::enabled() {
                        println!("DOWNLOAD abgesagt rahmen pdf={pdf}");
                    }
                    emit(UserEvent::Nav(NavEvent { webview: webview as usize, kind: NavKind::FrameFile { pdf } }));
                    return;
                }
                if crate::dev::enabled() {
                    println!("DOWNLOAD im Rahmen nach Klick pdf={pdf}");
                }
            } else if crate::dev::enabled() {
                println!("DOWNLOAD aus Antwort tab={main} anzeigbar={shows} anhang={attachment}");
            }
        }
        // Ohne wrys Methode bekäme der Download niemanden, der ihm ein Ziel
        // nennt – WebKit bricht ihn dann von selbst ab.
        if let Some(original) = ORIGINAL_RESPONSE_DOWNLOAD.get() {
            let original: DownloadFn = std::mem::transmute(*original);
            original(this, cmd, webview, response, download);
        }
    }
}

/// Hat diese WebView gerade die Tastatur – ist sie (oder etwas in ihr) im
/// Fenster das, was Tastendrücke bekommt?
pub fn has_keyboard(webview: &WebView) -> bool {
    let w = webview.webview();
    unsafe {
        let window: Option<Retained<AnyObject>> = msg_send![&*w, window];
        let Some(window) = window else {
            return false;
        };
        let responder: Obj = msg_send![&*window, firstResponder];
        if responder.is_null() {
            return false;
        }
        let view: bool = msg_send![responder, isKindOfClass: class!(NSView)];
        view && msg_send![responder, isDescendantOf: &*w]
    }
}

/// Der Antwortkopf zu der Seite an dieser Adresse, falls er vorliegt.
pub fn response_for(webview: &WebView, url: &str) -> Option<crate::sitecheck::Response> {
    let wanted = without_fragment(url);
    RESPONSES.with(|all| {
        all.borrow().get(&key(webview)).and_then(|list| list.iter().rev().find(|response| response.url == wanted).cloned())
    })
}

/// Räumt auf: Antwortköpfe von WebViews, die es nicht mehr gibt.
pub fn keep_responses(live: &HashSet<usize>) {
    RESPONSES.with(|all| all.borrow_mut().retain(|webview, _| live.contains(webview)));
}

/// Was WebKit über die Verbindung der gezeigten Seite sagt: Kam alles
/// Gezeigte verschlüsselt an – und lief die Verbindung über TLS 1.0/1.1?
pub fn connection_facts(webview: &WebView) -> (bool, bool) {
    let w = webview.webview();
    unsafe {
        let only_secure: bool = msg_send![&*w, hasOnlySecureContent];
        let known: bool = msg_send![&*w, respondsToSelector: sel!(_negotiatedLegacyTLS)];
        let legacy: bool = known && msg_send![&*w, _negotiatedLegacyTLS];
        (only_secure, legacy)
    }
}

/// Die Merkmale der Cookies, die an diesen Rechner gehen würden – Name und
/// Schutzmerkmale, nie der Wert. Gefragt wird der Datenspeicher der WebView
/// selbst; so stimmt es auch in privaten und in Stealth-Fenstern.
pub fn cookie_flags(webview: &WebView, host: &str, done: impl Fn(Vec<crate::sitecheck::Cookie>) + 'static) {
    let w = webview.webview();
    let host = host.to_ascii_lowercase();
    unsafe {
        let config: Retained<AnyObject> = msg_send![&*w, configuration];
        let store: Retained<AnyObject> = msg_send![&*config, websiteDataStore];
        let jar: Retained<AnyObject> = msg_send![&*store, httpCookieStore];
        let found = block2::RcBlock::new(move |cookies: *mut AnyObject| {
            let mut list: Vec<crate::sitecheck::Cookie> = Vec::new();
            if !cookies.is_null() {
                let count: usize = msg_send![cookies, count];
                for index in 0..count.min(20_000) {
                    let cookie: Obj = msg_send![cookies, objectAtIndex: index];
                    let domain: Obj = msg_send![cookie, domain];
                    let name: Obj = msg_send![cookie, name];
                    if length_of(domain) > 255 || length_of(name) > 512 {
                        continue;
                    }
                    // Mit Punkt davor gilt ein Cookie auch für Unterbereiche, ohne nur für genau diesen Rechner.
                    let domain = string_of(domain).to_ascii_lowercase();
                    let fits = match domain.strip_prefix('.') {
                        Some(parent) => host == parent || host.ends_with(&format!(".{parent}")),
                        None => host == domain,
                    };
                    if !fits {
                        continue;
                    }
                    let secure: bool = msg_send![cookie, isSecure];
                    let http_only: bool = msg_send![cookie, isHTTPOnly];
                    let policy: Obj = msg_send![cookie, sameSitePolicy];
                    let policy = if length_of(policy) > 16 { String::new() } else { string_of(policy).to_ascii_lowercase() };
                    list.push(crate::sitecheck::Cookie {
                        name: string_of(name),
                        secure,
                        http_only,
                        same_site: if matches!(policy.as_str(), "lax" | "strict") { policy } else { String::new() },
                    });
                    if list.len() >= 200 {
                        break;
                    }
                }
            }
            done(list);
        });
        let _: () = msg_send![&*jar, getAllCookies: &*found];
    }
}

/// Läuft die App in der Sandbox von macOS?
pub fn in_sandbox() -> bool {
    static SANDBOXED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SANDBOXED.get_or_init(|| std::env::var_os("APP_SANDBOX_CONTAINER_ID").is_some())
}

/// Beginnt die Adresse mit genau diesem Schema?
fn has_scheme(url: &str, scheme: &str) -> bool {
    url.as_bytes().get(scheme.len()) == Some(&b':')
        && url.get(..scheme.len()).is_some_and(|start| start.eq_ignore_ascii_case(scheme))
}

/// Was ein Rahmen selbst laden kann. Alles andere (mailto:, tel:, zoommtg: …)
/// gehört einem anderen Programm.
fn loads_in_frame(lower: &str) -> bool {
    ["http:", "https:", "about:", "blob:", "data:", "file:", "javascrip", "applewe"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
}

/// Darf dieser Tab jetzt (noch) eine Datei laden? Höchstens fünf in zehn
/// Sekunden – eine Seite soll weder den Download-Ordner noch die Platte füllen.
pub fn download_slot(tab: u64) -> bool {
    thread_local! {
        static SLOTS: RefCell<HashMap<u64, (u32, Instant)>> = RefCell::new(HashMap::new());
    }
    SLOTS.with(|slots| {
        let mut slots = slots.borrow_mut();
        if slots.len() > 500 {
            slots.clear();
        }
        let entry = slots.entry(tab).or_insert((0, Instant::now()));
        if entry.1.elapsed() > Duration::from_secs(10) {
            *entry = (0, Instant::now());
        }
        entry.0 += 1;
        entry.0 <= 5
    })
}

/// Diese Adresse einmal so laden, wie sie ist – samt Tracking-Anhängsel.
pub fn load_as_is(url: &str) {
    AS_IS.with(|list| {
        let mut list = list.borrow_mut();
        if list.len() > 20 {
            list.clear();
        }
        list.push(url.to_string());
    });
}

/// Sitzt die Navigationsentscheidung an ihrem Platz? Ohne sie gäbe es keinen
/// der Schutzmechanismen in dieser Datei – dann lädt Zunder lieber nichts.
pub fn policy_hooked() -> bool {
    ORIGINAL_POLICY.get().is_some()
}

/// WebKit meldet, dass eine von Zunders Regellisten gegriffen hat. Gebraucht
/// wird das für die Liste „eigenes Netz“: Der Mensch am Browser erfährt, dass
/// eine Seite dorthin wollte – und kann es ihr erlauben.
extern "C-unwind" fn rule_list_action(_this: Obj, _cmd: Sel, webview: Obj, identifier: Obj, action: Obj, url: Obj) {
    unsafe {
        if action.is_null() || url.is_null() {
            return;
        }
        let name = string_of(identifier);
        let ads = name.starts_with(crate::blocker::FILTER_PREFIX) || name.starts_with(crate::blocker::BASIS_PREFIX);
        if !ads && !name.starts_with(crate::blocker::LOCAL_PREFIX) {
            return;
        }
        let known: bool = msg_send![action, respondsToSelector: sel!(blockedLoad)];
        if !known {
            return;
        }
        let blocked: bool = msg_send![action, blockedLoad];
        if !blocked {
            return;
        }
        if ads {
            // Werbung oder Tracker gesperrt: nur mitzählen – das Schild in der
            // Adresszeile zeigt die Zahl.
            if crate::blocker::count_blocked(webview as usize) {
                emit(UserEvent::Wake);
            }
            return;
        }
        // Eine Seite, die das ganze Netz abklopft, meldet sich höchstens einmal je Sekunde.
        let key = webview as usize;
        let fresh = LOCAL_REPORTED.with(|reported| {
            let mut reported = reported.borrow_mut();
            reported.retain(|_, at| at.elapsed() < Duration::from_secs(60));
            match reported.get(&key) {
                Some(at) if at.elapsed() < Duration::from_secs(1) => false,
                _ => {
                    reported.insert(key, Instant::now());
                    true
                }
            }
        });
        if fresh {
            let text: Obj = msg_send![url, absoluteString];
            emit(UserEvent::Nav(NavEvent { webview: key, kind: NavKind::LocalBlocked { url: string_of(text) } }));
        }
    }
}

/// Fragt, ob in der Seite gerade Ton oder Video läuft. Die Antwort kommt als
/// `NavKind::Media`.
pub fn request_media_state(webview: &WebView) {
    let w = webview.webview();
    let key = Retained::as_ptr(&w) as usize;
    unsafe {
        let supported: bool =
            msg_send![&*w, respondsToSelector: sel!(requestMediaPlaybackStateWithCompletionHandler:)];
        if !supported {
            return;
        }
        // WKMediaPlaybackStatePlaying = 1
        let done = block2::RcBlock::new(move |state: usize| {
            emit(UserEvent::Nav(NavEvent {
                webview: key,
                kind: NavKind::Media { playing: state == 1 },
            }));
        });
        let _: () = msg_send![&*w, requestMediaPlaybackStateWithCompletionHandler: &*done];
    }
}

extern "C-unwind" fn ui_did_close(_this: Obj, _cmd: Sel, webview: Obj) {
    emit(UserEvent::Nav(NavEvent {
        webview: webview as usize,
        kind: NavKind::Closed,
    }));
}

/// Rechner des Rahmens, der einen Dialog ausgelöst hat – laut WebKit. Die
/// Adresse der WebView taugt dafür nicht: Während eine Seite lädt, nennt sie
/// schon das Ziel, und der Dialog käme scheinbar von dort.
unsafe fn frame_host(frame: Obj) -> String {
    if frame.is_null() {
        return String::new();
    }
    let origin: Obj = msg_send![frame, securityOrigin];
    if origin.is_null() {
        return String::new();
    }
    let host: Obj = msg_send![origin, host];
    string_of(host)
}

/// Ist diese WebView gerade zu sehen? Ein Tab im Hintergrund bekommt keinen
/// Dialog – er legte sich sonst über die Seite, die man gerade vor sich hat.
unsafe fn is_shown(webview: Obj) -> bool {
    if webview.is_null() {
        return false;
    }
    let window: Obj = msg_send![webview, window];
    let hidden: bool = msg_send![webview, isHiddenOrHasHiddenAncestor];
    !window.is_null() && !hidden
}

/// Zeigt einen Dialog der Seite. Rückgabe: (bestätigt, eingegebener Text).
/// Nach dem ersten Dialog einer Website bietet jeder weitere ein Häkchen an,
/// mit dem sie sich stummschalten lässt – sonst könnte eine Seite den Browser
/// mit einer Dialogschleife blockieren.
unsafe fn run_dialog(
    webview: Obj,
    frame: Obj,
    message: &str,
    with_cancel: bool,
    input: Option<&str>,
) -> (bool, Option<String>) {
    let id = webview as usize;
    // Kein Dialog einer Seite legt sich über die Frage nach Kamera und
    // Mikrofon: Ein schneller Doppelklick auf seinen Knopf träfe sonst mit dem
    // zweiten Klick die Frage darunter.
    if !is_shown(webview) || media_asking() {
        return (false, None);
    }
    let host = frame_host(frame);
    let (count, muted) = DIALOGS.with(|d| {
        let mut d = d.borrow_mut();
        if d.len() > 500 {
            d.clear();
        }
        let state = d.entry(id).or_default();
        if state.host != host {
            // Eine andere Website in diesem Tab: Für sie beginnt die Zählung neu.
            *state = DialogState { host: host.clone(), count: 0, muted: false };
        }
        state.count += 1;
        (state.count, state.muted)
    });
    if muted {
        return (false, None);
    }

    let title = if host.is_empty() {
        t("dialog_title_plain").to_string()
    } else {
        t("dialog_title").replace("{host}", &host)
    };
    let alert: Retained<AnyObject> = msg_send![class!(NSAlert), new];
    let _: () = msg_send![&*alert, setMessageText: &*ns(&title)];
    // Sehr lange Texte kürzen, damit der Dialog bedienbar bleibt.
    let shown: String = message.chars().take(2000).collect();
    let _: () = msg_send![&*alert, setInformativeText: &*ns(&shown)];
    let _: Obj = msg_send![&*alert, addButtonWithTitle: &*ns(t("ok"))];
    if with_cancel {
        let _: Obj = msg_send![&*alert, addButtonWithTitle: &*ns(t("cancel"))];
    }
    if count > 1 {
        let _: () = msg_send![&*alert, setShowsSuppressionButton: true];
        let button: Obj = msg_send![&*alert, suppressionButton];
        if !button.is_null() {
            let _: () = msg_send![button, setTitle: &*ns(t("dialog_suppress"))];
        }
    }

    let mut field: Option<Retained<AnyObject>> = None;
    if let Some(default) = input {
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(300.0, 24.0));
        let allocated: Allocated<AnyObject> = msg_send![class!(NSTextField), alloc];
        let f: Retained<AnyObject> = msg_send![allocated, initWithFrame: frame];
        let default: String = default.chars().take(2000).collect();
        let _: () = msg_send![&*f, setStringValue: &*ns(&default)];
        let _: () = msg_send![&*alert, setAccessoryView: &*f];
        let window: Obj = msg_send![&*alert, window];
        if !window.is_null() {
            let _: () = msg_send![window, setInitialFirstResponder: &*f];
        }
        field = Some(f);
    }

    // NSAlertFirstButtonReturn = 1000
    let response: isize = msg_send![&*alert, runModal];
    let confirmed = response == 1000;

    if count > 1 {
        let button: Obj = msg_send![&*alert, suppressionButton];
        if !button.is_null() {
            let state: isize = msg_send![button, state];
            if state == 1 {
                DIALOGS.with(|d| {
                    if let Some(entry) = d.borrow_mut().get_mut(&id) {
                        entry.muted = true;
                    }
                });
            }
        }
    }

    let text = match (&field, confirmed) {
        (Some(f), true) => {
            let value: Obj = msg_send![&**f, stringValue];
            Some(string_of(value))
        }
        _ => None,
    };
    (confirmed, text)
}

extern "C-unwind" fn js_alert(
    _this: Obj,
    _cmd: Sel,
    webview: Obj,
    message: Obj,
    frame: Obj,
    done: &Block<dyn Fn()>,
) {
    unsafe {
        let message = string_of(message);
        run_dialog(webview, frame, &message, false, None);
    }
    done.call(());
}

extern "C-unwind" fn js_confirm(
    _this: Obj,
    _cmd: Sel,
    webview: Obj,
    message: Obj,
    frame: Obj,
    done: &Block<dyn Fn(Bool)>,
) {
    let ok = unsafe {
        let message = string_of(message);
        run_dialog(webview, frame, &message, true, None).0
    };
    done.call((Bool::new(ok),));
}

extern "C-unwind" fn js_prompt(
    _this: Obj,
    _cmd: Sel,
    webview: Obj,
    prompt: Obj,
    default_text: Obj,
    frame: Obj,
    done: &Block<dyn Fn(*mut NSString)>,
) {
    let answer = unsafe {
        let prompt = string_of(prompt);
        let default_text = string_of(default_text);
        run_dialog(webview, frame, &prompt, true, Some(&default_text)).1
    };
    match answer {
        Some(text) => {
            let value = ns(&text);
            done.call((Retained::as_ptr(&value) as *mut NSString,));
        }
        None => done.call((std::ptr::null_mut(),)),
    }
}

unsafe fn text_field(class: &AnyClass, y: f64, placeholder: &str) -> Retained<AnyObject> {
    let frame = NSRect::new(NSPoint::new(0.0, y), NSSize::new(300.0, 24.0));
    let allocated: Allocated<AnyObject> = msg_send![class, alloc];
    let field: Retained<AnyObject> = msg_send![allocated, initWithFrame: frame];
    let _: () = msg_send![&*field, setPlaceholderString: &*ns(placeholder)];
    field
}

/// Fragt Benutzername und Passwort für eine geschützte Seite ab.
unsafe fn run_login_dialog(host: &str, realm: &str, encrypted: bool) -> Option<(String, String)> {
    let alert: Retained<AnyObject> = msg_send![class!(NSAlert), new];
    let _: () = msg_send![&*alert, setMessageText: &*ns(&t("auth_title").replace("{host}", host))];
    let mut text = t("auth_text").replace("{host}", host);
    // Der Bereichsname kommt vom Server: ohne Zeilenumbrüche und Anführungszeichen
    // (damit er nicht wie Text des Browsers aussehen kann), als Zitat und gekürzt.
    let realm: String = realm
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '"' | '„' | '“' | '”' | '«' | '»'))
        .take(80)
        .collect();
    let realm = realm.trim();
    if !realm.is_empty() {
        text.push(' ');
        text.push_str(&t("auth_realm").replace("{realm}", &realm));
    }
    if !encrypted {
        text.push_str("\n\n");
        text.push_str(t("auth_insecure"));
    }
    let _: () = msg_send![&*alert, setInformativeText: &*ns(&text)];
    let _: Obj = msg_send![&*alert, addButtonWithTitle: &*ns(t("auth_login"))];
    let _: Obj = msg_send![&*alert, addButtonWithTitle: &*ns(t("cancel"))];

    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(300.0, 56.0));
    let allocated: Allocated<AnyObject> = msg_send![class!(NSView), alloc];
    let container: Retained<AnyObject> = msg_send![allocated, initWithFrame: frame];
    let user = text_field(class!(NSTextField), 32.0, t("auth_user"));
    let password = text_field(class!(NSSecureTextField), 0.0, t("auth_password"));
    let _: () = msg_send![&*container, addSubview: &*user];
    let _: () = msg_send![&*container, addSubview: &*password];
    let _: () = msg_send![&*user, setNextKeyView: &*password];
    let _: () = msg_send![&*alert, setAccessoryView: &*container];
    let window: Obj = msg_send![&*alert, window];
    if !window.is_null() {
        let _: () = msg_send![window, setInitialFirstResponder: &*user];
    }

    let response: isize = msg_send![&*alert, runModal];
    if response != 1000 {
        return None;
    }
    let user_value: Obj = msg_send![&*user, stringValue];
    let password_value: Obj = msg_send![&*password, stringValue];
    Some((string_of(user_value), string_of(password_value)))
}

/// Der Server verlangt eine Anmeldung. Zertifikatsprüfungen und alles andere
/// bleiben beim System – hier geht es nur um Benutzername und Passwort.
extern "C-unwind" fn nav_auth_challenge(
    _this: Obj,
    _cmd: Sel,
    webview: Obj,
    challenge: Obj,
    done: &Block<dyn Fn(isize, *mut AnyObject)>,
) {
    // NSURLSessionAuthChallengeDisposition
    const USE_CREDENTIAL: isize = 0;
    const DEFAULT_HANDLING: isize = 1;
    const CANCEL: isize = 2;
    unsafe {
        let space: Obj = msg_send![challenge, protectionSpace];
        if space.is_null() {
            done.call((DEFAULT_HANDLING, std::ptr::null_mut()));
            return;
        }
        let method: Obj = msg_send![space, authenticationMethod];
        let method = string_of(method);
        let wants_password = matches!(
            method.as_str(),
            "NSURLAuthenticationMethodHTTPBasic"
                | "NSURLAuthenticationMethodHTTPDigest"
                | "NSURLAuthenticationMethodNTLM"
                | "NSURLAuthenticationMethodDefault"
        );
        let failures: isize = msg_send![challenge, previousFailureCount];
        if !wants_password {
            done.call((DEFAULT_HANDLING, std::ptr::null_mut()));
            return;
        }
        if failures > 3 {
            done.call((CANCEL, std::ptr::null_mut()));
            return;
        }
        // Beim ersten Versuch eine bereits bekannte Anmeldung verwenden.
        if failures == 0 {
            let proposed: Obj = msg_send![challenge, proposedCredential];
            if !proposed.is_null() {
                let has_password: bool = msg_send![proposed, hasPassword];
                if has_password {
                    done.call((USE_CREDENTIAL, proposed));
                    return;
                }
            }
        }
        let host: Obj = msg_send![space, host];
        let host = string_of(host);
        let realm: Obj = msg_send![space, realm];
        // Verschlüsselt heißt hier: über https. (Auch manche Verfahren ohne
        // https gelten dem System als „sicher“ – das Passwort ginge trotzdem
        // über eine Verbindung, die jeder im Netz verändern kann.)
        let protocol: Obj = msg_send![space, protocol];
        let encrypted = string_of(protocol).eq_ignore_ascii_case("https");
        // Gefragt wird nur für die Seite selbst – nicht für ein Bild oder Skript,
        // das sie von einem fremden Rechner einbindet –, nur im sichtbaren Tab
        // und nicht in Serie.
        let page: Obj = if webview.is_null() { std::ptr::null_mut() } else { msg_send![webview, URL] };
        let page_host: Obj = if page.is_null() { std::ptr::null_mut() } else { msg_send![page, host] };
        let own = !host.is_empty() && string_of(page_host).eq_ignore_ascii_case(&host);
        let too_many = AUTH_PROMPTS.with(|prompts| {
            let mut prompts = prompts.borrow_mut();
            if prompts.len() > 500 {
                prompts.clear();
            }
            let entry = prompts.entry(webview as usize).or_insert((0, Instant::now()));
            if entry.1.elapsed() > Duration::from_secs(60) {
                *entry = (0, Instant::now());
            }
            entry.0 += 1;
            entry.0 > 4
        });
        if !own || too_many || !is_shown(webview) || media_asking() {
            done.call((CANCEL, std::ptr::null_mut()));
            return;
        }
        match run_login_dialog(&host, &string_of(realm), encrypted) {
            Some((user, password)) => {
                // NSURLCredentialPersistenceForSession = 1: gilt bis zum Beenden.
                let credential: Retained<AnyObject> = msg_send![
                    class!(NSURLCredential),
                    credentialWithUser: &*ns(&user),
                    password: &*ns(&password),
                    persistence: 1usize
                ];
                done.call((USE_CREDENTIAL, Retained::as_ptr(&credential) as *mut AnyObject));
            }
            None => done.call((CANCEL, std::ptr::null_mut())),
        }
    }
}

/// Der Mensch am Browser hat den Tab selbst woandershin geschickt (oder ihn
/// geschlossen): Dialogzähler und Stummschaltung dieser WebView beginnen neu.
pub fn reset_dialogs(webview_key: usize) {
    DIALOGS.with(|d| {
        d.borrow_mut().remove(&webview_key);
    });
    AUTH_PROMPTS.with(|p| {
        p.borrow_mut().remove(&webview_key);
    });
}

// ---------------------------------------------------------------------------
// Kamera und Mikrofon je Website
// ---------------------------------------------------------------------------
//
// Will eine Seite aufnehmen, fragt WebKit den Browser. Die Antwort gibt Zunder
// selbst: nach dem, was für die Website gemerkt ist – sonst mit einer Frage,
// die den Namen der Website nennt. Die Regeln dazu stehen in `media.rs`.

thread_local! {
    /// Gemerkte Entscheidungen je Website – so, wie sie in den Einstellungen stehen.
    static MEDIA_RULES: RefCell<crate::media::Rules> = const { RefCell::new(crate::media::Rules::new()) };
    /// Was je Tab abgelehnt wurde, seit der Mensch dort zuletzt selbst neu
    /// geladen oder eine Adresse aufgerufen hat.
    static MEDIA_REFUSED: RefCell<HashMap<usize, crate::media::Refusals>> = RefCell::new(HashMap::new());
    /// Tabs privater Fenster und von Stealth-Fenstern: Dort wird nichts gemerkt.
    static PRIVATE_VIEWS: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
    /// Steht gerade eine Frage zu Kamera oder Mikrofon auf dem Schirm?
    static MEDIA_ASKING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// So lange lässt sich „Erlauben“ nach dem Erscheinen der Frage nicht drücken.
const MEDIA_HOLD: f64 = 0.7;

/// Die gemerkten Entscheidungen zu Kamera und Mikrofon.
pub fn set_media_rules(rules: crate::media::Rules) {
    MEDIA_RULES.with(|known| *known.borrow_mut() = rules);
}

/// Ein neuer Tab ist entstanden. Für ihn beginnt alles frisch – auch dann,
/// wenn er im Speicher an der Stelle eines eben geschlossenen liegt und dessen
/// Vermerke noch nicht weggeräumt sind. `private`: Er gehört zu einem privaten
/// Fenster oder einem Stealth-Fenster.
pub fn media_new_view(key: usize, private: bool) {
    MEDIA_REFUSED.with(|refused| refused.borrow_mut().remove(&key));
    FRAME_CLICKS.with(|clicks| clicks.borrow_mut().forget(key));
    PRIVATE_VIEWS.with(|set| {
        let mut set = set.borrow_mut();
        if private {
            set.insert(key);
        } else {
            set.remove(&key);
        }
    });
}

/// Der Mensch am Browser lädt die Seite selbst neu oder ruft in diesem Tab
/// selbst eine Adresse auf: Was dort zu Kamera oder Mikrofon abgelehnt wurde,
/// gilt dann nicht mehr – Seiten dürfen wieder fragen. Lädt eine Seite sich
/// selbst neu oder schickt sie den Tab woandershin, bleibt es beim Nein; sonst
/// könnte sie die Frage in einer Schleife stellen, und die hält den ganzen
/// Browser an.
pub fn media_visit_over(webview: usize) {
    MEDIA_REFUSED.with(|refused| refused.borrow_mut().remove(&webview));
}

/// Der Mensch hat in diesem Tab selbst weitergeklickt oder geblättert: Die
/// Geduld des Tabs beginnt neu – einzelne Websites bleiben abgelehnt (siehe
/// `media::Refusals`).
pub fn media_user_moved(webview: usize) {
    MEDIA_REFUSED.with(|refused| {
        if let Some(tab) = refused.borrow_mut().get_mut(&webview) {
            tab.moved_on();
        }
    });
}

/// Steht hinter dieser Navigation ein Klick oder Tastendruck des Menschen –
/// nach WebKits eigener Auskunft? Ohne die Auskunft gilt: nein.
unsafe fn clicked_by_user(action: Obj) -> bool {
    let knows: bool = msg_send![action, respondsToSelector: sel!(_isUserInitiated)];
    knows && msg_send![action, _isUserInitiated]
}

/// Protokoll, Rechner und Anschluss einer Herkunft, wie WebKit sie nennt.
unsafe fn origin_parts(origin: Obj) -> (String, String, isize) {
    if origin.is_null() {
        return (String::new(), String::new(), 0);
    }
    let protocol: Obj = msg_send![origin, protocol];
    let host: Obj = msg_send![origin, host];
    let port: isize = msg_send![origin, port];
    (string_of(protocol).to_ascii_lowercase(), string_of(host), port)
}

/// Eine Seite möchte Kamera oder Mikrofon benutzen.
extern "C-unwind" fn media_permission(
    _this: Obj,
    _cmd: Sel,
    webview: Obj,
    origin: Obj,
    frame: Obj,
    kind: isize,
    decide: &Block<dyn Fn(isize)>,
) {
    // WKPermissionDecisionGrant = 1, WKPermissionDecisionDeny = 2. „Frag selbst“
    // (0) gibt Zunder nie zurück: WebKits eigene Frage merkt sich nichts.
    let (allowed, how) = unsafe { media_decision(webview, origin, frame, kind) };
    if crate::dev::enabled() {
        use std::io::Write;
        println!("MEDIA art={kind} {} ({how})", if allowed { "erlaubt" } else { "abgelehnt" });
        let _ = std::io::stdout().flush();
    }
    decide.call((if allowed { 1 } else { 2 },));
}

/// Rückgabe: erlaubt? – und in einem Wort, wie es dazu kam (für die Prüfhilfe).
unsafe fn media_decision(webview: Obj, origin: Obj, frame: Obj, kind: isize) -> (bool, &'static str) {
    use crate::media::{self, Verdict, Want};
    let Some(want) = Want::from_kind(kind) else {
        return (false, "unbekannte Art");
    };
    if webview.is_null() || frame.is_null() {
        return (false, "ohne Rahmen");
    }
    let id = webview as usize;
    let main: bool = msg_send![frame, isMainFrame];
    // Wer fragt (der Rahmen) – und in welcher Seite er steckt. Für die Seite
    // selbst ist das dasselbe; bei einem eingebetteten Rahmen nennt WebKit die
    // Seite im Tab eigens.
    let asker_origin: Obj = msg_send![frame, securityOrigin];
    let (asker_protocol, asker_host, asker_port) = origin_parts(asker_origin);
    let (protocol, host, port) = if main {
        (asker_protocol.clone(), asker_host.clone(), asker_port)
    } else {
        origin_parts(origin)
    };
    // Zunders eigene Seiten nehmen nichts auf.
    if protocol == "zunder" || asker_protocol == "zunder" {
        return (false, "eigene Seite");
    }
    let label = media::site_label(&host, port);
    let asker = media::site_label(&asker_host, asker_port);
    // Wer keinen Namen hat – eine Datei, ein abgeschottetes Dokument –,
    // bekommt nichts: Die Frage könnte nicht sagen, wer da fragt.
    if label.is_empty() || asker.is_empty() {
        return (false, "ohne Namen");
    }
    let site = media::site_key(media_protocol(&protocol), &host, port);
    let private = PRIVATE_VIEWS.with(|set| set.borrow().contains(&id));
    let remembered = |site: &Option<String>| {
        site.as_ref()
            .and_then(|site| MEDIA_RULES.with(|rules| rules.borrow().get(site).copied()))
            .unwrap_or_default()
    };
    // Unter diesem Namen steht ein Nein aus diesem Besuch: die Website – und
    // für einen Rahmen er selbst in ihr. So sperrt ein Nein an einen Rahmen
    // nicht die Seite, die ihn einbettet.
    let visit = if main { label.clone() } else { format!("{label} > {asker}") };
    let mut rule = remembered(&site);
    let (refused, exhausted) = MEDIA_REFUSED.with(|tabs| match tabs.borrow().get(&id) {
        Some(tab) if main => (tab.of(&label), tab.exhausted()),
        // Was der Seite abgelehnt wurde, bekommt auch kein Rahmen in ihr.
        Some(tab) => (media::with_refusals(tab.of(&label), tab.of(&visit)), tab.exhausted()),
        None => (media::Rule::default(), false),
    });
    if !main {
        // Ein gemerktes Nein gilt der Website überall – auch dort, wo sie nur
        // als Rahmen in einer anderen steckt.
        let asker_site = media::site_key(media_protocol(&asker_protocol), &asker_host, asker_port);
        rule = media::with_refusals(rule, remembered(&asker_site));
    }
    match media::verdict(want, rule, refused, main, private) {
        Verdict::Grant => return (true, "gemerkt"),
        Verdict::Deny => return (false, "gemerkt oder in diesem Besuch abgelehnt"),
        // Nach mehreren Nein in Folge fragt in diesem Tab niemand mehr.
        Verdict::Ask if exhausted => return (false, "Geduld des Tabs erschöpft"),
        Verdict::Ask => {}
    }
    // Ein Tab im Hintergrund bekommt keine Frage – und damit auch nichts.
    if !is_shown(webview) {
        return (false, "Tab nicht zu sehen");
    }
    // Die Frage legt sich über keinen anderen Dialog – und keiner über sie
    // (siehe `run_dialog`): Wer auf den einen klickt, soll nie den anderen treffen.
    if modal_open() {
        return (false, "ein anderer Dialog ist offen");
    }
    // Ist die Liste der gemerkten Websites voll, gibt es das Häkchen nicht –
    // statt eines Häkchens, das dann nichts bewirkt.
    let can_remember = main
        && !private
        && site.as_ref().is_some_and(|site| MEDIA_RULES.with(|rules| media::has_room(&rules.borrow(), site)));
    let Some((allowed, remember)) = media_prompt(want, &asker, &label, main, can_remember) else {
        return (false, "es steht schon eine Frage offen");
    };
    if !allowed {
        // Für diesen Besuch ist die Frage beantwortet.
        MEDIA_REFUSED.with(|tabs| tabs.borrow_mut().entry(id).or_default().refuse(&visit, want));
    }
    if let (true, Some(site)) = (remember && can_remember, site) {
        let kept = MEDIA_RULES.with(|rules| {
            let mut rules = rules.borrow_mut();
            media::remember(&mut rules, &site, want, allowed).then(|| rules.get(&site).copied()).flatten()
        });
        if let Some(rule) = kept {
            emit(UserEvent::MediaRule { site, rule });
            return (allowed, "gefragt, gemerkt");
        }
    }
    (allowed, "gefragt")
}

/// Das Protokoll, wie es beim Merken zählt. Prüfhilfe: Im Probelauf gibt es
/// keine verschlüsselten Seiten – dort zählt eine unverschlüsselte wie eine
/// verschlüsselte, damit sich auch das Merken prüfen lässt.
fn media_protocol(protocol: &str) -> &str {
    match protocol {
        "http" if crate::dev::var_os("ZUNDER_MEDIA_HTTP").is_some() => "https",
        other => other,
    }
}

/// Der Name, unter dem sich Zunder die Website dieser Adresse merkt – wenn es
/// für sie überhaupt etwas merkt.
pub fn media_site_of(address: &str) -> Option<String> {
    let parsed = url::Url::parse(address).ok()?;
    let port = parsed.port().map_or(0, |port| port as isize);
    crate::media::site_key(media_protocol(parsed.scheme()), parsed.host_str()?, port)
}

/// Läuft gerade irgendein modaler Dialog der App?
unsafe fn modal_open() -> bool {
    let app: Retained<AnyObject> = msg_send![class!(NSApplication), sharedApplication];
    let window: Obj = msg_send![&*app, modalWindow];
    !window.is_null()
}

/// Steht gerade eine Frage zu Kamera oder Mikrofon auf dem Schirm? Solange
/// öffnet keine Seite einen eigenen Dialog darüber.
fn media_asking() -> bool {
    MEDIA_ASKING.with(std::cell::Cell::get)
}

/// Vermerk „es steht eine Frage offen“. Er verschwindet, wie auch immer die
/// Frage endet – sonst bekäme nach einem Abbruch nie wieder jemand eine.
struct Asking;

impl Asking {
    fn begin() -> Option<Asking> {
        (!MEDIA_ASKING.with(|asking| asking.replace(true))).then_some(Asking)
    }
}

impl Drop for Asking {
    fn drop(&mut self) {
        MEDIA_ASKING.with(|asking| asking.set(false));
    }
}

/// Ein Taktgeber, der mit seinem Besitzer endet.
struct Ticking(Retained<AnyObject>);

impl Drop for Ticking {
    fn drop(&mut self) {
        unsafe {
            let _: () = msg_send![&*self.0, invalidate];
        }
    }
}

/// Fragt, ob die Website aufnehmen darf. Rückgabe: (erlaubt, merken) – oder
/// nichts, wenn schon eine solche Frage offen ist.
unsafe fn media_prompt(
    want: crate::media::Want,
    asker: &str,
    site: &str,
    main: bool,
    can_remember: bool,
) -> Option<(bool, bool)> {
    use crate::media::Want;
    // Eine zweite Frage stapelt sich nicht über die erste.
    let _asking = Asking::begin()?;
    let title = t(match want {
        Want::Camera => "media_ask_camera",
        Want::Microphone => "media_ask_mic",
        Want::Both => "media_ask_both",
    })
    .replace("{site}", asker);
    let mut text = String::new();
    if !main {
        // Nicht die Seite selbst fragt, sondern etwas, das sie einbettet.
        if site == asker {
            text.push_str(t("media_frame"));
        } else {
            text.push_str(&t("media_frame_on").replace("{site}", site));
        }
        text.push_str("\n\n");
    }
    text.push_str(t("media_text"));
    if crate::dev::enabled() {
        use std::io::Write;
        println!("MEDIAFRAGE {title} || {} || merken={can_remember}", text.replace('\n', " "));
        let _ = std::io::stdout().flush();
    }

    let alert: Retained<AnyObject> = msg_send![class!(NSAlert), new];
    let _: () = msg_send![&*alert, setMessageText: &*ns(&title)];
    let _: () = msg_send![&*alert, setInformativeText: &*ns(&text)];
    // Der erste Knopf ist der, den die Eingabetaste drückt: das Nein.
    let _: Obj = msg_send![&*alert, addButtonWithTitle: &*ns(t("media_deny"))];
    let allow: Option<Retained<AnyObject>> = msg_send![&*alert, addButtonWithTitle: &*ns(t("media_allow"))];
    if can_remember {
        let _: () = msg_send![&*alert, setShowsSuppressionButton: true];
        let button: Obj = msg_send![&*alert, suppressionButton];
        if !button.is_null() {
            let _: () = msg_send![button, setTitle: &*ns(&t("media_remember").replace("{site}", site))];
        }
    }
    // „Erlauben“ lässt sich erst drücken, wenn die Frage einen Augenblick lang
    // ununterbrochen zuoberst stand: Eine Seite, die zu schnellen Klicks
    // verleitet und die Frage genau darunter aufgehen lässt, bekommt so kein
    // Ja untergeschoben. Legt sich etwas anderes darüber oder ist Zunder nicht
    // vorn, beginnt die Zeit von vorn. Fehlt der Taktgeber, bleibt der Knopf
    // gesperrt – dann lässt sich nur ablehnen.
    let mut ticking = None;
    if let Some(allow) = allow.as_ref() {
        let _: () = msg_send![&**allow, setEnabled: false];
        let button = allow.clone();
        let panel: Obj = msg_send![&*alert, window];
        let panel = panel as usize;
        // Prüfhilfe: Die Probe-App ist selten vorn.
        let always_front = crate::dev::var_os("ZUNDER_MODAL_FRONT").is_some();
        let since: std::rc::Rc<std::cell::Cell<Option<Instant>>> = std::rc::Rc::default();
        let watch = block2::RcBlock::new(move |_timer: Obj| {
            let app: Retained<AnyObject> = msg_send![class!(NSApplication), sharedApplication];
            let modal: Obj = msg_send![&*app, modalWindow];
            let active: bool = msg_send![&*app, isActive];
            let ready = if modal as usize == panel && (active || always_front) {
                let start = since.get().unwrap_or_else(Instant::now);
                since.set(Some(start));
                start.elapsed().as_secs_f64() >= MEDIA_HOLD
            } else {
                since.set(None);
                false
            };
            let _: () = msg_send![&*button, setEnabled: ready];
        });
        let timer: Option<Retained<AnyObject>> =
            msg_send![class!(NSTimer), timerWithTimeInterval: 0.1f64, repeats: true, block: &*watch];
        if let Some(timer) = timer {
            let run_loop: Retained<AnyObject> = msg_send![class!(NSRunLoop), currentRunLoop];
            let _: () = msg_send![&*run_loop, addTimer: &*timer, forMode: &*ns("NSModalPanelRunLoopMode")];
            ticking = Some(Ticking(timer));
        }
    }

    // NSAlertSecondButtonReturn = 1001
    let response: isize = msg_send![&*alert, runModal];
    drop(ticking);
    let allowed = response == 1001;
    let mut remember = false;
    if can_remember {
        let button: Obj = msg_send![&*alert, suppressionButton];
        if !button.is_null() {
            let state: isize = msg_send![button, state];
            remember = state == 1;
        }
    }
    Some((allowed, remember))
}

/// Beendet eine laufende Aufnahme in diesem Tab – wenn jemand der Website
/// Kamera oder Mikrofon wieder nimmt, soll das sofort gelten.
pub fn stop_capture(webview: &WebView, camera: bool, microphone: bool) {
    let w = webview.webview();
    unsafe {
        // WKMediaCaptureStateNone = 0
        let known: bool = msg_send![&*w, respondsToSelector: sel!(setCameraCaptureState:completionHandler:)];
        if camera && known {
            let _: () = msg_send![&*w, setCameraCaptureState: 0isize, completionHandler: Option::<&Block<dyn Fn()>>::None];
        }
        let known: bool = msg_send![&*w, respondsToSelector: sel!(setMicrophoneCaptureState:completionHandler:)];
        if microphone && known {
            let _: () =
                msg_send![&*w, setMicrophoneCaptureState: 0isize, completionHandler: Option::<&Block<dyn Fn()>>::None];
        }
    }
}

thread_local! {
    /// Welche Methoden nachgerüstet wurden – für die Prüfhilfe.
    static HOOKS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

pub fn installed_hooks() -> Vec<String> {
    HOOKS.with(|h| h.borrow().clone())
}

unsafe fn add_method(class: &AnyClass, selector: Sel, imp: Imp, types: &CStr) -> bool {
    if class.instance_method(selector).is_some() {
        // wry bringt die Methode inzwischen selbst mit – dann nichts überschreiben.
        return false;
    }
    let raw = class as *const AnyClass as *mut AnyClass;
    let added = ffi::class_addMethod(raw, selector, imp, types.as_ptr()).as_bool();
    if added {
        HOOKS.with(|h| h.borrow_mut().push(selector.name().to_string_lossy().into_owned()));
    }
    added
}

/// Rüstet die Delegate-Klassen von wry nach. Die Klassen werden über die
/// Delegates einer schon bestehenden WebView gefunden (der Rahmen-WebView);
/// das muss vor dem Erzeugen des ersten Inhalts-Tabs passieren.
pub fn install_hooks(existing: &WebView) {
    use std::mem::transmute;
    thread_local! {
        static DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    if DONE.with(|d| d.replace(true)) {
        return;
    }
    let w = existing.webview();
    unsafe {
        let nav: Option<Retained<AnyObject>> = msg_send![&*w, navigationDelegate];
        let ui: Option<Retained<AnyObject>> = msg_send![&*w, UIDelegate];
        let (Some(nav), Some(ui)) = (nav, ui) else {
            return;
        };
        let nav = nav.class();
        let ui = ui.class();
        // Die Navigationsentscheidung von wry bekommt einen Vorposten.
        let previous = ffi::class_replaceMethod(
            nav as *const AnyClass as *mut AnyClass,
            sel!(webView:decidePolicyForNavigationAction:decisionHandler:),
            transmute::<PolicyFn, Imp>(nav_policy),
            c"v@:@@@?".as_ptr(),
        );
        if let Some(previous) = previous {
            let _ = ORIGINAL_POLICY.set(previous);
            HOOKS.with(|h| h.borrow_mut().push("decidePolicyForNavigationAction(ersetzt)".into()));
        }
        // Ebenso die Entscheidung über Antworten: Davor liest der Seitencheck den Antwortkopf mit.
        let previous = ffi::class_replaceMethod(
            nav as *const AnyClass as *mut AnyClass,
            sel!(webView:decidePolicyForNavigationResponse:decisionHandler:),
            transmute::<PolicyFn, Imp>(nav_response),
            c"v@:@@@?".as_ptr(),
        );
        match previous {
            Some(previous) => {
                let _ = ORIGINAL_RESPONSE.set(previous);
                HOOKS.with(|h| h.borrow_mut().push("decidePolicyForNavigationResponse(ersetzt)".into()));
            }
            None => HOOKS.with(|h| h.borrow_mut().push("decidePolicyForNavigationResponse".into())),
        }
        // Und die Übernahme eines Downloads, der aus einer Antwort wurde: Davor
        // sagt Zunder ab, was WebKit für einen eingebetteten Rahmen von sich aus
        // herunterladen will.
        let previous = ffi::class_replaceMethod(
            nav as *const AnyClass as *mut AnyClass,
            sel!(webView:navigationResponse:didBecomeDownload:),
            transmute::<DownloadFn, Imp>(nav_response_download),
            c"v@:@@@".as_ptr(),
        );
        match previous {
            Some(previous) => {
                let _ = ORIGINAL_RESPONSE_DOWNLOAD.set(previous);
                HOOKS.with(|h| h.borrow_mut().push("navigationResponseDidBecomeDownload(ersetzt)".into()));
            }
            None => HOOKS.with(|h| h.borrow_mut().push("navigationResponseDidBecomeDownload".into())),
        }
        add_method(
            nav,
            sel!(webView:didStartProvisionalNavigation:),
            transmute::<extern "C-unwind" fn(Obj, Sel, Obj, Obj), Imp>(nav_started),
            c"v@:@@",
        );
        add_method(
            nav,
            sel!(webView:didReceiveServerRedirectForProvisionalNavigation:),
            transmute::<extern "C-unwind" fn(Obj, Sel, Obj, Obj), Imp>(nav_redirected),
            c"v@:@@",
        );
        add_method(
            nav,
            sel!(webView:didFailProvisionalNavigation:withError:),
            transmute::<extern "C-unwind" fn(Obj, Sel, Obj, Obj, Obj), Imp>(nav_failed_provisional),
            c"v@:@@@",
        );
        add_method(
            nav,
            sel!(webView:didFailNavigation:withError:),
            transmute::<extern "C-unwind" fn(Obj, Sel, Obj, Obj, Obj), Imp>(nav_failed_committed),
            c"v@:@@@",
        );
        add_method(
            nav,
            sel!(webView:didReceiveAuthenticationChallenge:completionHandler:),
            transmute::<
                extern "C-unwind" fn(Obj, Sel, Obj, Obj, &Block<dyn Fn(isize, *mut AnyObject)>),
                Imp,
            >(nav_auth_challenge),
            c"v@:@@@?",
        );
        add_method(
            nav,
            sel!(_webView:contentRuleListWithIdentifier:performedAction:forURL:),
            transmute::<extern "C-unwind" fn(Obj, Sel, Obj, Obj, Obj, Obj), Imp>(rule_list_action),
            c"v@:@@@@",
        );
        // Kamera und Mikrofon: Die Frage stellt Zunder selbst. wry erführe
        // dabei nicht, welche Website fragt – und WebKits eigene Frage merkt
        // sich nichts.
        type MediaFn = extern "C-unwind" fn(Obj, Sel, Obj, Obj, Obj, isize, &Block<dyn Fn(isize)>);
        let previous = ffi::class_replaceMethod(
            ui as *const AnyClass as *mut AnyClass,
            sel!(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:),
            transmute::<MediaFn, Imp>(media_permission),
            c"v@:@@@q@?".as_ptr(),
        );
        HOOKS.with(|h| {
            h.borrow_mut().push(
                if previous.is_some() { "requestMediaCapturePermission(ersetzt)" } else { "requestMediaCapturePermission" }
                    .into(),
            )
        });
        add_method(
            ui,
            sel!(webViewDidClose:),
            transmute::<extern "C-unwind" fn(Obj, Sel, Obj), Imp>(ui_did_close),
            c"v@:@",
        );
        add_method(
            ui,
            sel!(webView:runJavaScriptAlertPanelWithMessage:initiatedByFrame:completionHandler:),
            transmute::<extern "C-unwind" fn(Obj, Sel, Obj, Obj, Obj, &Block<dyn Fn()>), Imp>(
                js_alert,
            ),
            c"v@:@@@@?",
        );
        add_method(
            ui,
            sel!(webView:runJavaScriptConfirmPanelWithMessage:initiatedByFrame:completionHandler:),
            transmute::<extern "C-unwind" fn(Obj, Sel, Obj, Obj, Obj, &Block<dyn Fn(Bool)>), Imp>(
                js_confirm,
            ),
            c"v@:@@@@?",
        );
        add_method(
            ui,
            sel!(webView:runJavaScriptTextInputPanelWithPrompt:defaultText:initiatedByFrame:completionHandler:),
            transmute::<
                extern "C-unwind" fn(Obj, Sel, Obj, Obj, Obj, Obj, &Block<dyn Fn(*mut NSString)>),
                Imp,
            >(js_prompt),
            c"v@:@@@@@?",
        );
    }
}

// ---------------------------------------------------------------------------
// Draht zu den Seiten (abgeschottete Skriptwelt)
// ---------------------------------------------------------------------------
//
// Das Skript des Passwort-Tresors läuft in jeder Seite in einer eigenen Welt
// (WKContentWorld): Es sieht dasselbe Dokument wie die Seite, aber die Skripte
// der Seite sehen weder seine Variablen noch seinen Nachrichtenkanal. Was über
// diesen Kanal kommt, stammt deshalb wirklich von Zunders Skript – und WebKit
// selbst nennt dazu den Rahmen und die Herkunft.

/// Zunders Skript in jeder Seite: erkennt Anmeldeformulare und füllt sie aus.
const LOGINS_JS: &str = include_str!("../ui/logins.js");
/// Beobachter für Stealth-Fenster: zählt, was Seiten über das Gerät abfragen.
const WATCH_JS: &str = include_str!("../ui/watch.js");
const BRIDGE_NAME: &str = "zunder";

/// Nachricht von Zunders Skript in einer Seite.
#[derive(Debug)]
pub struct BridgeMessage {
    /// Adresse der WKWebView – Schlüssel zum Wiederfinden des Tabs.
    pub webview: usize,
    pub main_frame: bool,
    /// Herkunft des Rahmens laut WebKit, z. B. `https://example.com`.
    pub origin: String,
    pub body: String,
}

/// Nachricht des Skripts, das in der Welt der Seite mitläuft (`inject.js`),
/// oder einer der eingebauten Seiten. Kommt aus der Seite und ist damit
/// grundsätzlich nicht vertrauenswürdig – Rahmen und Adresse nennt WebKit.
#[derive(Debug)]
pub struct PagePing {
    pub webview: usize,
    /// Adresse des Dokuments, das die Nachricht geschickt hat.
    pub origin: String,
    pub body: String,
    /// Kam sie aus der Seite selbst – oder aus einem eingebetteten Rahmen?
    pub main_frame: bool,
}

/// Kanal der Seiten-Welt. Ihn nimmt Zunder selbst entgegen (statt über wry):
/// Dort bricht ein Text mit halbem Zeichenpaar das Programm ab – eine einzige
/// Skriptzeile einer Webseite hätte genügt.
const PAGE_CHANNEL: &str = "ipc";

extern "C-unwind" fn bridge_received(_this: Obj, _cmd: Sel, _controller: Obj, message: Obj) {
    if message.is_null() {
        return;
    }
    unsafe {
        let body: Obj = msg_send![message, body];
        if body.is_null() {
            return;
        }
        let is_text: bool = msg_send![body, isKindOfClass: class!(NSString)];
        if !is_text || length_of(body) > 16384 {
            return;
        }
        let body = string_of(body);
        let frame: Obj = msg_send![message, frameInfo];
        if frame.is_null() {
            return;
        }
        let main_frame: bool = msg_send![frame, isMainFrame];
        let webview: Obj = msg_send![message, webView];
        let name: Obj = msg_send![message, name];
        if string_of(name) == PAGE_CHANNEL {
            // Unterrahmen haben dem Browser nichts zu melden – außer der Zahl
            // ihrer Abfragen für die Tarn-Ampel (`q:`), die nichts auslöst.
            if body.len() > 4096 || (!main_frame && !body.starts_with("q:")) {
                return;
            }
            let request: Obj = msg_send![frame, request];
            let url: Obj = if request.is_null() { std::ptr::null_mut() } else { msg_send![request, URL] };
            let text: Obj = if url.is_null() { std::ptr::null_mut() } else { msg_send![url, absoluteString] };
            let origin = string_of(text);
            // Die Rückfrage-Seiten melden die Zustimmung („unverschlüsselt
            // öffnen“, „trotz ähnlicher Adresse öffnen“, „dieser Website das
            // eigene Netz erlauben“). Das wird sofort vermerkt – noch bevor die
            // Seite weiterleitet – und nur von ihnen angenommen. Laden kann
            // diese Seiten allein der Browser (siehe `nav_policy`).
            let from_error_page = url::Url::parse(&origin)
                .is_ok_and(|u| u.scheme() == "zunder" && u.host_str() == Some("error"));
            if let Some(host) = body.strip_prefix("x:") {
                if from_error_page {
                    allow_http(host);
                }
                return;
            }
            // „Trotzdem öffnen“ auf der Warnung vor einer gemeldeten Betrugsseite.
            if let Some(address) = body.strip_prefix("g:") {
                if from_error_page {
                    crate::blocker::guard_allow(address);
                }
                return;
            }
            if let Some(host) = body.strip_prefix("k:") {
                if from_error_page {
                    SITES.with(|sites| sites.borrow_mut().allow(host));
                }
                return;
            }
            if let Some(site) = body.strip_prefix("L:") {
                if !(from_error_page && allow_local_site(&crate::sites::split_host(site).1)) {
                    return;
                }
            }
            emit(UserEvent::Ping(PagePing { webview: webview as usize, origin, body, main_frame }));
            return;
        }
        let security: Obj = msg_send![frame, securityOrigin];
        if security.is_null() {
            return;
        }
        let protocol: Obj = msg_send![security, protocol];
        let host: Obj = msg_send![security, host];
        let port: isize = msg_send![security, port];
        let (protocol, mut host) = (string_of(protocol), string_of(host));
        if host.contains(':') && !host.starts_with('[') {
            host = format!("[{host}]");
        }
        let origin = if port > 0 {
            format!("{protocol}://{host}:{port}")
        } else {
            format!("{protocol}://{host}")
        };
        emit(UserEvent::Bridge(BridgeMessage {
            webview: webview as usize,
            main_frame,
            origin,
            body,
        }));
    }
}

/// Der eine Empfänger für alle Tabs. Die Klasse entsteht beim ersten Gebrauch.
fn bridge_handler() -> Retained<AnyObject> {
    thread_local! {
        static HANDLER: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
    }
    HANDLER.with(|slot| {
        slot.borrow_mut()
            .get_or_insert_with(|| unsafe {
                let name = c"ZunderPageBridge";
                let class = match AnyClass::get(name) {
                    Some(class) => class,
                    None => {
                        let mut builder = objc2::runtime::ClassBuilder::new(name, class!(NSObject))
                            .expect("Klassenname ist frei");
                        builder.add_method::<AnyObject, _>(
                            sel!(userContentController:didReceiveScriptMessage:),
                            bridge_received as extern "C-unwind" fn(Obj, Sel, Obj, Obj),
                        );
                        builder.register()
                    }
                };
                msg_send![class, new]
            })
            .clone()
    })
}

/// Ein Tab, den eine Seite öffnet, bekommt von WebKit die Konfiguration des
/// öffnenden Tabs – samt dessen Liste von Skripten und Sperrlisten (ein und
/// dasselbe Objekt). Hier bekommt er seine eigene: Sonst gälte für beide Tabs
/// immer dasselbe, auch wenn sie längst verschiedene Websites zeigen.
pub fn own_controller(config: &WKWebViewConfiguration, stealth: bool) {
    unsafe {
        let fresh: Retained<AnyObject> = msg_send![class!(WKUserContentController), new];
        let _: () = msg_send![config, setUserContentController: &*fresh];
    }
    attach_bridge(config);
    if !(guards().hardened || stealth) {
        attach_pdfs(config);
    }
    if stealth {
        attach_watch(config);
    }
}

/// Stealth-Fenster: der Beobachter, der mitzählt, was Seiten abfragen – in der
/// Welt der Seite und in jedem Rahmen, gleich zu Beginn des Dokuments.
fn attach_watch(config: &WKWebViewConfiguration) {
    unsafe {
        let controller = config.userContentController();
        let allocated: Allocated<AnyObject> = msg_send![class!(WKUserScript), alloc];
        // WKUserScriptInjectionTimeAtDocumentStart = 0
        let script: Retained<AnyObject> =
            msg_send![allocated, initWithSource: &*ns(WATCH_JS), injectionTime: 0isize, forMainFrameOnly: false];
        let _: () = msg_send![&*controller, addUserScript: &*script];
    }
}

/// Hängt Zunders Skript und seinen Nachrichtenkanal an die Konfiguration
/// eines Inhalts-Tabs – beides in der abgeschotteten Welt, nur im Hauptrahmen.
fn attach_bridge(config: &WKWebViewConfiguration) {
    let handler = bridge_handler();
    unsafe {
        let controller = config.userContentController();
        let world: Retained<AnyObject> = msg_send![class!(WKContentWorld), defaultClientWorld];
        let _: () = msg_send![
            &*controller,
            addScriptMessageHandler: &*handler,
            contentWorld: &*world,
            name: &*ns(BRIDGE_NAME)
        ];
        // Der Kanal der Seiten-Welt (inject.js, Start- und Fehlerseite).
        let page_world: Retained<AnyObject> = msg_send![class!(WKContentWorld), pageWorld];
        let _: () = msg_send![
            &*controller,
            addScriptMessageHandler: &*handler,
            contentWorld: &*page_world,
            name: &*ns(PAGE_CHANNEL)
        ];
        let allocated: Allocated<AnyObject> = msg_send![class!(WKUserScript), alloc];
        // WKUserScriptInjectionTimeAtDocumentStart = 0
        let script: Retained<AnyObject> = msg_send![
            allocated,
            initWithSource: &*ns(LOGINS_JS),
            injectionTime: 0isize,
            forMainFrameOnly: true,
            inContentWorld: &*world
        ];
        let _: () = msg_send![&*controller, addUserScript: &*script];
    }
}

// ---------------------------------------------------------------------------
// Eingebettete PDFs (`zunder-pdf:`)
// ---------------------------------------------------------------------------
//
// Hilft PDF-Rahmen, die WebKit leer lässt – was und warum, steht in `pdfs.rs`.
// Hier: das Skript in der Seite, die Stelle, die Dateien annimmt und wieder
// ausliefert, und die Anmeldung der eigenen Adresse bei WebKit.

const PDFS_JS: &str = include_str!("../ui/pdfs.js");

thread_local! {
    static PDF_SHELF: RefCell<crate::pdfs::Shelf> = RefCell::new(crate::pdfs::Shelf::default());
}

/// Schlüssel dieses Starts: Mit ihm beginnt jede Lieferung von Zunders Skript.
/// Er steht nur in dessen abgeschotteter Welt – die Skripte einer Seite sehen
/// ihn nicht und können deshalb selbst nichts abliefern.
fn pdf_key() -> &'static str {
    static KEY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    KEY.get_or_init(|| crate::pdfs::random_hex().unwrap_or_default())
}

/// Im Tab steht eine neue Seite: Was die alte abgelegt hat, gilt nicht mehr.
/// Aufgerufen in dem Augenblick, in dem WebKit die neue Seite übernimmt – also
/// bevor sie selbst etwas laden kann, und erst nachdem die alte nichts mehr
/// abliefern kann. (Beim Aufbruch wäre es zu früh: Die alte Seite läuft dann
/// noch, und ein Aufbruch, aus dem nur ein Download wird, ließe sie stehen.)
pub fn pdfs_page_left(view: usize) {
    PDF_SHELF.with(|shelf| shelf.borrow_mut().forget(view));
    // Auch was der Mensch auf der alten Seite für einen ihrer Rahmen
    // angeklickt hat, gilt für die neue nicht.
    FRAME_CLICKS.with(|clicks| clicks.borrow_mut().forget(view));
}

/// Für wen eine Antwort auf eine Anfrage an `zunder-pdf:` bestimmt ist.
#[derive(Clone, Copy, PartialEq)]
enum PdfReader {
    /// Zunders Skript: Es läuft unter der Herkunft der Seite und muss die
    /// Kennung lesen können.
    Script,
    /// Der Rahmen, der die Datei zeigt.
    Frame,
}

/// Eine Absage: Der Abruf scheitert, als gäbe es die Adresse nicht – ohne
/// Antwort, deren Zeilen oder Nummer ein Skript der Seite lesen könnte.
unsafe fn pdf_refuse(task: Obj) {
    // NSURLErrorUnsupportedURL = -1002
    let error: Retained<AnyObject> = msg_send![
        class!(NSError),
        errorWithDomain: &*ns("NSURLErrorDomain"),
        code: -1002isize,
        userInfo: Option::<&AnyObject>::None
    ];
    let _: () = msg_send![task, didFailWithError: &*error];
}

/// Antwort auf eine Anfrage an `zunder-pdf:`.
unsafe fn pdf_reply(task: Obj, url: Obj, status: isize, kind: &str, body: &[u8], reader: PdfReader) {
    let headers: Retained<AnyObject> = msg_send![class!(NSMutableDictionary), new];
    let set = |name: &str, value: &str| {
        let _: () = msg_send![&*headers, setObject: &*ns(value), forKey: &*ns(name)];
    };
    set("Content-Type", kind);
    set("Content-Length", &body.len().to_string());
    set("Cache-Control", "no-store");
    // Was hier herausgeht, ist ein PDF oder ein kurzer Text – WebKit soll es
    // nie als Seite oder als Skript deuten.
    set("X-Content-Type-Options", "nosniff");
    match reader {
        PdfReader::Script => set("Access-Control-Allow-Origin", "*"),
        PdfReader::Frame => {
            // Der Rahmen steht in einer fremden Seite – auch in einer, die von
            // allem Eingebetteten eine ausdrückliche Erlaubnis verlangt.
            set("Cross-Origin-Resource-Policy", "cross-origin");
            set("Cross-Origin-Embedder-Policy", "require-corp");
        }
    }
    let allocated: Allocated<AnyObject> = msg_send![class!(NSHTTPURLResponse), alloc];
    let response: Option<Retained<AnyObject>> = msg_send![
        allocated,
        initWithURL: url,
        statusCode: status,
        HTTPVersion: &*ns("HTTP/1.1"),
        headerFields: &*headers
    ];
    let Some(response) = response else {
        pdf_refuse(task);
        return;
    };
    let _: () = msg_send![task, didReceiveResponse: &*response];
    if !body.is_empty() {
        let data = objc2_foundation::NSData::with_bytes(body);
        let _: () = msg_send![task, didReceiveData: &*data];
    }
    let _: () = msg_send![task, didFinish];
}

/// WebKit reicht jede Anfrage an `zunder-pdf:` hierher – samt der WebView,
/// aus der sie kommt. Beantwortet wird sie sofort, noch in diesem Aufruf.
extern "C-unwind" fn pdf_task_start(_this: Obj, _cmd: Sel, webview: Obj, task: Obj) {
    use crate::pdfs::{Ask, Refusal};
    if task.is_null() {
        return;
    }
    unsafe {
        let request: Obj = msg_send![task, request];
        let url: Obj = if request.is_null() { std::ptr::null_mut() } else { msg_send![request, URL] };
        if url.is_null() {
            pdf_refuse(task);
            return;
        }
        let text: Obj = msg_send![url, absoluteString];
        let address = if length_of(text) > 2048 { String::new() } else { string_of(text) };
        let method: Obj = msg_send![request, HTTPMethod];
        let method = if length_of(method) > 16 { String::new() } else { string_of(method) };
        let view = webview as usize;
        const TEXT: &str = "text/plain; charset=utf-8";
        match crate::pdfs::route(&method, &address) {
            Ask::Put => {
                // Nur ein Körper am Stück: So schickt ihn Zunders Skript. Alles
                // andere (ein Strom, eine Datei von der Platte) wird nicht gelesen.
                let limit = crate::pdfs::Limits::default().doc + crate::pdfs::KEY_LEN;
                let data: Option<Retained<objc2_foundation::NSData>> = msg_send![request, HTTPBody];
                let size = data.as_ref().map_or(0, |data| data.len());
                let result = match data.as_ref() {
                    Some(data) if size > 0 && size <= limit => {
                        // Gelesen wird an Ort und Stelle: Die Daten gehören der
                        // Anfrage und ändern sich nicht, solange `data` lebt.
                        let body = data.as_bytes_unchecked();
                        PDF_SHELF.with(|shelf| {
                            shelf.borrow_mut().put(view, pdf_key(), body, Instant::now(), crate::pdfs::random_hex)
                        })
                    }
                    _ => Err(Refusal::Size),
                };
                if crate::dev::enabled() {
                    use std::io::Write;
                    match &result {
                        Ok(token) => println!("PDF abgelegt bytes={size} kennung={token}"),
                        Err(why) => println!("PDF abgelehnt bytes={size} ({why:?})"),
                    }
                    let _ = std::io::stdout().flush();
                }
                match result {
                    Ok(token) => pdf_reply(task, url, 200, TEXT, token.as_bytes(), PdfReader::Script),
                    // „Gerade zu viel“ erfährt nur, wer den Schlüssel hatte – also
                    // Zunders Skript. Es versucht es später noch einmal.
                    Err(Refusal::Busy) => pdf_reply(task, url, 429, TEXT, b"", PdfReader::Script),
                    Err(_) => pdf_refuse(task),
                }
            }
            Ask::Doc(token) => {
                let found = PDF_SHELF.with(|shelf| shelf.borrow().get(view, &token));
                if crate::dev::enabled() {
                    use std::io::Write;
                    println!("PDF abgeholt kennung={token} {}", if found.is_some() { "geliefert" } else { "unbekannt" });
                    let _ = std::io::stdout().flush();
                }
                match found {
                    Some(bytes) => pdf_reply(task, url, 200, "application/pdf", &bytes, PdfReader::Frame),
                    None => pdf_refuse(task),
                }
            }
            Ask::Unknown => pdf_refuse(task),
        }
    }
}

/// Die Antwort ist längst geschickt, wenn WebKit einen Abruf abbricht.
extern "C-unwind" fn pdf_task_stop(_this: Obj, _cmd: Sel, _webview: Obj, _task: Obj) {}

/// Die eine Stelle für alle Tabs. Die Klasse entsteht beim ersten Gebrauch.
fn pdf_handler() -> Retained<AnyObject> {
    thread_local! {
        static HANDLER: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
    }
    HANDLER.with(|slot| {
        slot.borrow_mut()
            .get_or_insert_with(|| unsafe {
                let name = c"ZunderPdfShelf";
                let class = match AnyClass::get(name) {
                    Some(class) => class,
                    None => {
                        let mut builder = objc2::runtime::ClassBuilder::new(name, class!(NSObject))
                            .expect("Klassenname ist frei");
                        builder.add_method::<AnyObject, _>(
                            sel!(webView:startURLSchemeTask:),
                            pdf_task_start as extern "C-unwind" fn(Obj, Sel, Obj, Obj),
                        );
                        builder.add_method::<AnyObject, _>(
                            sel!(webView:stopURLSchemeTask:),
                            pdf_task_stop as extern "C-unwind" fn(Obj, Sel, Obj, Obj),
                        );
                        builder.register()
                    }
                };
                msg_send![class, new]
            })
            .clone()
    })
}

/// Hängt die Hilfe für PDF-Rahmen an die Konfiguration eines Inhalts-Tabs:
/// die Stelle hinter `zunder-pdf:` und Zunders Skript (abgeschottete Welt, nur
/// im Hauptrahmen).
fn attach_pdfs(config: &WKWebViewConfiguration) {
    let key = pdf_key();
    if key.len() != crate::pdfs::KEY_LEN {
        return;
    }
    unsafe {
        // Zwei Zusagen von WebKit braucht es, sonst lüde der Rahmen die Adresse
        // gar nicht erst: Die Schutzregeln einer Seite (`frame-src`) lassen sie
        // zu, und in einer verschlüsselt geladenen Seite gilt sie nicht als
        // unsicherer Inhalt. Was unter der Adresse zu holen ist, bestimmt allein
        // der Browser – siehe `pdfs.rs`. Kennt das System die Zusagen nicht,
        // bleibt alles, wie WebKit es von sich aus macht.
        let pool: Option<Retained<AnyObject>> = msg_send![config, processPool];
        let Some(pool) = pool else {
            return;
        };
        let lenient: bool = msg_send![&*pool, respondsToSelector: sel!(_registerURLSchemeAsBypassingContentSecurityPolicy:)];
        let secure: bool = msg_send![&*pool, respondsToSelector: sel!(_registerURLSchemeAsSecure:)];
        if !(lenient && secure) {
            return;
        }
        let scheme = ns(crate::pdfs::SCHEME);
        // Ein Tab, den eine Seite öffnet, bringt die Stelle schon mit (seine
        // Konfiguration ist eine Kopie) – zweimal anmelden lässt WebKit nicht zu.
        let present: Option<Retained<AnyObject>> = msg_send![config, urlSchemeHandlerForURLScheme: &*scheme];
        if present.is_none() {
            let handler = pdf_handler();
            let _: () = msg_send![config, setURLSchemeHandler: &*handler, forURLScheme: &*scheme];
        }
        let _: () = msg_send![&*pool, _registerURLSchemeAsBypassingContentSecurityPolicy: &*scheme];
        let _: () = msg_send![&*pool, _registerURLSchemeAsSecure: &*scheme];

        let controller = config.userContentController();
        let world: Retained<AnyObject> = msg_send![class!(WKContentWorld), defaultClientWorld];
        let source = PDFS_JS.replace("%KEY%", key);
        let allocated: Allocated<AnyObject> = msg_send![class!(WKUserScript), alloc];
        // WKUserScriptInjectionTimeAtDocumentStart = 0
        let script: Retained<AnyObject> = msg_send![
            allocated,
            initWithSource: &*ns(&source),
            injectionTime: 0isize,
            forMainFrameOnly: true,
            inContentWorld: &*world
        ];
        let _: () = msg_send![&*controller, addUserScript: &*script];
    }
}

/// Führt ein Skript in Zunders abgeschotteter Welt aus (im Hauptrahmen) und
/// meldet das Ergebnis, sofern es Text ist. Die Skripte der Seite können weder
/// mitlesen noch die dabei benutzten Funktionen unterschieben.
pub fn eval_isolated(webview: &WebView, script: &str, done: impl Fn(Option<String>) + 'static) {
    let w = webview.webview();
    let block = block2::RcBlock::new(move |result: *mut AnyObject, _error: *mut AnyObject| {
        let text = unsafe {
            if result.is_null() {
                None
            } else {
                let is_text: bool = msg_send![result, isKindOfClass: class!(NSString)];
                is_text.then(|| string_of(result))
            }
        };
        done(text);
    });
    unsafe {
        let world: Retained<AnyObject> = msg_send![class!(WKContentWorld), defaultClientWorld];
        let _: () = msg_send![
            &*w,
            evaluateJavaScript: &*ns(script),
            inFrame: Option::<&AnyObject>::None,
            inContentWorld: &*world,
            completionHandler: &*block
        ];
    }
}

/// Ruft eine Funktion in Zunders abgeschotteter Welt auf (im Hauptrahmen).
/// `body` ist ihr Rumpf, `payload` kommt als Argument an – also nie als Teil
/// des Quelltexts. Was auch immer das Skript an Fehlern meldet, kann deshalb
/// nichts davon enthalten. So gehen Passwörter in die Seite.
pub fn call_isolated(webview: &WebView, body: &str, payload: &str, done: impl Fn(Option<String>) + 'static) {
    let w = webview.webview();
    let block = block2::RcBlock::new(move |result: *mut AnyObject, _error: *mut AnyObject| {
        let text = unsafe {
            if result.is_null() {
                None
            } else {
                let is_text: bool = msg_send![result, isKindOfClass: class!(NSString)];
                is_text.then(|| string_of(result))
            }
        };
        done(text);
    });
    unsafe {
        let world: Retained<AnyObject> = msg_send![class!(WKContentWorld), defaultClientWorld];
        let arguments: Retained<AnyObject> =
            msg_send![class!(NSDictionary), dictionaryWithObject: &*ns(payload), forKey: &*ns("payload")];
        let _: () = msg_send![
            &*w,
            callAsyncJavaScript: &*ns(body),
            arguments: &*arguments,
            inFrame: Option::<&AnyObject>::None,
            inContentWorld: &*world,
            completionHandler: &*block
        ];
    }
}

/// Prüfhilfe: ein echter Mausklick in eine WebView (in Punkten ab ihrer linken
/// oberen Ecke). Anders als ein im Skript nachgestellter Klick gilt er der
/// Seite als vom Menschen ausgelöst.
pub fn click(webview: &WebView, x: f64, y: f64) {
    let w = webview.webview();
    unsafe {
        let window: Option<Retained<AnyObject>> = msg_send![&*w, window];
        let Some(window) = window else {
            return;
        };
        let bounds: NSRect = msg_send![&*w, bounds];
        let flipped: bool = msg_send![&*w, isFlipped];
        let local = NSPoint::new(x, if flipped { y } else { bounds.size.height - y });
        let place: NSPoint = msg_send![&*w, convertPoint: local, toView: Option::<&AnyObject>::None];
        let number: isize = msg_send![&*window, windowNumber];
        let process: Retained<AnyObject> = msg_send![class!(NSProcessInfo), processInfo];
        let uptime: f64 = msg_send![&*process, systemUptime];
        // NSEventTypeLeftMouseDown = 1, NSEventTypeLeftMouseUp = 2
        for (kind, pressure) in [(1usize, 1.0f32), (2usize, 0.0f32)] {
            let event: Option<Retained<AnyObject>> = msg_send![
                class!(NSEvent),
                mouseEventWithType: kind,
                location: place,
                modifierFlags: 0usize,
                timestamp: uptime,
                windowNumber: number,
                context: Option::<&AnyObject>::None,
                eventNumber: 0isize,
                clickCount: 1isize,
                pressure: pressure
            ];
            let Some(event) = event else {
                return;
            };
            if kind == 1 {
                let _: () = msg_send![&*w, mouseDown: &*event];
            } else {
                let _: () = msg_send![&*w, mouseUp: &*event];
            }
        }
    }
}

/// Prüfhilfe: Text so eingeben, wie die Tastatur es täte – in das Feld, das in
/// der WebView gerade den Fokus hat. Der Seite gilt das als echte Eingabe.
pub fn type_text(webview: &WebView, text: &str) {
    let w = webview.webview();
    unsafe {
        // NSNotFound: an der Einfügemarke.
        let range = objc2_foundation::NSRange { location: isize::MAX as usize, length: 0 };
        let _: () = msg_send![&*w, insertText: &*ns(text), replacementRange: range];
    }
}

// ---------------------------------------------------------------------------
// Touch ID, Zwischenablage, Dateiauswahl
// ---------------------------------------------------------------------------

#[link(name = "LocalAuthentication", kind = "framework")]
extern "C" {}

thread_local! {
    /// Laufende Abfragen: Der Kontext muss leben, bis die Antwort da ist.
    static AUTH_CONTEXTS: RefCell<HashMap<u64, Retained<AnyObject>>> = RefCell::new(HashMap::new());
}

/// Hat dieser Mac Touch ID (oder eine gekoppelte Apple Watch) eingerichtet?
pub fn biometrics_available() -> bool {
    if crate::dev::var_os("ZUNDER_AUTH").is_some() {
        return true;
    }
    unsafe {
        let context: Retained<AnyObject> = msg_send![class!(LAContext), new];
        // LAPolicyDeviceOwnerAuthenticationWithBiometrics = 1
        msg_send![&*context, canEvaluatePolicy: 1isize, error: std::ptr::null_mut::<*mut AnyObject>()]
    }
}

/// Fragt nach Touch ID – oder, wo es das nicht gibt, nach dem Passwort des
/// Macs. Die Antwort kommt als `UserEvent::Auth` mit derselben Nummer.
/// `reason` vervollständigt den Satz „Zunder versucht, …“.
pub fn authenticate(reason: &str, token: u64) {
    let Some(proxy) = PROXY.with(|p| p.borrow().clone()) else {
        return;
    };
    // Nur mit Prüfhilfe: Die Antwort kommt aus der Umgebung statt vom Menschen.
    if let Some(answer) = crate::dev::var("ZUNDER_AUTH") {
        let _ = proxy.send_event(UserEvent::Auth { token, ok: answer == "ok" });
        return;
    }
    unsafe {
        let context: Retained<AnyObject> = msg_send![class!(LAContext), new];
        // Die Antwort kommt auf einem anderen Thread – deshalb nur die Nummer weiterreichen.
        let reply = block2::RcBlock::new(move |success: Bool, _error: *mut AnyObject| {
            let _ = proxy.send_event(UserEvent::Auth { token, ok: success.as_bool() });
        });
        // LAPolicyDeviceOwnerAuthentication = 2: Touch ID, Apple Watch oder Passwort des Macs
        let _: () = msg_send![&*context, evaluatePolicy: 2isize, localizedReason: &*ns(reason), reply: &*reply];
        AUTH_CONTEXTS.with(|contexts| contexts.borrow_mut().insert(token, context));
    }
}

/// Die Abfrage mit dieser Nummer ist beantwortet.
pub fn auth_finished(token: u64) {
    AUTH_CONTEXTS.with(|contexts| {
        contexts.borrow_mut().remove(&token);
    });
}

/// Die Zwischenablage. Nur mit Prüfhilfe (ZUNDER_PASTEBOARD) eine eigene,
/// benannte – damit ein Testlauf nicht die echte überschreibt.
unsafe fn pasteboard() -> Retained<AnyObject> {
    if let Some(name) = crate::dev::var("ZUNDER_PASTEBOARD") {
        return msg_send![class!(NSPasteboard), pasteboardWithName: &*ns(&name)];
    }
    msg_send![class!(NSPasteboard), generalPasteboard]
}

/// Legt Text in die Zwischenablage. Mit `secret` so markiert, dass
/// Zwischenablage-Verwalter ihn nicht aufzeichnen. Liefert den Zählerstand
/// der Zwischenablage danach – daran lässt sich später erkennen, ob sie
/// inzwischen jemand anders gefüllt hat.
pub fn copy_text(text: &str, secret: bool) -> isize {
    unsafe {
        let board = pasteboard();
        // Ein Passwort bleibt auf diesem Mac: Die gemeinsame Zwischenablage
        // (iPhone, iPad, andere Macs) bekommt es nicht.
        // NSPasteboardContentsCurrentHostOnly = 1
        let _: isize = if secret {
            msg_send![&*board, prepareForNewContentsWithOptions: 1usize]
        } else {
            msg_send![&*board, clearContents]
        };
        let _: bool = msg_send![&*board, setString: &*ns(text), forType: &*ns("public.utf8-plain-text")];
        if secret {
            let _: bool = msg_send![&*board, setString: &*ns(""), forType: &*ns("org.nspasteboard.ConcealedType")];
        }
        msg_send![&*board, changeCount]
    }
}

/// Leert die Zwischenablage, wenn seit `count` nichts Neues hineinkam.
pub fn clear_clipboard_if(count: isize) {
    unsafe {
        let board = pasteboard();
        let current: isize = msg_send![&*board, changeCount];
        if current == count {
            let _: isize = msg_send![&*board, clearContents];
        }
    }
}

/// Lässt eine Datei wählen – zum Öffnen oder (mit `save_as`) als Ziel zum
/// Sichern. Der Dialog hängt als Blatt am Fenster; das Ergebnis kommt als
/// `UserEvent::FileChosen`.
pub fn choose_file(window_webview: &WebView, window: tao::window::WindowId, purpose: &'static str, save_as: Option<&str>) {
    let ns_window = window_webview.ns_window();
    unsafe {
        let panel: Retained<AnyObject> = match save_as {
            Some(name) => {
                let panel: Retained<AnyObject> = msg_send![class!(NSSavePanel), savePanel];
                let _: () = msg_send![&*panel, setNameFieldStringValue: &*ns(name)];
                panel
            }
            None => {
                let panel: Retained<AnyObject> = msg_send![class!(NSOpenPanel), openPanel];
                let _: () = msg_send![&*panel, setCanChooseFiles: true];
                let _: () = msg_send![&*panel, setCanChooseDirectories: false];
                let _: () = msg_send![&*panel, setAllowsMultipleSelection: false];
                panel
            }
        };
        let chosen = panel.clone();
        let done = block2::RcBlock::new(move |response: isize| {
            // NSModalResponseOK = 1
            let path = if response == 1 {
                let url: Option<Retained<AnyObject>> = msg_send![&*chosen, URL];
                url.and_then(|url| {
                    let path: Option<Retained<NSString>> = msg_send![&*url, path];
                    path.map(|p| p.to_string())
                })
            } else {
                None
            };
            emit(UserEvent::FileChosen { window, purpose, path });
        });
        let _: () = msg_send![&*panel, beginSheetModalForWindow: &*ns_window, completionHandler: &*done];
    }
}

// ---------------------------------------------------------------------------
// Sonstiges
// ---------------------------------------------------------------------------

/// Löscht Cookies, Website-Daten und Zwischenspeicher des normalen Profils.
pub fn clear_website_data(tag: u64) {
    unsafe {
        let store: Retained<AnyObject> = msg_send![class!(WKWebsiteDataStore), defaultDataStore];
        let types: Retained<AnyObject> = msg_send![class!(WKWebsiteDataStore), allWebsiteDataTypes];
        let since: Retained<AnyObject> = msg_send![class!(NSDate), distantPast];
        // Die Rückmeldung braucht nur, wer auf das Ende wartet (Löschen beim Beenden).
        let done = block2::RcBlock::new(move || emit(UserEvent::DataCleared(tag)));
        let _: () = msg_send![&*store, removeDataOfTypes: &*types, modifiedSince: &*since, completionHandler: &*done];
    }
}

/// Welche Websites auf diesem Mac Daten abgelegt haben – je Website die Arten
/// (`cookies`, `storage`, `cache`). Private und Stealth-Fenster haben eigene,
/// flüchtige Speicher und tauchen hier nicht auf.
pub fn site_data(done: impl Fn(Vec<(String, Vec<&'static str>)>) + 'static) {
    unsafe {
        let store: Retained<AnyObject> = msg_send![class!(WKWebsiteDataStore), defaultDataStore];
        let types: Retained<AnyObject> = msg_send![class!(WKWebsiteDataStore), allWebsiteDataTypes];
        let found = block2::RcBlock::new(move |records: *mut AnyObject| {
            let mut sites: Vec<(String, Vec<&'static str>)> = Vec::new();
            if !records.is_null() {
                let count: usize = msg_send![records, count];
                for index in 0..count.min(5000) {
                    let record: Obj = msg_send![records, objectAtIndex: index];
                    let name: Obj = msg_send![record, displayName];
                    let kinds: Obj = msg_send![record, dataTypes];
                    let all: Obj = if kinds.is_null() { std::ptr::null_mut() } else { msg_send![kinds, allObjects] };
                    let mut found: Vec<&'static str> = Vec::new();
                    let total: usize = if all.is_null() { 0 } else { msg_send![all, count] };
                    for position in 0..total {
                        let kind: Obj = msg_send![all, objectAtIndex: position];
                        let kind = string_of(kind);
                        let group = if kind.contains("Cookie") {
                            "cookies"
                        } else if kind.contains("Cache") {
                            "cache"
                        } else {
                            "storage"
                        };
                        if !found.contains(&group) {
                            found.push(group);
                        }
                    }
                    found.sort();
                    let name = string_of(name);
                    if !name.is_empty() {
                        sites.push((name.chars().take(253).collect(), found));
                    }
                }
            }
            sites.sort();
            done(sites);
        });
        let _: () = msg_send![&*store, fetchDataRecordsOfTypes: &*types, completionHandler: &*found];
    }
}

/// Löscht alles, was die genannten Websites auf diesem Mac abgelegt haben –
/// ohne Namen: alles von allen.
pub fn remove_site_data(names: Vec<String>, done: impl Fn() + Clone + 'static) {
    unsafe {
        let store: Retained<AnyObject> = msg_send![class!(WKWebsiteDataStore), defaultDataStore];
        let types: Retained<AnyObject> = msg_send![class!(WKWebsiteDataStore), allWebsiteDataTypes];
        let (keep_store, keep_types) = (store.clone(), types.clone());
        let found = block2::RcBlock::new(move |records: *mut AnyObject| {
            if records.is_null() {
                done();
                return;
            }
            let chosen: Retained<AnyObject> = msg_send![class!(NSMutableArray), array];
            let count: usize = msg_send![records, count];
            for index in 0..count {
                let record: Obj = msg_send![records, objectAtIndex: index];
                let name: Obj = msg_send![record, displayName];
                if names.is_empty() || names.contains(&string_of(name)) {
                    let _: () = msg_send![&*chosen, addObject: record];
                }
            }
            let done = done.clone();
            let finished = block2::RcBlock::new(move || done());
            let _: () = msg_send![&*keep_store, removeDataOfTypes: &*keep_types, forDataRecords: &*chosen, completionHandler: &*finished];
        });
        let _: () = msg_send![&*store, fetchDataRecordsOfTypes: &*types, completionHandler: &*found];
    }
}

#[link(name = "CoreServices", kind = "framework")]
extern "C" {
    fn LSSetDefaultHandlerForURLScheme(scheme: *const NSString, handler: *const NSString) -> i32;
}

/// Bittet macOS, Zunder als Standardbrowser einzutragen (macOS fragt nach).
/// Geht nur, wenn Zunder als App-Bundle läuft. Rückgabe: ob angefragt wurde.
pub fn request_default_browser() -> bool {
    unsafe {
        let bundle: Retained<AnyObject> = msg_send![class!(NSBundle), mainBundle];
        let id: Option<Retained<NSString>> = msg_send![&*bundle, bundleIdentifier];
        let Some(id) = id else {
            return false;
        };
        for scheme in ["http", "https"] {
            let scheme = ns(scheme);
            LSSetDefaultHandlerForURLScheme(Retained::as_ptr(&scheme), Retained::as_ptr(&id));
        }
        true
    }
}

/// Schreibt ein Bild dessen, was eine WebView gerade zeigt, als PNG (Prüfhilfe).
pub fn snapshot(webview: &WebView, path: std::path::PathBuf) {
    let w = webview.webview();
    let done = block2::RcBlock::new(move |image: *mut AnyObject, _error: *mut AnyObject| {
        if image.is_null() {
            return;
        }
        unsafe {
            let tiff: Option<Retained<AnyObject>> = msg_send![image, TIFFRepresentation];
            let Some(tiff) = tiff else { return };
            let rep: Option<Retained<AnyObject>> =
                msg_send![class!(NSBitmapImageRep), imageRepWithData: &*tiff];
            let Some(rep) = rep else { return };
            let props: Retained<AnyObject> = msg_send![class!(NSDictionary), dictionary];
            // NSBitmapImageFileTypePNG = 4
            let png: Option<Retained<AnyObject>> =
                msg_send![&*rep, representationUsingType: 4usize, properties: &*props];
            if let Some(png) = png {
                let target = ns(&path.to_string_lossy());
                let _: bool = msg_send![&*png, writeToFile: &*target, atomically: true];
            }
        }
    });
    unsafe {
        let _: () = msg_send![&*w, takeSnapshotWithConfiguration: Option::<&AnyObject>::None, completionHandler: &*done];
    }
}

/// Macht von einer WebView ein verkleinertes Bild (JPEG) – für die Übersicht
/// aller Tabs. Das Bild bleibt im Arbeitsspeicher.
pub fn snapshot_jpeg(webview: &WebView, width: f64, done: impl Fn(Option<Vec<u8>>) + 'static) {
    let w = webview.webview();
    let block = block2::RcBlock::new(move |image: *mut AnyObject, _error: *mut AnyObject| {
        if image.is_null() {
            done(None);
            return;
        }
        let bytes = unsafe {
            let tiff: Option<Retained<AnyObject>> = msg_send![image, TIFFRepresentation];
            let rep: Option<Retained<AnyObject>> = match &tiff {
                Some(tiff) => msg_send![class!(NSBitmapImageRep), imageRepWithData: &**tiff],
                None => None,
            };
            rep.and_then(|rep| {
                let quality: Retained<AnyObject> = msg_send![class!(NSNumber), numberWithDouble: 0.72f64];
                let props: Retained<AnyObject> = msg_send![
                    class!(NSDictionary),
                    dictionaryWithObject: &*quality,
                    forKey: &*ns("NSImageCompressionFactor")
                ];
                // NSBitmapImageFileTypeJPEG = 3
                let data: Option<Retained<AnyObject>> =
                    msg_send![&*rep, representationUsingType: 3usize, properties: &*props];
                data.map(|data| {
                    let length: usize = msg_send![&*data, length];
                    let pointer: *const u8 = msg_send![&*data, bytes];
                    if pointer.is_null() || length == 0 || length > 4_000_000 {
                        Vec::new()
                    } else {
                        std::slice::from_raw_parts(pointer, length).to_vec()
                    }
                })
            })
        };
        done(bytes.filter(|bytes| !bytes.is_empty()));
    });
    unsafe {
        let config: Retained<AnyObject> = msg_send![class!(WKSnapshotConfiguration), new];
        let wide: Retained<AnyObject> = msg_send![class!(NSNumber), numberWithDouble: width];
        let _: () = msg_send![&*config, setSnapshotWidth: &*wide];
        let _: () = msg_send![&*w, takeSnapshotWithConfiguration: &*config, completionHandler: &*block];
    }
}

/// Zeichnet eine Ansicht sofort neu (statt beim nächsten Durchlauf).
pub fn force_display(ns_view: *mut std::ffi::c_void) {
    let view = ns_view as *mut AnyObject;
    if view.is_null() {
        return;
    }
    unsafe {
        let _: () = msg_send![view, setNeedsDisplay: true];
        let _: () = msg_send![view, displayIfNeeded];
    }
}

/// Abstand des roten Fensterknopfs von der Oberkante des Fensters.
pub fn traffic_light_top(webview: &WebView) -> Option<f64> {
    let window = webview.ns_window();
    unsafe {
        let button: *mut AnyObject = msg_send![&*window, standardWindowButton: 0isize];
        if button.is_null() {
            return None;
        }
        let frame: NSRect = msg_send![&*window, frame];
        let bounds: NSRect = msg_send![button, bounds];
        let rect: NSRect = msg_send![button, convertRect: bounds, toView: Option::<&AnyObject>::None];
        Some(frame.size.height - rect.origin.y - rect.size.height)
    }
}

/// Prüfhilfe: Lage der drei Fensterknöpfe (Abstand von links und von oben).
pub fn traffic_lights(webview: &WebView) -> String {
    let window = webview.ns_window();
    let mut parts = Vec::new();
    unsafe {
        let frame: NSRect = msg_send![&*window, frame];
        for kind in 0..3isize {
            let button: *mut AnyObject = msg_send![&*window, standardWindowButton: kind];
            if button.is_null() {
                continue;
            }
            let bounds: NSRect = msg_send![button, bounds];
            let rect: NSRect = msg_send![button, convertRect: bounds, toView: Option::<&AnyObject>::None];
            let hidden: bool = msg_send![button, isHidden];
            parts.push(format!(
                "x={:.0} oben={:.0} {:.0}x{:.0}{}",
                rect.origin.x,
                frame.size.height - rect.origin.y - rect.size.height,
                rect.size.width,
                rect.size.height,
                if hidden { " versteckt" } else { "" }
            ));
        }
    }
    parts.join(" | ")
}

thread_local! {
    /// Prüfhilfe: Zähler, mit dem sich schon geplante Eingriffe in Dialoge
    /// verwerfen lassen (siehe `forget_modal_plans`).
    static MODAL_PLANS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Prüfhilfe: verwirft alles, was für einen Dialog geplant, aber noch nicht
/// geschehen ist. Solche Eingriffe warten sonst auf den nächsten Dialog – und
/// träfen dann den falschen.
pub fn forget_modal_plans() {
    MODAL_PLANS.with(|plans| plans.set(plans.get() + 1));
}

/// Prüfhilfe: führt etwas aus, sobald ein Dialog seit `seconds` offen sein kann.
unsafe fn in_modal_after(seconds: f64, action: impl Fn() + 'static) {
    let planned = MODAL_PLANS.with(std::cell::Cell::get);
    let run = block2::RcBlock::new(move |_timer: Obj| {
        if MODAL_PLANS.with(std::cell::Cell::get) == planned {
            action();
        }
    });
    let timer: Option<Retained<AnyObject>> =
        msg_send![class!(NSTimer), timerWithTimeInterval: seconds, repeats: false, block: &*run];
    if let Some(timer) = timer {
        let run_loop: Retained<AnyObject> = msg_send![class!(NSRunLoop), currentRunLoop];
        let _: () = msg_send![&*run_loop, addTimer: &*timer, forMode: &*ns("NSModalPanelRunLoopMode")];
    }
}

/// Prüfhilfe: bricht einen modalen Dialog nach der angegebenen Zeit ab.
pub fn abort_modal_after(seconds: f64) {
    unsafe {
        in_modal_after(seconds, || {
            let app: Retained<AnyObject> = msg_send![class!(NSApplication), sharedApplication];
            let _: () = msg_send![&*app, abortModal];
        });
    }
}

/// Prüfhilfe: Die Seite gilt auch dann als sichtbar, wenn das Probefenster
/// verdeckt ist. Sonst wartet WebKit mit manchem – etwa mit der Frage nach der
/// Kamera –, bis jemand das Fenster nach vorn holt.
pub fn stay_visible(webview: &WebView) {
    if crate::dev::var_os("ZUNDER_ALWAYS_VISIBLE").is_none() {
        return;
    }
    let w = webview.webview();
    unsafe {
        let known: bool = msg_send![&*w, respondsToSelector: sel!(_setWindowOcclusionDetectionEnabled:)];
        if known {
            let _: () = msg_send![&*w, _setWindowOcclusionDetectionEnabled: false];
        }
    }
}

/// Prüfhilfe: drückt nach der angegebenen Zeit einen Knopf des offenen Dialogs
/// – den, dessen Aufschrift den Text enthält. Ein gesperrter Knopf bleibt es.
pub fn click_modal_after(seconds: f64, title: &str) {
    use std::io::Write;
    let wanted = title.to_string();
    unsafe {
        in_modal_after(seconds, move || {
            let app: Retained<AnyObject> = msg_send![class!(NSApplication), sharedApplication];
            let window: Obj = msg_send![&*app, modalWindow];
            let content: Obj = if window.is_null() { std::ptr::null_mut() } else { msg_send![window, contentView] };
            match find_button(content, &wanted, 0) {
                Some(button) => {
                    let enabled: bool = msg_send![button, isEnabled];
                    println!("MODAL {wanted}: {}", if enabled { "gedrückt" } else { "gesperrt" });
                    let _ = std::io::stdout().flush();
                    if enabled {
                        let _: () = msg_send![button, performClick: Option::<&AnyObject>::None];
                    }
                }
                None => {
                    println!("MODAL {wanted}: {}", if window.is_null() { "kein Dialog" } else { "kein solcher Knopf" });
                    let _ = std::io::stdout().flush();
                }
            }
        });
    }
}

/// Sucht in einer Ansicht den Knopf, dessen Aufschrift den Text enthält.
unsafe fn find_button(view: Obj, title: &str, depth: usize) -> Option<Obj> {
    if view.is_null() || depth > 12 {
        return None;
    }
    let is_button: bool = msg_send![view, isKindOfClass: class!(NSButton)];
    if is_button {
        let label: Obj = msg_send![view, title];
        if string_of(label).contains(title) {
            return Some(view);
        }
    }
    let subviews: Obj = msg_send![view, subviews];
    if subviews.is_null() {
        return None;
    }
    let count: usize = msg_send![subviews, count];
    for index in 0..count.min(200) {
        let child: Obj = msg_send![subviews, objectAtIndex: index];
        if let Some(found) = find_button(child, title, depth + 1) {
            return Some(found);
        }
    }
    None
}

/// Prüfhilfe: erzwingt helles oder dunkles Erscheinungsbild der App.
pub fn force_appearance(dark: bool) {
    unsafe {
        let name = ns(if dark { "NSAppearanceNameDarkAqua" } else { "NSAppearanceNameAqua" });
        let appearance: Option<Retained<AnyObject>> =
            msg_send![class!(NSAppearance), appearanceNamed: &*name];
        let app: Retained<AnyObject> = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![&*app, setAppearance: appearance.as_deref()];
    }
}

/// Lädt eine Datei von der Platte. Die Seite darf dabei den Ordner lesen, in
/// dem sie liegt – sonst fehlten ihr Bilder und Stile daneben.
pub fn load_file(webview: &WebView, file_url: &str) -> bool {
    let w = webview.webview();
    unsafe {
        let url: Option<Retained<AnyObject>> = msg_send![class!(NSURL), URLWithString: &*ns(file_url)];
        let Some(url) = url else {
            return false;
        };
        let folder: Option<Retained<AnyObject>> = msg_send![&*url, URLByDeletingLastPathComponent];
        let Some(folder) = folder else {
            return false;
        };
        let _: Option<Retained<AnyObject>> =
            msg_send![&*w, loadFileURL: &*url, allowingReadAccessToURL: &*folder];
        true
    }
}

/// Öffnet eine Adresse oder Datei mit dem dafür zuständigen Programm – über
/// den Arbeitsplatz von macOS, ohne ein Hilfsprogramm zu starten (das geht
/// auch aus der Sandbox heraus).
pub fn open_external(target: &str) {
    unsafe {
        let url: Option<Retained<AnyObject>> = if target.starts_with('/') {
            let file: Retained<AnyObject> = msg_send![class!(NSURL), fileURLWithPath: &*ns(target)];
            Some(file)
        } else {
            msg_send![class!(NSURL), URLWithString: &*ns(target)]
        };
        let Some(url) = url else {
            return;
        };
        let workspace: Retained<AnyObject> = msg_send![class!(NSWorkspace), sharedWorkspace];
        let _: bool = msg_send![&*workspace, openURL: &*url];
    }
}

/// Zeigt eine Datei im Finder.
pub fn reveal_in_finder(path: &std::path::Path) {
    unsafe {
        let file: Retained<AnyObject> = msg_send![class!(NSURL), fileURLWithPath: &*ns(&path.to_string_lossy())];
        let list: Retained<AnyObject> = msg_send![class!(NSArray), arrayWithObject: &*file];
        let workspace: Retained<AnyObject> = msg_send![class!(NSWorkspace), sharedWorkspace];
        let _: () = msg_send![&*workspace, activateFileViewerSelectingURLs: &*list];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anmeldung_einer_internen_seite_gilt_einer_webview_und_einmal() {
        let now = Instant::now();
        let app = "zunder://app-time/".to_string();
        let mut list = Vec::new();
        expect_in(&mut list, 1, Some(app.clone()), now);
        // Eine andere WebView – etwa der Tab einer Webseite, die es im selben
        // Augenblick mit derselben Adresse versucht – löst sie nicht ein.
        assert!(!expected_in(&mut list, 2, &app, now));
        assert!(!expected_in(&mut list, 1, "zunder://app-weather/", now));
        assert!(expected_in(&mut list, 1, &app, now));
        // Einmal: Danach ist sie verbraucht.
        assert!(!expected_in(&mut list, 1, &app, now));
        assert!(list.is_empty());
        // Eine neue Anmeldung löst die ältere derselben WebView ab; die einer
        // anderen bleibt stehen.
        expect_in(&mut list, 1, Some(app.clone()), now);
        expect_in(&mut list, 2, Some(app.clone()), now);
        expect_in(&mut list, 1, Some("zunder://newtab/".to_string()), now);
        assert_eq!(list.len(), 2);
        assert!(!expected_in(&mut list, 1, &app, now));
        assert!(expected_in(&mut list, 1, "zunder://newtab/", now));
        assert!(expected_in(&mut list, 2, &app, now));
        // Nach Ablauf der Frist gilt sie nicht mehr.
        expect_in(&mut list, 3, Some(app.clone()), now);
        assert!(!expected_in(&mut list, 3, &app, now + EXPECT_WINDOW));
        assert!(list.is_empty());
        // Schickt der Browser die WebView danach ins Netz, ist die Anmeldung überholt.
        expect_in(&mut list, 4, Some(app.clone()), now);
        expect_in(&mut list, 4, None, now);
        assert!(!expected_in(&mut list, 4, &app, now));
        // Nur eigene Adressen lassen sich anmelden.
        assert_eq!(internal_address("zunder://app-time/").as_deref(), Some("zunder://app-time/"));
        assert_eq!(internal_address("https://example.com/"), None);
        assert_eq!(internal_address("kein url"), None);
    }

    #[test]
    fn angekuendigte_dateinamen_kommen_der_reihe_nach_und_verfallen() {
        let now = Instant::now();
        let mut list = Vec::new();
        name_in(&mut list, 1, "zeiten.csv".into(), now);
        name_in(&mut list, 1, "zeiten-2026-11.csv".into(), now);
        name_in(&mut list, 2, "andere.csv".into(), now);
        // Je WebView in der Reihenfolge der Ankündigung, jede einmal.
        assert_eq!(name_out(&mut list, 1, now).as_deref(), Some("zeiten.csv"));
        assert_eq!(name_out(&mut list, 2, now).as_deref(), Some("andere.csv"));
        assert_eq!(name_out(&mut list, 1, now).as_deref(), Some("zeiten-2026-11.csv"));
        assert_eq!(name_out(&mut list, 1, now), None);
        // Mehr als vier warten nicht: Die älteste weicht.
        for index in 0..6 {
            name_in(&mut list, 7, format!("datei-{index}.csv"), now);
        }
        assert_eq!(list.len(), 4);
        assert_eq!(name_out(&mut list, 7, now).as_deref(), Some("datei-2.csv"));
        // Nach der Frist gilt keine mehr.
        assert_eq!(name_out(&mut list, 7, now + FILE_NAME_WINDOW), None);
        assert!(list.is_empty());
    }
}
