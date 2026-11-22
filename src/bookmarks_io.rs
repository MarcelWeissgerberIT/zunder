//! Lesezeichen aus anderen Browsern übernehmen und als Datei sichern.
//!
//! Gelesen werden zwei Formate: die HTML-Datei, die Chrome, Firefox, Safari,
//! Edge und Brave unter „Lesezeichen exportieren“ schreiben, und die Datei, in
//! der Browser auf Chromium-Basis ihre Lesezeichen selbst ablegen. Die zweite
//! lässt sich ohne Umweg direkt übernehmen, wenn so ein Browser installiert ist.
//! Geschrieben wird immer die HTML-Datei – die liest jeder Browser wieder ein.

use std::path::PathBuf;

use serde_json::{json, Value};
use tao::window::WindowId;

use crate::app::{App, NoticeAction};
use crate::i18n::t;
use crate::store::Entry;

#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    pub url: String,
    pub title: String,
    /// Der Ordner, in dem das Lesezeichen lag (der innerste); leer: in keinem.
    pub folder: String,
}

const MAX_ENTRIES: usize = 50_000;

fn web(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
}

/// Wandelt Zeichenverweise (`&amp;`, `&#228;`, `&#xE4;` …) in Text.
fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        // Nur ein kurzes Stück voraus suchen – ein Verweis ist nie länger, und
        // eine Datei voller „&“ soll nicht jedes Mal bis ans Ende durchsucht werden.
        let end = rest.as_bytes()[1..].iter().take(10).position(|b| *b == b';').map(|i| i + 1);
        let decoded = end.and_then(|end| {
            let name = &rest[1..end];
            let c = match name {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                "nbsp" => ' ',
                _ => {
                    let code = match name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => name.strip_prefix('#')?.parse::<u32>().ok()?,
                    };
                    char::from_u32(code).filter(|c| !c.is_control())?
                }
            };
            Some((c, end))
        });
        match decoded {
            Some((c, end)) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Entfernt eingebettete Elemente aus einem Titel und fasst Leerraum zusammen.
fn plain_text(html: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for c in html.chars() {
        match c {
            '<' => inside = true,
            '>' => inside = false,
            _ if !inside => out.push(c),
            _ => {}
        }
    }
    decode_entities(&out).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Wert eines Attributs in einem öffnenden Element (`href="…"`), ohne auf Groß- und Kleinschreibung zu achten.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find(name) {
        let at = from + found;
        from = at + name.len();
        // Der Name muss für sich stehen und ein Gleichheitszeichen folgen.
        let before = lower[..at].chars().last();
        if before.is_some_and(|c| !c.is_whitespace()) {
            continue;
        }
        let after = tag[from..].trim_start();
        let Some(value) = after.strip_prefix('=') else {
            continue;
        };
        let value = value.trim_start();
        let text = match value.chars().next() {
            Some(quote @ ('"' | '\'')) => value[1..].split(quote).next().unwrap_or(""),
            _ => value.split_whitespace().next().unwrap_or(""),
        };
        return Some(decode_entities(text));
    }
    None
}

/// Die HTML-Ausfuhr der Browser: eine Folge von `<A HREF="…">Titel</A>`,
/// Ordner als `<H3>Name</H3>` mit einer eingerückten Liste (`<DL>`) dahinter.
/// Zunder kennt eine Ebene von Ordnern: Es zählt der innerste.
fn parse_html(text: &str) -> Vec<Imported> {
    let lower = text.to_ascii_lowercase();
    let mut items = Vec::new();
    // Ordner, in denen die Stelle gerade steckt (leer: eine Liste ohne Namen).
    let mut stack: Vec<String> = Vec::new();
    let mut heading: Option<String> = None;
    let mut from = 0;
    while let Some(found) = lower[from..].find('<') {
        let start = from + found;
        from = start + 1;
        let rest = &lower[from..];
        let is = |name: &str| {
            rest.strip_prefix(name)
                .is_some_and(|after| after.starts_with(|c: char| c.is_whitespace() || c == '>'))
        };
        if is("/dl") {
            stack.pop();
        } else if is("dl") {
            if stack.len() < 64 {
                stack.push(heading.take().unwrap_or_default());
            }
        } else if is("h3") {
            let Some(tag_end) = rest.find('>').map(|i| from + i) else {
                break;
            };
            let Some(close) = lower[tag_end..].find("</h3").map(|i| tag_end + i) else {
                break;
            };
            // Die Lesezeichenleiste selbst ist kein Ordner.
            let toolbar = lower[start..tag_end].contains("personal_toolbar_folder");
            let name = crate::store::clean_folder(&plain_text(&text[tag_end + 1..close])).unwrap_or_default();
            heading = Some(if toolbar { String::new() } else { name });
            from = close;
        } else if is("a") {
            let Some(tag_end) = rest.find('>').map(|i| from + i) else {
                break;
            };
            let Some(close) = lower[tag_end..].find("</a").map(|i| tag_end + i) else {
                break;
            };
            let tag = &text[start..tag_end];
            if let Some(url) = attribute(tag, "href") {
                if web(&url) {
                    let folder = stack.iter().rev().find(|name| !name.is_empty()).cloned().unwrap_or_default();
                    items.push(Imported { url, title: plain_text(&text[tag_end + 1..close]), folder });
                    if items.len() >= MAX_ENTRIES {
                        break;
                    }
                }
            }
            from = close;
        }
    }
    items
}

/// Die Lesezeichen-Datei der Browser auf Chromium-Basis (Chrome, Brave, Edge, Vivaldi).
fn parse_chromium(text: &str) -> Option<Vec<Imported>> {
    let data: Value = serde_json::from_str(text).ok()?;
    let roots = data.get("roots")?.as_object()?;
    fn walk(node: &Value, depth: usize, folder: &str, out: &mut Vec<Imported>) {
        if depth > 40 || out.len() >= MAX_ENTRIES {
            return;
        }
        match node.get("type").and_then(Value::as_str) {
            Some("url") => {
                let url = node.get("url").and_then(Value::as_str).unwrap_or("");
                if web(url) {
                    let title = node.get("name").and_then(Value::as_str).unwrap_or("");
                    out.push(Imported {
                        url: url.to_string(),
                        title: title.split_whitespace().collect::<Vec<_>>().join(" "),
                        folder: folder.to_string(),
                    });
                }
            }
            _ => {
                // Die obersten Knoten (Leiste, „Weitere“) sind keine Ordner.
                let own = if depth == 0 {
                    None
                } else {
                    node.get("name").and_then(Value::as_str).and_then(crate::store::clean_folder)
                };
                let folder = own.as_deref().unwrap_or(folder);
                if let Some(children) = node.get("children").and_then(Value::as_array) {
                    for child in children {
                        walk(child, depth + 1, folder, out);
                    }
                }
            }
        }
    }
    let mut items = Vec::new();
    // Feste Reihenfolge: erst die Lesezeichenleiste, dann der Rest.
    for name in ["bookmark_bar", "other", "synced"] {
        if let Some(node) = roots.get(name) {
            walk(node, 0, "", &mut items);
        }
    }
    Some(items)
}

/// Liest Lesezeichen aus dem Inhalt einer Datei – das Format wird erkannt.
pub fn parse(text: &str) -> Vec<Imported> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if text.trim_start().starts_with('{') {
        if let Some(items) = parse_chromium(text) {
            return items;
        }
    }
    parse_html(text)
}

