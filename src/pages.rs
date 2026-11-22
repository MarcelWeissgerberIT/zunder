//! Reaktion auf alles, was in einem Tab passiert: Laden, Titel, Icons,
//! Pop-ups, Downloads, Fehler.

use serde_json::json;
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
            PageEvent::Ping { origin, body } => self.page_ping(window, index, &origin, &body),
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
        // Der Lesemodus gehört zur alten Seite.
        tab.reader = false;
        tab.readable = false;
        win.dirty = true;
        win.send(json!({ "t": "committed", "id": tab_id_of(win, index) }));
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
        if !self.wall_noticed.insert(site.clone()) {
            return;
        }
        let tab_id = tab.id;
        let text = t("n_adwall").replace("{site}", &site);
        let label = t("n_adwall_off").replace("{site}", &site);
        self.notice(window, text, Some((&label, NoticeAction::BlockerOff(window, tab_id, site))), 30000);
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
        crate::app::debug_timing("seite_geladen");
    }

    /// Nachricht des mitlaufenden Skripts. Der Inhalt kommt aus der Seite und
    /// wird nur als Hinweis gewertet; geprüft wird alles gegen WebKit selbst.
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
            self.allow_local_site(&site);
            return;
        }

        if body == "a" {
            // Die Seite hat die Tastatur bekommen. Stehen zwei Tabs
            // nebeneinander, folgt die Symbolleiste dem, in dem gearbeitet wird.
            // Mehr bewirkt die Meldung nicht – und nur innerhalb des Paars.
            let id = win.tabs[index].id;
            if win.in_split(id) && win.active != index {
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
            if from_start_page && !private {
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
        if REFUSED_SCHEMES.contains(&scheme.as_str()) {
            self.notice(window, t("n_external_refused").replace("{scheme}", &scheme), None, 8000);
            return;
        }
        let text = match app_for_scheme(&scheme) {
            Some(app) => t("n_external").replace("{app}", app),
            None => t("n_external_generic").replace("{scheme}", &scheme),
        };
        self.notice(
            window,
            text,
            Some((t("open"), NoticeAction::OpenExternal(parsed.to_string()))),
            15000,
        );
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
        tab.upgrade = None;
        tab.pending = None;
        tab.typed = false;
        if let Some(webview) = &tab.webview {
            tab.loading = true;
            native::expect_internal(page.as_str());
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
        let text = t("n_local_window").replace("{target}", &target);
        let action = replay.then(|| (t("open"), NoticeAction::OpenTab(url.to_string())));
        self.notice(window, text, action, 12000);
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
        if !over_network || target.is_empty() || crate::shield::is_local_host(&site) || !self.local_noticed.insert(site.clone()) {
            return;
        }
        let tab_id = tab.id;
        let text = t("n_local_blocked").replace("{site}", &site).replace("{target}", &target);
        self.notice(
            window,
            text,
            Some((t("allow"), NoticeAction::AllowLocal(window, tab_id, site))),
            14000,
        );
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
                if let Some(webview) = &tab.webview {
                    tab.loading = true;
                    native::expect_internal(page.as_str());
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
