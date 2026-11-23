//! Kamera und Mikrofon je Website.
//!
//! Will eine Seite aufnehmen, fragt Zunder selbst – mit dem Namen der Website,
//! die fragt. Die Antwort gilt für diese eine Anfrage; mit dem Häkchen
//! „merken“ gilt sie für die Website, bis man sie in den Einstellungen wieder
//! zurücknimmt. Hier steht nur, was sich ohne Fenster entscheiden lässt; den
//! Dialog und den Anschluss an WebKit hat `native.rs`.
//!
//! Die Regeln, kurz:
//! * Ein gemerktes Nein gilt überall – für alles, was die Website einbettet,
//!   dort, wo sie selbst nur als Rahmen in einer anderen steckt, und in
//!   privaten Fenstern.
//! * Ein gemerktes Ja gilt nur für die Seite selbst, nicht für Rahmen, die sie
//!   einbettet, und nicht in privaten Fenstern: Dort fragt Zunder jedes Mal.
//! * Gemerkt wird nur für verschlüsselte Websites (`https`).
//! * Ein Nein ohne „merken“ gilt für die Website in diesem Tab, bis der Mensch
//!   selbst neu lädt oder selbst eine Adresse aufruft. Die Frage hält den
//!   ganzen Browser an – keine Seite soll ihn damit festhalten können, auch
//!   nicht im Wechsel mit einer zweiten (`Refusals`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Mehr Websites merkt sich Zunder nicht.
pub const MAX_SITES: usize = 500;

/// Was eine Website darf. `None`: nicht entschieden – dann wird gefragt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub microphone: Option<bool>,
}

impl Rule {
    pub fn is_empty(&self) -> bool {
        self.camera.is_none() && self.microphone.is_none()
    }
}

/// Die gemerkten Entscheidungen: Website → Regel.
pub type Rules = BTreeMap<String, Rule>;

/// Was eine Seite gerade haben möchte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Want {
    Camera,
    Microphone,
    Both,
}

impl Want {
    /// WebKits Kennzahl (`WKMediaCaptureType`).
    pub fn from_kind(kind: isize) -> Option<Want> {
        match kind {
            0 => Some(Want::Camera),
            1 => Some(Want::Microphone),
            2 => Some(Want::Both),
            _ => None,
        }
    }

    pub fn camera(self) -> bool {
        matches!(self, Want::Camera | Want::Both)
    }

    pub fn microphone(self) -> bool {
        matches!(self, Want::Microphone | Want::Both)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Grant,
    Deny,
    Ask,
}

/// Entscheidet, soweit es ohne Rückfrage geht.
///
/// `rule` ist das Gemerkte für die Website im Tab, `refused` das, was in
/// diesem Besuch schon abgelehnt wurde.
pub fn verdict(want: Want, rule: Rule, refused: Rule, main_frame: bool, private: bool) -> Verdict {
    let no = |remembered: Option<bool>, here: Option<bool>| remembered == Some(false) || here == Some(false);
    if (want.camera() && no(rule.camera, refused.camera)) || (want.microphone() && no(rule.microphone, refused.microphone)) {
        return Verdict::Deny;
    }
    let yes = (!want.camera() || rule.camera == Some(true)) && (!want.microphone() || rule.microphone == Some(true));
    if yes && main_frame && !private {
        return Verdict::Grant;
    }
    Verdict::Ask
}

/// Nach so vielen abgelehnten Fragen in Folge fragt in einem Tab keine Seite
/// mehr, bis der Mensch dort selbst weitergeht (Klick, Adresse, Neuladen).
pub const PATIENCE: u8 = 3;

/// Was in einem Tab abgelehnt wurde, seit der Mensch dort zuletzt selbst neu
/// geladen oder eine Adresse aufgerufen hat.
///
/// Gemerkt wird je Website – nicht nur für die, die gerade im Tab steht: Zwei
/// Websites, die einander im Wechsel aufrufen, bekämen sonst nach jedem
/// Wechsel eine neue Frage. Und nach `PATIENCE` abgelehnten Fragen in Folge
/// ist für alle Schluss, wie viele Namen eine Seite auch aufbietet – bis der
/// Mensch selbst weiterklickt (`moved_on`).
#[derive(Clone, Debug, Default)]
pub struct Refusals {
    sites: Vec<(String, Rule)>,
    /// Abgelehnte Fragen, seit der Mensch zuletzt selbst weitergegangen ist.
    count: u8,
}

impl Refusals {
    /// Was dieser Website in diesem Tab schon abgelehnt wurde.
    pub fn of(&self, site: &str) -> Rule {
        self.sites.iter().find(|(known, _)| known == site).map(|(_, rule)| *rule).unwrap_or_default()
    }

