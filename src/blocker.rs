//! Werbe- und Tracker-Blocker – und der Schutz des eigenen Netzes.
//!
//! Gesperrt wird in drei Schichten, alle im Regelformat von WebKit. WebKit
//! prüft Anfragen dann selbst – ohne Umweg über die App und ohne dass der
//! Blocker die Seiten mitlesen könnte.
//!
//! 1. Die Basisliste (`DOMAINS`): die bekanntesten Werbe- und Tracking-Dienste.
//!    Sie steckt im Programm, ist in Millisekunden übersetzt und gilt deshalb
//!    vom ersten Abruf an.
//! 2. Die großen Filterlisten (EasyList, EasyList Germany, EasyPrivacy – rund
//!    140.000 Regeln): Sie liegen der App bei, werden alle paar Tage
//!    aufgefrischt und von `filters.rs` übersetzt. Das Ergebnis hebt WebKit
//!    auf; übersetzt wird nur neu, wenn sich die Listen ändern. Bis die erste
//!    Übersetzung steht (wenige Sekunden nach dem allerersten Start), gilt
//!    die Basisliste allein.
//! 3. Die Liste „eigenes Netz“ schirmt das eigene Netz ab: Eine Website aus dem
//!    Internet darf im Hintergrund weder den Router noch den Drucker noch
//!    einen Dienst auf diesem Mac ansprechen (Bilder, Skripte, fetch,
//!    WebSockets, eingebettete Rahmen). Seiten, die selbst im eigenen Netz
//!    liegen, dürfen das weiterhin – und einzelne Websites, denen man es
//!    ausdrücklich erlaubt.
//!
//! Die Schichten 1 und 2 lassen sich je Website abschalten (`set_site_off`):
//! Dann hängen sie am Tab nicht, solange er diese Website zeigt. Schicht 3
//! bleibt davon unberührt.
//!
//! Grenze: Geprüft wird die Adresse, nicht, wohin der Name am Ende zeigt. Ein
//! öffentlicher Name, der auf ein Gerät im eigenen Netz umgebogen wurde, fällt
//! hier nicht auf.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_foundation::NSString;
use serde::{Deserialize, Serialize};
use tao::event_loop::EventLoopProxy;
use wry::{WebView, WebViewExtMacOS};

use crate::app::UserEvent;
use crate::filters;

/// Kennungen der Werbelisten beginnen so – daran erkennt `native::rule_list_action`
/// einen gesperrten Abruf für den Zähler.
pub const BASIS_PREFIX: &str = "zunder-basis";
pub const FILTER_PREFIX: &str = "zunder-filter";

/// Kennung der Listen gemeldeter Betrugs- und Schadseiten.
pub const GUARD_PREFIX: &str = "zunder-schutz";

/// Ein Satz großer Listen: woher er kommt und wie er behandelt wird.
struct Kind {
    /// Kennung der übersetzten Listen (`<prefix>-<n>`).
    prefix: &'static str,
    sources: &'static [filters::Source],
    /// Name der Vermerk-Datei im Profil.
    state_file: &'static str,
    /// Liegen die Listen der App bei – oder kommen sie nur aus dem Netz?
    bundled: bool,
    /// So lange gilt ein Stand, bis online nach einem neueren gesehen wird.
    refresh_after: Duration,
    /// Listen gemeldeter Betrugs- und Schadseiten (andere Lesart, siehe `filters`).
    guard: bool,
}

/// Satz 0: Werbung und Tracker (die Listen nennen vier Tage). Satz 1:
/// gemeldete Betrugs- und Schadseiten – die veralten schnell, also zweimal täglich.
static KINDS: [Kind; 2] = [
    Kind {
        prefix: FILTER_PREFIX,
        sources: filters::SOURCES,
        state_file: "state.json",
        bundled: true,
        refresh_after: Duration::from_secs(4 * 24 * 60 * 60),
        guard: false,
    },
    Kind {
        prefix: GUARD_PREFIX,
        sources: filters::GUARD_SOURCES,
        state_file: "guard.json",
        bundled: false,
        refresh_after: Duration::from_secs(12 * 60 * 60),
        guard: true,
    },
];

/// Bekannte Werbe- und Tracking-Dienste. Gesperrt wird nur, wenn sie von einer
/// FREMDEN Seite eingebunden werden – der direkte Besuch bleibt möglich.
/// Bewusst nicht dabei: Tag-Manager und Anmelde-Widgets, deren Sperre Seiten
/// kaputt machen kann.
const DOMAINS: &[&str] = &[
    // Google-Werbung und -Messung
    "doubleclick.net", "googlesyndication.com", "googleadservices.com", "google-analytics.com",
    "googletagservices.com", "2mdn.net", "admob.com", "adservice.google.com", "adservice.google.de",
    // Große Werbenetze und Börsen
    "amazon-adsystem.com", "adnxs.com", "adsrvr.org", "criteo.com", "criteo.net", "taboola.com",
    "outbrain.com", "rubiconproject.com", "pubmatic.com", "openx.net", "casalemedia.com",
    "indexww.com", "adform.net", "smartadserver.com", "teads.tv", "bidswitch.net", "3lift.com",
    "sharethrough.com", "media.net", "zedo.com", "advertising.com", "adcolony.com", "applovin.com",
    "serving-sys.com", "flashtalking.com", "mathtag.com", "turn.com", "contextweb.com",
    "sovrn.com", "lijit.com", "gumgum.com", "spotxchange.com", "spotx.tv", "fwmrm.net",
    "springserve.com", "simpli.fi", "adroll.com", "perfectaudience.com", "stackadapt.com",
    "mgid.com", "revcontent.com", "zemanta.com", "popads.net", "propellerads.com", "exoclick.com",
    "juicyads.com", "adsterra.com", "yieldmo.com", "undertone.com", "districtm.io", "emxdgt.com",
    "rhythmone.com", "brealtime.com", "adtechus.com", "adhigh.net", "smaato.net", "inmobi.com",
    "unityads.unity3d.com", "moatads.com", "adsafeprotected.com", "doubleverify.com",
    // Datenhändler und Abgleich von Kennungen
    "scorecardresearch.com", "quantserve.com", "quantcount.com", "krxd.net", "bluekai.com",
    "exelator.com", "demdex.net", "everesttech.net", "omtrdc.net", "2o7.net", "rlcdn.com",
    "tapad.com", "agkn.com", "crwdcntrl.net", "liadm.com", "id5-sync.com", "eyeota.net",
    "imrworldwide.com", "adsymptotic.com", "bounceexchange.com", "zeotap.com", "permutive.com",
    // Verhaltensaufzeichnung und Auswertung
    "hotjar.com", "mouseflow.com", "fullstory.com", "crazyegg.com", "mixpanel.com", "amplitude.com",
    "chartbeat.com", "chartbeat.net", "statcounter.com", "histats.com", "clarity.ms",
    "nr-data.net", "luckyorange.com", "inspectlet.com", "smartlook.com", "heapanalytics.com",
    "kissmetrics.com", "woopra.com", "parsely.com", "segment.io",
    // Zählpixel sozialer Netze
    "ads-twitter.com", "analytics.twitter.com", "bat.bing.com", "ads.linkedin.com",
    "px.ads.linkedin.com", "analytics.tiktok.com", "ads.pinterest.com", "ct.pinterest.com",
    "tr.snapchat.com", "sc-static.net", "ads.yahoo.com", "analytics.yahoo.com",
    "mc.yandex.ru", "an.yandex.ru",
    // Im deutschsprachigen Raum verbreitet
    "ioam.de", "ivwbox.de", "meetrics.net", "emetriq.de", "adition.com", "yieldlab.net",
    "yieldlove.com", "smartclip.net", "plista.com", "ligatus.com", "adup-tech.com", "theadex.com",
    "nuggad.net", "xplosion.de", "econda-monitor.de", "etracker.com", "etracker.de",
    "tisoomi-services.com", "adspirit.de", "adscale.de", "mairdumont.com", "intellitxt.com",
    "webtrekk.net", "wt-eu02.net", "wt-safetag.com", "piwik.pro", "trbo.com", "orbidder.otto.de",
];

