//! Erzeugt die WebViews eines Fensters: eine für den Rahmen (Tabs, Adressleiste,
//! Seitenleisten) und eine je Tab für den Seiteninhalt.
//!
//! Die Rückrufe der WebViews verändern nie selbst den Zustand der App. Sie
//! schicken nur Ereignisse in die Ereignisschleife – dort wird alles
//! nacheinander und an einer Stelle verarbeitet.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use objc2::rc::Retained;
use objc2_web_kit::{WKWebView, WKWebViewConfiguration, WKWebsiteDataStore};
use tao::event_loop::EventLoopProxy;
use tao::window::{Window, WindowId};
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::http::Request;
use wry::{
    NewWindowResponse, PageLoadEvent, PermissionResponse, Rect, WebView, WebViewBuilder,
    WebViewBuilderExtDarwin, WebViewBuilderExtMacos, WebViewExtMacOS,
};

use crate::app::{TabId, UserEvent};
use crate::assets::{self, PageCtx, Role};
use crate::native;

/// Skript, das in jeder Seite (nur im Hauptrahmen) mitläuft: meldet das
/// Seiten-Icon und Adresswechsel ohne Neuladen.
const INJECT_JS: &str = include_str!("../ui/inject.js");

/// Ereignisse aus einem Inhalts-Tab.
#[derive(Debug)]
pub enum PageEvent {
    Title(String),
    /// Die neue Seite wurde übernommen (Adresse steht fest).
    Committed(String),
    Finished(String),
    /// Nachricht des mitlaufenden Skripts. Kommt aus der Seite und ist damit
    /// grundsätzlich nicht vertrauenswürdig. `origin` stammt dagegen von WebKit.
    /// Angenommen wird sie in `native::bridge_received` – nicht über wry.
    Ping { origin: String, body: String, main: bool },
    /// Ein Pop-up wurde erzeugt und wartet in `Shared::popups`.
    Popup,
    OpenInNewTab(String),
    /// Die Seite will ein anderes Programm öffnen (mailto:, tel:, …).
    External(String),
    DownloadStarted { url: String, path: PathBuf },
    /// Ein Download wurde nicht zugelassen (zu viele auf einmal, unzulässiges Ziel).
    DownloadRefused,
    DownloadFinished { url: String, ok: bool },
    Crashed,
}

/// Ein von einer Seite geöffnetes Fenster, das als Tab übernommen wird.
pub struct Popup {
    pub window: WindowId,
    pub opener: TabId,
    pub tab: TabId,
    pub webview: WebView,
}

/// Was alle WebViews eines Fensters gemeinsam brauchen.
pub struct Shared {
    pub proxy: EventLoopProxy<UserEvent>,
    pub window: Rc<Window>,
    pub window_id: WindowId,
    pub private: bool,
    /// Stealth-Fenster: Jeder Tab bekommt seinen eigenen flüchtigen Datenspeicher
    /// (siehe `stealth.rs`) – und geht durch den Tunnel, wenn es einen gibt.
    pub stealth: bool,
    pub route: Option<crate::stealth::Route>,
    /// Gemeinsamer flüchtiger Datenspeicher eines privaten Fensters.
    pub store: Option<Retained<WKWebsiteDataStore>>,
    pub popups: Rc<RefCell<Vec<Popup>>>,
    pub ids: Rc<Cell<TabId>>,
    pub newtab: Arc<Mutex<String>>,
    pub ua_suffix: String,
    /// Quelltexte, die gerade in einem Tab angezeigt werden.
    pub sources: assets::Sources,
    /// Vorschaubilder der Tabs für die Übersicht.
    pub thumbs: assets::Thumbs,
    /// Dieses Fenster beginnt mit dem Ladebild (beim Start der App).
    pub splash: bool,
}

impl Shared {
    pub fn next_id(&self) -> TabId {
        let id = self.ids.get() + 1;
        self.ids.set(id);
        id
    }

    fn page_ctx(&self, role: Role) -> PageCtx {
        PageCtx {
            private: self.private,
            stealth: self.stealth,
            tor: self.route.as_ref().map(crate::stealth::Route::label),
            newtab: self.newtab.clone(),
            sources: self.sources.clone(),
            thumbs: self.thumbs.clone(),
            splash: self.splash && role == Role::Chrome,
            role,
        }
    }
}