/// Schreibt Lesezeichen als HTML-Datei, die jeder Browser wieder einliest –
/// erst die ohne Ordner, dann Ordner für Ordner.
pub fn to_html(entries: &[Entry]) -> String {
    let mut out = String::from(
        "<!DOCTYPE NETSCAPE-Bookmark-file-1>\n\
         <!-- This is an automatically generated file. -->\n\
         <META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=UTF-8\">\n\
         <TITLE>Bookmarks</TITLE>\n<H1>Bookmarks</H1>\n<DL><p>\n",
    );
    let line = |entry: &Entry, indent: &str| {
        let title = if entry.title.is_empty() { &entry.url } else { &entry.title };
        format!(
            "{indent}<DT><A HREF=\"{}\" ADD_DATE=\"{}\">{}</A>\n",
            escape_html(&entry.url),
            entry.ts.max(0),
            escape_html(title)
        )
    };
    for entry in entries.iter().filter(|entry| entry.folder.is_empty()) {
        out.push_str(&line(entry, "    "));
    }
    let mut folders: Vec<&str> = Vec::new();
    for entry in entries {
        if !entry.folder.is_empty() && !folders.contains(&entry.folder.as_str()) {
            folders.push(&entry.folder);
        }
    }
    for folder in folders {
        out.push_str(&format!("    <DT><H3>{}</H3>\n    <DL><p>\n", escape_html(folder)));
        for entry in entries.iter().filter(|entry| entry.folder == folder) {
            out.push_str(&line(entry, "        "));
        }
        out.push_str("    </DL><p>\n");
    }
    out.push_str("</DL><p>\n");
    out
}