/// Zusätzliche Muster für einzelne Adressen statt ganzer Dienste.
const PATTERNS: &[&str] = &[
    // Facebook-Pixel (das Anmelde-Widget von Facebook bleibt nutzbar)
    r"^[^:]+://+connect\.facebook\.net/.*/fbevents\.js",
    r"^[^:]+://+www\.facebook\.com/tr[/?]",
];

/// Kennung der Liste „eigenes Netz“ – beginnt immer so, damit sich Meldungen
/// von WebKit zuordnen lassen (siehe `native::rule_list_action`).
pub const LOCAL_PREFIX: &str = "zunder-netz";

/// Vor jedem Rechnernamen: Schema, `://` und gegebenenfalls Zugangsdaten.
/// Gemeint sind nur Abrufe übers Netz – http, https, ws, wss –, nicht Zunders
/// eigene Seiten oder Dateien (ein „oder“ kennt die Regelsprache nicht; die
/// vier beginnen mit h oder w).
const HOST_START: &str = r"^[hw][a-z]+://+([^/@]*@)?";

/// Rechner im eigenen Netz, als Muster für den Rechnernamen. WebKits
/// Regelsprache kennt kein „oder“ – deshalb ein Muster je Fall.
const LOCAL_HOSTS: &[&str] = &[
    // dieser Mac
    r"localhost",
    r"[^/:]*\.localhost",
    r"127\.[0-9]+\.[0-9]+\.[0-9]+",
    r"0\.0\.0\.0",
    r"\[::1\]",
    r"\[::\]",
    r"\[::ffff:[^/]*\]",
    // private Netze, verbindungslokale Adressen, Netze hinter Anbieter-NAT
    r"10\.[0-9]+\.[0-9]+\.[0-9]+",
    r"192\.168\.[0-9]+\.[0-9]+",
    r"172\.1[6-9]\.[0-9]+\.[0-9]+",
    r"172\.2[0-9]\.[0-9]+\.[0-9]+",
    r"172\.3[01]\.[0-9]+\.[0-9]+",
    r"169\.254\.[0-9]+\.[0-9]+",
    r"100\.6[4-9]\.[0-9]+\.[0-9]+",
    r"100\.[7-9][0-9]\.[0-9]+\.[0-9]+",
    r"100\.1[01][0-9]\.[0-9]+\.[0-9]+",
    r"100\.12[0-7]\.[0-9]+\.[0-9]+",
    r"\[f[cd][0-9a-f]*:[^/]*\]",
    r"\[fe[89ab][0-9a-f]:[^/]*\]",
    // Namen, die es nur im eigenen Netz gibt
    r"[^/.:@\[]+",
    r"[^/:]*\.local",
    r"[^/:]*\.lan",
    r"[^/:]*\.internal",
    r"[^/:]*\.intranet",
    r"[^/:]*\.home\.arpa",
    r"fritz\.box",
    r"[^/:]*\.fritz\.box",
    r"speedport\.ip",
    r"easy\.box",
    r"kabel\.box",
    r"o2\.box",
    // Öffentliche Namen, die absichtlich auf Adressen im eigenen Netz zeigen
    r"[^/:]*\.nip\.io",
    r"[^/:]*\.sslip\.io",
    r"[^/:]*\.xip\.io",
    r"localtest\.me",
    r"[^/:]*\.localtest\.me",
    r"lvh\.me",
    r"[^/:]*\.lvh\.me",
    r"vcap\.me",
    r"[^/:]*\.vcap\.me",
];

/// Die Regeln „eigenes Netz“. `allowed` sind Websites, die dorthin dürfen.
fn local_rules_json(allowed: &[String]) -> String {
    let host_patterns: Vec<String> = LOCAL_HOSTS
        .iter()
        // Mit dem Punkt, der hinter jedem Rechnernamen stehen darf: `localhost.` ist `localhost`.
        .map(|host| format!("{HOST_START}{host}\\.?[:/]"))
        .collect();
    // Wer darf: Seiten, die selbst im eigenen Netz liegen, Dateien, interne
    // Seiten – und die ausdrücklich erlaubten Websites.
    let mut exempt = host_patterns.clone();
    exempt.push("^file:".to_string());
    exempt.push("^zunder:".to_string());
    for site in allowed {
        let plausible = !site.is_empty() && site.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'));
        if plausible {
            exempt.push(format!("{HOST_START}([^/:]*\\.)?{}\\.?[:/]", site.replace('.', "\\.")));
        }
    }
    let rules: Vec<serde_json::Value> = host_patterns
        .iter()
        .map(|pattern| {
            serde_json::json!({
                "trigger": { "url-filter": pattern, "unless-top-url": exempt },
                "action": { "type": "block" },
            })
        })
        .collect();
    serde_json::Value::Array(rules).to_string()
}

fn rules_json() -> String {
    let mut rules: Vec<serde_json::Value> = Vec::with_capacity(DOMAINS.len() + PATTERNS.len());
    for domain in DOMAINS {
        let escaped = domain.replace('.', "\\.");
        rules.push(serde_json::json!({
            "trigger": {
                "url-filter": format!("^[^:]+://+([^:/]+\\.)?{escaped}\\.?[:/]"),
                "load-type": ["third-party"],
            },
            "action": { "type": "block" },
        }));
    }
    for pattern in PATTERNS {
        rules.push(serde_json::json!({
            "trigger": { "url-filter": pattern, "load-type": ["third-party"] },
            "action": { "type": "block" },
        }));
    }
    serde_json::Value::Array(rules).to_string()
}

type Slot = Rc<RefCell<Option<Retained<AnyObject>>>>;

// ---------------------------------------------------------------------------
// Was die Navigationsentscheidung über die Werbelisten wissen muss

/// Eine Gruppe von Regellisten, die gemeinsam an einem Tab hängt oder nicht.
#[derive(Default)]
struct Layer {
    lists: Vec<Retained<AnyObject>>,
    /// Abgelöste Listen, deren Platz nicht neu besetzt wurde.
    retired: Vec<Retained<AnyObject>>,
    /// Zählt hoch, wenn sich `lists` ändert.
    generation: u64,
}

impl Layer {
    /// Setzt die geltenden Listen. Ohne Änderung passiert nichts.
    fn replace(&mut self, current: Vec<Retained<AnyObject>>) {
        let same = self.lists.len() == current.len()
            && self.lists.iter().zip(&current).all(|(a, b)| Retained::as_ptr(a) == Retained::as_ptr(b));
        if same {
            return;
        }
        let name = |list: &Retained<AnyObject>| -> String {
            let identifier: Option<Retained<NSString>> = unsafe { msg_send![&**list, identifier] };
            identifier.map(|identifier| identifier.to_string()).unwrap_or_default()
        };
        let names: HashSet<String> = current.iter().map(name).collect();
        // Was keinen Nachfolger gleichen Namens hat, muss ausdrücklich aus den Tabs.
        let old = std::mem::replace(&mut self.lists, current);
        self.retired = old.into_iter().filter(|list| !names.contains(&name(list))).collect();
        self.generation += 1;
    }