    /// Ist die Geduld des Tabs erschöpft? Dann stellt dort niemand mehr eine
    /// Frage. Wer ohne Frage auskommt – eine Website mit gemerktem Ja –, ist
    /// davon nicht betroffen.
    pub fn exhausted(&self) -> bool {
        self.count >= PATIENCE
    }

    /// Ein Nein auf die Frage einer Website.
    pub fn refuse(&mut self, site: &str, want: Want) {
        self.count = self.count.saturating_add(1);
        match self.sites.iter_mut().find(|(known, _)| known == site) {
            Some((_, rule)) => answer(rule, want, false),
            None => {
                // Die Liste bleibt kurz: Fällt die älteste Website heraus, darf
                // sie zwar wieder fragen – aber nur, wenn die Geduld es zulässt.
                if self.sites.len() >= REFUSED_SITES {
                    self.sites.remove(0);
                }
                let mut rule = Rule::default();
                answer(&mut rule, want, false);
                self.sites.push((site.to_string(), rule));
            }
        }
    }

    /// Der Mensch ist in diesem Tab selbst weitergegangen (ein Klick auf einen
    /// Link, ein abgeschicktes Formular): Die Geduld beginnt neu. Was einzelnen
    /// Websites abgelehnt wurde, bleibt.
    pub fn moved_on(&mut self) {
        self.count = 0;
    }
}

/// So viele Websites mit einem Nein führt ein Tab.
const REFUSED_SITES: usize = 24;

/// Nimmt zu einer Regel die Verbote einer zweiten hinzu: Ein Nein aus einer
/// von beiden bleibt ein Nein. (Ein Ja der zweiten zählt nicht.)
pub fn with_refusals(rule: Rule, other: Rule) -> Rule {
    let no = |own: Option<bool>, theirs: Option<bool>| if theirs == Some(false) { Some(false) } else { own };
    Rule { camera: no(rule.camera, other.camera), microphone: no(rule.microphone, other.microphone) }
}

/// Trägt eine Antwort in eine Regel ein – für genau das, wonach gefragt wurde.
pub fn answer(rule: &mut Rule, want: Want, allowed: bool) {
    if want.camera() {
        rule.camera = Some(allowed);
    }
    if want.microphone() {
        rule.microphone = Some(allowed);
    }
}

/// Der Name, unter dem sich Zunder eine Website merkt: ihr Rechner, bei einem
/// besonderen Anschluss mit Nummer. Nur für verschlüsselte Websites – alles
/// andere wird jedes Mal gefragt.
pub fn site_key(protocol: &str, host: &str, port: isize) -> Option<String> {
    if !protocol.eq_ignore_ascii_case("https") {
        return None;
    }
    let label = site_label(host, port);
    (!label.is_empty() && site_ok(&label)).then_some(label)
}

/// Der Name einer Website, wie er im Dialog steht (leer, wenn sie keinen hat).
pub fn site_label(host: &str, port: isize) -> String {
    let host = host.trim().trim_end_matches('.');
    // WebKit nennt Rechner in der ASCII-Schreibweise. Käme doch einmal ein Name
    // mit anderen Zeichen, wird er umgeschrieben: Im Dialog steht kein
    // Buchstabe, der einem anderen nur gleicht.
    let host = if host.is_ascii() {
        host.to_ascii_lowercase()
    } else {
        url::Host::parse(host).map(|host| host.to_string()).unwrap_or_default()
    };
    if host.is_empty() {
        return String::new();
    }
    // Eine IPv6-Adresse braucht ihre Klammern, sonst wäre der Anschluss nicht zu erkennen.
    let host = if host.contains(':') && !host.starts_with('[') { format!("[{host}]") } else { host };
    if (1..=65535).contains(&port) && port != 443 {
        format!("{host}:{port}")
    } else {
        host
    }
}

/// Sieht das aus wie der Name einer Website? Geprüft wird, bevor etwas
/// gespeichert oder aus den Einstellungen gelesen wird.
pub fn site_ok(site: &str) -> bool {
    !site.is_empty()
        && site.len() <= 260
        && site.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':' | b'[' | b']'))
}

