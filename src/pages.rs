//! Reaktion auf alles, was in einem Tab passiert: Laden, Titel, Icons,
//! Pop-ups, Downloads, Fehler.

use serde_json::{json, Value};
use tao::window::WindowId;

use crate::app::{ActiveDownload, App, NoticeAction, Tab, TabId};
use crate::i18n::t;
use crate::native::{self, NavEvent, NavKind};
use crate::omnibox;
use crate::store::host_of;
use crate::webviews::PageEvent;

fn is_web(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// Schema, Rechner und Port zweier Adressen stimmen überein.
fn same_origin(a: &str, b: &str) -> bool {
    let (Ok(a), Ok(b)) = (url::Url::parse(a), url::Url::parse(b)) else {
        return false;
    };
    a.scheme() == b.scheme() && a.host_str() == b.host_str() && a.port_or_known_default() == b.port_or_known_default()
}

/// Derselbe Rechner – für die Frage, ob eine von WebKit gemeldete Navigation
/// die ist, die der Browser selbst gerade angestoßen hat.
fn same_host(a: &str, b: &str) -> bool {
    let (Ok(a), Ok(b)) = (url::Url::parse(a), url::Url::parse(b)) else {
        return false;
    };
    a.host_str() == b.host_str()
}

/// Name des Programms, das ein Schema üblicherweise öffnet – für die Rückfrage.
fn app_for_scheme(scheme: &str) -> Option<&'static str> {
    Some(match scheme {
        "mailto" => "Mail",
        "tel" | "facetime" | "facetime-audio" => "FaceTime",
        "sms" | "imessage" => "Nachrichten",
        "maps" => "Karten",
        "itms-apps" | "macappstore" | "itms-appss" => "App Store",
        "music" | "itms" => "Musik",
        "zoommtg" | "zoomus" => "Zoom",
        "msteams" => "Microsoft Teams",
        "slack" => "Slack",
        "spotify" => "Spotify",
        "vscode" => "Visual Studio Code",
        "tg" => "Telegram",
        "whatsapp" => "WhatsApp",
        "discord" => "Discord",
        "figma" => "Figma",
        "notion" => "Notion",
        "obsidian" => "Obsidian",
        _ => return None,
    })
}

/// Schemata, die Zunder aus einer Webseite heraus nie an ein anderes Programm
/// weiterreicht: Sie hängen fremde Netzlaufwerke ein (dabei wandern
/// Anmeldedaten zum fremden Rechner), öffnen Fernzugriffe oder steuern das
/// System. Andere Browser fragen hier nur nach – ein Klick zu viel genügt dann.
const REFUSED_SCHEMES: &[&str] = &[
    "smb", "afp", "nfs", "cifs", "ftp", "ftps", "sftp", "ssh", "telnet", "vnc", "rdp", "dav", "davs", "webdav",
    "x-apple.systempreferences", "shortcuts", "applescript", "help", "x-help-action", "x-man-page",
    "itms-services", "ms-msdt", "ms-officecmd", "search-ms", "jar", "vbscript",
];

/// Wie dicht hintereinander eine Seite nach einem anderen Programm fragen darf.
/// Solange eine Frage frisch dasteht, kann die Seite sie nicht durch eine
/// andere ersetzen, während man schon auf „Öffnen“ zielt.
const EXTERNAL_PAUSE: std::time::Duration = std::time::Duration::from_secs(3);