    /// Hängt die Listen an oder nimmt sie ab. Eine neu übersetzte Liste
    /// ersetzt dabei die alte gleichen Namens.
    unsafe fn apply(&self, controller: *mut AnyObject, want: bool) {
        for list in &self.retired {
            let _: () = msg_send![controller, removeContentRuleList: &**list];
        }
        for list in &self.lists {
            if want {
                let _: () = msg_send![controller, addContentRuleList: &**list];
            } else {
                let _: () = msg_send![controller, removeContentRuleList: &**list];
            }
        }
    }
}

#[derive(Default)]
struct Ads {
    enabled: bool,
    /// Basisliste und große Filterlisten: Werbung und Tracker.
    layer: Layer,
    /// Websites, auf denen nicht gesperrt wird.
    off: HashSet<String>,
    /// Gemeldete Betrugs- und Schadseiten – unabhängig vom Werbeblocker.
    guard: Layer,
    guard_enabled: bool,
    /// Einträge der Listen, die jemand in dieser Sitzung ausdrücklich übergangen
    /// hat (`h:<rechner>` oder `p:<rechner><pfad>`, siehe `GuardIndex::matched`).
    guard_off: HashSet<String>,
    /// Verzeichnis der gemeldeten Adressen – für die Warnung, bevor ein Tab
    /// eine solche Seite überhaupt öffnet (`guard_hit`).
    guard_index: Option<filters::GuardIndex>,
    /// Tabs von Stealth-Fenstern: Dort wird gesperrt und das eigene Netz
    /// abgeschirmt, auch wenn beides sonst ausgeschaltet ist.
    forced: HashSet<usize>,
    /// Die Liste „eigenes Netz“ – sie hängt an jedem Tab, gleich welche Website.
    local: Option<Retained<AnyObject>>,
    local_enabled: bool,
    /// Je Tab: was dort gerade gilt.
    tabs: HashMap<usize, TabAds>,
    /// Gesperrte Abrufe je Tab seit dem letzten Seitenwechsel.
    blocked: HashMap<usize, u32>,
    /// Der Zähler hat sich geändert, die Oberfläche weiß es noch nicht.
    blocked_pending: bool,
}

#[derive(Default)]
struct TabAds {
    /// Hängen die Listen dran – und in welchem Stand? `None`: noch nie abgeglichen.
    attached: Option<(bool, u64)>,
    /// Dasselbe für die Listen gemeldeter Betrugs- und Schadseiten.
    guarded: Option<(bool, u64)>,
    /// Die Seite, zu der der Tab zuletzt aufgebrochen ist (ohne `#abschnitt`).
    /// Daran zeigt sich, ob ein Abruf eine neue Seite ist oder nur ein Sprung
    /// innerhalb derselben – WebKit selbst gibt darüber keine verlässliche Auskunft.
    page: String,
    /// Die Seite, die der Tab wirklich zeigt (zuletzt übernommen). Nach ihr
    /// richten sich die Listen – nicht nach einer Adresse, zu der der Tab nur
    /// aufgebrochen ist und nie ankam.
    shown: String,
}

fn page_of(url: &str) -> &str {
    url.split('#').next().unwrap_or("")
}

thread_local! {
    static ADS: RefCell<Ads> = RefCell::new(Ads::default());
}

/// Bringt eine Schicht von Listen in den gewünschten Zustand. Ohne
/// `may_detach` werden Listen nur angehängt, nie abgenommen: Dann bleibt, was
/// hängt, bis die Seite wirklich da ist. Zurück kommt, was jetzt gilt.
unsafe fn settle_layer(
    layer: &Layer,
    controller: *mut AnyObject,
    current: Option<(bool, u64)>,
    want: bool,
    may_detach: bool,
) -> Option<(bool, u64)> {
    let target = match current {
        _ if want => true,
        None => false,
        Some(_) if may_detach => false,
        Some((attached, _)) => attached,
    };
    let state = (target, layer.generation);
    if current != Some(state) {
        layer.apply(controller, target);
    }
    Some(state)
}

/// Hängt die Werbelisten an einen Tab oder nimmt sie ab – je nachdem, welche
/// Website er lädt. Ohne Änderung passiert nichts.
///
/// `may_detach`: Die Listen dürfen auch abgenommen werden. Das gilt, wenn die
/// Seite da ist oder der Mensch am Browser dorthin wollte – nicht, wenn eine
/// Seite den Tab per Skript woandershin schickt. Sonst würde sie die Listen
/// los, indem sie zu einer Website „aufbricht“, für die der Blocker aus ist,
/// und nie dort ankommt.
unsafe fn sync_tab(ads: &mut Ads, key: usize, controller: *mut AnyObject, url: &str, may_detach: bool) {
    if controller.is_null() {
        return;
    }
    // Zunders eigene Seiten (Start-, Warn- und Fehlerseite) gehen die Listen
    // nichts an: Eine Regel aus einer Liste soll dort weder etwas sperren noch
    // etwas ausblenden – schon gar nicht den Text einer Warnung.
    let internal = url.starts_with("zunder:");
    let forced = ads.forced.contains(&key);
    let off = crate::sites::domain_of(url).is_some_and(|site| ads.off.contains(&site));
    let want = (ads.enabled || forced) && !off && !internal;
    let current = ads.tabs.get(&key).and_then(|tab| tab.attached);
    let state = settle_layer(&ads.layer, controller, current, want, may_detach);
    ads.tabs.entry(key).or_default().attached = state;
    // Gemeldete Betrugs- und Schadseiten: Diese Listen hängen auch dort, wo
    // der Werbeblocker für die Website abgeschaltet ist. Ausgenommen ist nur
    // eine Seite, deren Eintrag jemand in dieser Sitzung ausdrücklich übergangen
    // hat – WebKit sperrte sie sonst trotzdem. Auf den übrigen Seiten desselben
    // Rechners hängen die Listen weiter.
    let exempt = ads
        .guard_index
        .as_ref()
        .and_then(|index| index.matched(url))
        .is_some_and(|entry| ads.guard_off.contains(&entry));
    let want = (ads.guard_enabled || forced) && !exempt && !internal;
    let current = ads.tabs.get(&key).and_then(|tab| tab.guarded);
    let state = settle_layer(&ads.guard, controller, current, want, may_detach);
    ads.tabs.entry(key).or_default().guarded = state;
}

unsafe fn controller_of(webview: *mut AnyObject) -> *mut AnyObject {
    let config: *mut AnyObject = msg_send![webview, configuration];
    if config.is_null() {
        return std::ptr::null_mut();
    }
    msg_send![config, userContentController]
}

/// Der Tab hat eine Seite übernommen: Jetzt – und erst jetzt – gelten die
/// Listen genau so, wie es zu dieser Seite gehört.
pub fn committed(webview: &WebView, url: &str) {
    let raw = Retained::as_ptr(&webview.webview()) as *mut AnyObject;
    ADS.with(|ads| {
        let mut ads = ads.borrow_mut();
        let key = raw as usize;
        let Some(tab) = ads.tabs.get_mut(&key) else {
            return;
        };
        tab.shown = url.to_string();
        tab.page = page_of(url).to_string();
        unsafe {
            let controller = controller_of(raw);
            sync_tab(&mut ads, key, controller, url, true);
        }
    });
}

