//! Eingebaute Apps.
//!
//! Eine App ist eine eigene Seite des Browsers in einem Tab –
//! `zunder://app-<name>/` –, geladen aus dem Programm selbst. Sie darf mehr als
//! eine Webseite, aber nur, was hier für sie eingetragen ist:
//!
//!   - **Netz:** nur zu den Diensten, die bei ihr stehen (Daten und Bilder
//!     getrennt). Das setzt WebKit
//!     selbst durch – über die Schutzregel, mit der die Seite ausgeliefert wird
//!     (`csp`). Alles andere im Netz erreicht sie nicht.
//!   - **Standort:** nur nach Zustimmung je App, und nie genauer als auf zwei
//!     Nachkommastellen (gut ein Kilometer).
//!   - **Speicher:** ein kleiner eigener Zettel beim Browser (`STORE_MAX`) und –
//!     wo es für sie eingetragen ist – Karteikarten: viele kleine Einträge,
//!     jeder unter seiner Kennung (`RECORD_MAX`). Beides liest keine andere App
//!     und keine Webseite. Es liegt beim Browser wie die Lesezeichen, nicht bei
//!     den Website-Daten: „Verlauf und Website-Daten löschen“ lässt es stehen.
//!
//! Mit dem Browser spricht eine App über den Kanal der Seiten
//! (`native::bridge_received`): Der nennt die Adresse des Dokuments, das eine
//! Nachricht schickt – angenommen wird sie hier nur von der App selbst
//! (`pages::app_message`). Die Antwort prüft selbst, wo sie ankommt (`reply`):
//! Zeigt der Tab inzwischen etwas anderes, verpufft sie. Laden kann eine App
//! allein der Browser: über die Startseite, das Menü oder die Adressleiste, nie
//! eine Webseite (`native::nav_policy`).

use serde_json::{json, Value};

/// Eine eingebaute App und was sie darf.
pub struct App {
    /// Kurzname – steht in der Adresse (`zunder://app-<id>/`).
    pub id: &'static str,
    /// Schlüssel ihres Namens und ihrer Kurzbeschreibung in `i18n`.
    pub name: &'static str,
    pub about: &'static str,
    /// Name ihres Symbols im Sprite (`#i-<icon>`).
    pub icon: &'static str,
    /// Vorlage ihrer Seite und die Dateien, die die Seite nachlädt.
    pub page: &'static str,
    pub files: &'static [&'static str],
    /// Dienste im Netz, die sie ansprechen darf (Schema und Rechner).
    pub connect: &'static [&'static str],
    /// Dienste, von denen sie Bilder laden darf.
    pub images: &'static [&'static str],
    /// Darf sie nach dem Standort fragen?
    pub location: bool,
    /// So viele Karteikarten darf sie beim Browser ablegen (0: keine).
    pub records: usize,
    /// Dateien, die sie selbst erzeugt und bei den Downloads sichert, dürfen so
    /// enden (ohne Punkt, klein). Leer: Sie sichert keine.
    pub exports: &'static [&'static str],
}

pub const APPS: &[App] = &[
    App {
        id: "weather",
        name: "app_weather",
        about: "app_weather_about",
        icon: "weather",
        page: "app-weather.html",
        files: &["apps.css", "apps.js", "app-weather.css", "app-weather.js", "app-weather-map.js"],
        // Wetter und Ortssuche: Open-Meteo. Regenradar: Daten des Deutschen
        // Wetterdienstes, ausgeliefert von Bright Sky. Postleitzahlen: Nominatim
        // (OpenStreetMap) – Open-Meteo kennt die deutschen nicht.
        connect: &[
            "https://api.open-meteo.com",
            "https://geocoding-api.open-meteo.com",
            "https://api.brightsky.dev",
            "https://nominatim.openstreetmap.org",
        ],
        // Satellitenbild hinter dem Radar: NASA Blue Marble, ausgeliefert von GIBS.
        images: &["https://gibs.earthdata.nasa.gov"],
        location: true,
        records: 0,
        exports: &[],
    },
    // Zeiterfassung: Sie geht gar nicht ins Netz. Was gebucht ist, liegt als
    // Karteikarten beim Browser – genug für viele Jahre.
    App {
        id: "time",
        name: "app_time",
        about: "app_time_about",
        icon: "timer",
        page: "app-time.html",
        files: &["apps.css", "apps.js", "app-time.css", "app-time.js"],
        connect: &[],
        images: &[],
        location: false,
        records: 20_000,
        // Die Zeiten als Tabelle.
        exports: &["csv"],
    },
];

