//! Befehle: alles, was die Oberfläche oder die Menüleiste auslösen kann.

use serde_json::{json, Value};
use tao::window::WindowId;

use crate::app::{App, NoticeAction, Target, UserEvent, CHROME_HEIGHT, ZOOM_STEPS};
use crate::i18n::t;
use crate::native;
use crate::omnibox::{self, ENGINES};

/// Suche in der Seite. Läuft in der Seite und liefert [Treffer-Nr., Anzahl].
const FIND_JS: &str = include_str!("../ui/find.js");

/// Darf diese Fläche diesen Befehl schicken? Der Rahmen darf alles; die
/// Seitenleiste des Assistenten nur, was den Assistenten betrifft; die
/// Sprechblase des Passwort-Tresors nur ihre eigenen Antworten. Wer eine der
/// kleinen Flächen überlistete, käme so nicht an Einstellungen, Passwortliste
/// oder Navigation.
pub fn allowed_from(from: crate::assets::Role, body: &str) -> bool {
    use crate::assets::Role;
    let command = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|message| message.get("c").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default();
    match from {
        Role::Chrome => true,
        Role::Assistant => command.starts_with("ai"),
        Role::Bubble => matches!(
            command.as_str(),
            "pwBubbleReady" | "pwBubbleSize" | "pwBubbleClose" | "pwFill" | "pwUse" | "pwSave" | "pwNever"
                | "pwDismiss" | "pwTimeout" | "pwManage"
        ),
        Role::Content => false,
    }
}

/// Liegt diese Datei wirklich im Download-Ordner? Nur dann öffnet Zunder sie
/// auf Klick – der Eintrag in der Liste ist nur eine Zeile in einer Datei.
fn inside_downloads(path: &std::path::Path) -> bool {
    let folder = crate::dev::build_var_os("ZUNDER_DOWNLOAD_DIR")
        .map(std::path::PathBuf::from)
        .or_else(dirs::download_dir);
    let (Some(folder), Ok(real)) = (folder, path.canonicalize()) else {
        return false;
    };
    folder.canonicalize().is_ok_and(|folder| real.starts_with(folder))
}