pub fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
    Rect {
        position: LogicalPosition::new(x, y).into(),
        size: LogicalSize::new(width.max(1.0), height.max(1.0)).into(),
    }
}

/// Schemata, die im Tab selbst geladen werden. Alles andere (mailto:, tel:, …)
/// gehört einem anderen Programm.
fn loads_in_tab(url: &str) -> bool {
    let scheme = url.split(':').next().unwrap_or("").to_ascii_lowercase();
    matches!(
        scheme.as_str(),
        "http" | "https" | "about" | "blob" | "data" | "file" | "zunder" | "javascript" | "applewebdata"
    )
}

/// In einem Bau mit Prüfhilfe landen Downloads mit `ZUNDER_DOWNLOAD_DIR` in
/// einem anderen Ordner als „Downloads“ – vorhandene Dateien werden dabei nie
/// überschrieben.
fn redirect_download(path: &mut PathBuf) {
    let Some(dir) = crate::dev::build_var_os("ZUNDER_DOWNLOAD_DIR").map(PathBuf::from) else {
        return;
    };
    let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return;
    };
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), format!(".{ext}")),
        _ => (name.clone(), String::new()),
    };
    let mut target = dir.join(&name);
    let mut counter = 1;
    while target.exists() {
        target = dir.join(format!("{stem} ({counter}){extension}"));
        counter += 1;
    }
    *path = target;
}

/// Liegt das Ziel eines Downloads dort, wo es hingehört?
fn download_target_ok(path: &std::path::Path) -> bool {
    let folder = crate::dev::build_var_os("ZUNDER_DOWNLOAD_DIR")
        .map(PathBuf::from)
        .or_else(dirs::download_dir);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let plain = !name.is_empty() && !name.starts_with('.') && !name.contains(['/', '\\', '\0']);
    plain && folder.is_some_and(|folder| path.parent() == Some(folder.as_path()))
}

/// Die Rahmen-WebView eines Fensters.
pub fn build_chrome(shared: &Rc<Shared>, bounds: Rect) -> wry::Result<WebView> {
    let proxy = shared.proxy.clone();
    let window_id = shared.window_id;
    let ctx = shared.page_ctx(Role::Chrome);
    native::expect_internal("zunder://ui/index.html");

    WebViewBuilder::new()
        .with_bounds(bounds)
        .with_transparent(true)
        // In privaten Fenstern lädt auch der Rahmen (Seiten-Icons) nichts dauerhaft.
        .with_incognito(shared.private)
        .with_custom_protocol("zunder".into(), move |_id, request| {
            assets::serve(&request, &ctx)
        })
        .with_url("zunder://ui/index.html")
        .with_ipc_handler(move |request: Request<String>| {
            // Befehle nimmt der Browser nur von seiner eigenen Oberfläche an.
            let uri = request.uri();
            if uri.scheme_str() == Some("zunder") && uri.host() == Some("ui") {
                let _ = proxy.send_event(UserEvent::Ui {
                    window: window_id,
                    from: Role::Chrome,
                    body: request.into_body(),
                });
            }
        })
        .with_navigation_handler(|url| url.starts_with("zunder://ui/"))
        .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
        .with_accept_first_mouse(true)
        .with_back_forward_navigation_gestures(false)
        .with_devtools(crate::dev::enabled())
        .build_as_child(&*shared.window)
}