/// Der Tab ist zu einer Adresse aufgebrochen und nicht angekommen (abgebrochen,
/// Download, Antwort ohne Inhalt): Die Listen gelten wieder für die Seite, die
/// er zeigt.
///
/// # Safety
/// `webview` muss eine lebende WKWebView sein.
pub unsafe fn revert(webview: *mut AnyObject) {
    ADS.with(|ads| {
        let mut ads = ads.borrow_mut();
        let key = webview as usize;
        let Some(tab) = ads.tabs.get_mut(&key) else {
            return;
        };
        let shown = tab.shown.clone();
        tab.page = page_of(&shown).to_string();
        let controller = controller_of(webview);
        sync_tab(&mut ads, key, controller, &shown, true);
    });
}

/// Will dieser Tab zu einer gemeldeten Betrugs- oder Schadseite? (Aus
/// `native::nav_policy`, bevor irgendetwas geladen wird.)
pub fn guard_hit(key: usize, url: &str) -> bool {
    ADS.with(|ads| {
        let ads = ads.borrow();
        if !(ads.guard_enabled || ads.forced.contains(&key)) {
            return false;
        }
        let Some(index) = ads.guard_index.as_ref() else {
            return false;
        };
        index.matched(url).is_some_and(|entry| !ads.guard_off.contains(&entry))
    })
}

/// Diese Adresse in dieser Sitzung trotz Warnung öffnen (von der
/// Rückfrage-Seite). Übergangen wird genau der Eintrag der Listen, der sie
/// trifft: Ist der ganze Rechner gemeldet, gilt es für ihn – ist nur eine
/// einzelne Adresse gemeldet (ein Formular auf einem großen Dienst), dann nur
/// für sie. Vor allem anderen, was über denselben Rechner gemeldet ist, warnt
/// Zunder weiter.
pub fn guard_allow(url: &str) {
    if url.len() > 4096 {
        return;
    }
    ADS.with(|ads| {
        let mut ads = ads.borrow_mut();
        let Some(entry) = ads.guard_index.as_ref().and_then(|index| index.matched(url)) else {
            return;
        };
        if ads.guard_off.len() < 500 {
            ads.guard_off.insert(entry);
        }
    });
}

/// Ein Tab bricht zu einer Adresse auf (aus `native::nav_policy`, bevor der
/// Abruf losgeht): Die Listen folgen der Website. Bei einer neuen Seite – oder
/// beim Neuladen (`reload`) – beginnt auch der Zähler neu; ein Sprung innerhalb
/// derselben Seite lässt ihn stehen.
///
/// `by_user`: Der Mensch am Browser hat das angestoßen (Eingabe, Klick,
/// Verlauf). Nur dann werden Listen schon jetzt abgenommen, siehe `sync_tab`.
///
/// # Safety
/// `webview` muss eine lebende WKWebView sein.
pub unsafe fn site_changed(webview: *mut AnyObject, url: &str, reload: bool, by_user: bool) {
    ADS.with(|ads| {
        let mut ads = ads.borrow_mut();
        let key = webview as usize;
        // Nur Tabs – nie die Oberfläche des Browsers selbst.
        let Some(tab) = ads.tabs.get_mut(&key) else {
            return;
        };
        let fresh = reload || page_of(&tab.page) != page_of(url);
        tab.page = page_of(url).to_string();
        if fresh && ads.blocked.remove(&key).is_some() {
            ads.blocked_pending = true;
        }
        let controller = controller_of(webview);
        sync_tab(&mut ads, key, controller, url, by_user);
    });
}

/// Rüstet einen Tab mit den Listen aus, wie sie gerade gelten: „eigenes Netz“
/// immer, die Werbelisten je nach Website. Eine neu übersetzte Liste ersetzt
/// dabei die alte gleichen Namens.
///
/// `fresh`: Der Tab ist gerade entstanden und lädt `url`. Sonst zählt die
/// Seite, zu der er zuletzt aufgebrochen ist – `url` nur, wenn keine bekannt ist.
pub fn equip(webview: &WebView, url: &str, fresh: bool) {
    let controller = webview.manager();
    ADS.with(|ads| {
        let mut ads = ads.borrow_mut();
        let key = crate::native::key(webview);
        if let Some(list) = ads.local.clone() {
            unsafe {
                if ads.local_enabled || ads.forced.contains(&key) {
                    let _: () = msg_send![&*controller, addContentRuleList: &*list];
                } else {
                    let _: () = msg_send![&*controller, removeContentRuleList: &*list];
                }
            }
        }
        let (known, shown) = match (fresh, ads.tabs.get(&key)) {
            // Eine neue WebView kann im Speicher dort liegen, wo früher eine andere lag.
            (true, _) | (false, None) => (None, String::new()),
            // Es zählt die Seite, die der Tab zeigt – nicht eine, zu der er nur aufgebrochen ist.
            (false, Some(tab)) => {
                let seen = if tab.shown.is_empty() { tab.page.clone() } else { tab.shown.clone() };
                (Some(seen).filter(|page| !page.is_empty()), tab.shown.clone())
            }
        };
        let page = known.unwrap_or_else(|| page_of(url).to_string());
        ads.tabs.insert(key, TabAds { attached: None, guarded: None, page: page_of(&page).to_string(), shown });
        let controller = Retained::as_ptr(&controller) as *mut AnyObject;
        unsafe { sync_tab(&mut ads, key, controller, &page, true) };
    });
}

/// Dieser Tab gehört zu einem Stealth-Fenster (vor `equip` aufrufen).
pub fn force(key: usize) {
    ADS.with(|ads| ads.borrow_mut().forced.insert(key));
}

/// Räumt die Vermerke zu Tabs weg, die es nicht mehr gibt.
pub fn keep_only(live: &HashSet<usize>) {
    ADS.with(|ads| {
        let mut ads = ads.borrow_mut();
        ads.forced.retain(|key| live.contains(key));
        ads.tabs.retain(|key, _| live.contains(key));
        ads.blocked.retain(|key, _| live.contains(key));
    });
}

/// WebKit meldet einen gesperrten Abruf. `true`: Die Oberfläche soll bald
/// nachsehen (nur beim ersten seit dem letzten Nachsehen).
pub fn count_blocked(key: usize) -> bool {
    ADS.with(|ads| {
        let mut ads = ads.borrow_mut();
        if !ads.tabs.contains_key(&key) {
            return false;
        }
        let count = ads.blocked.entry(key).or_insert(0);
        *count = count.saturating_add(1);
        !std::mem::replace(&mut ads.blocked_pending, true)
    })
}

pub fn blocked(key: usize) -> u32 {
    ADS.with(|ads| ads.borrow().blocked.get(&key).copied().unwrap_or(0))
}

pub fn blocked_pending() -> bool {
    ADS.with(|ads| ads.borrow().blocked_pending)
}

pub fn take_blocked_pending() -> bool {
    ADS.with(|ads| std::mem::take(&mut ads.borrow_mut().blocked_pending))
}

/// Ist der Blocker für die Website dieser Adresse abgeschaltet?
pub fn site_off(url: &str) -> bool {
    crate::sites::domain_of(url).is_some_and(|site| ADS.with(|ads| ads.borrow().off.contains(&site)))
}

// ---------------------------------------------------------------------------
// Die großen Filterlisten