/// Browser, deren Lesezeichen sich direkt übernehmen lassen: (Kennung, Name, Ordner unter „Application Support“).
const SOURCES: &[(&str, &str, &str)] = &[
    ("chrome", "Google Chrome", "Google/Chrome"),
    ("brave", "Brave", "BraveSoftware/Brave-Browser"),
    ("edge", "Microsoft Edge", "Microsoft Edge"),
    ("vivaldi", "Vivaldi", "Vivaldi"),
];

fn source_file(id: &str) -> Option<PathBuf> {
    let (_, _, folder) = SOURCES.iter().find(|(source, _, _)| *source == id)?;
    // Nur mit Prüfhilfe: ein anderer Ort statt der echten Browser-Daten.
    let root = match crate::dev::var_os("ZUNDER_IMPORT_ROOT") {
        Some(root) => PathBuf::from(root),
        // In der Sandbox liegt „Application Support“ der anderen Browser
        // außerhalb des eigenen Behälters; lesen darf Zunder dort genau diese
        // Lesezeichen-Dateien (siehe assets/Zunder.entitlements).
        None => match crate::app::home_outside_sandbox() {
            Some(home) => home.join("Library/Application Support"),
            None => dirs::data_dir()?,
        },
    };
    let file = root.join(folder).join("Default").join("Bookmarks");
    file.is_file().then_some(file)
}

/// Welche dieser Browser hier Lesezeichen haben – für die Knöpfe in der Seitenleiste.
pub fn sources() -> Vec<Value> {
    SOURCES
        .iter()
        .filter(|(id, _, _)| source_file(id).is_some())
        .map(|(id, name, _)| json!({ "id": id, "name": name }))
        .collect()
}