/// Die Seitenleiste des Assistenten. Wie der Rahmen zeigt sie nur eingebaute
/// Oberfläche und darf dem Browser Befehle schicken; Webseiten kommen nie
/// hinein. Was sie anzeigt, bleibt im Arbeitsspeicher.
pub fn build_assistant(shared: &Rc<Shared>, bounds: Rect) -> wry::Result<WebView> {
    let proxy = shared.proxy.clone();
    let window_id = shared.window_id;
    let ctx = shared.page_ctx(Role::Assistant);
    native::expect_internal("zunder://ui/assistant.html");

    WebViewBuilder::new()
        .with_bounds(bounds)
        // Durchsichtig, bis die Oberfläche geladen ist – so blitzt im dunklen
        // Modus keine weiße Fläche auf.
        .with_transparent(true)
        .with_incognito(true)
        .with_custom_protocol("zunder".into(), move |_id, request| {
            assets::serve(&request, &ctx)
        })
        .with_url("zunder://ui/assistant.html")
        .with_ipc_handler(move |request: Request<String>| {
            let uri = request.uri();
            if uri.scheme_str() == Some("zunder") && uri.host() == Some("ui") {
                let _ = proxy.send_event(UserEvent::Ui {
                    window: window_id,
                    from: Role::Assistant,
                    body: request.into_body(),
                });
            }
        })
        .with_navigation_handler(|url| url.starts_with("zunder://ui/"))
        .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
        .with_accept_first_mouse(true)
        .with_back_forward_navigation_gestures(false)
        .with_devtools(crate::dev::enabled())
        .build_as_child(&*shared.window)
}

/// Die Sprechblase des Passwort-Tresors: ein kleines Stück eigener Oberfläche,
/// das über der Seite schwebt (Vorschläge am Anmeldefeld, Frage „sichern?“).
/// Anders als ein Menü des Rahmens verdeckt sie nur ihre eigene Fläche – die
/// Seite bleibt daneben bedienbar. Webseiten kommen nie hinein.
pub fn build_bubble(shared: &Rc<Shared>, bounds: Rect) -> wry::Result<WebView> {
    let proxy = shared.proxy.clone();
    let window_id = shared.window_id;
    let ctx = shared.page_ctx(Role::Bubble);
    native::expect_internal("zunder://ui/bubble.html");

    WebViewBuilder::new()
        .with_bounds(bounds)
        .with_visible(false)
        .with_transparent(true)
        .with_incognito(true)
        .with_custom_protocol("zunder".into(), move |_id, request| {
            assets::serve(&request, &ctx)
        })
        .with_url("zunder://ui/bubble.html")
        .with_ipc_handler(move |request: Request<String>| {
            let uri = request.uri();
            if uri.scheme_str() == Some("zunder") && uri.host() == Some("ui") {
                let _ = proxy.send_event(UserEvent::Ui {
                    window: window_id,
                    from: Role::Bubble,
                    body: request.into_body(),
                });
            }
        })
        .with_navigation_handler(|url| url.starts_with("zunder://ui/"))
        .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
        .with_accept_first_mouse(true)
        .with_back_forward_navigation_gestures(false)
        .with_devtools(crate::dev::enabled())
        .build_as_child(&*shared.window)
}