/// Was über die übersetzten Listen im Profil vermerkt ist (`filters/state.json`).
#[derive(Serialize, Deserialize, Default, Clone, Debug)]
struct FilterState {
    /// Woraus die übersetzten Listen gebaut sind: Fassung des Übersetzers und
    /// Stand jeder Quelle. Passt das nicht mehr, wird neu übersetzt.
    #[serde(default)]
    key: String,
    /// Die Plätze (`zunder-filter-<n>`), die WebKit aufgehoben hat.
    #[serde(default)]
    slots: Vec<u32>,
    #[serde(default)]
    rules: usize,
    #[serde(default)]
    stamps: Vec<String>,
    /// Wann zuletzt erfolgreich online nachgesehen wurde (Sekunden seit 1970).
    #[serde(default)]
    checked: u64,
    /// Dieses WebKit versteht die neueren Typnamen nicht (siehe `filters::convert`).
    #[serde(default)]
    compat: bool,
}

struct Big {
    /// Welcher Satz das ist (Stelle in `KINDS`).
    set: usize,
    kind: &'static Kind,
    lists: Vec<(u32, Retained<AnyObject>)>,
    state: FilterState,
    building: bool,
    fetching: bool,
    /// Während des Baus kamen neuere Quellen: danach gleich noch einmal.
    again: bool,
    /// Wann zuletzt online nachgesehen wurde (gleich, wie es ausging) – damit
    /// ein Auffrischen, das immer wieder scheitert, nicht jede Minute neu losläuft.
    tried: Option<std::time::Instant>,
    /// `…/filters` im Profil: aufgefrischte Listen, Vermerk, Speicher von WebKit.
    dir: PathBuf,
    store: Option<Retained<AnyObject>>,
}

/// Nachrichten der Hintergrund-Arbeit an die App.
pub enum FilterEvent {
    Converted {
        set: usize,
        lists: Vec<String>,
        rules: usize,
        stamps: Vec<String>,
        compat: bool,
        /// Nur beim Satz der gemeldeten Betrugs- und Schadseiten.
        index: Option<filters::GuardIndex>,
    },
    Fetched { set: usize, ok: bool, changed: bool },
    /// Das Verzeichnis der gemeldeten Adressen, wenn die übersetzten Listen
    /// schon bereitlagen und nur noch das Verzeichnis fehlte.
    Index(filters::GuardIndex),
}

impl std::fmt::Debug for FilterEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Converted { set, lists, rules, .. } => write!(f, "Converted(Satz {set}, {} Listen, {rules} Regeln)", lists.len()),
            Self::Fetched { set, ok, changed } => write!(f, "Fetched(Satz {set}, ok: {ok}, changed: {changed})"),
            Self::Index(index) => write!(f, "Index({} Einträge)", index.len()),
        }
    }
}

