//! Lesemodus: zeigt den Text einer Seite aufgeräumt an – ohne Menüs, Werbung
//! und Seitenleisten.
//!
//! Die Arbeit macht ein Skript in Zunders abgeschotteter Welt der Seite
//! (`ui/reader.js`): Es sucht den Artikel, baut daraus eine bereinigte Fassung
//! und legt sie über die Seite. Alles bleibt im Dokument der Seite selbst –
//! der Lesemodus bekommt dadurch keinerlei Sonderrechte, und es entsteht keine
//! eigene Seite, die fremde Inhalte unter Zunders Namen zeigen würde.

use std::sync::OnceLock;

use serde_json::{json, Map, Value};
use tao::window::WindowId;

use crate::app::{App, TabId, UserEvent};
use crate::i18n::t;
use crate::native;

const READER_JS: &str = include_str!("../ui/reader.js");
const SPRITE: &str = include_str!("../ui/icons.svg");

/// Schriftgröße im Lesemodus (Punkt), wenn noch keine gewählt wurde.
const DEFAULT_SIZE: i64 = 19;

/// Zerlegt die Formen eines Icons aus dem Sprite in eine Beschreibung, aus der
/// das Skript die Elemente einzeln aufbaut. So muss in der Seite nie HTML-Text
/// eingesetzt werden.
fn shapes(name: &str) -> Value {
    let start = format!("<symbol id=\"i-{name}\"");
    let Some(from) = SPRITE.find(&start) else {
        return Value::Array(Vec::new());
    };
    let rest = &SPRITE[from..];
    let Some(open_end) = rest.find('>') else {
        return Value::Array(Vec::new());
    };
    let Some(close) = rest.find("</symbol>") else {
        return Value::Array(Vec::new());
    };
    let body = &rest[open_end + 1..close];

    // Stapel offener Elemente; das unterste sammelt die Formen des Icons.
    let mut stack: Vec<(String, Map<String, Value>, Vec<Value>)> = vec![(String::new(), Map::new(), Vec::new())];
    let finish = |stack: &mut Vec<(String, Map<String, Value>, Vec<Value>)>| {
        if stack.len() > 1 {
            let (tag, attrs, children) = stack.pop().expect("nicht leer");
            let node = json!({ "tag": tag, "attrs": attrs, "children": children });
            stack.last_mut().expect("Wurzel bleibt").2.push(node);
        }
    };
    for piece in body.split('<').skip(1) {
        let Some(end) = piece.find('>') else {
            continue;
        };
        let inner = &piece[..end];
        if inner.starts_with('/') {
            finish(&mut stack);
            continue;
        }
        let closed = inner.ends_with('/');
        let inner = inner.trim_end_matches('/').trim();
        let (tag, mut attributes) = inner.split_once(' ').unwrap_or((inner, ""));
        if !matches!(tag, "g" | "path" | "circle" | "ellipse" | "line" | "rect" | "polyline" | "polygon") {
            continue;
        }
        let mut attrs = Map::new();
        while let Some((key, after)) = attributes.split_once("=\"") {
            let Some((value, remaining)) = after.split_once('"') else {
                break;
            };
            attrs.insert(key.trim().to_string(), Value::from(value));
            attributes = remaining;
        }
        stack.push((tag.to_string(), attrs, Vec::new()));
        if closed {
            finish(&mut stack);
        }
    }
    while stack.len() > 1 {
        finish(&mut stack);
    }
    Value::Array(stack.pop().map(|root| root.2).unwrap_or_default())
}

fn reader_icons() -> &'static Value {
    static ICONS: OnceLock<Value> = OnceLock::new();
    ICONS.get_or_init(|| json!({ "close": shapes("close"), "minus": shapes("minus"), "plus": shapes("plus") }))
}

impl App {
    /// Schaltet den Lesemodus der aktiven Seite ein oder aus.
    pub fn toggle_reader(&mut self, window: WindowId) {
        let Some(tab) = self.windows.get(&window).and_then(|w| w.active_tab()) else {
            return;
        };
        if !(tab.url.starts_with("http://") || tab.url.starts_with("https://")) {
            return;
        }
        let Some(webview) = &tab.webview else {
            return;
        };
        let size = self
            .store
            .setting("reader_size")
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(DEFAULT_SIZE)
            .clamp(14, 30);
        let options = json!({
            "size": size,
            "icons": reader_icons(),
            "strings": {
                "close": t("r_close"),
                "smaller": t("r_smaller"),
                "larger": t("r_larger"),
                "minutes": t("r_minutes"),
            },
        });
        let script = format!("{READER_JS}({options})");
        let proxy = self.proxy.clone();
        let tab_id = tab.id;
        native::eval_isolated(webview, &script, move |result| {
            let _ = proxy.send_event(UserEvent::PageAnswer { window, tab: tab_id, what: "reader", result });
        });
    }

    /// Antwort von Zunders Skript in einer Seite.
    pub fn page_answer(&mut self, window: WindowId, tab_id: TabId, what: &str, result: Option<String>) {
        if what == "source" {
            self.show_source(window, tab_id, result);
            return;
        }
        if what != "reader" {
            return;
        }
        let Some(win) = self.windows.get_mut(&window) else {
            return;
        };
        let Some(tab) = win.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return;
        };
        tab.reader = result.as_deref() == Some("on");
        win.dirty = true;
        if !matches!(result.as_deref(), Some("on") | Some("off")) {
            self.notice(window, t("r_none").to_string(), None, 4000);
        }
    }

    /// Der Lesemodus meldet sich aus der Seite: geschlossen oder Schriftgröße geändert.
    pub fn reader_message(&mut self, window: WindowId, index: usize, data: &Value) {
        if let Some(size) = data.get("size").and_then(Value::as_i64) {
            self.store.set_setting("reader_size", &size.clamp(14, 30).to_string());
        }
        if data.get("on").and_then(Value::as_bool) == Some(false) {
            if let Some(win) = self.windows.get_mut(&window) {
                if let Some(tab) = win.tabs.get_mut(index) {
                    tab.reader = false;
                }
                win.dirty = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_shapes_from_sprite() {
        let close = shapes("close");
        let groups = close.as_array().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0]["tag"], "g");
        assert!(groups[0]["attrs"]["transform"].as_str().unwrap().starts_with("translate("));
        let lines = groups[0]["children"].as_array().unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["tag"], "line");
        assert_eq!(lines[0]["attrs"]["stroke"], "currentColor");
        assert!(lines[0]["attrs"]["x1"].as_str().is_some());
        // Unbekannte Namen ergeben nichts statt eines Fehlers.
        assert_eq!(shapes("gibt-es-nicht"), Value::Array(Vec::new()));
        // Jedes Icon des Lesemodus ist vorhanden.
        for name in ["close", "minus", "plus"] {
            assert!(!shapes(name).as_array().unwrap().is_empty(), "{name}");
        }
    }
}
