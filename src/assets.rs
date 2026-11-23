//! Liefert die eingebaute Oberfläche und die internen Seiten über das
//! Schema `zunder://` aus.
//!
//!   zunder://ui/…      Symbolleiste, Tabs, Seitenleisten, der Assistent und die
//!                      Sprechblase des Passwort-Tresors (nur diese drei eigenen
//!                      Oberflächen können dem Browser Befehle schicken)
//!   zunder://newtab/   Startseite eines neuen Tabs (samt ihren Hintergründen unter wp/)
//!   zunder://error/    Fehlerseite
//!   zunder://app-…/    eingebaute Apps (`apps.rs`) – jede mit ihrer eigenen
//!                      Schutzregel, die sagt, wohin sie ins Netz darf
//!
//! Interne Seiten in einem Tab haben keinerlei Sonderrechte: Sie bekommen ihre
//! Daten beim Ausliefern mit und können dem Browser nichts befehlen. (Eine App
//! kann ihn um das bitten, was für sie eingetragen ist – siehe `apps.rs`.)
//!
//! Jede WebView bekommt nur, was zu ihrer Aufgabe gehört ([`Role`]): Ein Tab
//! erhält weder die Symbolleiste noch den Assistenten, die Oberfläche keine
//! Seiten für Tabs. Dass ein Tab eine interne Seite überhaupt nur lädt, wenn
//! der Browser selbst das will, entscheidet `native::nav_policy`.

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use wry::http::{Request, Response};

use crate::i18n;
use crate::omnibox::ENGINES;

macro_rules! ui {
    ($name:literal) => {
        include_bytes!(concat!("../ui/", $name)) as &'static [u8]
    };
}

/// Wofür eine WebView da ist – danach richtet sich, was sie ausgeliefert bekommt.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    /// Der Rahmen eines Fensters: Tabs, Adressleiste, Seitenleisten.
    Chrome,
    /// Die Seitenleiste des Assistenten.
    Assistant,
    /// Die Sprechblase des Passwort-Tresors.
    Bubble,
    /// Ein Tab: Startseite und Fehlerseite – sonst nichts Eingebautes.
    Content,
}

/// Dateien, die Startseite und Fehlerseite nachladen.
const CONTENT_FILES: &[&str] = &["pages.css", "newtab.js", "error.js", "source.js", "appicon.png"];

/// Quelltexte, die gerade in einem Tab angezeigt werden: Nummer → (Adresse,
/// Text, gekürzt?). Sie liegen nur im Arbeitsspeicher.
pub type Sources = Arc<Mutex<std::collections::HashMap<u64, (String, String, bool)>>>;

/// Was eine WebView beim Ausliefern wissen muss.
/// Vorschaubilder der Tabs für die Übersicht: Tab → JPEG. Nur im Arbeitsspeicher.
pub type Thumbs = Arc<Mutex<std::collections::HashMap<u64, Vec<u8>>>>;

#[derive(Clone)]
pub struct PageCtx {
    pub private: bool,
    /// Stealth-Fenster – und, wenn es über Tor läuft, über welchen Zugang.
    pub stealth: bool,
    pub tor: Option<String>,
    pub role: Role,
    /// Der Rahmen dieses Fensters beginnt mit dem Ladebild (nur beim Start der App).
    pub splash: bool,
    pub sources: Sources,
    pub thumbs: Thumbs,
    /// Vorbereitete Daten der Startseite (JSON), von der App aktuell gehalten.
    pub newtab: Arc<Mutex<String>>,
}