struct Build {
    queue: VecDeque<(u32, String)>,
    /// Wie viele Listen es werden müssen. Fehlt am Ende eine, gilt der ganze
    /// Bau als gescheitert – und es bleibt bei den Listen von vorher.
    expected: usize,
    done: Vec<(u32, Retained<AnyObject>)>,
    key: String,
    rules: usize,
    stamps: Vec<String>,
    compat: bool,
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn state_key(stamps: &[String], compat: bool) -> String {
    format!("{}:{}:{}", filters::CONVERTER, u8::from(compat), stamps.join(":"))
}

/// Die Listen, die der App beiliegen.
fn bundled_dir() -> Option<PathBuf> {
    if let Some(dir) = crate::dev::var_os("ZUNDER_FILTER_DIR") {
        return Some(PathBuf::from(dir));
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.parent()?.join("Resources").join("filters");
    dir.is_dir().then_some(dir)
}

/// Woher aufgefrischt wird. `None`: gar nicht (Probe-Apps greifen nur dann ins
/// Netz, wenn eine Prüfung es ausdrücklich verlangt). `Some(None)`: von den
/// Betreibern der Listen.
fn update_base() -> Option<Option<String>> {
    if crate::dev::BUILT_IN {
        crate::dev::var("ZUNDER_FILTER_URL").map(Some)
    } else {
        Some(None)
    }
}

fn head_stamp(path: &Path) -> Option<String> {
    let mut head = vec![0u8; 4096];
    let read = std::fs::File::open(path).ok()?.read(&mut head).ok()?;
    filters::stamp(&String::from_utf8_lossy(&head[..read]))
}

fn newer(candidate: &str, than: &str) -> bool {
    (candidate.len(), candidate) > (than.len(), than)
}

/// Je Quelle die neuere Fassung: die beiliegende oder die aufgefrischte.
fn active_sources(dir: &Path, kind: &Kind) -> Vec<(PathBuf, String)> {
    let bundled = if kind.bundled { bundled_dir() } else { None };
    kind.sources
        .iter()
        .filter_map(|source| {
            let name = format!("{}.txt", source.name);
            let shipped = bundled.as_ref().map(|dir| dir.join(&name)).and_then(|path| Some((head_stamp(&path)?, path)));
            let fetched = Some(dir.join(&name)).and_then(|path| Some((head_stamp(&path)?, path)));
            match (shipped, fetched) {
                (Some((old, _)), Some((new, path))) if newer(&new, &old) => Some((path, new)),
                (Some((stamp, path)), _) | (None, Some((stamp, path))) => Some((path, stamp)),
                (None, None) => None,
            }
        })
        .collect()
}

fn save_state(dir: &Path, kind: &Kind, state: &FilterState) {
    if let Ok(text) = serde_json::to_string(state) {
        let _ = std::fs::write(dir.join(kind.state_file), text);
    }
}

/// Liest die Quellen und übersetzt sie – im Hintergrund, die Oberfläche läuft weiter.
fn start_build(big: &Rc<RefCell<Big>>, proxy: &EventLoopProxy<UserEvent>) {
    let (sources, compat, set, guard) = {
        let mut big = big.borrow_mut();
        if big.building {
            big.again = true;
            return;
        }
        let sources = active_sources(&big.dir, big.kind);
        if sources.is_empty() || big.store.is_none() {
            return;
        }
        big.building = true;
        (sources, big.state.compat, big.set, big.kind.guard)
    };
    let proxy = proxy.clone();
    std::thread::spawn(move || {
        let mut texts = Vec::new();
        let mut stamps = Vec::new();
        for (path, stamp) in &sources {
            match std::fs::read_to_string(path) {
                Ok(text) if filters::plausible(&text) => {
                    texts.push(text);
                    stamps.push(stamp.clone());
                }
                _ => {}
            }
        }
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let converted = match guard {
            true => filters::convert_guard(&refs, compat),
            false => filters::convert(&refs, compat),
        };
        let index = guard.then(|| filters::guard_index(&refs));
        let _ = proxy.send_event(UserEvent::Filters(FilterEvent::Converted {
            set,
            index,
            lists: converted.lists,
            rules: converted.stats.rules(),
            stamps,
            compat,
        }));
    });
}

/// Übersetzt die nächste Liste – eine nach der anderen, damit der Speicher im Rahmen bleibt.
fn compile_next(big: Rc<RefCell<Big>>, proxy: EventLoopProxy<UserEvent>, build: Rc<RefCell<Build>>) {
    let next = build.borrow_mut().queue.pop_front();
    let store = big.borrow().store.clone();
    let prefix = big.borrow().kind.prefix;
    let (Some((slot, rules)), Some(store)) = (next, store) else {
        finish_build(big, proxy, build);
        return;
    };
    let (big_next, proxy_next, build_next) = (big.clone(), proxy.clone(), build.clone());
    let done = RcBlock::new(move |compiled: *mut AnyObject, _error: *mut AnyObject| {
        if let Some(compiled) = unsafe { Retained::retain(compiled) } {
            build_next.borrow_mut().done.push((slot, compiled));
        }
        compile_next(big_next.clone(), proxy_next.clone(), build_next.clone());
    });
    unsafe {
        let _: () = msg_send![
            &*store,
            compileContentRuleListForIdentifier: &*NSString::from_str(&format!("{prefix}-{slot}")),
            encodedContentRuleList: &*NSString::from_str(&rules),
            completionHandler: &*done
        ];
    }
}

fn finish_build(big: Rc<RefCell<Big>>, proxy: EventLoopProxy<UserEvent>, build: Rc<RefCell<Build>>) {
    let (done, key, rules, stamps, compat, expected) = {
        let mut build = build.borrow_mut();
        (
            std::mem::take(&mut build.done),
            std::mem::take(&mut build.key),
            build.rules,
            std::mem::take(&mut build.stamps),
            build.compat,
            build.expected,
        )
    };
    let again = {
        let mut big = big.borrow_mut();
        big.building = false;
        if !done.is_empty() && done.len() < expected {
            // WebKit hat einen Teil der Listen nicht angenommen. Ein halber Satz
            // wäre ein Schutz mit Löchern, von denen niemand weiß: Es bleibt bei
            // den Listen von vorher, und was neu übersetzt wurde, kommt wieder weg.
            if let Some(store) = big.store.clone() {
                for (slot, _) in &done {
                    let removed = RcBlock::new(|_error: *mut AnyObject| {});
                    unsafe {
                        let _: () = msg_send![
                            &*store,
                            removeContentRuleListForIdentifier: &*NSString::from_str(&format!("{}-{slot}", big.kind.prefix)),
                            completionHandler: &*removed
                        ];
                    }
                }
            }
            std::mem::take(&mut big.again)
        } else if done.is_empty() {
            // Nichts ließ sich übersetzen: einmal in der Schreibweise für
            // ältere WebKit-Fassungen versuchen, sonst bleibt alles beim Alten.
            let retry = !compat;
            if retry {
                big.state.compat = true;
            }
            retry
        } else {
            let slots: Vec<u32> = done.iter().map(|(slot, _)| *slot).collect();
            if let Some(store) = big.store.clone() {
                for old in big.state.slots.iter().filter(|old| !slots.contains(old)) {
                    let removed = RcBlock::new(|_error: *mut AnyObject| {});
                    unsafe {
                        let _: () = msg_send![
                            &*store,
                            removeContentRuleListForIdentifier: &*NSString::from_str(&format!("{}-{old}", big.kind.prefix)),
                            completionHandler: &*removed
                        ];
                    }
                }
            }
            big.lists = done;
            big.state.key = key;
            big.state.slots = slots;
            big.state.rules = rules;
            big.state.stamps = stamps;
            big.state.compat = compat;
            save_state(&big.dir, big.kind, &big.state);
            std::mem::take(&mut big.again)
        }
    };
    if again {
        start_build(&big, &proxy);
    }
    let _ = proxy.send_event(UserEvent::BlockerReady);
}

/// Holt die Listen neu von ihren Betreibern. Übernommen wird nur, was wie eine
/// vollständige, neuere Liste aussieht – nie eine Fehlerseite, nie ein Bruchstück.
fn fetch_lists(
    dir: PathBuf,
    set: usize,
    current: Vec<(String, u64)>,
    base: Option<String>,
    proxy: EventLoopProxy<UserEvent>,
) {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .timeout(Duration::from_secs(180))
        .redirects(2)
        .https_only(base.is_none())
        .user_agent("Zunder")
        .build();
    let mut ok = true;
    let mut changed = false;
    for (source, (stamp, size)) in KINDS[set].sources.iter().zip(current) {
        let url = match &base {
            Some(base) => format!("{}/{}.txt", base.trim_end_matches('/'), source.name),
            None => source.url.to_string(),
        };
        let mut text = String::new();
        let fetched = agent
            .get(&url)
            .call()
            .ok()
            .and_then(|response| response.into_reader().take(12_000_001).read_to_string(&mut text).ok());
        // Eine Liste, die plötzlich nur noch halb so groß ist, ist kaputt oder
        // manipuliert – dann bleibt die alte.
        let sound = fetched.is_some() && filters::plausible(&text) && text.len() as u64 * 2 >= size;
        if !sound {
            ok = false;
            continue;
        }
        let fresh = filters::stamp(&text).filter(|fresh| stamp.is_empty() || newer(fresh, &stamp));
        if fresh.is_none() {
            continue;
        }
        let target = dir.join(format!("{}.txt", source.name));
        let partial = dir.join(format!("{}.txt.neu", source.name));
        match std::fs::write(&partial, &text).and_then(|_| std::fs::rename(&partial, &target)) {
            Ok(()) => changed = true,
            Err(_) => ok = false,
        }
    }
    let _ = proxy.send_event(UserEvent::Filters(FilterEvent::Fetched { set, ok, changed }));
}

pub struct Blocker {
    /// Die übersetzte Basisliste (WKContentRuleList), sobald WebKit fertig ist.
    list: Slot,
    pub enabled: bool,
    /// Die Liste „eigenes Netz“. Sie wird neu übersetzt, wenn sich ändert,
    /// welche Websites dorthin dürfen.
    local: Slot,
    pub local_enabled: bool,
    /// Die großen Listen: Satz 0 Werbung und Tracker, Satz 1 gemeldete
    /// Betrugs- und Schadseiten (siehe `KINDS`).
    sets: Vec<Rc<RefCell<Big>>>,
    /// Die Werbelisten von selbst auffrischen.
    pub auto_update: bool,
    /// Vor gemeldeten Betrugs- und Schadseiten schützen.
    pub guard_enabled: bool,
    /// Wie viele Übersetzungen noch laufen. Solange beim Start eine läuft,
    /// warten die ersten Tabs – sonst lüde eine Seite kurz ganz ohne Schutz.
    /// (Das erstmalige Übersetzen der großen Listen zählt nicht mit: Es dauert
    /// Sekunden, und bis dahin schützt die Basisliste.)
    pending: Rc<Cell<u32>>,
    proxy: EventLoopProxy<UserEvent>,
}

impl Blocker {
    /// Startet die Übersetzung der kleinen Listen und holt die großen aus dem
    /// Speicher von WebKit. Sobald etwas bereitsteht, kommt
    /// `UserEvent::BlockerReady` und alle offenen Tabs werden nachgerüstet.
    pub fn new(
        enabled: bool,
        local_enabled: bool,
        allowed: &[String],
        off: HashSet<String>,
        auto_update: bool,
        guard_enabled: bool,
        data_dir: &Path,
        proxy: EventLoopProxy<UserEvent>,
    ) -> Self {
        let dir = data_dir.join("filters");
        let _ = std::fs::create_dir_all(dir.join("webkit"));
        // Ein eigener Speicher im Profil: Mit dem Profil verschwinden auch die Listen.
        let store: Option<Retained<AnyObject>> = unsafe {
            let url: Option<Retained<AnyObject>> = msg_send![
                class!(NSURL),
                fileURLWithPath: &*NSString::from_str(&dir.join("webkit").to_string_lossy()),
                isDirectory: true
            ];
            url.and_then(|url| msg_send![class!(WKContentRuleListStore), storeWithURL: &*url])
        };
        ADS.with(|ads| {
            let mut ads = ads.borrow_mut();
            ads.enabled = enabled;
            ads.local_enabled = local_enabled;
            ads.guard_enabled = guard_enabled;
            ads.off = off;
        });
        let sets = KINDS
            .iter()
            .enumerate()
            .map(|(set, kind)| {
                let state: FilterState = std::fs::read_to_string(dir.join(kind.state_file))
                    .ok()
                    .and_then(|text| serde_json::from_str(&text).ok())
                    .unwrap_or_default();
                Rc::new(RefCell::new(Big {
                    set,
                    kind,
                    lists: Vec::new(),
                    state,
                    building: false,
                    fetching: false,
                    again: false,
                    tried: None,
                    dir: dir.clone(),
                    store: store.clone(),
                }))
            })
            .collect();
        let blocker = Self {
            list: Rc::new(RefCell::new(None)),
            enabled,
            local: Rc::new(RefCell::new(None)),
            local_enabled,
            sets,
            auto_update,
            guard_enabled,
            pending: Rc::new(Cell::new(0)),
            proxy,
        };
        blocker.compile(&format!("{BASIS_PREFIX}-1"), &rules_json(), blocker.list.clone());
        blocker.compile_local(allowed);
        for set in &blocker.sets {
            blocker.restore_big(set);
        }
        // Meldet sich WebKit nicht, geht es trotzdem weiter.
        let proxy = blocker.proxy.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(3));
            let _ = proxy.send_event(UserEvent::BlockerReady);
        });
        blocker
    }

    /// Holt die großen Listen aus dem Speicher von WebKit – oder übersetzt sie
    /// neu, wenn sich Quellen oder Übersetzer geändert haben.
    fn restore_big(&self, set: &Rc<RefCell<Big>>) {
        let (slots, store, fits, prefix) = {
            let big = set.borrow();
            let stamps: Vec<String> = active_sources(&big.dir, big.kind).into_iter().map(|(_, stamp)| stamp).collect();
            let fits = !stamps.is_empty()
                && !big.state.slots.is_empty()
                && big.state.key == state_key(&stamps, big.state.compat);
            (big.state.slots.clone(), big.store.clone(), fits, big.kind.prefix)
        };
        let Some(store) = store else {
            return;
        };
        if !fits {
            start_build(set, &self.proxy);
            return;
        }
        // Die übersetzten Listen liegen bereit. Das Verzeichnis für die Warnung
        // vor dem Öffnen entsteht aus denselben Quellen neu – im Hintergrund.
        if set.borrow().kind.guard {
            let sources = active_sources(&set.borrow().dir, set.borrow().kind);
            let proxy = self.proxy.clone();
            std::thread::spawn(move || {
                let texts: Vec<String> = sources.iter().filter_map(|(path, _)| std::fs::read_to_string(path).ok()).collect();
                let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
                let _ = proxy.send_event(UserEvent::Filters(FilterEvent::Index(filters::guard_index(&refs))));
            });
        }
        let remaining = Rc::new(Cell::new(slots.len()));
        let missing = Rc::new(Cell::new(false));
        for slot in slots {
            self.pending.set(self.pending.get() + 1);
            let (big, pending, proxy) = (set.clone(), self.pending.clone(), self.proxy.clone());
            let (remaining, missing) = (remaining.clone(), missing.clone());
            let done = RcBlock::new(move |found: *mut AnyObject, _error: *mut AnyObject| {
                pending.set(pending.get().saturating_sub(1));
                match unsafe { Retained::retain(found) } {
                    Some(found) => big.borrow_mut().lists.push((slot, found)),
                    None => missing.set(true),
                }
                remaining.set(remaining.get().saturating_sub(1));
                if remaining.get() == 0 {
                    if missing.get() {
                        start_build(&big, &proxy);
                    }
                    let _ = proxy.send_event(UserEvent::BlockerReady);
                }
            });
            unsafe {
                let _: () = msg_send![
                    &*store,
                    lookUpContentRuleListForIdentifier: &*NSString::from_str(&format!("{prefix}-{slot}")),
                    completionHandler: &*done
                ];
            }
        }
    }

    /// Nachricht der Hintergrund-Arbeit (Übersetzen, Auffrischen).
    pub fn on_event(&mut self, event: FilterEvent) {
        match event {
            FilterEvent::Index(index) => ADS.with(|ads| ads.borrow_mut().guard_index = Some(index)),
            FilterEvent::Converted { set, lists, rules, stamps, compat, index } => {
                let Some(big) = self.sets.get(set).cloned() else {
                    return;
                };
                if let Some(index) = index {
                    ADS.with(|ads| ads.borrow_mut().guard_index = Some(index));
                }
                if lists.is_empty() {
                    big.borrow_mut().building = false;
                    return;
                }
                // Neue Listen bekommen eigene Nummern: So überschreibt ein Bau,
                // der scheitert, nichts von dem, was gerade gilt.
                let base = {
                    let big = big.borrow();
                    let next = big.state.slots.iter().max().map_or(0, |highest| (highest / 100 + 1) * 100);
                    if next > 900_000 { 0 } else { next }
                };
                let build = Build {
                    expected: lists.len(),
                    queue: lists.into_iter().enumerate().map(|(slot, rules)| (base + slot as u32, rules)).collect(),
                    done: Vec::new(),
                    key: state_key(&stamps, compat),
                    rules,
                    stamps,
                    compat,
                };
                compile_next(big, self.proxy.clone(), Rc::new(RefCell::new(build)));
            }
            FilterEvent::Fetched { set, ok, changed } => {
                let Some(big) = self.sets.get(set).cloned() else {
                    return;
                };
                {
                    let mut big = big.borrow_mut();
                    big.fetching = false;
                    if ok {
                        big.state.checked = now_secs();
                        save_state(&big.dir, big.kind, &big.state);
                    }
                }
                if changed {
                    start_build(&big, &self.proxy);
                }
            }
        }
    }

    /// Sieht online nach neueren Listen. Ohne `force` nur, wenn es an der Zeit
    /// ist und der jeweilige Schutz eingeschaltet ist. `true`: Es wurde angestoßen.
    pub fn refresh(&self, force: bool) -> bool {
        let Some(base) = update_base() else {
            return false;
        };
        let mut started = false;
        for set in &self.sets {
            let mut big = set.borrow_mut();
            if big.fetching || big.building || big.store.is_none() {
                continue;
            }
            let wanted = match big.kind.guard {
                true => self.guard_enabled,
                false => self.auto_update && self.enabled,
            };
            let due = wanted && now_secs().saturating_sub(big.state.checked) >= big.kind.refresh_after.as_secs();
            if !due && !(force && !big.kind.guard) {
                continue;
            }
            // Nach einem Versuch – auch einem gescheiterten – ist eine halbe
            // Stunde Ruhe, außer jemand drückt selbst auf „Jetzt auffrischen“.
            if !force && big.tried.is_some_and(|at| at.elapsed() < Duration::from_secs(1800)) {
                continue;
            }
            big.tried = Some(std::time::Instant::now());
            let active = active_sources(&big.dir, big.kind);
            let current: Vec<(String, u64)> = big
                .kind
                .sources
                .iter()
                .map(|source| {
                    let name = format!("{}.txt", source.name);
                    active
                        .iter()
                        .find(|(path, _)| path.file_name().is_some_and(|file| file.to_string_lossy() == name))
                        .map(|(path, stamp)| (stamp.clone(), std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)))
                        .unwrap_or_default()
                })
                .collect();
            big.fetching = true;
            let (dir, index, proxy, base) = (big.dir.clone(), big.set, self.proxy.clone(), base.clone());
            std::thread::spawn(move || fetch_lists(dir, index, current, base, proxy));
            started = true;
        }
        started
    }

    /// Für die Einstellungen: wie viele Regeln gelten, von wann die Listen sind.
    pub fn info(&self) -> serde_json::Value {
        let guard = self.sets[1].borrow();
        let guard_rules = if guard.lists.is_empty() { 0 } else { guard.state.rules };
        let big = self.sets[0].borrow();
        let ready = !big.lists.is_empty();
        serde_json::json!({
            "rules": if ready { big.state.rules } else { 0 },
            "stamp": if ready { big.state.stamps.iter().max().cloned().unwrap_or_default() } else { String::new() },
            "busy": big.building || big.fetching,
            "lists": big.lists.len(),
            "titles": filters::SOURCES.iter().map(|source| source.title).collect::<Vec<_>>(),
            "canRefresh": update_base().is_some(),
            // Gemeldete Betrugs- und Schadseiten: wie viele Adressen gerade gesperrt sind.
            "guardRules": guard_rules,
            "guardBusy": guard.building || guard.fetching,
        })
    }

    /// Übersetzt die Liste „eigenes Netz“ neu – mit den Websites, die dorthin dürfen.
    pub fn compile_local(&self, allowed: &[String]) {
        self.compile(&format!("{LOCAL_PREFIX}-1"), &local_rules_json(allowed), self.local.clone());
    }

    fn compile(&self, identifier: &str, rules: &str, slot: Slot) {
        let proxy = self.proxy.clone();
        let pending = self.pending.clone();
        pending.set(pending.get() + 1);
        let done = RcBlock::new(move |compiled: *mut AnyObject, _error: *mut AnyObject| {
            pending.set(pending.get().saturating_sub(1));
            if let Some(compiled) = unsafe { Retained::retain(compiled) } {
                *slot.borrow_mut() = Some(compiled);
            }
            // Auch nach einem Fehler: Niemand soll auf eine Liste warten, die nicht kommt.
            let _ = proxy.send_event(UserEvent::BlockerReady);
        });
        unsafe {
            let store: Option<Retained<AnyObject>> =
                msg_send![class!(WKContentRuleListStore), defaultStore];
            match store {
                Some(store) => {
                    let _: () = msg_send![
                        &*store,
                        compileContentRuleListForIdentifier: &*NSString::from_str(identifier),
                        encodedContentRuleList: &*NSString::from_str(rules),
                        completionHandler: &*done
                    ];
                }
                None => self.pending.set(self.pending.get().saturating_sub(1)),
            }
        }
    }

    /// Läuft noch eine Übersetzung, auf die der Start wartet?
    pub fn pending(&self) -> bool {
        self.pending.get() > 0
    }

    /// Nach Ablauf der Wartefrist: nicht länger auf WebKit warten.
    pub fn give_up_waiting(&self) {
        self.pending.set(0);
    }

    /// Gibt es die Liste „eigenes Netz“ (für die Prüfhilfe)?
    pub fn local_ready(&self) -> bool {
        self.local.borrow().is_some()
    }

    /// Macht bekannt, welche Listen jetzt gelten (nach jeder fertigen
    /// Übersetzung). Danach rüstet `apply` die Tabs um.
    pub fn publish(&self) {
        let sorted = |set: &Rc<RefCell<Big>>| -> Vec<Retained<AnyObject>> {
            let mut lists: Vec<(u32, Retained<AnyObject>)> = set.borrow().lists.clone();
            lists.sort_by_key(|(slot, _)| *slot);
            lists.into_iter().map(|(_, list)| list).collect()
        };
        let mut current: Vec<Retained<AnyObject>> = Vec::new();
        if let Some(list) = self.list.borrow().as_ref() {
            current.push(list.clone());
        }
        current.extend(sorted(&self.sets[0]));
        let guard = sorted(&self.sets[1]);
        self.settle();
        ADS.with(|ads| {
            let mut ads = ads.borrow_mut();
            ads.layer.replace(current);
            ads.guard.replace(guard);
        });
    }

    /// Rüstet einen bestehenden Tab nach den Einstellungen um (siehe `equip`).
    pub fn apply(&self, webview: &WebView, url: &str) {
        self.settle();
        equip(webview, url, false);
    }

    /// Rüstet einen gerade angelegten Tab aus, der `url` lädt.
    pub fn apply_new(&self, webview: &WebView, url: &str) {
        self.settle();
        equip(webview, url, true);
    }

    fn settle(&self) {
        ADS.with(|ads| {
            let mut ads = ads.borrow_mut();
            ads.enabled = self.enabled;
            ads.guard_enabled = self.guard_enabled;
            ads.local_enabled = self.local_enabled;
            ads.local = self.local.borrow().clone();
        });
    }

    /// Schaltet den Blocker für eine Website ab oder wieder an. `site` ist die
    /// Hauptdomain (`beispiel.de`).
    pub fn set_site_off(&self, site: &str, off: bool) {
        ADS.with(|ads| {
            let mut ads = ads.borrow_mut();
            match off {
                true => ads.off.insert(site.to_string()),
                false => ads.off.remove(site),
            };
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_are_valid_json() {
        let parsed: serde_json::Value = serde_json::from_str(&rules_json()).unwrap();
        let rules = parsed.as_array().unwrap();
        assert_eq!(rules.len(), DOMAINS.len() + PATTERNS.len());
        assert_eq!(
            rules[0]["trigger"]["url-filter"],
            "^[^:]+://+([^:/]+\\.)?doubleclick\\.net\\.?[:/]"
        );
    }

    /// Prüft die Muster mit einem gewöhnlichen Vergleich nach – WebKits
    /// Regelsprache ist eine Teilmenge davon. Ohne Bibliothek für reguläre
    /// Ausdrücke bleibt es bei den Fällen, auf die es ankommt.
    #[test]
    fn local_rules_cover_the_own_network() {
        let parsed: serde_json::Value = serde_json::from_str(&local_rules_json(&["plex.tv".to_string(), "a b".to_string()])).unwrap();
        let rules = parsed.as_array().unwrap();
        assert_eq!(rules.len(), LOCAL_HOSTS.len());
        for rule in rules {
            assert_eq!(rule["action"]["type"], "block");
            let filter = rule["trigger"]["url-filter"].as_str().unwrap();
            // Kein „oder“ – WebKit lehnte die ganze Liste sonst ab.
            assert!(!filter.contains('|'), "{filter}");
            assert!(filter.starts_with(HOST_START) && filter.ends_with("[:/]"), "{filter}");
            let exempt = rule["trigger"]["unless-top-url"].as_array().unwrap();
            // Alle Geräte-Muster, Dateien, interne Seiten und die eine brauchbare erlaubte Website.
            assert_eq!(exempt.len(), LOCAL_HOSTS.len() + 3);
            assert!(exempt.iter().any(|e| e == "^zunder:"));
            assert!(exempt.iter().any(|e| e.as_str().is_some_and(|e| e.ends_with("([^/:]*\\.)?plex\\.tv\\.?[:/]"))));
        }
        let filters: Vec<&str> = rules.iter().map(|r| r["trigger"]["url-filter"].as_str().unwrap()).collect();
        assert!(filters.contains(&"^[hw][a-z]+://+([^/@]*@)?192\\.168\\.[0-9]+\\.[0-9]+\\.?[:/]"));
        assert!(filters.contains(&"^[hw][a-z]+://+([^/@]*@)?localhost\\.?[:/]"));
    }
}