/// So groß ist der Zettel einer App höchstens (Bytes). Mehr passte auch nicht
/// durch den Kanal der Seiten.
pub const STORE_MAX: usize = 3000;

/// So groß ist eine Karteikarte höchstens (Bytes ihres JSON-Textes) …
pub const RECORD_MAX: usize = 1500;
/// … und so lang ihre Kennung: Buchstaben, Ziffern, „-“ und „_“.
pub const RECORD_ID_MAX: usize = 40;
/// So viele Karten gehen höchstens in einer Antwort an die Seite – und zusammen
/// so viele Bytes. Wer mehr hat, blättert weiter (`Op::List`).
pub const LIST_MAX: usize = 500;
pub const LIST_BYTES: usize = 200_000;

const HOST_PREFIX: &str = "app-";

/// Die Adresse einer App.
pub fn url_of(id: &str) -> String {
    format!("zunder://{HOST_PREFIX}{id}/")
}

pub fn by_id(id: &str) -> Option<&'static App> {
    APPS.iter().find(|app| app.id == id)
}

/// Die App hinter einem Rechnernamen (`app-weather`).
pub fn by_host(host: &str) -> Option<&'static App> {
    by_id(host.strip_prefix(HOST_PREFIX)?)
}

/// Die App, zu der diese Adresse gehört – ihre Seite oder eine ihrer Dateien.
pub fn by_url(url: &str) -> Option<&'static App> {
    let parsed = url::Url::parse(url).ok()?;
    let plain = parsed.port().is_none() && parsed.username().is_empty() && parsed.password().is_none();
    if parsed.scheme() != "zunder" || !plain {
        return None;
    }
    by_host(parsed.host_str()?)
}

/// Ist das genau die Adresse, unter der eine App beginnt? Nur die lädt ein
/// Tab – ohne Anhängsel, in denen etwas stehen könnte, das die Seite auswertet.
pub fn is_root(url: &str) -> bool {
    by_url(url).is_some_and(|app| url == url_of(app.id))
}

/// Was in der Adressleiste eingetippt eine App öffnet: ihre Adresse, mit oder
/// ohne Schrägstrich am Ende.
pub fn typed(input: &str) -> Option<String> {
    let lower = input.trim().to_ascii_lowercase();
    let wanted = lower.strip_suffix('/').unwrap_or(&lower);
    APPS.iter().map(|app| url_of(app.id)).find(|url| url.trim_end_matches('/') == wanted)
}

/// Das Land, in dem dieser Mac zu Hause ist – aus der bevorzugten Sprache des
/// Systems (`de-DE` → `de`). Damit sucht eine App zuerst dort, etwa nach einer
/// Postleitzahl. Leer, wenn es sich nicht ablesen lässt.
pub fn region_of(language: &str) -> String {
    let region = language.rsplit(['-', '_']).next().unwrap_or("");
    let plain = region.len() == 2 && region.chars().all(|c| c.is_ascii_alphabetic()) && language.len() > 2;
    if plain {
        region.to_ascii_lowercase()
    } else {
        String::new()
    }
}

/// Die Schutzregel, mit der die Seite einer App ausgeliefert wird: nur eigene
/// Skripte und Stile, Bilder nur aus dem Programm oder selbst gezeichnet – und
/// ins Netz nur zu den Diensten der App.
pub fn csp(app: &App) -> String {
    let connect = if app.connect.is_empty() { "'none'".to_string() } else { app.connect.join(" ") };
    let images: String = app.images.iter().map(|service| format!(" {service}")).collect();
    format!(
        "default-src 'none'; script-src zunder:; style-src zunder: 'unsafe-inline'; \
         img-src zunder: data: blob:{images}; connect-src {connect}; base-uri 'none'; \
         form-action 'none'; frame-ancestors 'none'"
    )
}