/// Eine WebView für den Inhalt eines Tabs. `config` ist nur bei Pop-ups
/// gesetzt – dann stammt sie von WebKit und muss unverändert verwendet werden.
pub fn build_content(
    shared: &Rc<Shared>,
    tab: TabId,
    bounds: Rect,
    url: Option<&str>,
    config: Option<Retained<WKWebViewConfiguration>>,
) -> wry::Result<WebView> {
    // Ließ sich der Tunnel für diesen Tab nicht einrichten, lädt er gar nichts
    // – lieber keine Seite als eine, die am Tunnel vorbeigeht.
    let unrouted = std::cell::Cell::new(false);
    let config = config.unwrap_or_else(|| {
        if !shared.stealth {
            return native::content_config(shared.store.as_deref(), &shared.ua_suffix, false);
        }
        // Stealth: eine eigene Insel je Tab – und ihr Verkehr durch den Tunnel.
        let island = native::private_store();
        if let Some(route) = &shared.route {
            let routed = native::route_store(&island, &route.host, route.port, &format!("zunder-{tab}"));
            unrouted.set(!routed);
        }
        native::content_config(Some(&island), &shared.ua_suffix, true)
    });

    let proxy = shared.proxy.clone();
    let window_id = shared.window_id;
    let send = move |ev: PageEvent| {
        let _ = proxy.send_event(UserEvent::Page {
            window: window_id,
            tab,
            ev,
        });
    };
    let ctx = shared.page_ctx(Role::Content);

    let on_title = send.clone();
    let on_load = send.clone();
    let on_nav = send.clone();
    let on_new_window = send.clone();
    let on_download = send.clone();
    let on_download_done = send.clone();
    let on_crash = send.clone();
    let popup_shared = shared.clone();

    let mut builder = WebViewBuilder::new()
        .with_bounds(bounds)
        .with_visible(false)
        .with_webview_configuration(config)
        .with_custom_protocol("zunder".into(), move |_id, request| {
            assets::serve(&request, &ctx)
        })
        .with_initialization_script(INJECT_JS)
        .with_document_title_changed_handler(move |title| on_title(PageEvent::Title(title)))
        .with_on_page_load_handler(move |event, url| {
            on_load(match event {
                PageLoadEvent::Started => PageEvent::Committed(url),
                PageLoadEvent::Finished => PageEvent::Finished(url),
            })
        })
        .with_navigation_handler(move |url| {
            if loads_in_tab(&url) {
                true
            } else {
                on_nav(PageEvent::External(url));
                false
            }
        })
        .with_new_window_req_handler(move |url, features| {
            // Das neue Fenster wird ein Tab. Damit Anmelde-Pop-ups mit der
            // öffnenden Seite sprechen können, muss die WebView sofort und mit
            // der von WebKit gelieferten Konfiguration entstehen.
            let id = popup_shared.next_id();
            let config = features.opener.target_configuration.clone();
            native::own_controller(&config, popup_shared.stealth);
            match build_content(&popup_shared, id, rect(0.0, 0.0, 800.0, 600.0), None, Some(config)) {
                Ok(webview) => {
                    // Die Schutzlisten sofort – der erste Abruf des neuen Tabs läuft gleich los.
                    crate::blocker::equip(&webview, &url, true);
                    let raw: Retained<WKWebView> = Retained::into_super(webview.webview());
                    popup_shared.popups.borrow_mut().push(Popup {
                        window: window_id,
                        opener: tab,
                        tab: id,
                        webview,
                    });
                    on_new_window(PageEvent::Popup);
                    NewWindowResponse::Create { webview: raw }
                }
                Err(_) => {
                    on_new_window(PageEvent::OpenInNewTab(url));
                    NewWindowResponse::Deny
                }
            }
        })
        .with_download_started_handler(move |url, path| {
            redirect_download(path);
            // Eine Datei landet immer unmittelbar im Download-Ordner, nie als
            // versteckte Datei – gleich, welchen Namen die Seite vorschlägt. Und
            // keine Seite lädt Dateien in Serie.
            if !download_target_ok(path) || !native::download_slot(tab) {
                on_download(PageEvent::DownloadRefused);
                return false;
            }
            on_download(PageEvent::DownloadStarted {
                url,
                path: path.clone(),
            });
            true
        })
        .with_download_completed_handler(move |url, _path, ok| {
            on_download_done(PageEvent::DownloadFinished { url, ok })
        })
        // Kamera und Mikrofon: WebKit fragt je Seite nach, nichts wird still erlaubt.
        .with_permission_handler(|_kind| PermissionResponse::Default)
        .with_on_web_content_process_terminate_handler(move || on_crash(PageEvent::Crashed))
        .with_back_forward_navigation_gestures(true)
        .with_autoplay(false)
        .with_accept_first_mouse(true)
        // Keine Link-Vorschau beim kräftigen Klick: Sie lädt die Adresse in
        // einer eigenen Ansicht – vorbei an den Prüfungen beim Navigieren, an
        // den Listen und, im Stealth-Fenster, am Tunnel.
        .with_allow_link_preview(false)
        // „Element untersuchen“ gibt es nur, wenn es in den Einstellungen
        // eingeschaltet ist: Mit ihm könnten auch Safaris Entwicklerwerkzeuge
        // in jede Seite hineinsehen.
        .with_devtools(native::guards().devtools);

    if let Some(url) = url {
        builder = builder.with_url(url);
    }
    let webview = builder.build_as_child(&*shared.window)?;
    if shared.stealth {
        // Noch bevor der erste Abruf entschieden wird: Für diesen Tab gelten
        // alle Schutzschalter und der Blocker, gleich was sonst eingestellt ist.
        let key = native::key(&webview);
        native::mark_stealth(key);
        if unrouted.get() {
            native::mark_unrouted(key);
        }
        crate::blocker::force(key);
    }
    Ok(webview)
}