fn embedded(name: &str) -> Option<&'static [u8]> {
    Some(match name {
        "index.html" => ui!("index.html"),
        "chrome.css" => ui!("chrome.css"),
        "chrome.js" => ui!("chrome.js"),
        "icons.svg" => ui!("icons.svg"),
        "appicon.png" => ui!("appicon.png"),
        "pages.css" => ui!("pages.css"),
        "newtab.html" => ui!("newtab.html"),
        "newtab.js" => ui!("newtab.js"),
        "error.html" => ui!("error.html"),
        "error.js" => ui!("error.js"),
        "source.html" => ui!("source.html"),
        "source.js" => ui!("source.js"),
        "assistant.html" => ui!("assistant.html"),
        "assistant.css" => ui!("assistant.css"),
        "assistant.js" => ui!("assistant.js"),
        "bubble.html" => ui!("bubble.html"),
        "bubble.css" => ui!("bubble.css"),
        "bubble.js" => ui!("bubble.js"),
        "apps.css" => ui!("apps.css"),
        "apps.js" => ui!("apps.js"),
        "app-weather.html" => ui!("app-weather.html"),
        "app-weather.css" => ui!("app-weather.css"),
        "app-weather.js" => ui!("app-weather.js"),
        "app-weather-map.js" => ui!("app-weather-map.js"),
        "app-time.html" => ui!("app-time.html"),
        "app-time.css" => ui!("app-time.css"),
        "app-time.js" => ui!("app-time.js"),
        "app-security.html" => ui!("app-security.html"),
        "app-security.css" => ui!("app-security.css"),
        "app-security.js" => ui!("app-security.js"),
        _ => return None,
    })
}

/// Für die Entwicklung: Mit `ZUNDER_UI_DIR=<ordner>` kommt die Oberfläche
/// frisch von der Platte statt aus dem Programm.
fn dev_dir() -> Option<&'static PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| crate::dev::build_var_os("ZUNDER_UI_DIR").map(PathBuf::from))
        .as_ref()
}

fn load(name: &str) -> Option<Cow<'static, [u8]>> {
    let safe = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
        && !name.contains("..");
    if !safe {
        return None;
    }
    if let Some(dir) = dev_dir() {
        if let Ok(bytes) = std::fs::read(dir.join(name)) {
            return Some(Cow::Owned(bytes));
        }
    }
    embedded(name).map(Cow::Borrowed)
}

fn mime(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
}

/// JSON so einbetten, dass es das umgebende <script>-Element nicht beenden kann.
fn embed_json(json: &str) -> String {
    format!(
        "<script id=\"data\" type=\"application/json\">{}</script>",
        json.replace('<', "\\u003c")
    )
}

fn page(name: &str, data_json: &str) -> Option<Vec<u8>> {
    page_with(name, data_json, "")
}

/// Wie [`page`], mit einer Klasse am `<body>` – so steht das Ladebild schon im
/// allerersten Bild da, statt erst nach dem Start des Skripts.
fn page_with(name: &str, data_json: &str, body_class: &str) -> Option<Vec<u8>> {
    let template = load(name)?;
    let template = String::from_utf8_lossy(&template);
    let icons = load("icons.svg")?;
    let icons = String::from_utf8_lossy(&icons);
    Some(
        template
            .replace("<!--ICONS-->", &icons)
            .replace("<!--DATA-->", &embed_json(data_json))
            .replace("{{boot}}", body_class)
            .replace("{{lang}}", i18n::lang_code())
            .into_bytes(),
    )
}

fn chrome_data(ctx: &PageCtx) -> String {
    let engines: Vec<serde_json::Value> = ENGINES
        .iter()
        .map(|e| serde_json::json!({ "id": e.id, "name": e.name }))
        .collect();
    format!(
        "{{\"strings\":{},\"private\":{},\"stealth\":{},\"tor\":{},\"engines\":{},\"version\":\"{}\"}}",
        i18n::json(),
        ctx.private,
        ctx.stealth,
        ctx.tor.is_some(),
        serde_json::Value::Array(engines),
        env!("CARGO_PKG_VERSION")
    )
}

/// Was Zunders eigene Seiten dürfen: nur Eigenes. Bilder kommen aus dem
/// Programm selbst oder stehen als Text in der Seite (Seiten-Icons) – aus dem
/// Netz lädt die Oberfläche nichts.
const CSP: &str = "default-src 'none'; script-src zunder:; style-src zunder: 'unsafe-inline'; \
                   img-src zunder: data:; media-src zunder:; base-uri 'none'; \
                   form-action 'none'; frame-ancestors 'none'";