/// Die Apps für die Startseite: Name, Beschreibung, Symbol, Adresse.
pub fn list() -> Value {
    Value::Array(
        APPS.iter()
            .map(|app| {
                json!({
                    "id": app.id,
                    "name": crate::i18n::t(app.name),
                    "about": crate::i18n::t(app.about),
                    "icon": app.icon,
                    "url": url_of(app.id),
                })
            })
            .collect(),
    )
}

/// Die App, die diese Datei selbst erzeugt hat: Ihre Adresse beginnt mit
/// `blob:` und der Adresse der App (`blob:zunder://app-time/…`).
pub fn own_file(url: &str) -> Option<&'static App> {
    by_url(url.strip_prefix("blob:")?)
}

/// Darf diese App eine Datei dieses Namens sichern? Nur mit einer der Endungen,
/// die für sie eingetragen sind.
pub fn may_export(app: &App, name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, ending)| app.exports.contains(&ending.to_ascii_lowercase().as_str()))
}

/// Unter diesen Namen liegt in den Einstellungen des Browsers, was zu einer App gehört.
pub fn data_key(id: &str) -> String {
    format!("app.{id}.data")
}

pub fn location_key(id: &str) -> String {
    format!("app.{id}.location")
}

/// Was eine App vom Browser möchte.
#[derive(Debug, PartialEq)]
pub enum Op {
    /// Den Standort – nach Zustimmung, gerundet.
    Location,
    /// Die Zustimmung zum Standort zurücknehmen.
    Forget,
    /// Ihren Zettel lesen.
    Load,
    /// Ihren Zettel neu schreiben (ein JSON-Objekt als Text).
    Save(String),
    /// Ihre Karteikarten lesen – nach Kennung geordnet, ab der nach `after`.
    List { after: String, limit: usize },
    /// Eine Karteikarte ablegen oder ersetzen (ein JSON-Objekt als Text).
    Put { id: String, data: String },
    /// Eine Karteikarte wegwerfen.
    Remove(String),
    /// Ankündigen, wie die Datei heißt, die sie gleich selbst erzeugt und
    /// sichert (`ui/apps.js`, `saveFile`).
    FileName(String),
}

/// Eine Anfrage: laufende Nummer (unter der die Antwort zurückkommt) und Wunsch.
#[derive(Debug, PartialEq)]
pub struct Call {
    pub n: u32,
    pub op: Op,
}

/// Liest eine Anfrage, wie die Seite einer App sie schickt:
/// `{"n":7,"op":"save","data":"{…}"}`. Was sich nicht lesen lässt, fällt weg –
/// mit der Nummer der Anfrage, wenn wenigstens die zu erkennen war: Dann
/// erfährt die Seite, dass daraus nichts wird, statt auf eine Antwort zu warten.
pub fn parse(body: &str) -> Result<Call, Option<u32>> {
    let value: Value = serde_json::from_str(body).map_err(|_| None)?;
    let n = value.get("n").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).ok_or(None)?;
    let op = wish(&value).ok_or(Some(n))?;
    Ok(Call { n, op })
}

fn wish(value: &Value) -> Option<Op> {
    let text = |name: &str| value.get(name).and_then(Value::as_str);
    Some(match text("op")? {
        "location" => Op::Location,
        "forget" => Op::Forget,
        "load" => Op::Load,
        "save" => Op::Save(object_text(text("data")?, STORE_MAX)?),
        "list" => {
            let after = match value.get("after") {
                None => String::new(),
                Some(after) => match after.as_str()? {
                    "" => String::new(),
                    id => record_id(id)?,
                },
            };
            let limit = match value.get("limit") {
                None => LIST_MAX,
                Some(limit) => usize::try_from(limit.as_u64()?).unwrap_or(LIST_MAX).clamp(1, LIST_MAX),
            };
            Op::List { after, limit }
        }
        "put" => Op::Put { id: record_id(text("id")?)?, data: object_text(text("data")?, RECORD_MAX)? },
        "remove" => Op::Remove(record_id(text("id")?)?),
        "filename" => Op::FileName(file_name(text("name")?)?),
        _ => return None,
    })
}