impl App {
    pub fn handle_page(&mut self, window: WindowId, tab_id: TabId, event: PageEvent) {
        // Pop-ups betreffen (noch) keinen vorhandenen Tab.
        if matches!(event, PageEvent::Popup) {
            self.adopt_popups(window);
            return;
        }
        let Some(index) = self.tab_index(window, tab_id) else {
            return;
        };
        match event {
            PageEvent::Popup => {}
            PageEvent::Title(title) => {
                let Some(win) = self.windows.get_mut(&window) else {
                    return;
                };
                let private = win.private;
                let tab = &mut win.tabs[index];
                // Den Titel bestimmt die Seite – also ohne Steuerzeichen und
                // nicht länger, als irgendwo Platz ist.
                let title: String = title.chars().filter(|c| !c.is_control()).take(300).collect();
                if tab.title == title {
                    return;
                }
                // Der Titel gehört zur übernommenen Seite, nicht zu einer, die noch lädt.
                tab.title = title.clone();
                win.dirty = true;
                if !private && is_web(&tab.url) && tab.pending.is_none() {
                    self.store.update_title(&tab.url, &title);
                }
                self.session_changed();
            }
            PageEvent::Committed(url) => self.page_committed(window, index, url),
            PageEvent::Finished(url) => self.page_finished(window, index, url),
            PageEvent::Ping { origin, body, main } => match body.strip_prefix("q:") {
                Some(counts) => self.page_asked(window, index, &origin, main, counts),
                // Eine eingebaute App bittet den Browser um etwas.
                None if main && body.starts_with("A:") => self.app_message(window, index, &origin, &body[2..]),
                None => self.page_ping(window, index, &origin, &body),
            },
            PageEvent::OpenInNewTab(url) => {
                if let Some(url) = omnibox::as_url(&url) {
                    self.new_tab(window, Some(url), true, Some(tab_id));
                }
            }
            PageEvent::External(url) => self.external_request(window, tab_id, &url),
            PageEvent::DownloadStarted { url, path } => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                // Aus einem privaten Fenster kommt nichts in die Download-Liste.
                let private = self.windows.get(&window).is_some_and(|w| w.private);
                let id = if private { 0 } else { self.store.add_download(&url, &path.to_string_lossy()) };
                self.active_downloads.push(ActiveDownload { id, url, path });
                self.notice(
                    window,
                    t("n_download_started").replace("{name}", &name),
                    None,
                    3500,
                );
                // Ein Tab, der nur für diesen Download aufging, hat nichts zu zeigen.
                let empty_popup = self.windows.get(&window).is_some_and(|win| {
                    win.tabs.get(index).is_some_and(|tab| {
                        tab.opener.is_some() && tab.url.is_empty() && win.tabs.len() > 1
                    })
                });
                if empty_popup {
                    self.close_tab(window, index);
                } else if let Some(win) = self.windows.get_mut(&window) {
                    // Die Navigation wurde zum Download – der Tab lädt nichts mehr.
                    if let Some(tab) = win.tabs.get_mut(index) {
                        tab.loading = false;
                        tab.pending = None;
                    }
                }
                self.mark_all_dirty();
            }
            PageEvent::DownloadFinished { url, ok } => {
                let Some(position) = self.active_downloads.iter().position(|d| d.url == url) else {
                    return;
                };
                let download = self.active_downloads.remove(position);
                let bytes = std::fs::metadata(&download.path)
                    .map(|m| m.len() as i64)
                    .unwrap_or(0);
                if download.id != 0 {
                    self.store.finish_download(download.id, ok, bytes);
                }
                let name = download
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if ok {
                    self.notice(
                        window,
                        t("n_download_done").replace("{name}", &name),
                        Some((t("d_show"), NoticeAction::Reveal(download.path))),
                        6000,
                    );
                } else {
                    self.notice(
                        window,
                        t("n_download_failed").replace("{name}", &name),
                        None,
                        6000,
                    );
                }
                self.mark_all_dirty();
            }
            PageEvent::DownloadRefused => {
                if self.last_refusal.is_none_or(|at| at.elapsed() > std::time::Duration::from_secs(8)) {
                    self.last_refusal = Some(std::time::Instant::now());
                    self.notice(window, t("n_download_refused").to_string(), None, 7000);
                }
            }
            PageEvent::Crashed => {
                if let Some(webview) = self
                    .windows
                    .get(&window)
                    .and_then(|w| w.tabs.get(index))
                    .and_then(|t| t.webview.as_ref())
                {
                    let _ = webview.reload();
                }
                self.notice(window, t("n_crashed").to_string(), None, 5000);
            }
        }
    }

    pub fn mark_all_dirty(&mut self) {
        for win in self.windows.values_mut() {
            win.dirty = true;
        }
    }

    /// Übernimmt frisch erzeugte Pop-ups als Tabs hinter dem öffnenden Tab.
    fn adopt_popups(&mut self, window: WindowId) {
        let waiting: Vec<_> = {
            let mut queue = self.popups.borrow_mut();
            let (mine, others): (Vec<_>, Vec<_>) =
                queue.drain(..).partition(|p| p.window == window);
            *queue = others;
            mine
        };
        for popup in waiting {
            let Some(win) = self.windows.get_mut(&window) else {
                return;
            };
            let mut tab = Tab::new(popup.tab, String::new());
            tab.opener = Some(popup.opener);
            tab.loading = true;
            tab.progress = 0.1;
            self.blocker.apply(&popup.webview, "");
            tab.webview = Some(popup.webview);
            let position = win
                .tabs
                .iter()
                .position(|t| t.id == popup.opener)
                .map(|i| i + 1)
                .unwrap_or(win.tabs.len());
            win.tabs.insert(position, tab);
            if position <= win.active {
                win.active += 1;
            }
            self.activate_tab(window, position);
        }
    }

    fn page_committed(&mut self, window: WindowId, index: usize, url: String) {
        let Some(win) = self.windows.get_mut(&window) else {
            return;
        };
        let private = win.private;
        let tab = &mut win.tabs[index];
        if host_of(&tab.url) != host_of(&url) {
            tab.favicon = None;
        }
        if tab.url != url {
            tab.title.clear();
        }
        tab.url = url;
        tab.pending = None;
        // Eine neue Seite: Was die alte abgefragt hat, zählt nicht mehr.
        tab.asked.clear();
        // Was als Nächstes lädt, zeigt die Adressleiste nur, wenn der Browser
        // selbst wieder dorthin schickt.
        tab.pending_shown = false;
        tab.loading = true;
        // Die Seite steht – ein Verschlüsselungsversuch ist damit entschieden.
        tab.upgrade = None;
        tab.idle_ticks = 0;
        if let Some(webview) = &tab.webview {
            // WebKit behält die Zoomstufe je WebView; sie gilt hier je Tab.
            if (tab.zoom - 1.0).abs() > 0.001 {
                let _ = webview.zoom(tab.zoom);
            }
        }
        // Für oft besuchte Seiten das gemerkte Icon sofort zeigen.
        if tab.favicon.is_none() && !private {
            tab.favicon = self.store.favicon_for(&tab.url);
        }
        // Der Lesemodus gehört zur alten Seite – ebenso ihre Sprache und ihre Übersetzung.
        tab.reader = false;
        tab.readable = false;
        tab.lang.clear();
        let translated = tab.translation.take();
        // Der Seitencheck beginnt neu – mit dem, was der Server zu dieser Seite mitgeschickt hat.
        tab.check = crate::sitecheck::State::default();
        if let Some(webview) = &tab.webview {
            tab.check.response = native::response_for(webview, &tab.url);
            // Die Seite ist da: Die Listen gelten jetzt für genau sie.
            crate::blocker::committed(webview, &tab.url);
        }
        tab.check.refresh(&tab.url);
        win.dirty = true;
        win.send(json!({ "t": "committed", "id": tab_id_of(win, index) }));
        let (committed_tab, committed_site) = (tab_id_of(win, index), crate::shield::host_of(&win.tabs[index].url));
        if let Some(translation) = translated {
            // Stand dort noch der Fortschritt einer Übersetzung, hat er sich erledigt.
            win.send(json!({ "t": "noticeHide", "id": translation.notice_id() }));
            self.notices.remove(&translation.notice_id());
        }
        // Was die vorige Website in diesem Tab gefragt hat, gilt für die neue nicht.
        self.notices_end(window, committed_tab, Some(&committed_site));
        self.session_changed();
        // Passwort-Tresor: Wurde gerade eine Anmeldung abgeschickt, darf er jetzt fragen.
        self.login_committed(window, index);
        self.adblock_wall_hint(window, index);
    }

    /// Manche Websites schicken Besucher mit Werbeblocker auf eine Sperrseite.
    /// Die zu umgehen ist nicht Sache des Browsers – er sagt aber, woran es
    /// liegt, und bietet an, den Blocker für genau diese Website abzuschalten.
    fn adblock_wall_hint(&mut self, window: WindowId, index: usize) {
        let Some(tab) = self.windows.get(&window).and_then(|w| w.tabs.get(index)) else {
            return;
        };
        let Ok(parsed) = url::Url::parse(&tab.url) else {
            return;
        };
        let path = parsed.path().to_ascii_lowercase();
        let wall = ["adblockwall", "adblock-wall", "adblock_wall", "adblocker-wall", "adblockdetect", "adblock-detect", "adblocker-detect"]
            .iter()
            .any(|mark| path.contains(mark));
        if !wall || !is_web(&tab.url) || !self.settings.blocker || crate::blocker::site_off(&tab.url) {
            return;
        }
        let Some(site) = crate::sites::domain_of(&tab.url) else {
            return;
        };
        if self.wall_noticed.contains(&site) {
            return;
        }
        let tab_id = tab.id;
        let text = t("n_adwall").replace("{site}", &site);
        let label = t("n_adwall_off").replace("{site}", &site);
        if self.notice_from(window, tab_id, text, Some((&label, NoticeAction::BlockerOff(window, tab_id, site.clone()))), 30000) {
            self.wall_noticed.insert(site);
        }
    }

    fn page_finished(&mut self, window: WindowId, index: usize, url: String) {
        let Some(win) = self.windows.get_mut(&window) else {
            return;
        };
        let private = win.private;
        let tab = &mut win.tabs[index];
        tab.loading = false;
        tab.progress = 1.0;
        tab.pending = None;
        tab.pending_shown = false;
        // WebKit nennt hier unter Umständen schon die Adresse, zu der es als
        // Nächstes geht. Die Adresse des Tabs ändert sich aber nur mit der
        // Übernahme einer Seite – oder innerhalb derselben Herkunft.
        if !url.is_empty() && (tab.url.is_empty() || same_origin(&url, &tab.url)) {
            tab.url = url;
        }
        if tab.title.is_empty() {
            if let Some(webview) = &tab.webview {
                tab.title = native::title(webview);
            }
        }
        let typed = std::mem::take(&mut tab.typed);
        if !private && is_web(&tab.url) {
            self.store.record_visit(&tab.url, &tab.title, typed);
            // Wo man schon war, davor wird nicht mehr gewarnt.
            native::with_sites(|sites| sites.visited(&tab.url));
        }
        win.dirty = true;
        let active = win.active == index;
        self.session_changed();
        // Die fertige Seite, solange sie vorn ist: ihr Vorschaubild für die Übersicht.
        if active {
            self.capture_thumb(window, index);
        }
        // Jetzt steht die Seite da: Skripte, Formulare, Cookies ansehen.
        if let Some(tab_id) = self.windows.get(&window).and_then(|win| win.tabs.get(index)).map(|tab| tab.id) {
            self.check_page(window, tab_id);
        }
        crate::app::debug_timing("seite_geladen");
    }

    /// Nachricht des mitlaufenden Skripts. Der Inhalt kommt aus der Seite und
    /// wird nur als Hinweis gewertet; geprüft wird alles gegen WebKit selbst.
    /// Stealth-Fenster: Der Beobachter in der Seite (`ui/watch.js`) meldet,
    /// was sie bisher über das Gerät abgefragt hat. Das ist Auskunft der Seite
    /// über sich selbst und löst nichts aus – es landet nur in der Tarn-Ampel.
    fn page_asked(&mut self, window: WindowId, index: usize, origin: &str, main: bool, counts: &str) {
        /// Die Arten, die der Beobachter kennt – etwas anderes wird nicht angenommen.
        const KINDS: &[&str] = &[
            "screen", "canvas", "webgl", "device", "agent", "language", "timezone", "fonts", "storage",
            "cookie_write", "cookie_read", "referrer", "location", "media", "clipboard", "notify", "permissions",
            "beacon",
        ];
        let Some(win) = self.windows.get_mut(&window) else {
            return;
        };
        if !win.stealth || !is_web(origin) {
            return;
        }
        let Some(tab) = win.tabs.get_mut(index) else {
            return;
        };
        // Keine Seite hält den Browser mit Meldungen auf Trab.
        if tab.asked_rate.0.elapsed() >= std::time::Duration::from_secs(1) {
            tab.asked_rate = (std::time::Instant::now(), 0);
        }
        tab.asked_rate.1 += 1;
        if tab.asked_rate.1 > 20 {
            return;
        }
        // Die Seite selbst – oder ein eingebetteter Rahmen (dann unter seinem Namen).
        let frame = match main {
            true if host_of(origin) == host_of(&tab.url) => String::new(),
            true => return,
            false => host_of(origin),
        };
        if !main && frame.is_empty() {
            return;
        }
        if !tab.asked.contains_key(&frame) && tab.asked.len() >= 24 {
            return;
        }
        let Ok(Value::Object(reported)) = serde_json::from_str::<Value>(counts) else {
            return;
        };
        let entry = tab.asked.entry(frame).or_default();
        let mut changed = false;
        for kind in KINDS {
            if let Some(count) = reported.get(*kind).and_then(Value::as_u64) {
                let count = count.min(9999) as u32;
                changed |= entry.insert(kind, count) != Some(count);
            }
        }
        if changed && win.active == index {
            win.ampel_pending = true;
        }
    }

    fn page_ping(&mut self, window: WindowId, index: usize, origin: &str, body: &str) {
        let Some(win) = self.windows.get_mut(&window) else {
            return;
        };
        let private = win.private;
        if let Some(site) = body.strip_prefix("L:") {
            // Auf der Rückfrage-Seite wurde einer Website der Zugriff ins eigene
            // Netz dauerhaft erlaubt. Dass die Meldung von dort kommt, ist
            // schon geprüft (`native::bridge_received`).
            let site = site.to_string();
            let private = private || self.windows.get(&window).is_none_or(|win| win.stealth);
            self.allow_local_site(&site, private);
            return;
        }

        if body == "a" {
            // Die Seite hat die Tastatur bekommen. Stehen zwei Tabs
            // nebeneinander, folgt die Symbolleiste dem, in dem gearbeitet wird.
            // Mehr bewirkt die Meldung nicht – und nur innerhalb des Paars.
            let id = win.tabs[index].id;
            // Die Meldung kommt aus der Welt der Seite – also zählt sie nur, wenn
            // diese Seite die Tastatur wirklich hat. Sonst könnte eine Seite sich
            // selbst zur aktiven Hälfte erklären, und Symbolleiste wie Tastenkürzel
            // gälten ihr statt der Seite, in der gearbeitet wird.
            let focused = win.tabs[index].webview.as_ref().is_some_and(native::has_keyboard);
            if focused && win.in_split(id) && win.active != index {
                let previous = win.active;
                if let Some(tab) = win.tabs.get_mut(previous) {
                    tab.last_used = std::time::Instant::now();
                }
                win.active = index;
                win.last_progress = 0.0;
                win.dirty = true;
            }
            return;
        }
        if let Some(change) = body.strip_prefix("b:") {
            // Die Startseite ist der Schreibtisch für Lesezeichen: Dort lassen sie
            // sich anordnen und in Ordner legen – mehr nicht (kein Anlegen, kein
            // Löschen). Angenommen wird das nur von ihr selbst; laden kann sie
            // allein der Browser. Private Fenster zeigen keine Lesezeichen.
            let from_start_page = url::Url::parse(origin)
                .is_ok_and(|u| u.scheme() == "zunder" && u.host_str() == Some("newtab"));
            if from_start_page && !private && self.settings.start_bookmarks {
                if let Ok(data) = serde_json::from_str::<serde_json::Value>(change) {
                    let id = data.get("id").and_then(serde_json::Value::as_i64).unwrap_or(0);
                    let folder = data.get("folder").and_then(serde_json::Value::as_str).unwrap_or("");
                    let before = data.get("before").and_then(serde_json::Value::as_i64).filter(|b| *b > 0);
                    if id > 0 && folder.len() <= 240 && self.store.move_bookmark(id, folder, before) {
                        self.bookmarks_changed(None, "");
                    }
                }
            }
            return;
        }
        if let Some(id) = body.strip_prefix("w:") {
            // Die Startseite hat einen Hintergrund gewählt. Angenommen wird das
            // nur von ihr selbst (den Absender nennt WebKit) und nur für einen
            // der eingebauten Hintergründe – oder keinen.
            let from_start_page = url::Url::parse(origin)
                .is_ok_and(|u| u.scheme() == "zunder" && u.host_str() == Some("newtab"));
            if from_start_page && !private && (id.is_empty() || crate::wallpapers::exists(id)) {
                self.store.set_setting("wallpaper", id);
                self.refresh_newtab();
            }
            return;
        }
        let tab = &mut win.tabs[index];

        if body == "u" {
            // Die Seite hat ihre Adresse ohne Neuladen geändert.
            let Some(webview) = &tab.webview else {
                return;
            };
            let Ok(current) = webview.url() else {
                return;
            };
            // Ohne Neuladen kann eine Seite ihre Adresse nur innerhalb der eigenen
            // Herkunft ändern. Alles andere wäre die Adresse einer Seite, die erst
            // noch lädt – die gilt erst mit ihrer Übernahme.
            if tab.pending.is_none() && !current.is_empty() && current != tab.url && same_origin(&current, &tab.url) {
                tab.url = current;
                win.dirty = true;
                // Als Besuch zählt das nicht: Sonst könnte eine Seite durch
                // bloßes Ändern ihrer Adresse den Verlauf fluten und sich in
                // Vorschlägen und auf der Startseite nach vorn schieben.
                let _ = private;
                self.session_changed();
            } else {
                // Auch ohne neue Adresse können sich Vor/Zurück geändert haben.
                win.dirty = true;
            }
            return;
        }
    }

    /// Eine Seite will ein anderes Programm öffnen – nie ohne Rückfrage.
    fn external_request(&mut self, window: WindowId, tab: TabId, url: &str) {
        if url.len() > 4000 {
            return;
        }
        let Ok(parsed) = url::Url::parse(url) else {
            return;
        };
        let scheme = parsed.scheme().to_string();
        if self.last_external.is_some_and(|at| at.elapsed() < EXTERNAL_PAUSE) {
            return;
        }
        // Nur der Tab, den man gerade vor sich hat, darf danach fragen.
        if !self.windows.get(&window).is_some_and(|win| win.shows(tab)) {
            return;
        }
        self.last_external = Some(std::time::Instant::now());
        // Der Hinweis nennt die Website, die fragt – und gehört zu ihr: Zeigt
        // der Tab etwas anderes, gilt der Knopf nicht mehr.
        let site = self.asking_site(window, tab);
        if REFUSED_SCHEMES.contains(&scheme.as_str()) {
            let text = t("n_external_refused").replace("{site}", &site).replace("{scheme}", &scheme);
            self.notice_from(window, tab, text, None, 8000);
            return;
        }
        let text = match app_for_scheme(&scheme) {
            Some(app) => t("n_external").replace("{site}", &site).replace("{app}", app),
            None => t("n_external_generic").replace("{site}", &site).replace("{scheme}", &scheme),
        };
        self.notice_from(
            window,
            tab,
            text,
            Some((t("open"), NoticeAction::OpenExternal(parsed.to_string()))),
            15000,
        );
    }

    /// Wie die Website heißt, deren Seite ein Tab gerade zeigt – für Hinweise,
    /// die sagen, wer etwas möchte. Lässt sie sich nicht nennen: „Diese Seite“.
    fn asking_site(&self, window: WindowId, tab: TabId) -> String {
        self.windows
            .get(&window)
            .and_then(|win| win.tabs.iter().find(|candidate| candidate.id == tab))
            .map(|tab| crate::shield::host_of(&tab.url))
            .filter(|host| !host.is_empty())
            .unwrap_or_else(|| t("n_this_page").to_string())
    }

    /// Ein Tab wollte eine unverschlüsselte Seite laden. Zuerst wird die
    /// verschlüsselte Fassung versucht; gibt es keine, fragt die Fehlerseite nach.
    fn insecure_request(&mut self, window: WindowId, index: usize, url: String) {
        let Some(tab) = self.windows.get_mut(&window).and_then(|w| w.tabs.get_mut(index)) else {
            return;
        };
        let host = crate::shield::host_of(&url);
        let typed = tab.typed;
        // Schickt derselbe Rechner gleich wieder ins Unverschlüsselte zurück –
        // sofort per Weiterleitung oder nach dem Laden per Skript –, hat ein
        // weiterer Versuch keinen Sinn.
        // Gezählt werden nur Versuche, die dicht aufeinander folgen – wer von
        // Hand mehrere alte http-Links derselben Seite anklickt, ist keine Schleife.
        let tries = &mut tab.upgrade_tries;
        if tries.0 != host || tries.2.elapsed() > std::time::Duration::from_secs(5) {
            *tries = (host.clone(), 0, std::time::Instant::now());
        }
        tries.1 = tries.1.saturating_add(1);
        tries.2 = std::time::Instant::now();
        let again = tries.1 > 2
            || tab
                .upgrade
                .as_ref()
                .is_some_and(|u| u.host == host && u.at.elapsed() < std::time::Duration::from_secs(10));
        match (crate::shield::https_version(&url), again) {
            (Some(secure), false) => {
                tab.upgrade = Some(crate::app::Upgrade { host, plain: url, at: std::time::Instant::now() });
                let _ = typed;
                self.reissue(window, index, &secure);
            }
            _ => self.show_insecure_warning(window, index, &url, None),
        }
    }

    /// Lädt eine Adresse, die an die Stelle einer angehaltenen Navigation tritt
    /// (verschlüsselt statt unverschlüsselt, ohne Tracking-Anhängsel).
    ///
    /// Hat der Browser selbst dorthin geschickt (Eingabe, Lesezeichen), lädt
    /// er einfach die neue Adresse. Wollte dagegen die SEITE dorthin, geht sie
    /// selbst noch einmal los – als gewöhnlicher Link ohne Absenderangabe. So
    /// bleibt es ihre Navigation: Die Adressleiste zeigt bis zur Übernahme die
    /// alte Adresse, und Cookies, die nur bei eigenen Aufrufen mitgehen sollen,
    /// bleiben zu Hause.
    fn reissue(&mut self, window: WindowId, index: usize, url: &str) {
        const BODY: &str = "try {\
            var a = document.createElementNS('http://www.w3.org/1999/xhtml', 'a');\
            a.href = payload;\
            a.rel = 'noreferrer';\
            a.click();\
            return 'ok';\
        } catch (error) { return 'error'; }";
        let Some(tab) = self.windows.get_mut(&window).and_then(|w| w.tabs.get_mut(index)) else {
            return;
        };
        let ours = tab.pending_shown && tab.pending.as_deref().is_some_and(|pending| same_host(pending, url));
        if ours {
            let typed = tab.typed;
            self.navigate(window, index, url, typed);
            return;
        }
        let Some(webview) = &tab.webview else {
            return;
        };
        let tab_id = tab.id;
        let proxy = self.proxy.clone();
        let target = url.to_string();
        native::call_isolated(webview, BODY, url, move |result| {
            if result.as_deref() != Some("ok") {
                let _ = proxy.send_event(crate::app::UserEvent::Reissue { window, tab: tab_id, url: target.clone() });
            }
        });
    }

    /// Zeigt statt der unverschlüsselten Seite die Nachfrage.
    pub fn show_insecure_warning(&mut self, window: WindowId, index: usize, plain: &str, detail: Option<&str>) {
        self.show_interstitial(
            window,
            index,
            &[("u", plain), ("p", plain), ("c", "0"), ("d", ""), ("m", detail.unwrap_or("")), ("e", "")],
        );
    }

    /// Zeigt in einem Tab statt der gewünschten Seite eine Rückfrage – die
    /// Fehlerseite in einer ihrer Fassungen. Nur der Browser kann sie laden.
    fn show_interstitial(&mut self, window: WindowId, index: usize, pairs: &[(&str, &str)]) {
        let Some(win) = self.windows.get_mut(&window) else {
            return;
        };
        let Some(tab) = win.tabs.get_mut(index) else {
            return;
        };
        let mut page = url::Url::parse("zunder://error/").expect("feste Adresse");
        for (key, value) in pairs {
            page.query_pairs_mut().append_pair(key, value);
        }
        // Das Siegel dieses Starts: Ohne es lädt Zunder keine Warnseite.
        page.query_pairs_mut().append_pair("z", native::seal());
        tab.upgrade = None;
        tab.pending = None;
        tab.typed = false;
        if let Some(webview) = &tab.webview {
            tab.loading = true;
            native::expect_internal(native::key(webview), page.as_str());
            let _ = webview.load_url(page.as_str());
        }
        win.dirty = true;
    }

    /// Eine Website wollte zu einem Gerät im eigenen Netz. Ging es um den Tab
    /// selbst, fragt er nach; wollte sie ein neues Fenster, bleibt es bei einem Hinweis.
    fn local_request(&mut self, window: WindowId, index: usize, url: &str, from: &str, in_place: bool, replay: bool) {
        let target = crate::shield::host_of(url);
        if in_place {
            self.show_interstitial(
                window,
                index,
                &[("v", "localnet"), ("u", url), ("f", from), ("r", if replay { "1" } else { "0" })],
            );
            return;
        }
        let Some(tab) = self.windows.get(&window).and_then(|win| win.tabs.get(index)).map(|tab| tab.id) else {
            return;
        };
        let site = self.asking_site(window, tab);
        let text = t("n_local_window").replace("{site}", &site).replace("{target}", &target);
        let action = replay.then(|| (t("open"), NoticeAction::OpenTab(url.to_string())));
        self.notice_from(window, tab, text, action, 12000);
    }

    /// Eine Seite wollte im Hintergrund ein Gerät im eigenen Netz ansprechen und
    /// wurde daran gehindert. Das erfährt man einmal je Website – samt der
    /// Möglichkeit, es ihr zu erlauben.
    fn local_blocked(&mut self, window: WindowId, index: usize, url: &str) {
        let Some(tab) = self.windows.get(&window).and_then(|w| w.tabs.get(index)) else {
            return;
        };
        let Some(site) = crate::sites::domain_of(&tab.url) else {
            return;
        };
        let target = crate::shield::host_of(url);
        let over_network = ["http://", "https://", "ws://", "wss://"].iter().any(|scheme| url.starts_with(scheme));
        if !over_network || target.is_empty() || crate::shield::is_local_host(&site) || self.local_noticed.contains(&site) {
            return;
        }
        let tab_id = tab.id;
        let text = t("n_local_blocked").replace("{site}", &site).replace("{target}", &target);
        // „Einmal je Website“ zählt erst, wenn der Hinweis wirklich zu sehen war –
        // ein Tab im Hintergrund meldet sich, sobald er vorn ist und es wieder versucht.
        if self.notice_from(window, tab_id, text, Some((t("allow"), NoticeAction::AllowLocal(window, tab_id, site.clone()))), 14000) {
            self.local_noticed.insert(site);
        }
    }

    // -----------------------------------------------------------------------
    // Eingebaute Apps (`apps.rs`)
    // -----------------------------------------------------------------------

    /// Schickt einer App die Antwort auf ihre Anfrage – nur, solange der Tab
    /// noch genau diese App zeigt. Was der Browser über den Tab weiß, hinkt dem
    /// Geschehen aber hinterher; deshalb prüft die Antwort an Ort und Stelle
    /// noch einmal selbst, in welchem Dokument sie läuft (`apps::reply`).
    pub fn app_reply(&self, window: WindowId, tab: TabId, app: &str, n: u32, answer: Value) {
        let shown = crate::apps::url_of(app);
        let webview = self
            .windows
            .get(&window)
            .and_then(|win| win.tabs.iter().find(|candidate| candidate.id == tab))
            .filter(|tab| tab.url == shown)
            .and_then(|tab| tab.webview.as_ref());
        if let Some(webview) = webview {
            let _ = webview.evaluate_script(&crate::apps::reply(app, n, answer));
        }
    }

    /// Eine eingebaute App bittet den Browser um etwas: ihren Zettel, ihre
    /// Karteikarten, den Standort. Angenommen wird das nur von der Seite der
    /// App selbst – unter genau ihrer Adresse, die allein der Browser lädt –,
    /// und nur, was für diese App eingetragen ist.
    fn app_message(&mut self, window: WindowId, index: usize, origin: &str, body: &str) {
        use crate::apps::Op;
        let Some(app) = crate::apps::by_url(origin).filter(|_| crate::apps::is_root(origin)) else {
            return;
        };
        let Some(win) = self.windows.get(&window) else {
            return;
        };
        let Some(tab) = win.tabs.get(index).filter(|tab| tab.url == origin) else {
            return;
        };
        let (private, stealth, tab_id) = (win.private, win.stealth, tab.id);
        let view = tab.webview.as_ref().map(native::key);
        let call = match crate::apps::parse(body) {
            Ok(call) => call,
            // Unbrauchbar, aber mit lesbarer Nummer: Die Seite erfährt es,
            // statt auf eine Antwort zu warten.
            Err(Some(n)) => {
                self.app_reply(window, tab_id, app.id, n, json!({ "state": "invalid" }));
                return;
            }
            Err(None) => return,
        };
        let n = call.n;
        match call.op {
            // Der Zettel der App. Private Fenster lesen und schreiben ihn nicht.
            // Lässt sich die Datenbank nicht lesen oder schreiben, erfährt die
            // App genau das – ein leerer Zettel wäre eine falsche Auskunft, auf
            // die hin sie den gesicherten überschriebe.
            Op::Load => {
                let stored = if private { Ok(None) } else { self.store.try_setting(&crate::apps::data_key(app.id)) };
                let answer = match stored {
                    Ok(data) => {
                        let data = data.filter(|text| !text.is_empty()).unwrap_or_else(|| "{}".to_string());
                        json!({ "state": "ok", "data": data, "kept": !private })
                    }
                    Err(()) => json!({ "state": "failed" }),
                };
                self.app_reply(window, tab_id, app.id, n, answer);
            }
            Op::Save(data) => {
                let state = if private {
                    "private"
                } else if self.store.try_set_setting(&crate::apps::data_key(app.id), &data) {
                    "ok"
                } else {
                    "failed"
                };
                // Die Zeit-App hat ihren Zettel geändert (womöglich die laufende
                // Uhr) – die Symbolleiste soll den Timer-Knopf sofort nachziehen.
                if app.id == "time" && state == "ok" {
                    for win in self.windows.values_mut() {
                        win.dirty = true;
                    }
                }
                self.app_reply(window, tab_id, app.id, n, json!({ "state": state }));
            }
            // Die Karteikarten der App – nur, wo welche für sie eingetragen
            // sind. Private Fenster lesen und schreiben auch sie nicht.
            Op::List { after, limit } => {
                let found = if private || app.records == 0 {
                    Some((Vec::new(), false))
                } else {
                    self.store.app_records(app.id, &after, limit, crate::apps::LIST_BYTES)
                };
                let answer = match found {
                    Some((items, more)) => {
                        // Weiter geht es nach der letzten Karte dieser Antwort.
                        let next = if more { items.last().map(|(id, _)| id.clone()) } else { None };
                        let items: Vec<Value> = items.into_iter().map(|(id, data)| json!([id, data])).collect();
                        json!({ "state": "ok", "items": items, "next": next, "kept": !private && app.records > 0 })
                    }
                    None => json!({ "state": "failed" }),
                };
                self.app_reply(window, tab_id, app.id, n, answer);
            }
            Op::Put { id, data } => {
                use crate::store::RecordPut;
                let state = if app.records == 0 {
                    "refused"
                } else if private {
                    "private"
                } else {
                    match self.store.put_app_record(app.id, &id, &data, app.records) {
                        RecordPut::Stored => "ok",
                        RecordPut::Full => "full",
                        RecordPut::Failed => "failed",
                    }
                };
                self.app_reply(window, tab_id, app.id, n, json!({ "state": state }));
            }
            Op::Remove(id) => {
                let state = if app.records == 0 {
                    "refused"
                } else if private {
                    "private"
                } else if self.store.remove_app_record(app.id, &id) {
                    "ok"
                } else {
                    "failed"
                };
                self.app_reply(window, tab_id, app.id, n, json!({ "state": state }));
            }
            // Die App sichert gleich eine Datei, die sie selbst erzeugt hat –
            // mit einer der Endungen, die für sie eingetragen sind.
            Op::FileName(name) => {
                let state = match view.filter(|_| crate::apps::may_export(app, &name)) {
                    Some(view) => {
                        native::expect_file_name(view, name);
                        "ok"
                    }
                    None => "refused",
                };
                self.app_reply(window, tab_id, app.id, n, json!({ "state": state }));
            }
            // Der Browser ruft eine Adresse ab und prüft sie – im Hintergrund,
            // damit der Browser weiterläuft. Das Ergebnis kommt als `UserEvent::Scan`.
            Op::Scan(input) => {
                if !app.scans {
                    self.app_reply(window, tab_id, app.id, n, json!({ "state": "refused" }));
                } else if stealth {
                    // Ein Stealth-Fenster läuft durch den Tunnel; ein eigener
                    // Abruf ginge daran vorbei und verriete die echte Adresse.
                    self.app_reply(window, tab_id, app.id, n, json!({ "state": "refused", "why": "stealth" }));
                } else if !crate::security::begin() {
                    // Es laufen schon zu viele Prüfungen – gleich noch einmal.
                    self.app_reply(window, tab_id, app.id, n, json!({ "state": "busy" }));
                } else {
                    let (proxy, id) = (self.proxy.clone(), app.id);
                    std::thread::spawn(move || {
                        // Selbst bei einer Panik in `scan` wird die Zählung freigegeben.
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| crate::security::scan(&input)))
                            .unwrap_or_else(|_| json!({ "state": "failed" }));
                        crate::security::finish();
                        let _ = proxy.send_event(crate::app::UserEvent::Scan { window, tab: tab_id, app: id, n, result });
                    });
                    // Bis das Ergebnis da ist: „läuft“ – die Seite zeigt so lange, dass geprüft wird.
                    self.app_reply(window, tab_id, app.id, n, json!({ "state": "scanning" }));
                }
            }
            // Der Browser schickt eine frei zusammengestellte Anfrage und reicht
            // die rohe Antwort zurück. Wie beim Scan: im Hintergrund, begrenzt,
            // im Stealth-Fenster gar nicht.
            Op::Request(input) => {
                if !app.requests {
                    self.app_reply(window, tab_id, app.id, n, json!({ "state": "refused" }));
                } else if stealth {
                    self.app_reply(window, tab_id, app.id, n, json!({ "state": "refused", "why": "stealth" }));
                } else if !crate::inspect::begin() {
                    self.app_reply(window, tab_id, app.id, n, json!({ "state": "busy" }));
                } else {
                    let (proxy, id) = (self.proxy.clone(), app.id);
                    std::thread::spawn(move || {
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| crate::inspect::run(&input)))
                            .unwrap_or_else(|_| json!({ "state": "failed" }));
                        crate::inspect::finish();
                        let _ = proxy.send_event(crate::app::UserEvent::Inspect { window, tab: tab_id, app: id, n, result });
                    });
                    self.app_reply(window, tab_id, app.id, n, json!({ "state": "sending" }));
                }
            }
            // Der Posteingang lässt den Browser Atlas abrufen. Lesen/Schreiben
            // der Einstellung läuft hier (Hauptthread), der eigentliche Abruf im
            // Hintergrund. Der Token bleibt nativ; im Stealth-Fenster gar nichts.
            Op::Api(input) => {
                use crate::inbox::Action;
                if !app.api {
                    self.app_reply(window, tab_id, app.id, n, json!({ "state": "refused" }));
                } else if stealth {
                    self.app_reply(window, tab_id, app.id, n, json!({ "state": "refused", "why": "stealth" }));
                } else {
                    match crate::inbox::action(&input) {
                        Action::Bad => self.app_reply(window, tab_id, app.id, n, json!({ "state": "invalid" })),
                        Action::Status => {
                            let base = self.store.try_setting(crate::inbox::BASE_KEY).ok().flatten().unwrap_or_default();
                            self.app_reply(window, tab_id, app.id, n, json!({ "state": "ok", "base": base, "hasToken": crate::inbox::has_token() }));
                        }
                        Action::Set { base, token } => {
                            if private {
                                self.app_reply(window, tab_id, app.id, n, json!({ "state": "private" }));
                            } else if !self.store.try_set_setting(crate::inbox::BASE_KEY, &base) {
                                self.app_reply(window, tab_id, app.id, n, json!({ "state": "failed" }));
                            } else {
                                if let Some(token) = token {
                                    crate::inbox::set_token(&token);
                                }
                                self.app_reply(window, tab_id, app.id, n, json!({ "state": "ok", "base": base, "hasToken": crate::inbox::has_token() }));
                            }
                        }
                        Action::Clear => {
                            if private {
                                self.app_reply(window, tab_id, app.id, n, json!({ "state": "private" }));
                            } else {
                                crate::inbox::clear_token();
                                let base = self.store.try_setting(crate::inbox::BASE_KEY).ok().flatten().unwrap_or_default();
                                self.app_reply(window, tab_id, app.id, n, json!({ "state": "ok", "base": base, "hasToken": false }));
                            }
                        }
                        Action::Get { path, query } => {
                            let base = self.store.try_setting(crate::inbox::BASE_KEY).ok().flatten().filter(|base| !base.is_empty());
                            if let Some(base) = base {
                                if !crate::inbox::begin() {
                                    self.app_reply(window, tab_id, app.id, n, json!({ "state": "busy" }));
                                } else {
                                    let (proxy, id) = (self.proxy.clone(), app.id);
                                    std::thread::spawn(move || {
                                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| crate::inbox::fetch(&base, &path, &query)))
                                            .unwrap_or_else(|_| json!({ "state": "failed" }));
                                        crate::inbox::finish();
                                        let _ = proxy.send_event(crate::app::UserEvent::Api { window, tab: tab_id, app: id, n, result });
                                    });
                                    self.app_reply(window, tab_id, app.id, n, json!({ "state": "loading" }));
                                }
                            } else {
                                self.app_reply(window, tab_id, app.id, n, json!({ "state": "no_base" }));
                            }
                        }
                        // Einen neuen Vorgang anlegen (POST). Wie Get: Adresse hier,
                        // Abruf im Hintergrund. Auch im privaten Fenster erlaubt (ein
                        // Server-Schreiben, kein lokaler Stand); im Stealth gesperrt (oben).
                        Action::Create { body } => {
                            let base = self.store.try_setting(crate::inbox::BASE_KEY).ok().flatten().filter(|base| !base.is_empty());
                            if let Some(base) = base {
                                if !crate::inbox::begin() {
                                    self.app_reply(window, tab_id, app.id, n, json!({ "state": "busy" }));
                                } else {
                                    let (proxy, id) = (self.proxy.clone(), app.id);
                                    std::thread::spawn(move || {
                                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| crate::inbox::create(&base, &body)))
                                            .unwrap_or_else(|_| json!({ "state": "failed" }));
                                        crate::inbox::finish();
                                        let _ = proxy.send_event(crate::app::UserEvent::Api { window, tab: tab_id, app: id, n, result });
                                    });
                                    self.app_reply(window, tab_id, app.id, n, json!({ "state": "loading" }));
                                }
                            } else {
                                self.app_reply(window, tab_id, app.id, n, json!({ "state": "no_base" }));
                            }
                        }
                    }
                }
            }
            Op::Forget => {
                self.store.set_setting(&crate::apps::location_key(app.id), "");
                self.geo_waiters.retain(|(_, _, waiting, _)| *waiting != app.id);
                self.app_reply(window, tab_id, app.id, n, json!({ "state": "ok" }));
            }
            Op::Location => {
                if !app.location {
                    self.app_reply(window, tab_id, app.id, n, json!({ "state": "refused" }));
                } else if stealth {
                    // Ein Stealth-Fenster gibt nichts über das Gerät preis –
                    // auch nicht, wo es steht.
                    self.app_reply(window, tab_id, app.id, n, json!({ "state": "refused", "why": "stealth" }));
                } else if self.store.setting(&crate::apps::location_key(app.id)).as_deref() == Some("1") {
                    self.app_locate(window, tab_id, app.id, n);
                } else {
                    // Erst fragen – in der Leiste des Browsers, nicht in der Seite.
                    let text = t("n_app_location").replace("{app}", t(app.name));
                    let action = NoticeAction::AppLocation(window, tab_id, app.id, n);
                    let asked = self.notice_from(window, tab_id, text, Some((t("allow"), action)), 30000);
                    let state = if asked { "asking" } else { "failed" };
                    self.app_reply(window, tab_id, app.id, n, json!({ "state": state }));
                }
            }
        }
    }

    /// Der Mensch am Browser hat einer App den Standort erlaubt. Das gilt, bis
    /// er es in der App zurücknimmt – in einem privaten Fenster nur für dieses
    /// eine Mal: Dort merkt sich der Browser auch eine Zustimmung nicht.
    pub fn app_location_allowed(&mut self, window: WindowId, tab: TabId, app: &'static str, n: u32) {
        let Some((private, stealth)) = self.windows.get(&window).map(|win| (win.private, win.stealth)) else {
            return;
        };
        if stealth || crate::apps::by_id(app).is_none_or(|known| !known.location) {
            return;
        }
        if !private {
            self.store.set_setting(&crate::apps::location_key(app), "1");
        }
        self.app_locate(window, tab, app, n);
    }

    /// Die Frage nach dem Standort ist verschwunden, ohne dass jemand „Erlauben“
    /// gewählt hat (abgelaufen, geschlossen, verdrängt): Die App erfährt es,
    /// statt weiter zu warten.
    pub fn app_location_unanswered(&self, window: WindowId, tab: TabId, app: &'static str, n: u32) {
        self.app_reply(window, tab, app, n, json!({ "state": "unanswered" }));
    }

    /// Fragt macOS nach dem Standort; die Antwort kommt über `geo_arrived`.
    fn app_locate(&mut self, window: WindowId, tab: TabId, app: &'static str, n: u32) {
        self.geo_waiters.retain(|(waiting_window, waiting_tab, _, _)| !(*waiting_window == window && *waiting_tab == tab));
        if self.geo_waiters.len() >= 8 {
            self.geo_waiters.remove(0);
        }
        self.geo_waiters.push((window, tab, app, n));
        self.app_reply(window, tab, app, n, json!({ "state": "locating" }));
        crate::geo::request();
    }

    /// macOS hat den Standort genannt oder verweigert: Jede App, die darauf
    /// wartet, bekommt ihn – gerundet (`apps::position`).
    pub fn geo_arrived(&mut self, fix: crate::geo::Fix) {
        use crate::geo::Fix;
        if crate::dev::enabled() {
            println!("GEO {fix:?} wartend={}", self.geo_waiters.len());
        }
        let answer = match fix {
            Fix::At(latitude, longitude) => match crate::apps::position(latitude, longitude) {
                Some((latitude, longitude)) => json!({ "state": "ok", "lat": latitude, "lon": longitude }),
                None => json!({ "state": "failed" }),
            },
            Fix::Denied => json!({ "state": "denied" }),
            Fix::Failed => json!({ "state": "failed" }),
        };
        for (window, tab, app, n) in std::mem::take(&mut self.geo_waiters) {
            self.app_reply(window, tab, app, n, answer.clone());
        }
    }

    /// WebKit wollte den Inhalt eines eingebetteten Rahmens von sich aus
    /// herunterladen – mit der Härtung und im Stealth-Fenster macht es das mit
    /// PDFs, die es dort nicht anzeigt. Zunder hat den Download abgesagt. Das
    /// erfährt man einmal je Website: Sonst bliebe der leere Rahmen ein Rätsel.
    fn frame_file_refused(&mut self, window: WindowId, index: usize, pdf: bool) {
        let Some(win) = self.windows.get(&window) else {
            return;
        };
        let Some(tab) = win.tabs.get(index) else {
            return;
        };
        let stealth = win.stealth;
        // Gezählt wird nach dem Teil des Namens, der zählt – wie beim Zugriff
        // ins eigene Netz: Unterbereiche derselben Website melden sich nicht einzeln.
        let name = crate::sites::domain_of(&tab.url).unwrap_or_else(|| crate::shield::host_of(&tab.url));
        let known = (stealth, name);
        if self.frame_files_noticed.contains(&known) {
            return;
        }
        let tab_id = tab.id;
        let site = self.asking_site(window, tab_id);
        // Warum es sich nicht anzeigen lässt: Stealth-Fenster, Zunders Härtung –
        // oder der Blockierungsmodus von macOS, der für jede WebView gilt.
        let key = if !pdf {
            "n_frame_file"
        } else if stealth {
            "n_frame_pdf_stealth"
        } else if self.settings.hardened {
            "n_frame_pdf"
        } else {
            "n_frame_pdf_system"
        };
        // „Einmal je Website“ zählt erst, wenn der Hinweis wirklich zu sehen war:
        // Ein Tab im Hintergrund meldet sich, wenn er vorn ist und es wieder
        // vorkommt – und verdrängt ihn gleich ein anderer Hinweis, nimmt
        // `noticeGone` die Zählung zurück.
        if let Some(notice) = self.notice_from_id(window, tab_id, t(key).replace("{site}", &site), None, 12000) {
            if self.frame_files_noticed.len() >= 500 {
                self.frame_files_noticed.clear();
            }
            self.frame_files_noticed.insert(known.clone());
            if self.frame_file_notices.len() >= 16 {
                self.frame_file_notices.clear();
            }
            self.frame_file_notices.insert(notice, (known, std::time::Instant::now()));
        }
    }

    /// Ereignisse der nachgerüsteten WebKit-Rückrufe.
    pub fn handle_nav(&mut self, event: NavEvent) {
        let Some((window, index)) = self.find_webview(event.webview) else {
            return;
        };
        match event.kind {
            NavKind::Started | NavKind::Redirected => {
                let Some(win) = self.windows.get_mut(&window) else {
                    return;
                };
                let tab = &mut win.tabs[index];
                if let Some(webview) = &tab.webview {
                    // Während des Ladens nennt WebKit hier schon die Zieladresse.
                    if let Ok(target) = webview.url() {
                        if !target.is_empty() && target != tab.url {
                            // Gezeigt wird sie nur, wenn der Browser selbst dorthin
                            // geschickt hat (oder der Server von dort weiterleitet) –
                            // nicht, wenn die Seite sich woandershin schickt.
                            let ours = tab.pending_shown
                                && (matches!(event.kind, NavKind::Redirected)
                                    || tab.pending.as_deref().is_some_and(|p| same_host(p, &target)));
                            tab.pending_shown = ours;
                            tab.pending = Some(target);
                        }
                    }
                }
                tab.loading = true;
                tab.idle_ticks = 0;
                if tab.progress < 0.1 || tab.progress >= 1.0 {
                    tab.progress = 0.1;
                }
                win.dirty = true;
            }
            NavKind::Failed {
                provisional,
                code,
                domain,
                description,
                url,
            } => {
                // Kein Fehler: abgebrochen (-999) oder in einen Download bzw.
                // ein anderes Programm umgeleitet (102).
                let harmless = code == -999 || (domain == "WebKitErrorDomain" && code == 102);
                let Some(win) = self.windows.get_mut(&window) else {
                    return;
                };
                let tab = &mut win.tabs[index];
                let failed = if url.is_empty() {
                    tab.pending.clone().unwrap_or_default()
                } else {
                    url
                };
                tab.loading = false;
                tab.pending = None;
                tab.pending_shown = false;
                tab.typed = false;
                win.dirty = true;
                if harmless || !provisional || !is_web(&failed) {
                    return;
                }
                // War das der Versuch, eine unverschlüsselte Adresse verschlüsselt
                // zu laden? Dann fragt die Fehlerseite, ob es unverschlüsselt weitergehen soll.
                let plain = tab
                    .upgrade
                    .take()
                    .filter(|u| crate::shield::host_of(&failed) == u.host)
                    .map(|u| u.plain);
                let mut page = url::Url::parse("zunder://error/").expect("feste Adresse");
                page.query_pairs_mut()
                    .append_pair("u", plain.as_deref().unwrap_or(&failed))
                    .append_pair("c", &code.to_string())
                    .append_pair("d", &domain)
                    .append_pair("m", &description.chars().take(300).collect::<String>())
                    .append_pair("e", omnibox::engine(&self.settings.engine).search);
                if let Some(plain) = &plain {
                    page.query_pairs_mut().append_pair("p", plain);
                }
                page.query_pairs_mut().append_pair("z", native::seal());
                if let Some(webview) = &tab.webview {
                    tab.loading = true;
                    native::expect_internal(native::key(webview), page.as_str());
                    let _ = webview.load_url(page.as_str());
                }
            }
            NavKind::OpenInTab { url, foreground } => {
                let opener = self
                    .windows
                    .get(&window)
                    .and_then(|w| w.tabs.get(index))
                    .map(|t| t.id);
                if let Some(position) = self.new_tab(window, Some(url), foreground, opener) {
                    // Ein Tab im Hintergrund soll trotzdem sofort laden.
                    if !foreground {
                        self.ensure_webview(window, position);
                    }
                }
            }
            NavKind::Media { playing } => self.media_answer(window, index, playing),
            NavKind::Rewritten { url, original } => {
                // Dieselbe Seite ohne Tracking-Anhängsel. Hängt der Server sie
                // hartnäckig wieder an, bleibt die Adresse einmal, wie sie ist –
                // sonst drehte sich der Tab im Kreis.
                let Some(tab) = self.windows.get_mut(&window).and_then(|w| w.tabs.get_mut(index)) else {
                    return;
                };
                if tab.rewrites.1.elapsed() > std::time::Duration::from_secs(10) {
                    tab.rewrites = (0, std::time::Instant::now());
                }
                tab.rewrites.0 = tab.rewrites.0.saturating_add(1);
                if tab.rewrites.0 > 3 {
                    native::load_as_is(&original);
                    self.reissue(window, index, &original);
                } else {
                    self.reissue(window, index, &url);
                }
            }
            NavKind::InsecureForm { url } => {
                // Ein Formular sollte unverschlüsselt abgeschickt werden. Es wurde
                // nicht gesendet; die Rückfrage zeigt, wohin es gegangen wäre.
                self.show_insecure_warning(window, index, &url, None);
            }
            NavKind::Insecure { url } => self.insecure_request(window, index, url),
            NavKind::Lookalike { url, known } => self.show_interstitial(
                window,
                index,
                &[("v", "lookalike"), ("u", &url), ("k", &known.domain), ("h", &known.host)],
            ),
            NavKind::LocalNet { url, from, in_place, replay } => {
                self.local_request(window, index, &url, &from, in_place, replay)
            }
            NavKind::LocalBlocked { url } => self.local_blocked(window, index, &url),
            NavKind::FrameFile { pdf } => self.frame_file_refused(window, index, pdf),
            NavKind::OAuthCode { url } => self.oauth_returned(window, index, &url),
            NavKind::Guard { url, in_place } => {
                // Eine gemeldete Betrugs- oder Schadseite: im Tab die Warnung,
                // bei einem neuen Fenster ein Hinweis – geöffnet wird es nicht.
                if in_place {
                    self.show_interstitial(window, index, &[("v", "guard"), ("u", &url)]);
                } else {
                    let text = t("n_guard_window").replace("{host}", &host_of(&url));
                    self.warn(window, text, 9000);
                }
            }
            NavKind::Closed => {
                // window.close(): nur von der Seite selbst geöffnete Tabs schließen.
                let scripted = self
                    .windows
                    .get(&window)
                    .and_then(|w| w.tabs.get(index))
                    .is_some_and(|t| t.opener.is_some());
                if scripted {
                    self.close_tab(window, index);
                }
            }
        }
    }
}

fn tab_id_of(win: &crate::app::BrowserWindow, index: usize) -> TabId {
    win.tabs.get(index).map(|t| t.id).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_and_hosts() {
        assert!(same_origin("https://a.example/x?y", "https://a.example/z#w"));
        assert!(same_origin("https://a.example/", "https://a.example:443/"));
        assert!(!same_origin("https://a.example/", "http://a.example/"));
        assert!(!same_origin("https://a.example/", "https://b.example/"));
        assert!(!same_origin("https://a.example/", "https://a.example:8443/"));
        assert!(!same_origin("https://a.example/", "https://www.a.example/"));
        assert!(!same_origin("https://a.example/", ""));
        assert!(!same_origin("kein url", "kein url"));
        assert!(same_host("http://a.example/", "https://a.example:8443/x"));
        assert!(!same_host("https://a.example/", "https://evil.example/a.example"));
    }
}