fn respond(status: u16, content_type: &str, body: Cow<'static, [u8]>) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(status)
        .header("Content-Type", content_type)
        .header("Cache-Control", "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .header("X-Frame-Options", "DENY")
        .header("Cross-Origin-Resource-Policy", "same-origin")
        .header("Content-Security-Policy", CSP)
        .body(body)
        .unwrap_or_else(|_| Response::new(Cow::Borrowed(&[][..])))
}

/// Wie [`respond`], aber mit der Schutzregel einer eingebauten App statt der
/// allgemeinen: Sie nennt die Dienste, die diese App ansprechen darf.
fn respond_app(app: &crate::apps::App, content_type: &str, body: Cow<'static, [u8]>) -> Response<Cow<'static, [u8]>> {
    let mut response = respond(200, content_type, body);
    if let Ok(rule) = wry::http::HeaderValue::from_str(&crate::apps::csp(app)) {
        response.headers_mut().insert("Content-Security-Policy", rule);
    }
    response
}

fn not_found() -> Response<Cow<'static, [u8]>> {
    respond(404, "text/plain; charset=utf-8", Cow::Borrowed(b"404"))
}

pub fn serve(request: &Request<Vec<u8>>, ctx: &PageCtx) -> Response<Cow<'static, [u8]>> {
    let uri = request.uri();
    serve_path(request, uri.host().unwrap_or(""), uri.path().trim_start_matches('/'), ctx)
}