/// Liest die gespeicherten Regeln. Was nicht passt, fällt weg.
pub fn parse(text: &str) -> Rules {
    let raw: BTreeMap<String, Rule> = serde_json::from_str(text).unwrap_or_default();
    raw.into_iter().filter(|(site, rule)| site_ok(site) && !rule.is_empty()).take(MAX_SITES).collect()
}

/// Lässt sich für diese Website noch etwas merken? Nicht mehr, wenn die Liste
/// voll ist und die Website nicht schon darin steht.
pub fn has_room(rules: &Rules, site: &str) -> bool {
    rules.contains_key(site) || rules.len() < MAX_SITES
}

/// Merkt sich eine Antwort. Rückgabe: Hat sich etwas geändert?
pub fn remember(rules: &mut Rules, site: &str, want: Want, allowed: bool) -> bool {
    if !site_ok(site) || !has_room(rules, site) {
        return false;
    }
    let rule = rules.entry(site.to_string()).or_default();
    let before = *rule;
    answer(rule, want, allowed);
    *rule != before
}

/// Nimmt eine gemerkte Entscheidung zurück (`kind`: `camera` oder `mic`).
pub fn forget(rules: &mut Rules, site: &str, kind: &str) -> bool {
    let Some(rule) = rules.get_mut(site) else {
        return false;
    };
    let before = *rule;
    match kind {
        "camera" => rule.camera = None,
        "mic" => rule.microphone = None,
        _ => return false,
    }
    let changed = *rule != before;
    if rule.is_empty() {
        rules.remove(site);
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(camera: Option<bool>, microphone: Option<bool>) -> Rule {
        Rule { camera, microphone }
    }

    #[test]
    fn asks_until_something_is_remembered() {
        let none = Rule::default();
        assert_eq!(verdict(Want::Camera, none, none, true, false), Verdict::Ask);
        assert_eq!(verdict(Want::Both, rule(Some(true), None), none, true, false), Verdict::Ask);
        assert_eq!(verdict(Want::Camera, rule(Some(true), None), none, true, false), Verdict::Grant);
        assert_eq!(verdict(Want::Both, rule(Some(true), Some(true)), none, true, false), Verdict::Grant);
        // Das Mikrofon ist erlaubt – die Kamera damit noch lange nicht.
        assert_eq!(verdict(Want::Camera, rule(None, Some(true)), none, true, false), Verdict::Ask);
    }

    #[test]
    fn a_remembered_yes_is_for_the_page_itself_only() {
        let yes = rule(Some(true), Some(true));
        let none = Rule::default();
        // Ein eingebetteter Rahmen erbt das Ja nicht …
        assert_eq!(verdict(Want::Camera, yes, none, false, false), Verdict::Ask);
        // … und in einem privaten Fenster wird jedes Mal gefragt.
        assert_eq!(verdict(Want::Both, yes, none, true, true), Verdict::Ask);
    }

    #[test]
    fn a_no_holds_everywhere() {
        let none = Rule::default();
        let no_camera = rule(Some(false), Some(true));
        assert_eq!(verdict(Want::Camera, no_camera, none, true, false), Verdict::Deny);
        assert_eq!(verdict(Want::Both, no_camera, none, true, false), Verdict::Deny);
        assert_eq!(verdict(Want::Camera, no_camera, none, false, false), Verdict::Deny);
        assert_eq!(verdict(Want::Camera, no_camera, none, true, true), Verdict::Deny);
        assert_eq!(verdict(Want::Microphone, no_camera, none, true, false), Verdict::Grant);
        // Ein Nein aus diesem Besuch schlägt auch ein gemerktes Ja.
        let yes = rule(Some(true), Some(true));
        assert_eq!(verdict(Want::Microphone, yes, rule(None, Some(false)), true, false), Verdict::Deny);
        assert_eq!(verdict(Want::Camera, yes, rule(None, Some(false)), true, false), Verdict::Grant);
    }

    #[test]
    fn only_encrypted_sites_are_remembered() {
        assert_eq!(site_key("https", "Example.COM", 0).as_deref(), Some("example.com"));
        assert_eq!(site_key("https", "example.com.", 443).as_deref(), Some("example.com"));
        assert_eq!(site_key("https", "example.com", 8443).as_deref(), Some("example.com:8443"));
        assert_eq!(site_key("https", "2001:db8::1", 0).as_deref(), Some("[2001:db8::1]"));
        assert_eq!(site_key("http", "localhost", 8080), None);
        assert_eq!(site_key("file", "", 0), None);
        assert_eq!(site_key("https", "", 0), None);
        assert_eq!(site_key("https", "bad host", 0), None);
        assert_eq!(site_label("LocalHost", 8080), "localhost:8080");
        assert_eq!(site_label("", 0), "");
        // Ein Name mit Sonderzeichen erscheint in der ASCII-Schreibweise.
        assert_eq!(site_label("bücher.example", 0), "xn--bcher-kva.example");
        assert_eq!(site_key("https", "аpple.example", 0).as_deref(), Some("xn--pple-43d.example"));
    }

    #[test]
    fn answers_are_kept_and_taken_back() {
        let mut rules = Rules::new();
        assert!(remember(&mut rules, "example.com", Want::Both, true));
        assert!(!remember(&mut rules, "example.com", Want::Camera, true));
        assert!(remember(&mut rules, "example.com", Want::Microphone, false));
        assert_eq!(rules["example.com"], rule(Some(true), Some(false)));
        assert!(!remember(&mut rules, "kein name", Want::Camera, true));

        let text = serde_json::to_string(&rules).unwrap();
        assert_eq!(parse(&text), rules);
        // Unbrauchbares in der Einstellung fällt weg, statt alles zu verwerfen …
        let mixed = r#"{"example.com":{"camera":true},"<script>":{"camera":true},"leer.example":{}}"#;
        assert_eq!(parse(mixed).keys().cloned().collect::<Vec<_>>(), vec!["example.com".to_string()]);
        // … und Kaputtes ergibt eine leere Liste.
        assert!(parse("[1,2").is_empty());

        assert!(forget(&mut rules, "example.com", "camera"));
        assert_eq!(rules["example.com"], rule(None, Some(false)));
        assert!(!forget(&mut rules, "example.com", "camera"));
        assert!(!forget(&mut rules, "example.com", "anderes"));
        assert!(forget(&mut rules, "example.com", "mic"));
        assert!(rules.is_empty());
    }

    #[test]
    fn a_no_lasts_for_the_visit() {
        let none = Rule::default();
        let mut tab = Refusals::default();
        assert_eq!(tab.of("a.example"), none);
        tab.refuse("a.example", Want::Camera);
        assert_eq!(tab.of("a.example"), rule(Some(false), None));
        // Das Mikrofon war nicht gefragt – danach darf die Seite noch fragen.
        assert_eq!(verdict(Want::Microphone, none, tab.of("a.example"), true, false), Verdict::Ask);
        assert_eq!(verdict(Want::Camera, none, tab.of("a.example"), true, false), Verdict::Deny);
        // Eine andere Website im selben Tab ist davon nicht betroffen …
        assert_eq!(tab.of("b.example"), none);
        tab.refuse("b.example", Want::Both);
        // … und der Umweg über sie bringt der ersten keine neue Frage.
        assert_eq!(tab.of("a.example"), rule(Some(false), None));
        assert_eq!(tab.of("b.example"), rule(Some(false), Some(false)));
    }

    #[test]
    fn patience_has_an_end() {
        let mut tab = Refusals::default();
        for index in 0..PATIENCE {
            let site = format!("site{index}.example");
            assert_eq!(verdict(Want::Camera, Rule::default(), tab.of(&site), true, false), Verdict::Ask);
            tab.refuse(&site, Want::Camera);
        }
        // Jetzt stellt in diesem Tab niemand mehr eine Frage. Wer keine braucht
        // – eine Website mit gemerktem Ja –, ist davon nicht betroffen.
        assert!(tab.exhausted());
        assert_eq!(verdict(Want::Camera, rule(Some(true), None), tab.of("neu.example"), true, false), Verdict::Grant);
        assert_eq!(verdict(Want::Camera, Rule::default(), tab.of("site0.example"), true, false), Verdict::Deny);
        // Klickt der Mensch selbst weiter, darf eine neue Website wieder fragen –
        // die abgelehnten bleiben abgelehnt.
        tab.moved_on();
        assert!(!tab.exhausted());
        assert_eq!(verdict(Want::Camera, Rule::default(), tab.of("neu.example"), true, false), Verdict::Ask);
        assert_eq!(verdict(Want::Camera, Rule::default(), tab.of("site0.example"), true, false), Verdict::Deny);
        // Lädt der Mensch selbst neu, beginnt der Tab von vorn.
        tab = Refusals::default();
        assert_eq!(verdict(Want::Camera, Rule::default(), tab.of("site0.example"), true, false), Verdict::Ask);
    }

    #[test]
    fn a_no_follows_the_site_into_frames() {
        let none = Rule::default();
        // tracker.example ist die Kamera verboten. Steckt es als Rahmen in
        // einer anderen Website, bleibt es dabei – ihr Ja ändert daran nichts.
        let top = rule(Some(true), Some(true));
        let asker = rule(Some(false), None);
        let merged = with_refusals(top, asker);
        assert_eq!(merged, rule(Some(false), Some(true)));
        assert_eq!(verdict(Want::Camera, merged, none, false, false), Verdict::Deny);
        assert_eq!(verdict(Want::Microphone, merged, none, false, false), Verdict::Ask);
        // Ein Ja der eingebetteten Website zählt im Rahmen nicht.
        assert_eq!(with_refusals(none, rule(Some(true), Some(true))), none);

        // Ein Nein an einen Rahmen sperrt nicht die Seite, die ihn einbettet –
        // ein Nein an die Seite aber auch ihre Rahmen.
        let mut tab = Refusals::default();
        tab.refuse("news.example > ads.example", Want::Camera);
        assert_eq!(tab.of("news.example"), none);
        assert_eq!(with_refusals(tab.of("news.example"), tab.of("news.example > ads.example")), rule(Some(false), None));
        tab.refuse("news.example", Want::Microphone);
        assert_eq!(with_refusals(tab.of("news.example"), tab.of("news.example > andere.example")), rule(None, Some(false)));
    }

    #[test]
    fn a_full_list_offers_no_checkbox() {
        let mut rules = Rules::new();
        for index in 0..MAX_SITES {
            assert!(has_room(&rules, &format!("site{index}.example")));
            remember(&mut rules, &format!("site{index}.example"), Want::Camera, true);
        }
        assert!(!has_room(&rules, "eine-mehr.example"));
        assert!(has_room(&rules, "site3.example"));
    }

    #[test]
    fn many_names_do_not_grow_the_list() {
        let mut tab = Refusals::default();
        for index in 0..200 {
            tab.refuse(&format!("site{index}.example"), Want::Camera);
            tab.moved_on();
        }
        assert!(tab.sites.len() <= REFUSED_SITES);
        assert_eq!(tab.of("site199.example"), rule(Some(false), None));
    }

    #[test]
    fn the_list_has_an_end() {
        let mut rules = Rules::new();
        for index in 0..MAX_SITES {
            assert!(remember(&mut rules, &format!("site{index}.example"), Want::Camera, false));
        }
        assert!(!remember(&mut rules, "eine-mehr.example", Want::Camera, true));
        // Eine schon bekannte Website lässt sich weiter ändern.
        assert!(remember(&mut rules, "site7.example", Want::Camera, true));
        assert_eq!(rules.len(), MAX_SITES);
    }
}