/// Ein JSON-Objekt als Text, höchstens `max` Bytes – nichts anderes wird
/// aufbewahrt und später wieder in die Seite gegeben.
fn object_text(data: &str, max: usize) -> Option<String> {
    let object = data.len() <= max && serde_json::from_str::<Value>(data).is_ok_and(|parsed| parsed.is_object());
    object.then(|| data.to_string())
}

/// Der Name einer Datei, die eine App sichert: kurz, aus harmlosen Zeichen, mit
/// Endung – und weder versteckt noch ein Weg irgendwohin.
fn file_name(name: &str) -> Option<String> {
    let fits = !name.is_empty() && name.len() <= 80;
    let plain = name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    let named = name.rsplit_once('.').is_some_and(|(stem, ending)| !stem.is_empty() && !ending.is_empty());
    (fits && plain && named && !name.starts_with('.') && !name.contains("..")).then(|| name.to_string())
}

/// Die Kennung einer Karteikarte: kurz und nur aus harmlosen Zeichen.
fn record_id(id: &str) -> Option<String> {
    let fits = !id.is_empty() && id.len() <= RECORD_ID_MAX;
    let plain = id.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
    (fits && plain).then(|| id.to_string())
}

/// Ein Standort, wie eine App ihn bekommt: geprüft und auf zwei
/// Nachkommastellen gerundet (in Deutschland gut ein Kilometer).
pub fn position(latitude: f64, longitude: f64) -> Option<(f64, f64)> {
    let valid = latitude.is_finite() && longitude.is_finite() && latitude.abs() <= 90.0 && longitude.abs() <= 180.0;
    valid.then(|| (round(latitude), round(longitude)))
}