fn serve_path(request: &Request<Vec<u8>>, host: &str, file: &str, ctx: &PageCtx) -> Response<Cow<'static, [u8]>> {
    let html = |bytes: Option<Vec<u8>>| match bytes {
        Some(bytes) => respond(200, mime("x.html"), Cow::Owned(bytes)),
        None => not_found(),
    };
    let content = ctx.role == Role::Content;

    match (host, file) {
        // Die eigene Oberfläche: jede nur in der WebView, die für sie gebaut wurde.
        ("ui", "index.html") if ctx.role == Role::Chrome => html(page_with(
            "index.html",
            &chrome_data(ctx),
            if ctx.splash { "boot" } else { "" },
        )),
        // Die Glut-Szene hinter dem Ladebild – dieselben Dateien wie die
        // Live-Hintergründe der Startseite, nur für den Rahmen.
        ("ui", file) if ctx.role == Role::Chrome && file.starts_with("wp/") => {
            crate::wallpapers::serve(request, &file[3..])
        }
        ("ui", "assistant.html") if ctx.role == Role::Assistant => {
            html(page("assistant.html", &chrome_data(ctx)))
        }
        ("ui", "bubble.html") if ctx.role == Role::Bubble => html(page("bubble.html", &chrome_data(ctx))),
        ("newtab", "") if content => {
            let data = ctx.newtab.lock().map(|d| d.clone()).unwrap_or_default();
            let data = if ctx.private || data.is_empty() {
                "{}".to_string()
            } else {
                data
            };
            let json = format!(
                "{{\"strings\":{},\"private\":{},\"stealth\":{},\"tor\":{},\"apps\":{},\"data\":{}}}",
                i18n::json(),
                ctx.private,
                ctx.stealth,
                serde_json::Value::from(ctx.tor.clone()),
                crate::apps::list(),
                data
            );
            html(page("newtab.html", &json))
        }
        // Eine eingebaute App: ihre Seite …
        (host, "") if content && crate::apps::by_host(host).is_some() => {
            let Some(app) = crate::apps::by_host(host) else {
                return not_found();
            };
            let json = format!(
                "{{\"strings\":{},\"app\":\"{}\",\"private\":{},\"stealth\":{},\"lang\":\"{}\",\"region\":\"{}\"}}",
                i18n::json(),
                app.id,
                ctx.private,
                ctx.stealth,
                i18n::lang_code(),
                crate::apps::region_of(&crate::native::preferred_language())
            );
            match page(app.page, &json) {
                Some(bytes) => respond_app(app, mime("x.html"), Cow::Owned(bytes)),
                None => not_found(),
            }
        }
        // … und genau die Dateien, die zu ihr gehören.
        (host, name) if content && crate::apps::by_host(host).is_some_and(|app| app.files.contains(&name)) => {
            match (crate::apps::by_host(host), load(name)) {
                (Some(app), Some(bytes)) => respond_app(app, mime(name), bytes),
                _ => not_found(),
            }
        }
        ("error", "") if content => {
            let json = format!("{{\"strings\":{}}}", i18n::json());
            html(page("error.html", &json))
        }
        // Quelltext einer Seite: Welcher, sagt die Nummer in der Adresse.
        ("source", "") if content => {
            let id = request
                .uri()
                .query()
                .and_then(|query| url::form_urlencoded::parse(query.as_bytes()).find(|(key, _)| key == "id"))
                .and_then(|(_, value)| value.parse::<u64>().ok());
            let entry = id.and_then(|id| ctx.sources.lock().ok().and_then(|sources| sources.get(&id).cloned()));
            let (url, text, truncated) = entry.unwrap_or_default();
            let json = serde_json::json!({
                "url": url,
                "host": crate::shield::host_of(&url),
                "text": text,
                "truncated": truncated,
            });
            let json = format!("{{\"strings\":{},{}", i18n::json(), &json.to_string()[1..]);
            html(page("source.html", &json))
        }
        // Hintergründe der Startseite – nur für sie und nie in privaten Fenstern.
        ("newtab", file) if content && file.starts_with("wp/") && !ctx.private => {
            crate::wallpapers::serve(request, &file[3..])
        }
        // Vorschaubilder der Tabs – nur für den Rahmen, der die Übersicht zeigt.
        ("ui", name) if ctx.role == Role::Chrome && name.starts_with("thumb/") => {
            let bytes = name[6..]
                .parse::<u64>()
                .ok()
                .and_then(|tab| ctx.thumbs.lock().ok().and_then(|thumbs| thumbs.get(&tab).cloned()));
            match bytes {
                Some(bytes) => respond(200, "image/jpeg", Cow::Owned(bytes)),
                None => not_found(),
            }
        }
        // Statische Dateien; die HTML-Vorlagen selbst gibt es nur über die Wege oben.
        ("ui", name) if !content && !name.ends_with(".html") => match load(name) {
            Some(bytes) => respond(200, mime(name), bytes),
            None => not_found(),
        },
        ("newtab" | "error" | "source", name) if content && CONTENT_FILES.contains(&name) => match load(name) {
            Some(bytes) => respond(200, mime(name), bytes),
            None => not_found(),
        },
        _ => not_found(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(role: Role, private: bool, url: &str) -> u16 {
        let ctx = PageCtx {
            private,
            stealth: false,
            tor: None,
            role,
            splash: false,
            newtab: Arc::new(Mutex::new(String::new())),
            sources: Default::default(),
            thumbs: Default::default(),
        };
        let request = Request::builder().uri(url).body(Vec::new()).unwrap();
        serve(&request, &ctx).status().as_u16()
    }

    #[test]
    fn every_surface_gets_only_its_own_pages() {
        // Ein Tab bekommt Startseite und Fehlerseite samt ihren Dateien …
        for url in ["zunder://newtab/", "zunder://error/?u=x", "zunder://newtab/pages.css", "zunder://error/error.js", "zunder://newtab/appicon.png"] {
            assert_eq!(status(Role::Content, false, url), 200, "{url}");
        }
        // … aber nichts von der Oberfläche des Browsers.
        for url in [
            "zunder://ui/index.html",
            "zunder://ui/assistant.html",
            "zunder://ui/bubble.html",
            "zunder://ui/chrome.js",
            "zunder://ui/assistant.js",
            "zunder://ui/icons.svg",
            "zunder://newtab/chrome.js",
            "zunder://error/bubble.js",
            "zunder://newtab/newtab.html",
            "zunder://error/../ui/index.html",
        ] {
            assert_eq!(status(Role::Content, false, url), 404, "{url}");
        }
        // Jede eigene Oberfläche bekommt genau ihre Seite, keine für Tabs.
        assert_eq!(status(Role::Chrome, false, "zunder://ui/index.html"), 200);
        assert_eq!(status(Role::Chrome, false, "zunder://ui/chrome.js"), 200);
        assert_eq!(status(Role::Chrome, false, "zunder://ui/assistant.html"), 404);
        assert_eq!(status(Role::Chrome, false, "zunder://newtab/"), 404);
        assert_eq!(status(Role::Chrome, false, "zunder://error/"), 404);
        assert_eq!(status(Role::Assistant, false, "zunder://ui/assistant.html"), 200);
        assert_eq!(status(Role::Assistant, false, "zunder://ui/index.html"), 404);
        assert_eq!(status(Role::Bubble, false, "zunder://ui/bubble.html"), 200);
        assert_eq!(status(Role::Bubble, false, "zunder://ui/index.html"), 404);
    }

    fn response(role: Role, url: &str) -> Response<Cow<'static, [u8]>> {
        let ctx = PageCtx {
            private: false,
            stealth: false,
            tor: None,
            role,
            splash: false,
            newtab: Arc::new(Mutex::new(String::new())),
            sources: Default::default(),
            thumbs: Default::default(),
        };
        let request = Request::builder().uri(url).body(Vec::new()).unwrap();
        serve(&request, &ctx)
    }

    #[test]
    fn eine_app_bekommt_ihre_seite_ihre_dateien_und_ihre_schutzregel() {
        for app in crate::apps::APPS {
            let root = crate::apps::url_of(app.id);
            let page = response(Role::Content, &root);
            assert_eq!(page.status().as_u16(), 200, "{root}");
            // Ihre eigene Schutzregel – mit genau ihren Diensten.
            let rule = page.headers().get("Content-Security-Policy").unwrap().to_str().unwrap();
            assert_eq!(rule, crate::apps::csp(app));
            assert_eq!(page.headers().get("X-Frame-Options").unwrap(), "DENY");
            // Jede Datei, die die Seite nachlädt, ist da – und trägt dieselbe Regel.
            let html = String::from_utf8_lossy(page.body()).to_string();
            for file in app.files {
                assert!(html.contains(&format!("\"{file}\"")), "{file} wird von der Seite nicht geladen");
                let got = response(Role::Content, &format!("{root}{file}"));
                assert_eq!(got.status().as_u16(), 200, "{file}");
                assert_eq!(got.headers().get("Content-Security-Policy").unwrap().to_str().unwrap(), crate::apps::csp(app));
            }
            // Die Vorlage selbst, fremde Dateien und die Oberfläche des Browsers gibt es dort nicht.
            for other in [app.page, "chrome.js", "newtab.js", "index.html", "icons.svg", "../ui/chrome.js"] {
                assert_eq!(response(Role::Content, &format!("{root}{other}")).status().as_u16(), 404, "{other}");
            }
            // Und nur ein Tab bekommt sie – nicht der Rahmen, der Assistent oder die Sprechblase.
            for role in [Role::Chrome, Role::Assistant, Role::Bubble] {
                assert_eq!(response(role, &root).status().as_u16(), 404);
            }
        }
        // Eine App, die es nicht gibt, gibt es nicht.
        assert_eq!(response(Role::Content, "zunder://app-nix/").status().as_u16(), 404);
        // Die Startseite nennt die Apps mit genau ihren Adressen.
        let start = String::from_utf8_lossy(response(Role::Content, "zunder://newtab/").body()).to_string();
        assert!(start.contains("zunder://app-weather/"));
        // Die übrigen eigenen Seiten behalten die allgemeine Regel: nichts aus dem Netz.
        let rule = response(Role::Content, "zunder://newtab/").headers().get("Content-Security-Policy").unwrap().to_str().unwrap().to_string();
        assert_eq!(rule, CSP);
        assert!(!rule.contains("https:"));
    }
}