impl App {
    /// Nachricht aus der Oberfläche eines Fensters.
    pub fn handle_ui(&mut self, window: WindowId, body: &str, target: &Target) {
        let Ok(message) = serde_json::from_str::<Value>(body) else {
            return;
        };
        let command = message.get("c").and_then(Value::as_str).unwrap_or("");
        let text = |key: &str| {
            message
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        let number = |key: &str| message.get(key).and_then(Value::as_i64).unwrap_or(0);
        let flag = |key: &str| message.get(key).and_then(Value::as_bool).unwrap_or(false);

        if !self.windows.contains_key(&window) {
            return;
        }

        match command {
            "ready" => {
                if let Some(win) = self.windows.get_mut(&window) {
                    win.ui_became_ready();
                }
                self.send_bookmark_bar(window);
                crate::app::debug_timing("oberflaeche_bereit");
            }
            "navigate" => {
                let typed = text("text");
                let engine = omnibox::engine(&self.settings.engine);
                // Die Adresse einer eingebauten App lässt sich eintippen – nur
                // hier, wo der Mensch am Browser selbst schreibt.
                if let Some(url) = crate::apps::typed(&typed) {
                    self.open_url(window, &url, flag("newTab"), true);
                } else if let Some(url) = omnibox::resolve(&typed, engine) {
                    self.open_url(window, &url, flag("newTab"), true);
                }
            }
            "open" => {
                if let Some(url) = omnibox::as_url(&text("url")) {
                    self.open_url(window, &url, flag("newTab"), false);
                }
            }
            "suggest" => self.suggest(window, &text("text"), number("seq") as u64, flag("plain")),
            "suggestRemote" => self.suggest_remote(window, &text("text"), number("seq") as u64),
            // Das Ladebild ist zu sehen: Darunter darf jetzt die Seite erscheinen.
            "splashShown" => self.release_content(window),
            "overlay" => {
                if let Some(win) = self.windows.get_mut(&window) {
                    win.overlay = flag("open");
                    if win.overlay {
                        native::bring_to_front(&win.chrome);
                    }
                }
                if !flag("open") {
                    self.release_content(window);
                }
                self.layout(window);
                // Unter einem Menü haben Vorschläge am Feld nichts zu suchen.
                if flag("open") {
                    self.bubble_context_changed(window);
                }
            }
            "chromeHeight" => {
                if let Some(win) = self.windows.get_mut(&window) {
                    let height = message
                        .get("h")
                        .and_then(Value::as_f64)
                        .unwrap_or(CHROME_HEIGHT)
                        .clamp(CHROME_HEIGHT, CHROME_HEIGHT + 160.0);
                    win.chrome_height = height;
                }
                self.layout(window);
            }
            "focusContent" => self.focus_content(window),
            // Die Übersicht aller Tabs ging auf: Vorschaubilder machen.
            "overview" => self.capture_thumbs(window),
            "focusChrome" => {
                if let Some(win) = self.windows.get(&window) {
                    let _ = win.chrome.focus();
                }
            }
            "newTab" => {
                self.new_tab(window, None, true, None);
            }
            "closeTab" => {
                if let Some(index) = self.tab_index(window, number("id") as u64) {
                    self.close_tab(window, index);
                }
            }
            "activateTab" => {
                if let Some(index) = self.tab_index(window, number("id") as u64) {
                    self.activate_tab(window, index);
                }
            }
            "moveTab" => self.move_tab(window, number("id") as u64, number("index").max(0) as usize),
            "tabMenu" => {
                let Some(index) = self.tab_index(window, number("id") as u64) else {
                    return;
                };
                match text("action").as_str() {
                    "reload" => self.reload(window, index, false),
                    "duplicate" => self.duplicate_tab(window, index),
                    "close" => self.close_tab(window, index),
                    "closeOthers" => self.close_other_tabs(window, index),
                    // Diesen Tab neben den aktuellen stellen bzw. das beenden.
                    "split" => self.set_split(window, Some(number("id") as u64)),
                    "unsplit" => self.set_split(window, None),
                    "pin" => self.pin_tab(window, index, true),
                    "unpin" => self.pin_tab(window, index, false),
                    "groupNew" => self.group_new(window, index),
                    "groupAdd" => self.group_set(window, index, Some(number("group").max(0) as u32)),
                    "groupLeave" => self.group_set(window, index, None),
                    _ => {}
                }
            }
            // Tab-Gruppen: auf- und zuklappen, umbenennen, einfärben, auflösen, schließen.
            "groupToggle" => self.group_toggle(window, number("id").max(0) as u32),
            "groupName" => self.group_edit(window, number("id").max(0) as u32, Some(&text("name")), None),
            "groupColor" => self.group_edit(window, number("id").max(0) as u32, None, Some(number("color").clamp(0, 7) as u8)),
            "groupDissolve" => self.group_dissolve(window, number("id").max(0) as u32),
            "groupClose" => self.group_close(window, number("id").max(0) as u32),
            "cmd" => self.command(Some(window), &text("id"), target),
            "find" => self.find(window, &text("text"), flag("back")),
            "findClose" => {
                self.find_clear(window);
                self.focus_content(window);
            }
            "panel" => self.send_panel(window, &text("name"), &text("q")),
            "localSiteRemove" => {
                self.forbid_local_site(&text("site"));
                self.send_panel(window, "settings", "");
            }
            // Rechte je Website: eine Entscheidung zurücknehmen. Danach gilt für
            // die Website wieder das Übliche – Zunder fragt beim nächsten Mal.
            "rightRemove" => {
                let (site, kind) = (text("site"), text("kind"));
                match kind.as_str() {
                    "camera" | "mic" => self.media_forget(&site, &kind),
                    "localnet" => self.forbid_local_site(&site),
                    "blocker" => self.set_site_blocker(&site, false, false),
                    _ => {}
                }
                self.send_panel(window, "rights", &text("q"));
            }
            // Website-Daten: für eine Website löschen – oder für alle (ohne Namen).
            "siteDataDelete" => {
                let site = text("site");
                let query = text("q");
                let proxy = self.proxy.clone();
                let names = if site.is_empty() { Vec::new() } else { vec![site] };
                native::remove_site_data(names, move || {
                    let _ = proxy.send_event(UserEvent::SiteDataChanged { window, query: query.clone() });
                });
            }
            // Das Schild in der Adresszeile: Werbeblocker für diese Website aus oder an.
            "blockerSite" => self.set_blocker_for_site(window, flag("off"), &text("site")),
            // Einstellungen: den Blocker für eine Website wieder einschalten.
            "blockerSiteRemove" => {
                self.set_site_blocker(&text("site"), false, false);
                self.send_panel(window, "settings", "");
            }
            // Einstellungen: jetzt nach neueren Filterlisten sehen.
            "filtersRefresh" => {
                self.blocker.refresh(true);
                self.send_filter_info();
            }
            "historyDelete" => {
                self.sites_stale = true;
                self.store.delete_history(number("id"));
                self.send_panel(window, "history", &text("q"));
            }
            "bookmarkDelete" => {
                self.store.delete_bookmark(number("id"));
                self.bookmarks_changed(Some(window), &text("q"));
            }
            "bookmarkEdit" => {
                // Titel, Adresse und Ordner eines Lesezeichens ändern.
                let url = omnibox::as_url(&text("url")).filter(|u| u.starts_with("http://") || u.starts_with("https://"));
                match url {
                    Some(url) if self.store.update_bookmark(number("id"), &text("title"), &url) => {
                        let folder = text("folder");
                        let current = self.store.bookmarks("", 100_000).into_iter().find(|b| b.id == number("id"));
                        if current.is_some_and(|b| b.folder != folder) {
                            self.store.move_bookmark(number("id"), &folder, None);
                        }
                    }
                    _ => self.notice(window, t("bm_edit_failed").to_string(), None, 5000),
                }
                self.bookmarks_changed(Some(window), &text("q"));
            }
            "bookmarkMove" => {
                // In einen Ordner (leer: ohne) und dort vor ein anderes – oder ans Ende.
                let before = Some(number("before")).filter(|id| *id > 0);
                self.store.move_bookmark(number("id"), &text("folder"), before);
                self.bookmarks_changed(Some(window), &text("q"));
            }
            "folderAdd" => {
                if !self.store.add_folder(&text("name")) {
                    self.notice(window, t("bm_folder_exists").to_string(), None, 4000);
                }
                self.bookmarks_changed(Some(window), &text("q"));
            }
            "folderRename" => {
                if !self.store.rename_folder(&text("old"), &text("name")) {
                    self.notice(window, t("bm_folder_exists").to_string(), None, 4000);
                }
                self.bookmarks_changed(Some(window), &text("q"));
            }
            "folderDelete" => {
                self.store.delete_folder(&text("name"));
                self.bookmarks_changed(Some(window), &text("q"));
            }
            "downloadOpen" => {
                if let Some(path) = self.store.download_path(number("id")) {
                    if inside_downloads(std::path::Path::new(&path)) {
                        native::open_external(&path);
                    }
                }
            }
            "downloadReveal" => {
                if let Some(path) = self.store.download_path(number("id")) {
                    if inside_downloads(std::path::Path::new(&path)) {
                        native::reveal_in_finder(std::path::Path::new(&path));
                    }
                }
            }
            "downloadsClear" => {
                self.store.clear_downloads();
                self.send_panel(window, "downloads", "");
                self.mark_all_dirty();
            }
            "setting" => self.change_setting(window, &text("key"), message.get("value")),
            "clearData" => {
                self.sites_stale = true;
                self.store.clear_history();
                self.closed.clear();
                native::clear_website_data(crate::app::CLEAR_PLAIN);
                self.refresh_newtab();
                self.notice(window, t("s_cleared").to_string(), None, 4000);
                self.mark_all_dirty();
            }
            "defaultBrowser" => {
                let key = if native::request_default_browser() {
                    "n_default_browser"
                } else {
                    "n_default_browser_bundle"
                };
                self.notice(window, t(key).to_string(), None, 6000);
            }
            "noticeAction" => {
                let notice = number("id") as u64;
                // Der Knopf gilt nur, solange der Tab noch die Seite zeigt, die gefragt hat.
                let valid = self.notice_still_valid(notice);
                if let Some(action) = self.notices.remove(&notice).filter(|_| valid) {
                    match action {
                        NoticeAction::OpenExternal(url) => native::open_external(&url),
                        NoticeAction::Reveal(path) => native::reveal_in_finder(&path),
                        NoticeAction::AllowLocal(tab_window, tab, site) => {
                            let private = self.windows.get(&tab_window).is_none_or(|win| win.private || win.stealth);
                            self.allow_local_site(&site, private);
                            // Sobald die Regeln neu übersetzt sind, lädt der Tab neu.
                            self.reload_when_ready.push((tab_window, tab));
                        }
                        NoticeAction::OpenTab(url) => {
                            self.new_tab(window, Some(url), true, None);
                        }
                        NoticeAction::AppLocation(tab_window, tab, app, n) => {
                            self.app_location_allowed(tab_window, tab, app, n);
                        }
                        NoticeAction::BlockerOff(tab_window, tab, site) => {
                            let private = self.windows.get(&tab_window).is_some_and(|w| w.private);
                            self.set_site_blocker(&site, true, private);
                            // Zurück zur Startseite der Website – die Sperrseite selbst hilft nicht weiter.
                            if let Some(index) = self.tab_index(tab_window, tab) {
                                let home = self
                                    .windows
                                    .get(&tab_window)
                                    .and_then(|w| w.tabs.get(index))
                                    .and_then(|t| url::Url::parse(&t.url).ok())
                                    .filter(|u| matches!(u.scheme(), "http" | "https"))
                                    .map(|u| u.origin().ascii_serialization() + "/");
                                if let Some(home) = home {
                                    self.navigate(tab_window, index, &home, false);
                                }
                            }
                        }
                        NoticeAction::TranslateUndo(tab_window, tab) => self.translate_undo(tab_window, tab),
                        NoticeAction::EnableDevtools => {
                            self.settings.devtools = true;
                            self.store.set_setting("devtools", "1");
                            native::set_guards(self.settings.guards());
                            // Gilt ab dem Anlegen eines Tabs – also alle neu anlegen.
                            self.rebuild_tabs();
                            if let Some(webview) = self.windows.get(&window).and_then(|w| w.active_webview()) {
                                webview.open_devtools();
                            }
                        }
                    }
                }
            }
            "noticeGone" => {
                let gone = number("id") as u64;
                // Stand dort noch die Frage einer App nach dem Standort, ist sie
                // damit unbeantwortet geblieben.
                if let Some(NoticeAction::AppLocation(tab_window, tab, app, n)) = self.notices.remove(&gone) {
                    self.app_location_unanswered(tab_window, tab, app, n);
                }
                self.notice_owners.remove(&gone);
                // Der Hinweis auf ein nicht geladenes PDF war kaum zu sehen, weil
                // ihn gleich ein anderer verdrängt hat – oder er kam in der
                // Warteschlange gar nicht dran: Dann kommt er beim nächsten Mal
                // wieder. Schließt ihn der Mensch selbst oder läuft er ab, bleibt
                // es bei „gezeigt“.
                if let Some((known, since)) = self.frame_file_notices.remove(&gone) {
                    let unseen = match text("why").as_str() {
                        "dropped" => true,
                        "replaced" => since.elapsed() < std::time::Duration::from_millis(2500),
                        _ => false,
                    };
                    if unseen {
                        self.frame_files_noticed.remove(&known);
                    }
                }
            }
            // Der Seitencheck wird geöffnet: die Seite noch einmal frisch ansehen.
            "checkRefresh" => {
                if let Some(tab) = self.windows.get(&window).and_then(|win| win.active_tab()).map(|tab| tab.id) {
                    self.check_page(window, tab);
                }
            }
            "drag" => {
                if let Some(win) = self.windows.get(&window) {
                    let _ = win.window.drag_window();
                }
            }
            "zoomWindow" => {
                if let Some(win) = self.windows.get(&window) {
                    win.window.set_maximized(!win.window.is_maximized());
                }
            }
            // Alles, was die Seitenleiste des Assistenten meldet.
            other if other.starts_with("ai") => self.handle_assistant(window, other, &message),
            // Passwort-Tresor: Sprechblase, Schlüssel-Knopf und Verwaltung.
            other if other.starts_with("pw") => self.handle_logins(window, other, &message),
            "bookmarksImport" => {
                if let Some(win) = self.windows.get(&window) {
                    native::choose_file(&win.chrome, window, "bm-import", None);
                }
            }
            "bookmarksExport" => {
                if let Some(win) = self.windows.get(&window) {
                    native::choose_file(&win.chrome, window, "bm-export", Some(t("bm_export_name")));
                }
            }
            "bookmarksFrom" => self.import_bookmarks_from(window, &text("browser")),
            _ => {}
        }
    }

    /// Lädt eine Adresse im aktiven Tab oder in einem neuen.
    fn open_url(&mut self, window: WindowId, url: &str, new_tab: bool, typed: bool) {
        if new_tab {
            let opener = self
                .windows
                .get(&window)
                .and_then(|w| w.active_tab())
                .map(|t| t.id);
            if let Some(index) = self.new_tab(window, Some(url.to_string()), true, opener) {
                if let Some(tab) = self
                    .windows
                    .get_mut(&window)
                    .and_then(|w| w.tabs.get_mut(index))
                {
                    tab.typed = typed;
                }
            }
        } else {
            let active = self.windows[&window].active;
            self.navigate(window, active, url, typed);
        }
        self.focus_content(window);
    }

    pub fn focus_content(&mut self, window: WindowId) {
        if let Some(win) = self.windows.get(&window) {
            if let Some(webview) = win.active_webview() {
                let _ = webview.focus();
            }
        }
    }

    fn focus_address(&mut self, window: WindowId) {
        if let Some(win) = self.windows.get_mut(&window) {
            let _ = win.chrome.focus();
            win.send(json!({ "t": "focusAddress" }));
        }
    }

    /// Zeigt etwas in der Oberfläche an (Seitenleiste, Suchleiste …).
    /// `focus` gibt dem Rahmen dabei die Tastatur.
    fn show(&mut self, window: WindowId, what: &str, focus: bool) {
        if let Some(win) = self.windows.get_mut(&window) {
            if focus {
                let _ = win.chrome.focus();
            }
            win.send(json!({ "t": "show", "what": what }));
        }
    }

    /// Befehl aus der Menüleiste oder aus der Oberfläche.
    pub fn command(&mut self, window: Option<WindowId>, id: &str, target: &Target) {
        match id {
            "new_window" => {
                self.create_window(target, false, Vec::new(), 0, None);
                return;
            }
            "new_private" => {
                self.create_window(target, true, Vec::new(), 0, None);
                return;
            }
            "new_stealth" => {
                self.stealth_next = true;
                self.create_window(target, true, Vec::new(), 0, None);
                self.stealth_next = false;
                return;
            }
            "quit" => {
                if self.settings.clear_on_quit {
                    self.clean_quit();
                } else {
                    self.save_session();
                    self.quit = true;
                }
                return;
            }
            "quit_now" => {
                // Sicherheitsnetz nach dem Löschen beim Beenden.
                if self.quitting {
                    self.quit = true;
                }
                return;
            }
            _ => {}
        }

        let window = match window.filter(|w| self.windows.contains_key(w)) {
            Some(window) => window,
            None => {
                // Ohne offenes Fenster: Befehle, die eines brauchen, öffnen eines.
                let needs_window = matches!(
                    id,
                    "new_tab" | "open_location" | "history" | "bookmarks" | "downloads" | "settings" | "reopen_tab" | "passwords"
                ) || id.strip_prefix("app_").is_some_and(|app| crate::apps::by_id(app).is_some());
                if !needs_window {
                    return;
                }
                let Some(window) = self.create_window(target, false, Vec::new(), 0, None) else {
                    return;
                };
                if id == "new_tab" || id == "open_location" {
                    return;
                }
                window
            }
        };
        let active = self.windows[&window].active;
        let count = self.windows[&window].tabs.len();

        match id {
            "new_tab" => {
                self.new_tab(window, None, true, None);
            }
            // Eine eingebaute App in einem neuen Tab.
            id if id.strip_prefix("app_").is_some_and(|app| crate::apps::by_id(app).is_some()) => {
                let url = crate::apps::url_of(&id[4..]);
                self.new_tab(window, Some(url), true, None);
            }
            "open_location" => self.focus_address(window),
            "close_tab" => self.close_tab(window, active),
            "close_window" => self.close_window(window),
            "duplicate_tab" => self.duplicate_tab(window, active),
            "next_tab" => self.activate_tab(window, (active + 1) % count),
            "prev_tab" => self.activate_tab(window, (active + count - 1) % count),
            "reopen_tab" => {
                if let Some((url, _)) = self.closed.pop() {
                    self.new_tab(window, Some(url), true, None);
                }
            }
            "reload" => self.reload(window, active, false),
            "hard_reload" => self.reload(window, active, true),
            "stop" => {
                if let Some(win) = self.windows.get_mut(&window) {
                    if let Some(tab) = win.tabs.get_mut(active) {
                        if let Some(webview) = &tab.webview {
                            native::stop(webview);
                        }
                        tab.loading = false;
                        tab.pending = None;
                    }
                    win.dirty = true;
                }
            }
            "back" | "forward" => {
                if let Some(win) = self.windows.get_mut(&window) {
                    if let Some(webview) = win.active_webview() {
                        // Wer selbst blättert, hat Geduld für neue Fragen nach
                        // Kamera und Mikrofon.
                        native::media_user_moved(native::key(webview));
                        let _ = if id == "back" {
                            webview.go_back()
                        } else {
                            webview.go_forward()
                        };
                    }
                    win.dirty = true;
                }
            }
            "zoom_in" => self.zoom(window, 1),
            "zoom_out" => self.zoom(window, -1),
            "zoom_reset" => self.zoom(window, 0),
            "print" => {
                if let Some(webview) = self.windows.get(&window).and_then(|w| w.active_webview()) {
                    let _ = webview.print();
                }
            }
            "devtools" => {
                // Die Entwicklerwerkzeuge sind von Haus aus abgeschaltet (siehe
                // Einstellungen → Sicherheit): Dann sagt der Menüpunkt, wo sie
                // sich einschalten lassen.
                if !self.settings.devtools {
                    self.notice(
                        window,
                        t("n_devtools_off").to_string(),
                        Some((t("turn_on"), NoticeAction::EnableDevtools)),
                        10000,
                    );
                } else if let Some(webview) = self.windows.get(&window).and_then(|w| w.active_webview()) {
                    webview.open_devtools();
                }
            }
            "view_source" => self.view_source(window),
            "mobile_view" => self.toggle_mobile(window),
            "find" => self.show(window, "find", true),
            "find_next" => self.show(window, "findNext", false),
            "find_prev" => self.show(window, "findPrev", false),
            "history" | "bookmarks" | "downloads" | "settings" | "passwords" | "tab_search" | "tab_overview" => {
                self.show(window, id, true)
            }
            "command_palette" => self.show(window, "palette", true),
            "fill_login" => self.fill_command(window),
            "reader" => self.toggle_reader(window),
            "translate_page" => self.translate_page(window),
            "site_check" => self.show(window, "siteCheck", true),
            "clear_data" => self.show(window, "clearData", true),
            "bookmark_page" => self.toggle_bookmark(window),
            "read_later" => self.read_later(window),
            "copy_clean_url" => self.copy_clean_url(window),
            "bookmark_bar" => {
                self.settings.bm_bar = !self.settings.bm_bar;
                self.store.set_setting("bm_bar", if self.settings.bm_bar { "1" } else { "0" });
                let windows: Vec<WindowId> = self.windows.keys().copied().collect();
                for id in windows {
                    self.send_bookmark_bar(id);
                }
            }
            "assistant" => {
                self.set_assistant(window, None);
            }
            "assistant_summary" | "compare" => {
                // Öffnet die Seitenleiste und stößt dort die Zusammenfassung bzw.
                // die Auswahl zum Vergleichen an – derselbe Weg wie ein Klick.
                let action = if id == "compare" { "pick" } else { "summary" };
                if self.set_assistant(window, Some(true)) {
                    if let Some(win) = self.windows.get_mut(&window) {
                        win.assistant_send(json!({ "t": "aiRun", "action": action }));
                    }
                }
            }
            "unsplit" => self.set_split(window, None),
            _ => {
                if let Some(n) = id.strip_prefix("tab_").and_then(|n| n.parse::<usize>().ok()) {
                    // ⌘9 springt wie gewohnt zum letzten Tab.
                    let index = if n >= 9 { count - 1 } else { n - 1 };
                    if index < count {
                        self.activate_tab(window, index);
                    }
                }
            }
        }
    }

    /// Zeigt den Quelltext der Seite in einem neuen Tab. Abgerufen wird er in
    /// der Seite selbst – so, wie der Server ihn (ihr) liefert, nicht der
    /// inzwischen von Skripten umgebaute Stand.
    fn view_source(&mut self, window: WindowId) {
        const BODY: &str = "try {\
            var response = await fetch(location.href, { cache: 'force-cache', credentials: 'include' });\
            var text = await response.text();\
            return text.slice(0, 1000000);\
        } catch (error) {\
            return document.documentElement ? document.documentElement.outerHTML.slice(0, 1000000) : '';\
        }";
        let Some(tab) = self.windows.get(&window).and_then(|w| w.active_tab()) else {
            return;
        };
        let web = tab.url.starts_with("http://") || tab.url.starts_with("https://");
        let (true, Some(webview)) = (web, tab.webview.as_ref()) else {
            self.notice(window, t("n_source_none").to_string(), None, 4000);
            return;
        };
        let proxy = self.proxy.clone();
        let tab_id = tab.id;
        native::call_isolated(webview, BODY, "", move |result| {
            let _ = proxy.send_event(UserEvent::PageAnswer { window, tab: tab_id, what: "source", result });
        });
    }

    /// Der Quelltext ist da: in einem neuen Tab zeigen.
    pub(crate) fn show_source(&mut self, window: WindowId, tab: u64, text: Option<String>) {
        let url = self
            .windows
            .get(&window)
            .and_then(|w| w.tabs.iter().find(|t| t.id == tab))
            .map(|t| t.url.clone());
        let (Some(url), Some(text)) = (url, text.filter(|text| !text.is_empty())) else {
            self.notice(window, t("n_source_none").to_string(), None, 4000);
            return;
        };
        self.next_source += 1;
        let id = self.next_source;
        if let Ok(mut sources) = self.sources.lock() {
            // Nur die letzten paar bleiben liegen.
            while sources.len() >= 6 {
                let Some(oldest) = sources.keys().min().copied() else {
                    break;
                };
                sources.remove(&oldest);
            }
            let truncated = text.encode_utf16().count() >= 1_000_000;
            sources.insert(id, (url.clone(), text, truncated));
        }
        let mut page = url::Url::parse("zunder://source/").expect("feste Adresse");
        page.query_pairs_mut().append_pair("id", &id.to_string()).append_pair("u", &url);
        if let Some(index) = self.new_tab(window, None, true, Some(tab)) {
            self.navigate(window, index, page.as_str(), false);
        }
    }

    /// Schaltet für den aktiven Tab die mobile Ansicht um: Er gibt sich als
    /// iPhone aus, und die Seite lädt neu.
    fn toggle_mobile(&mut self, window: WindowId) {
        let Some(win) = self.windows.get_mut(&window) else {
            return;
        };
        let index = win.active;
        let Some(tab) = win.tabs.get_mut(index) else {
            return;
        };
        tab.mobile = !tab.mobile;
        let mobile = tab.mobile;
        if let Some(webview) = &tab.webview {
            native::set_mobile(webview, mobile);
        }
        win.dirty = true;
        self.reload(window, index, true);
        let key = if mobile { "n_mobile_on" } else { "n_mobile_off" };
        self.notice(window, t(key).to_string(), None, 3500);
    }

    pub(crate) fn reload(&mut self, window: WindowId, index: usize, ignore_cache: bool) {
        let Some(tab) = self.windows.get(&window).and_then(|w| w.tabs.get(index)) else {
            return;
        };
        // Auf der Fehlerseite heißt „neu laden“: die eigentliche Seite erneut
        // versuchen. Eine Warnung vor dem eigenen Netz lädt dagegen nur sich
        // selbst neu – über sie hinweg geht es allein mit ihrem Knopf.
        if let Some(target) = omnibox::error_retry(&tab.url) {
            self.navigate(window, index, &target, false);
            return;
        }
        if let Some(webview) = &tab.webview {
            // Wer selbst neu lädt, darf wieder nach Kamera und Mikrofon gefragt werden.
            native::media_visit_over(native::key(webview));
            if ignore_cache {
                native::reload_from_origin(webview);
            } else {
                let _ = webview.reload();
            }
        }
    }

    fn duplicate_tab(&mut self, window: WindowId, index: usize) {
        let Some(tab) = self.windows.get(&window).and_then(|w| w.tabs.get(index)) else {
            return;
        };
        let (url, id) = (tab.url.clone(), tab.id);
        self.new_tab(window, Some(url), true, Some(id));
    }

    fn close_other_tabs(&mut self, window: WindowId, keep: usize) {
        let Some(win) = self.windows.get(&window) else {
            return;
        };
        let Some(keep_id) = win.tabs.get(keep).map(|t| t.id) else {
            return;
        };
        self.activate_tab(window, keep);
        loop {
            let Some(win) = self.windows.get(&window) else {
                return;
            };
            // Angeheftete Tabs überstehen „Andere schließen“.
            let Some(victim) = win.tabs.iter().position(|t| t.id != keep_id && !t.pinned) else {
                return;
            };
            self.close_tab(window, victim);
        }
    }

    fn move_tab(&mut self, window: WindowId, tab: u64, to: usize) {
        let Some(win) = self.windows.get_mut(&window) else {
            return;
        };
        let Some(from) = win.tabs.iter().position(|t| t.id == tab) else {
            return;
        };
        let to = to.min(win.tabs.len() - 1);
        if from == to {
            return;
        }
        let active_id = win.tabs[win.active].id;
        let mut moved = win.tabs.remove(from);
        // Zwischen zwei Tabs derselben Gruppe abgelegt: gehört dazu. Aus der
        // eigenen Gruppe hinausgezogen: gehört nicht mehr dazu.
        let left = to.checked_sub(1).and_then(|i| win.tabs.get(i)).and_then(|t| t.group);
        let right = win.tabs.get(to).and_then(|t| t.group);
        if !moved.pinned {
            if left.is_some() && left == right {
                moved.group = left;
            } else if moved.group.is_some() && moved.group != left && moved.group != right {
                moved.group = None;
            }
        }
        win.tabs.insert(to, moved);
        win.active = win
            .tabs
            .iter()
            .position(|t| t.id == active_id)
            .unwrap_or(0);
        win.tidy_tabs();
        win.dirty = true;
        self.session_changed();
    }

    fn zoom(&mut self, window: WindowId, direction: i32) {
        let Some(win) = self.windows.get_mut(&window) else {
            return;
        };
        let active = win.active;
        let Some(tab) = win.tabs.get_mut(active) else {
            return;
        };
        let current = ZOOM_STEPS
            .iter()
            .position(|z| (*z - tab.zoom).abs() < 0.001)
            .unwrap_or(5);
        let next = match direction {
            0 => 5,
            d if d > 0 => (current + 1).min(ZOOM_STEPS.len() - 1),
            _ => current.saturating_sub(1),
        };
        tab.zoom = ZOOM_STEPS[next];
        if let Some(webview) = &tab.webview {
            let _ = webview.zoom(tab.zoom);
        }
        win.dirty = true;
    }

    fn toggle_bookmark(&mut self, window: WindowId) {
        let Some(tab) = self.windows.get(&window).and_then(|w| w.active_tab()) else {
            return;
        };
        let url = omnibox::bar_url(tab.shown_url());
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return;
        }
        let title = tab.title.clone();
        let message = if self.store.is_bookmarked(&url) {
            self.sites_stale = true;
            self.store.remove_bookmark(&url);
            "n_bookmark_removed"
        } else {
            self.sites_stale = true;
            self.store.add_bookmark(&url, &title);
            "n_bookmark_added"
        };
        self.notice(window, t(message).to_string(), None, 2200);
        self.bookmarks_changed(None, "");
    }