fn round(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/// Die Skriptzeile, mit der eine Antwort in die Seite der App zurückgeht. Die
/// Seite hält dafür `window.__zunderApp` bereit (`ui/apps.js`).
///
/// Die Zeile prüft selbst, wo sie läuft: Der Browser weiß nur mit Verzögerung,
/// was ein Tab gerade zeigt – ist dort inzwischen ein anderes Dokument, darf die
/// Antwort (der Zettel, der Standort) es nicht erreichen. Die Adresse des
/// Dokuments kann keine Seite fälschen, und unter der Adresse einer App liefert
/// nur der Browser selbst etwas aus.
pub fn reply(app: &str, n: u32, mut answer: Value) -> String {
    if let Some(fields) = answer.as_object_mut() {
        fields.insert("n".into(), json!(n));
    }
    let home = Value::String(url_of(app));
    // JSON ist gültiges JavaScript – bis auf „</script“ muss hier nichts
    // entschärft werden, und in einer Skriptzeile kommt das nicht vor.
    format!("if (window.location.href === {home} && typeof window.__zunderApp === 'function') {{ window.__zunderApp({answer}); }}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adressen_der_apps() {
        assert_eq!(url_of("weather"), "zunder://app-weather/");
        assert!(by_host("app-weather").is_some());
        assert!(by_host("weather").is_none());
        assert!(by_host("app-").is_none());
        assert!(by_host("app-nix").is_none());
        assert_eq!(by_url("zunder://app-weather/").map(|app| app.id), Some("weather"));
        assert_eq!(by_url("zunder://app-weather/app-weather.js").map(|app| app.id), Some("weather"));
        // Fremdes Schema, fremder Rechner, Anhängsel am Rechnernamen.
        for url in [
            "https://app-weather/",
            "zunder://newtab/",
            "zunder://app-weather.example.com/",
            "zunder://x@app-weather/",
            "zunder://:x@app-weather/",
            "zunder://app-weather:8080/",
            "app-weather",
            "",
        ] {
            assert!(by_url(url).is_none(), "{url}");
        }
        // Geladen wird nur die Adresse ohne jedes Anhängsel.
        assert!(is_root("zunder://app-weather/"));
        for url in ["zunder://app-weather/?x=1", "zunder://app-weather/#a", "zunder://app-weather/apps.js", "zunder://app-weather", "zunder://newtab/"] {
            assert!(!is_root(url), "{url}");
        }
    }

    #[test]
    fn land_aus_der_sprache_des_systems() {
        assert_eq!(region_of("de-DE"), "de");
        assert_eq!(region_of("en_US"), "us");
        assert_eq!(region_of("zh-Hans-CN"), "cn");
        assert_eq!(region_of("de-AT"), "at");
        // Ohne Land, mit Unsinn: lieber keines als ein falsches.
        for language in ["de", "en", "", "zh-Hans", "de-D3", "de-DEU", "x"] {
            assert_eq!(region_of(language), "", "{language}");
        }
    }

    #[test]
    fn eingetippte_adressen() {
        assert_eq!(typed("zunder://app-weather/").as_deref(), Some("zunder://app-weather/"));
        assert_eq!(typed("  ZUNDER://App-Weather ").as_deref(), Some("zunder://app-weather/"));
        for input in ["zunder://app-weather/x", "zunder://app-weather/?a", "app-weather", "wetter", "zunder://app-nix/", ""] {
            assert!(typed(input).is_none(), "{input}");
        }
    }

    #[test]
    fn schutzregel_nennt_nur_die_dienste_der_app() {
        let app = by_id("weather").unwrap();
        let rule = csp(app);
        assert!(rule.contains("default-src 'none'"));
        assert!(rule.contains(
            "connect-src https://api.open-meteo.com https://geocoding-api.open-meteo.com https://api.brightsky.dev https://nominatim.openstreetmap.org;"
        ));
        assert!(rule.contains("script-src zunder:;"));
        assert!(rule.contains("frame-ancestors 'none'"));
        // Kein Stern, kein beliebiges https, keine Rahmen, keine Formulare nach draußen.
        assert!(!rule.contains('*'));
        assert!(!rule.contains("https:;") && !rule.contains("https: "));
        assert!(!rule.contains("frame-src"));
        assert!(rule.contains("form-action 'none'"));
        assert!(rule.contains("img-src zunder: data: blob: https://gibs.earthdata.nasa.gov;"));
        // Jeder Dienst steht mit Schema und Rechner da, ohne Pfad.
        for app in APPS {
            for service in app.connect.iter().chain(app.images) {
                let parsed = url::Url::parse(service).unwrap();
                assert_eq!(parsed.scheme(), "https", "{service}");
                assert_eq!(parsed.path(), "/", "{service}");
                assert!(!service.ends_with('/'), "{service}");
            }
        }
        // Die Zeiterfassung erreicht im Netz gar nichts.
        let time = csp(by_id("time").unwrap());
        assert!(time.contains("connect-src 'none';"));
        assert!(time.contains("img-src zunder: data: blob:;"));
        assert!(!time.contains("http"));
    }

    #[test]
    fn anfragen_der_apps() {
        assert_eq!(parse(r#"{"n":1,"op":"location"}"#), Ok(Call { n: 1, op: Op::Location }));
        assert_eq!(parse(r#"{"n":2,"op":"forget"}"#), Ok(Call { n: 2, op: Op::Forget }));
        assert_eq!(parse(r#"{"n":3,"op":"load"}"#), Ok(Call { n: 3, op: Op::Load }));
        assert_eq!(
            parse(r#"{"n":4,"op":"save","data":"{\"orte\":[1,2]}"}"#),
            Ok(Call { n: 4, op: Op::Save("{\"orte\":[1,2]}".into()) })
        );
        // Kein Objekt, kein JSON, zu groß, ohne Inhalt: Die Seite erfährt es
        // unter der Nummer ihrer Anfrage.
        assert_eq!(parse(r#"{"n":5,"op":"save","data":"[1,2]"}"#), Err(Some(5)));
        assert_eq!(parse(r#"{"n":5,"op":"save","data":"kein json"}"#), Err(Some(5)));
        assert_eq!(parse(r#"{"n":5,"op":"save"}"#), Err(Some(5)));
        let big = format!("{{\"a\":\"{}\"}}", "x".repeat(STORE_MAX));
        let body = json!({ "n": 6, "op": "save", "data": big }).to_string();
        assert_eq!(parse(&body), Err(Some(6)));
        // Ein einzelnes halbes Zeichenpaar (abgeschnittenes Emoji) ist kein Text.
        assert_eq!(parse(r#"{"n":7,"op":"save","data":"{\"a\":\"\\ud83d\"}"}"#), Err(Some(7)));
        // Unbekannter Wunsch – mit Nummer; ohne brauchbare Nummer gibt es keine Antwort.
        assert_eq!(parse(r#"{"n":1,"op":"quit"}"#), Err(Some(1)));
        assert_eq!(parse(r#"{"n":1}"#), Err(Some(1)));
        assert_eq!(parse(r#"{"op":"load"}"#), Err(None));
        assert_eq!(parse(r#"{"n":-1,"op":"load"}"#), Err(None));
        assert_eq!(parse(r#"{"n":99999999999,"op":"load"}"#), Err(None));
        assert_eq!(parse(r#"{"n":"1","op":"load"}"#), Err(None));
        assert_eq!(parse("load"), Err(None));
        assert_eq!(parse(""), Err(None));
    }

    #[test]
    fn anfragen_zu_karteikarten() {
        assert_eq!(
            parse(r#"{"n":1,"op":"put","id":"e20261123-a1_B","data":"{\"d\":\"2026-11-23\",\"s\":540}"}"#),
            Ok(Call { n: 1, op: Op::Put { id: "e20261123-a1_B".into(), data: "{\"d\":\"2026-11-23\",\"s\":540}".into() } })
        );
        assert_eq!(parse(r#"{"n":2,"op":"remove","id":"p-1"}"#), Ok(Call { n: 2, op: Op::Remove("p-1".into()) }));
        assert_eq!(parse(r#"{"n":3,"op":"list"}"#), Ok(Call { n: 3, op: Op::List { after: String::new(), limit: LIST_MAX } }));
        assert_eq!(
            parse(r#"{"n":3,"op":"list","after":"e1","limit":20}"#),
            Ok(Call { n: 3, op: Op::List { after: "e1".into(), limit: 20 } })
        );
        // Die Menge je Antwort bleibt in Grenzen, gleich was die Seite verlangt.
        assert_eq!(parse(r#"{"n":3,"op":"list","after":"","limit":0}"#), Ok(Call { n: 3, op: Op::List { after: String::new(), limit: 1 } }));
        assert_eq!(
            parse(r#"{"n":3,"op":"list","limit":99999999}"#),
            Ok(Call { n: 3, op: Op::List { after: String::new(), limit: LIST_MAX } })
        );
        // Kennungen: kurz, nur Buchstaben, Ziffern, „-“ und „_“.
        let long = "a".repeat(RECORD_ID_MAX + 1);
        for id in ["", " ", "a b", "a/b", "../x", "ä", "a'b", "a%", long.as_str()] {
            let put = json!({ "n": 4, "op": "put", "id": id, "data": "{}" }).to_string();
            assert_eq!(parse(&put), Err(Some(4)), "{id}");
            let remove = json!({ "n": 4, "op": "remove", "id": id }).to_string();
            assert_eq!(parse(&remove), Err(Some(4)), "{id}");
        }
        let odd = json!({ "n": 4, "op": "list", "after": "a b" }).to_string();
        assert_eq!(parse(&odd), Err(Some(4)));
        assert_eq!(parse(r#"{"n":4,"op":"list","after":7}"#), Err(Some(4)));
        assert_eq!(parse(r#"{"n":4,"op":"list","limit":"viele"}"#), Err(Some(4)));
        assert_eq!(parse(r#"{"n":4,"op":"put","id":7,"data":"{}"}"#), Err(Some(4)));
        // Eine Karte ist ein JSON-Objekt und nicht größer als erlaubt.
        assert_eq!(parse(r#"{"n":5,"op":"put","id":"a","data":"[1]"}"#), Err(Some(5)));
        assert_eq!(parse(r#"{"n":5,"op":"put","id":"a","data":"7"}"#), Err(Some(5)));
        assert_eq!(parse(r#"{"n":5,"op":"put","id":"a"}"#), Err(Some(5)));
        let big = format!("{{\"a\":\"{}\"}}", "x".repeat(RECORD_MAX));
        let body = json!({ "n": 6, "op": "put", "id": "a", "data": big }).to_string();
        assert_eq!(parse(&body), Err(Some(6)));
        let fits = format!("{{\"a\":\"{}\"}}", "x".repeat(RECORD_MAX - 8));
        assert_eq!(fits.len(), RECORD_MAX);
        let body = json!({ "n": 6, "op": "put", "id": "a", "data": fits }).to_string();
        assert!(parse(&body).is_ok());
        // Der Name einer Datei, die die App sichert.
        assert_eq!(parse(r#"{"n":8,"op":"filename","name":"zeiten-2026-11.csv"}"#), Ok(Call { n: 8, op: Op::FileName("zeiten-2026-11.csv".into()) }));
        let long = format!("{}.csv", "a".repeat(80));
        for name in ["", ".csv", "zeiten", "zeiten.", "..csv", "a..csv", "../zeiten.csv", "a/b.csv", "a b.csv", "zeiten.csv\0", "ä.csv", long.as_str()] {
            let body = json!({ "n": 8, "op": "filename", "name": name }).to_string();
            assert_eq!(parse(&body), Err(Some(8)), "{name}");
        }
        assert_eq!(parse(r#"{"n":8,"op":"filename"}"#), Err(Some(8)));
        // Eine volle Antwort bleibt weit unter dem, was eine Skriptzeile verträgt,
        // und eine Karte passt samt Verpackung durch den Kanal der Seiten.
        assert!(LIST_BYTES >= RECORD_MAX && LIST_BYTES <= 1_000_000);
        assert!(RECORD_MAX * 2 + RECORD_ID_MAX + 64 <= 4096);
    }

    #[test]
    fn standort_wird_gerundet_und_geprueft() {
        assert_eq!(position(52.520008, 13.404954), Some((52.52, 13.4)));
        assert_eq!(position(-33.86785, 151.20732), Some((-33.87, 151.21)));
        assert_eq!(position(90.0, -180.0), Some((90.0, -180.0)));
        for (latitude, longitude) in [(90.01, 0.0), (0.0, 180.5), (f64::NAN, 0.0), (0.0, f64::INFINITY), (-91.0, 0.0)] {
            assert_eq!(position(latitude, longitude), None);
        }
    }

    #[test]
    fn antwort_traegt_die_nummer_der_anfrage() {
        let before = "if (window.location.href === \"zunder://app-weather/\" && typeof window.__zunderApp === 'function') { window.__zunderApp(";
        let after = "); }";
        let inner = |line: &str| -> Value {
            assert!(line.starts_with(before), "{line}");
            assert!(line.ends_with(after), "{line}");
            serde_json::from_str(&line[before.len()..line.len() - after.len()]).unwrap()
        };
        let value = inner(&reply("weather", 7, json!({ "state": "ok", "lat": 52.52 })));
        assert_eq!(value["n"], 7);
        assert_eq!(value["state"], "ok");
        // Text aus dem Zettel einer App bleibt Text – auch mit Anführungszeichen und Zeilenwechseln.
        let value = inner(&reply("weather", 1, json!({ "data": "{\"a\":\"x\\\"); alert(1); (\\\"\"}\n" })));
        assert_eq!(value["data"], "{\"a\":\"x\\\"); alert(1); (\\\"\"}\n");
        // Jede App bekommt ihre Antwort nur unter ihrer eigenen Adresse.
        assert!(reply("time", 1, json!({})).starts_with("if (window.location.href === \"zunder://app-time/\" && "));
    }

    /// Die Schlüssel der Texte, die ein Skript benutzt: `S.name` und `text('name'`.
    fn text_keys(script: &str) -> std::collections::BTreeSet<String> {
        let mut keys = std::collections::BTreeSet::new();
        for marker in ["S.", "text('"] {
            for (at, _) in script.match_indices(marker) {
                // „S.“ nur als eigener Name – nicht das Ende eines längeren.
                let alone = script[..at].chars().next_back().is_none_or(|before| !(before.is_ascii_alphanumeric() || before == '_' || before == '$'));
                let key: String = script[at + marker.len()..].chars().take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_').collect();
                if alone && key.contains('_') {
                    keys.insert(key);
                }
            }
        }
        keys
    }

    #[test]
    fn jeder_text_einer_app_ist_uebersetzt() {
        let scripts = [
            ("app-weather.js", include_str!("../ui/app-weather.js")),
            ("app-time.js", include_str!("../ui/app-time.js")),
        ];
        for (name, script) in scripts {
            let keys = text_keys(script);
            assert!(keys.len() > 20, "{name}: {} Schlüssel gefunden", keys.len());
            for key in keys {
                assert!(!crate::i18n::t(&key).is_empty(), "{name}: Text fehlt für {key}");
            }
        }
        // Die Suche selbst: findet beide Schreibweisen, nichts Halbes.
        let found = text_keys("x = S.tm_today; text('tm_of', {}); CSS.supports_x; S.close; S.tm_a + S.tm_b9");
        assert_eq!(found.into_iter().collect::<Vec<_>>(), ["tm_a", "tm_b9", "tm_of", "tm_today"]);
    }

    #[test]
    fn jede_app_ist_vollstaendig_eingetragen() {
        let mut ids = std::collections::HashSet::new();
        for app in APPS {
            assert!(ids.insert(app.id), "doppelt: {}", app.id);
            assert!(app.id.chars().all(|c| c.is_ascii_lowercase()), "{}", app.id);
            assert!(app.page.ends_with(".html"));
            assert!(app.files.iter().all(|file| !file.ends_with(".html")));
            assert!(!crate::i18n::t(app.name).is_empty(), "Name fehlt: {}", app.name);
            assert!(!crate::i18n::t(app.about).is_empty(), "Beschreibung fehlt: {}", app.about);
            // Im Menü steht sie unter „app_<id>“ mit dem Text „m_app_<id>“.
            assert!(!crate::i18n::t(&format!("m_app_{}", app.id)).is_empty(), "Menütext fehlt: {}", app.id);
        }
        // Die Zeiterfassung: ohne Netz, ohne Standort, mit Karteikarten. Das Wetter hat keine.
        let time = by_id("time").unwrap();
        assert!(time.connect.is_empty() && time.images.is_empty() && !time.location && time.records > 0);
        assert_eq!(by_id("weather").unwrap().records, 0);
        // Dateien sichert nur die Zeiterfassung – als Tabelle, sonst nichts.
        assert!(may_export(time, "zeiten-2026-11.csv") && may_export(time, "ZEITEN.CSV"));
        for name in ["zeiten.html", "zeiten.csv.app", "zeiten.command", "zeiten", "csv"] {
            assert!(!may_export(time, name), "{name}");
        }
        assert!(!may_export(by_id("weather").unwrap(), "wetter.csv"));
        assert_eq!(own_file("blob:zunder://app-time/9f0119cf-1b94-4b1a-8bfd-43ed79e36fdd").map(|app| app.id), Some("time"));
        for url in ["blob:https://example.com/9f01", "zunder://app-time/", "blob:zunder://newtab/9f01", "blob:zunder://app-nix/1", "https://example.com/zeiten.csv", ""] {
            assert!(own_file(url).is_none(), "{url}");
        }
        for app in APPS {
            assert!(app.exports.iter().all(|ending| !ending.is_empty() && ending.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())), "{}", app.id);
        }
        assert_eq!(url_of("time"), "zunder://app-time/");
        assert_eq!(typed("zunder://app-time").as_deref(), Some("zunder://app-time/"));
    }
}