fn read_limited(path: &std::path::Path) -> Option<String> {
    let size = std::fs::metadata(path).ok()?.len();
    if size > 50_000_000 {
        return None;
    }
    std::fs::read(path).ok().map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

impl App {
    fn take_bookmarks(&mut self, window: WindowId, text: &str) {
        let items = parse(text);
        if items.is_empty() {
            self.notice(window, t("bm_import_none").to_string(), None, 6000);
            return;
        }
        let rows: Vec<(String, String, String)> =
            items.into_iter().map(|item| (item.url, item.title, item.folder)).collect();
        let (added, known) = self.store.add_bookmarks(&rows);
        let text = t("bm_imported").replace("{n}", &added.to_string()).replace("{k}", &known.to_string());
        self.notice(window, text, None, 7000);
        self.bookmarks_changed(Some(window), "");
    }

    /// Lesezeichen aus einer gewählten Datei übernehmen.
    pub fn import_bookmarks(&mut self, window: WindowId, path: &str) {
        match read_limited(std::path::Path::new(path)) {
            Some(text) => self.take_bookmarks(window, &text),
            None => self.notice(window, t("bm_import_unreadable").to_string(), None, 5000),
        }
    }

    /// Lesezeichen direkt aus einem installierten Browser übernehmen.
    pub fn import_bookmarks_from(&mut self, window: WindowId, browser: &str) {
        match source_file(browser).and_then(|file| read_limited(&file)) {
            Some(text) => self.take_bookmarks(window, &text),
            None => self.notice(window, t("bm_import_unreadable").to_string(), None, 5000),
        }
    }

    pub fn export_bookmarks(&mut self, window: WindowId, path: &str) {
        let entries = self.store.bookmarks("", 200_000);
        match std::fs::write(path, to_html(&entries)) {
            Ok(()) => {
                let text = t("bm_exported").replace("{n}", &entries.len().to_string());
                let action = NoticeAction::Reveal(PathBuf::from(path));
                self.notice(window, text, Some((t("d_show"), action)), 8000);
            }
            Err(_) => self.notice(window, t("bm_export_failed").to_string(), None, 6000),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_export_of_other_browsers() {
        let text = r#"<!DOCTYPE NETSCAPE-Bookmark-file-1>
<META HTTP-EQUIV="Content-Type" CONTENT="text/html; charset=UTF-8">
<TITLE>Bookmarks</TITLE>
<H1>Bookmarks</H1>
<DL><p>
    <DT><H3 ADD_DATE="1" PERSONAL_TOOLBAR_FOLDER="true">Leiste</H3>
    <DL><p>
        <DT><A HREF="https://example.com/a?x=1&amp;y=2" ADD_DATE="1700000000" ICON="data:image/png;base64,AAAA">Beispiel &amp; S&#246;hne</A>
        <DT><a href='https://rust-lang.org/'>Rust   <b>Sprache</b></a>
        <DT><A HREF="javascript:alert(1)">Schadhaft</A>
        <DT><A HREF="place:sort=8">Meistbesucht</A>
        <DT><A HREF=https://ohne.anfuehrung.de/>Ohne</A>
    </DL><p>
    <article>kein Lesezeichen</article>
</DL><p>"#;
        let items = parse(text);
        assert_eq!(
            items,
            vec![
                Imported { url: "https://example.com/a?x=1&y=2".into(), title: "Beispiel & Söhne".into(), folder: String::new() },
                Imported { url: "https://rust-lang.org/".into(), title: "Rust Sprache".into(), folder: String::new() },
                Imported { url: "https://ohne.anfuehrung.de/".into(), title: "Ohne".into(), folder: String::new() },
            ]
        );
    }

    #[test]
    fn folders_survive_import_and_export() {
        let text = r#"<DL><p>
    <DT><A HREF="https://lose.example/">Lose</A>
    <DT><H3 PERSONAL_TOOLBAR_FOLDER="true">Lesezeichenleiste</H3>
    <DL><p>
        <DT><A HREF="https://leiste.example/">In der Leiste</A>
        <DT><H3>Arbeit &amp; Kunden</H3>
        <DL><p>
            <DT><A HREF="https://arbeit.example/">Arbeit</A>
            <DT><H3>Tief</H3>
            <DL><p>
                <DT><A HREF="https://tief.example/">Ganz unten</A>
            </DL><p>
            <DT><A HREF="https://arbeit2.example/">Wieder Arbeit</A>
        </DL><p>
        <DT><A HREF="https://danach.example/">Danach</A>
    </DL><p>
</DL><p>"#;
        let items = parse(text);
        let folders: Vec<(&str, &str)> = items.iter().map(|i| (i.title.as_str(), i.folder.as_str())).collect();
        assert_eq!(
            folders,
            vec![
                ("Lose", ""),
                ("In der Leiste", ""),
                ("Arbeit", "Arbeit & Kunden"),
                ("Ganz unten", "Tief"),
                ("Wieder Arbeit", "Arbeit & Kunden"),
                ("Danach", ""),
            ]
        );
        // Und wieder hinaus: Ordner bleiben Ordner.
        let entries: Vec<Entry> = items
            .iter()
            .enumerate()
            .map(|(i, item)| Entry { id: i as i64, url: item.url.clone(), title: item.title.clone(), ts: 0, icon: None, folder: item.folder.clone() })
            .collect();
        let back = parse(&to_html(&entries));
        assert_eq!(back.len(), items.len());
        for item in &items {
            assert!(back.iter().any(|b| b.url == item.url && b.folder == item.folder), "{}", item.url);
        }
    }

    #[test]
    fn chromium_file() {
        let text = r#"{"checksum":"x","roots":{
            "bookmark_bar":{"type":"folder","name":"Leiste","children":[
                {"type":"url","name":"Eins","url":"https://eins.example/"},
                {"type":"folder","name":"Ordner","children":[
                    {"type":"url","name":"Zwei  mit   Abstand","url":"http://zwei.example/x"},
                    {"type":"url","name":"intern","url":"chrome://settings/"}
                ]}
            ]},
            "other":{"type":"folder","children":[{"type":"url","name":"Drei","url":"https://drei.example/"}]},
            "synced":{"type":"folder","children":[]}
        },"version":1}"#;
        let items = parse(text);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].url, "https://eins.example/");
        assert_eq!(items[1].title, "Zwei mit Abstand");
        assert_eq!((items[0].folder.as_str(), items[1].folder.as_str()), ("", "Ordner"));
        assert_eq!(items[2].title, "Drei");
        // Kein gültiges JSON mit „roots“: als HTML gelesen, ergibt nichts.
        assert!(parse("{ kaputt").is_empty());
    }

    #[test]
    fn export_round_trip() {
        let entries = vec![
            Entry { id: 1, url: "https://a.example/?q=1&r=<2>".into(), title: "A \"und\" <B>".into(), ts: 1_700_000_000, icon: None, folder: String::new() },
            Entry { id: 2, url: "https://b.example/".into(), title: String::new(), ts: 0, icon: None, folder: String::new() },
        ];
        let html = to_html(&entries);
        assert!(html.starts_with("<!DOCTYPE NETSCAPE-Bookmark-file-1>"));
        let back = parse(&html);
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].url, entries[0].url);
        assert_eq!(back[0].title, entries[0].title);
        // Ohne Titel steht die Adresse als Titel.
        assert_eq!(back[1].title, "https://b.example/");
    }

    #[test]
    fn entities() {
        assert_eq!(decode_entities("a &amp; b &lt;c&gt; &quot;d&quot; &#39;e&#x27; &nbsp;"), "a & b <c> \"d\" 'e'  ");
        // Unbekanntes und Unvollständiges bleibt stehen.
        assert_eq!(decode_entities("AT&T &unbekannt; &#xZZ; 5 & 6 &"), "AT&T &unbekannt; &#xZZ; 5 & 6 &");
        // Steuerzeichen entstehen nicht.
        assert_eq!(decode_entities("&#0;&#10;x"), "&#0;&#10;x");
        // Ein Strichpunkt weit hinten gehört nicht mehr zum Verweis; Umlaute davor stören nicht.
        assert_eq!(decode_entities("&ämtlich; &amp;"), "&ämtlich; &");
        assert_eq!(decode_entities("&amp"), "&amp");
    }

    #[test]
    fn hostile_files_stay_fast() {
        // Eine Datei voller „&“ ohne Strichpunkt darf nicht quadratisch lange dauern.
        let started = std::time::Instant::now();
        let text = "&".repeat(2_000_000);
        assert_eq!(decode_entities(&text).len(), 2_000_000);
        // Viele unvollständige Lesezeichen hintereinander.
        let broken = "<a href=\"https://a.example/\">".repeat(50_000);
        assert!(parse(&broken).is_empty());
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }
}