    // Die aktuelle Seite in die Leseliste der App „Später lesen" legen – als
    // Karteikarte in deren Ablage (`app_records`), genau wie die App sie liest.
    fn read_later(&mut self, window: WindowId) {
        // Je Aufruf eine andere Kennung, auch bei zwei Seiten in derselben Millisekunde.
        static READ_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let Some(tab) = self.windows.get(&window).and_then(|w| w.active_tab()) else {
            return;
        };
        let url = omnibox::bar_url(tab.shown_url());
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            self.notice(window, t("rd_not_a_page").to_string(), None, 2200);
            return;
        }
        let title = tab.title.clone();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let seq = READ_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let id = format!("r-{now:x}{seq:x}");
        let record = serde_json::json!({ "k": "r", "url": url, "title": title, "when": now, "read": false }).to_string();
        let message = match self.store.put_app_record("read", &id, &record, 2000) {
            crate::store::RecordPut::Stored => "rd_saved",
            crate::store::RecordPut::Full => "rd_full",
            crate::store::RecordPut::Failed => "rd_failed",
        };
        self.notice(window, t(message).to_string(), None, 2200);
    }

    // Die Adresse der aktuellen Seite ohne Tracking-Parameter in die
    // Zwischenablage legen (`utm_…`, `fbclid`, `gclid` … fallen weg).
    fn copy_clean_url(&mut self, window: WindowId) {
        let Some(tab) = self.windows.get(&window).and_then(|w| w.active_tab()) else {
            return;
        };
        let url = omnibox::bar_url(tab.shown_url());
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            self.notice(window, t("uc_not_a_page").to_string(), None, 2200);
            return;
        }
        let (clean, removed) = omnibox::clean_tracking(&url);
        native::copy_text(&clean, false);
        let message = if removed > 0 {
            t("uc_copied_clean").replace("{n}", &removed.to_string())
        } else {
            t("uc_copied").to_string()
        };
        self.notice(window, message, None, 2500);
    }

    // ----- Suche in der Seite ---------------------------------------------

    fn find(&mut self, window: WindowId, query: &str, backwards: bool) {
        let Some(win) = self.windows.get(&window) else {
            return;
        };
        let Some(tab) = win.active_tab() else {
            return;
        };
        let Some(webview) = &tab.webview else {
            return;
        };
        let query = serde_json::to_string(query).unwrap_or_else(|_| "\"\"".into());
        // Die Suche läuft in Zunders abgeschotteter Welt und liefert Text: Die
        // Skripte der Seite können ihr weder Funktionen unterschieben noch ein
        // Ergebnis liefern, an dem der Browser sich verschluckt.
        let script = format!("JSON.stringify({FIND_JS}({query}, {}, false))", if backwards { -1 } else { 1 });
        let proxy = self.proxy.clone();
        let tab_id = tab.id;
        native::eval_isolated(webview, &script, move |json| {
            let _ = proxy.send_event(UserEvent::FindResult {
                window,
                tab: tab_id,
                json: json.filter(|j| j.len() <= 64).unwrap_or_default(),
            });
        });
    }

    fn find_clear(&mut self, window: WindowId) {
        if let Some(webview) = self.windows.get(&window).and_then(|w| w.active_webview()) {
            native::eval_isolated(webview, &format!("{FIND_JS}(\"\", 1, true); ''"), |_| {});
        }
    }

    // ----- Vorschläge der Adressleiste -------------------------------------

    fn suggest(&mut self, window: WindowId, input: &str, seq: u64, plain: bool) {
        let input = input.trim();
        let Some(win) = self.windows.get(&window) else {
            return;
        };
        let engine = omnibox::engine(&self.settings.engine);
        let mut items: Vec<Value> = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        let mut complete: Option<String> = None;

        if !input.is_empty() {
            // Vervollständigung aus dem Verlauf ("git" -> "github.com") – nicht,
            // wenn gerade gelöscht wurde.
            if !plain {
                complete = self.store.autocomplete(input);
            }
            let first = complete.as_deref().unwrap_or(input);
            match omnibox::as_url(first) {
                Some(url) => {
                    items.push(json!({ "k": "go", "title": omnibox::pretty_url(&url), "url": url, "sub": t("sug_open") }));
                    seen.push(url);
                    if complete.is_some() {
                        items.push(json!({
                            "k": "search", "title": input, "url": omnibox::search_url(engine, input),
                            "sub": t("sug_search_with").replace("{engine}", engine.name),
                        }));
                    }
                }
                None => {
                    complete = None;
                    items.push(json!({
                        "k": "search", "title": input, "url": omnibox::search_url(engine, input),
                        "sub": t("sug_search_with").replace("{engine}", engine.name),
                    }));
                }
            }

            // Bereits offene Tabs
            let needle = input.to_lowercase();
            let active_id = win.active_tab().map(|t| t.id);
            for tab in win.tabs.iter().filter(|t| Some(t.id) != active_id) {
                if items.len() >= 3 {
                    break;
                }
                // Verglichen wird mit der sichtbaren Adresse, nicht mit internen.
                let shown = omnibox::bar_url(&tab.url);
                let hit = tab.title.to_lowercase().contains(&needle)
                    || shown.to_lowercase().contains(&needle);
                if hit && !shown.is_empty() && !seen.contains(&shown) {
                    items.push(json!({
                        "k": "tab", "title": tab.display_title(), "url": shown,
                        "sub": t("sug_switch"), "id": tab.id, "icon": tab.favicon,
                    }));
                    seen.push(shown);
                }
            }

            // Lesezeichen, dann Verlauf
            for entry in self.store.bookmarks(input, 3) {
                if !seen.contains(&entry.url) {
                    items.push(json!({ "k": "bookmark", "title": entry.title, "url": entry.url, "icon": entry.icon }));
                    seen.push(entry.url);
                }
            }
            for entry in self.store.search_history(input, 6) {
                if items.len() >= 8 {
                    break;
                }
                if !seen.contains(&entry.url) {
                    items.push(json!({ "k": "history", "title": entry.title, "url": entry.url, "icon": entry.icon }));
                    seen.push(entry.url);
                }
            }
        }

        if let Some(win) = self.windows.get_mut(&window) {
            win.send(json!({ "t": "suggest", "seq": seq, "items": items, "complete": complete }));
        }
    }

    /// Fragt die Suchmaschine nach Vorschlägen – nur wenn eingeschaltet und nie
    /// in privaten Fenstern.
    fn suggest_remote(&mut self, window: WindowId, input: &str, seq: u64) {
        let input = input.trim().to_string();
        let private = self.windows.get(&window).is_none_or(|w| w.private);
        if private || !self.settings.suggest || input.len() < 2 || input.len() > 120 {
            return;
        }
        // Adressen gehen niemanden etwas an – nur echte Suchbegriffe senden.
        if omnibox::as_url(&input).is_some() || input.contains("://") {
            return;
        }
        let proxy = self.proxy.clone();
        let engine_id = self.settings.engine.clone();
        std::thread::spawn(move || {
            let items = omnibox::fetch_suggestions(omnibox::engine(&engine_id), &input);
            if !items.is_empty() {
                let _ = proxy.send_event(UserEvent::Suggestions {
                    window,
                    seq,
                    query: input,
                    items,
                });
            }
        });
    }

    // ----- Seitenleisten ---------------------------------------------------

    pub(crate) fn send_panel(&mut self, window: WindowId, name: &str, query: &str) {
        // Website-Daten kennt nur WebKit – die Liste kommt gleich als Ereignis nach.
        if name == "sitedata" {
            let proxy = self.proxy.clone();
            let query = query.chars().take(100).collect::<String>();
            native::site_data(move |sites| {
                let _ = proxy.send_event(UserEvent::SiteData { window, query: query.clone(), sites });
            });
            return;
        }
        let data = match name {
            "history" => json!({ "items": self.store.history_page(query, 400) }),
            "bookmarks" => json!({
                "items": self.store.bookmarks(query, 600),
                "folders": self.store.bookmark_folders(),
                "sources": crate::bookmarks_io::sources(),
            }),
            "passwords" => self.passwords_panel(query),
            "downloads" => {
                let items: Vec<Value> = self
                    .store
                    .downloads(120)
                    .into_iter()
                    .map(|d| {
                        let path = std::path::Path::new(&d.path);
                        let meta = std::fs::metadata(path).ok();
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        json!({
                            "id": d.id, "name": name, "url": d.url, "state": d.state, "ts": d.ts,
                            "bytes": meta.as_ref().map(|m| m.len()).unwrap_or(d.bytes.max(0) as u64),
                            "exists": meta.is_some(),
                        })
                    })
                    .collect();
                json!({ "items": items })
            }
            // Was Websites dürfen oder nicht dürfen – alles, was sich Zunder je
            // Website gemerkt hat, an einer Stelle.
            "rights" => {
                let mut sites: std::collections::BTreeMap<String, Vec<Value>> = std::collections::BTreeMap::new();
                for (site, rule) in &self.media_sites {
                    let rights = sites.entry(site.clone()).or_default();
                    if let Some(allowed) = rule.camera {
                        rights.push(json!({ "kind": "camera", "allowed": allowed }));
                    }
                    if let Some(allowed) = rule.microphone {
                        rights.push(json!({ "kind": "mic", "allowed": allowed }));
                    }
                }
                for site in &self.local_sites {
                    sites.entry(site.clone()).or_default().push(json!({ "kind": "localnet", "allowed": true }));
                }
                for site in &self.blocker_off {
                    sites.entry(site.clone()).or_default().push(json!({ "kind": "blocker", "allowed": true }));
                }
                let total = sites.len();
                let needle = query.trim().to_lowercase();
                let items: Vec<Value> = sites
                    .into_iter()
                    .filter(|(site, rights)| !rights.is_empty() && (needle.is_empty() || site.contains(&needle)))
                    .take(800)
                    .map(|(site, rights)| json!({ "site": site, "rights": rights }))
                    .collect();
                json!({ "items": items, "total": total })
            }
            "settings" => json!({
                "engine": self.settings.engine,
                "suggest": self.settings.suggest,
                "restore": self.settings.restore,
                "blocker": self.settings.blocker,
                "filters": self.blocker.info(),
                "filterUpdate": self.settings.filter_update,
                "guard": self.settings.guard,
                "blockerOff": self.blocker_off,
                "saver": self.settings.saver,
                "engines": ENGINES.iter().map(|e| json!({ "id": e.id, "name": e.name })).collect::<Vec<_>>(),
                "httpsOnly": self.settings.https_only,
                "stripTracking": self.settings.strip_tracking,
                "hardened": self.settings.hardened,
                "lookalike": self.settings.lookalike,
                "localNet": self.settings.local_net,
                "localSites": self.local_sites,
                "rightsCount": self.rights_count(),
                "devtools": self.settings.devtools,
                "splash": self.settings.splash,
                "startBookmarks": self.settings.start_bookmarks,
                "clearOnQuit": self.settings.clear_on_quit,
                "wallpaper": self.store.setting("wallpaper").unwrap_or_default(),
                "wallpapers": crate::wallpapers::list(),
            }),
            _ => return,
        };
        if let Some(win) = self.windows.get_mut(&window) {
            win.send(json!({ "t": "panel", "name": name, "q": query, "data": data }));
        }
    }

    fn change_setting(&mut self, window: WindowId, key: &str, value: Option<&Value>) {
        let on = value.and_then(Value::as_bool).unwrap_or(false);
        let store_flag = |store: &crate::store::Store, key: &str| {
            store.set_setting(key, if on { "1" } else { "0" });
        };
        match key {
            "engine" => {
                let id = value.and_then(Value::as_str).unwrap_or("");
                if ENGINES.iter().any(|e| e.id == id) {
                    self.settings.engine = id.to_string();
                    self.store.set_setting("engine", id);
                }
            }
            "wallpaper" => {
                let id = value.and_then(Value::as_str).unwrap_or("");
                if !id.is_empty() && !crate::wallpapers::exists(id) {
                    return;
                }
                self.store.set_setting("wallpaper", id);
                self.refresh_newtab();
            }
            "https_only" => {
                self.settings.https_only = on;
                store_flag(&self.store, key);
                native::set_guards(self.settings.guards());
            }
            "strip_tracking" => {
                self.settings.strip_tracking = on;
                store_flag(&self.store, key);
                native::set_guards(self.settings.guards());
            }
            "hardened" => {
                self.settings.hardened = on;
                store_flag(&self.store, key);
                native::set_guards(self.settings.guards());
                // Die Härtung gilt ab dem Anlegen eines Tabs – also alle neu anlegen.
                self.rebuild_tabs();
            }
            "lookalike" => {
                self.settings.lookalike = on;
                store_flag(&self.store, key);
                native::set_guards(self.settings.guards());
                native::with_sites(|sites| sites.enabled = on);
            }
            "local_net" => {
                self.settings.local_net = on;
                store_flag(&self.store, key);
                native::set_guards(self.settings.guards());
                self.blocker.local_enabled = on;
                self.apply_blocker();
            }
            "devtools" => {
                self.settings.devtools = on;
                store_flag(&self.store, key);
                native::set_guards(self.settings.guards());
                // Gilt ab dem Anlegen eines Tabs – also alle neu anlegen.
                self.rebuild_tabs();
            }
            "clear_on_quit" => {
                self.settings.clear_on_quit = on;
                store_flag(&self.store, key);
                if !on {
                    self.store.set_setting("clear_pending", "0");
                }
            }
            "pw_offer" => {
                self.settings.pw_offer = on;
                store_flag(&self.store, key);
                self.send_panel(window, "passwords", "");
                return;
            }
            "pw_auth" => {
                // Abschalten verlangt selbst Touch ID – das regelt der Tresor.
                self.set_fill_auth(window, on);
                return;
            }
            "suggest" => {
                self.settings.suggest = on;
                store_flag(&self.store, key);
            }
            "splash" => {
                self.settings.splash = on;
                store_flag(&self.store, key);
            }
            // Lesezeichen als Kacheln auf der Startseite – oder nicht.
            "start_bookmarks" => {
                self.settings.start_bookmarks = on;
                store_flag(&self.store, key);
                self.refresh_newtab();
                self.reload_start_pages();
            }
            "restore" => {
                self.settings.restore = on;
                store_flag(&self.store, key);
            }
            "saver" => {
                self.settings.saver = on;
                store_flag(&self.store, key);
            }
            "blocker" => {
                self.settings.blocker = on;
                self.blocker.enabled = on;
                store_flag(&self.store, key);
                self.apply_blocker();
                for win in self.windows.values_mut() {
                    win.dirty = true;
                }
            }
            "filter_update" => {
                self.settings.filter_update = on;
                self.blocker.auto_update = on;
                store_flag(&self.store, key);
            }
            "guard" => {
                self.settings.guard = on;
                self.blocker.guard_enabled = on;
                store_flag(&self.store, key);
                self.apply_blocker();
                // Eingeschaltet: gleich nach den Listen sehen, statt bis zur nächsten Runde zu warten.
                if on {
                    self.blocker.refresh(false);
                }
            }
            _ => return,
        }
        self.send_panel(window, "settings", "");
    }
}
