//! Standort des Geräts – für eingebaute Apps, nie für Webseiten.
//!
//! Gefragt wird macOS (CoreLocation), und zwar erst, wenn eine App den Standort
//! braucht und der Mensch am Browser es ihr erlaubt hat (`pages::app_message`).
//! macOS fragt beim ersten Mal zusätzlich selbst, ob Zunder den Standort
//! verwenden darf. Was zurückkommt, geht als Ereignis an die App
//! (`UserEvent::Geo`) – dort wird es gerundet, bevor es eine Seite erreicht.
//!
//! In einem Bau mit Prüfhilfe wird die echte Ortung nie angefasst: Dort gibt es
//! nur einen gestellten Standort (`ZUNDER_MOCK_LOCATION=<breite>,<länge>`).
//!
//! Alles hier läuft auf dem Hauptthread.

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Sel};
use objc2::{class, msg_send, sel, Encode, Encoding, RefEncode};

use crate::app::UserEvent;

/// Ergebnis einer Ortung.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fix {
    /// Breite und Länge, so genau wie macOS sie nennt.
    At(f64, f64),
    /// macOS gibt Zunder den Standort nicht (abgelehnt oder Ortung aus).
    Denied,
    /// Kein Standort zu bekommen.
    Failed,
}

/// Liest einen gestellten Standort: `52.52,13.40`. Alles andere heißt „abgelehnt“.
pub fn parse_mock(text: &str) -> Fix {
    let mut parts = text.split(',').map(|part| part.trim().parse::<f64>());
    match (parts.next(), parts.next(), parts.next()) {
        (Some(Ok(latitude)), Some(Ok(longitude)), None)
            if latitude.is_finite() && longitude.is_finite() && latitude.abs() <= 90.0 && longitude.abs() <= 180.0 =>
        {
            Fix::At(latitude, longitude)
        }
        _ => Fix::Denied,
    }
}

/// Fragt nach dem Standort. Die Antwort kommt als `UserEvent::Geo` – auch wenn
/// macOS erst den Menschen fragen muss.
pub fn request() {
    if crate::dev::BUILT_IN {
        // Probe-App: nie die echte Ortung, nie eine Frage von macOS.
        let fix = crate::dev::var("ZUNDER_MOCK_LOCATION").map_or(Fix::Failed, |text| parse_mock(&text));
        crate::native::post(UserEvent::Geo(fix));
        return;
    }
    real_request();
}

#[link(name = "CoreLocation", kind = "framework")]
extern "C" {}

/// `CLLocationCoordinate2D`
#[repr(C)]
#[derive(Clone, Copy)]
struct Coordinate {
    latitude: f64,
    longitude: f64,
}

unsafe impl Encode for Coordinate {
    const ENCODING: Encoding = Encoding::Struct("CLLocationCoordinate2D", &[Encoding::Double, Encoding::Double]);
}

unsafe impl RefEncode for Coordinate {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
}

type Obj = *mut AnyObject;

thread_local! {
    static MANAGER: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
    static DELEGATE: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
    /// Läuft gerade eine Anfrage? Dann löst eine zweite keine neue aus.
    static WAITING: Cell<bool> = const { Cell::new(false) };
}

fn finish(fix: Fix) {
    if WAITING.replace(false) {
        crate::native::post(UserEvent::Geo(fix));
    }
}

/// Der Stand der Erlaubnis bei macOS: 0 noch nicht gefragt, 1 gesperrt,
/// 2 abgelehnt, ab 3 erlaubt.
unsafe fn status_of(manager: &AnyObject) -> i32 {
    let modern: bool = msg_send![manager, respondsToSelector: sel!(authorizationStatus)];
    if modern {
        msg_send![manager, authorizationStatus]
    } else {
        msg_send![class!(CLLocationManager), authorizationStatus]
    }
}

unsafe fn locate(manager: &AnyObject) {
    let knows: bool = msg_send![manager, respondsToSelector: sel!(requestLocation)];
    if knows {
        let _: () = msg_send![manager, requestLocation];
    } else {
        finish(Fix::Failed);
    }
}

/// macOS meldet, dass sich die Erlaubnis geändert hat – auch gleich nach dem
/// Anlegen, und nachdem der Mensch die Frage beantwortet hat.
extern "C-unwind" fn authorization_changed(_this: Obj, _cmd: Sel, manager: Obj) {
    if manager.is_null() || !WAITING.get() {
        return;
    }
    unsafe {
        match status_of(&*manager) {
            0 => {}
            1 | 2 => finish(Fix::Denied),
            _ => locate(&*manager),
        }
    }
}

/// Dasselbe auf Systemen vor macOS 11.
extern "C-unwind" fn authorization_changed_old(this: Obj, cmd: Sel, manager: Obj, _status: i32) {
    authorization_changed(this, cmd, manager);
}

extern "C-unwind" fn located(_this: Obj, _cmd: Sel, _manager: Obj, locations: Obj) {
    if locations.is_null() {
        return;
    }
    unsafe {
        let last: Obj = msg_send![locations, lastObject];
        if last.is_null() {
            return;
        }
        let at: Coordinate = msg_send![last, coordinate];
        finish(Fix::At(at.latitude, at.longitude));
    }
}

extern "C-unwind" fn failed(_this: Obj, _cmd: Sel, _manager: Obj, error: Obj) {
    // kCLErrorDenied = 1: macOS gibt den Standort nicht heraus.
    let code: isize = if error.is_null() { 0 } else { unsafe { msg_send![error, code] } };
    finish(if code == 1 { Fix::Denied } else { Fix::Failed });
}

fn delegate() -> Retained<AnyObject> {
    DELEGATE.with(|slot| {
        slot.borrow_mut()
            .get_or_insert_with(|| unsafe {
                let name = c"ZunderLocator";
                let class = match AnyClass::get(name) {
                    Some(class) => class,
                    None => {
                        let mut builder =
                            objc2::runtime::ClassBuilder::new(name, class!(NSObject)).expect("Klassenname ist frei");
                        builder.add_method::<AnyObject, _>(
                            sel!(locationManagerDidChangeAuthorization:),
                            authorization_changed as extern "C-unwind" fn(Obj, Sel, Obj),
                        );
                        builder.add_method::<AnyObject, _>(
                            sel!(locationManager:didChangeAuthorizationStatus:),
                            authorization_changed_old as extern "C-unwind" fn(Obj, Sel, Obj, i32),
                        );
                        builder.add_method::<AnyObject, _>(
                            sel!(locationManager:didUpdateLocations:),
                            located as extern "C-unwind" fn(Obj, Sel, Obj, Obj),
                        );
                        builder.add_method::<AnyObject, _>(
                            sel!(locationManager:didFailWithError:),
                            failed as extern "C-unwind" fn(Obj, Sel, Obj, Obj),
                        );
                        builder.register()
                    }
                };
                msg_send![class, new]
            })
            .clone()
    })
}

fn manager() -> Retained<AnyObject> {
    MANAGER.with(|slot| {
        slot.borrow_mut()
            .get_or_insert_with(|| unsafe {
                let manager: Retained<AnyObject> = msg_send![class!(CLLocationManager), new];
                let delegate = delegate();
                let _: () = msg_send![&*manager, setDelegate: &*delegate];
                // Auf einen Kilometer genügt – genauer bekommt es ohnehin keine App.
                let _: () = msg_send![&*manager, setDesiredAccuracy: 1000.0f64];
                manager
            })
            .clone()
    })
}

fn real_request() {
    if WAITING.replace(true) {
        return;
    }
    let manager = manager();
    unsafe {
        match status_of(&manager) {
            // Noch nicht gefragt: macOS fragt jetzt – die Antwort kommt über
            // `authorization_changed`.
            0 => {
                let knows: bool = msg_send![&*manager, respondsToSelector: sel!(requestWhenInUseAuthorization)];
                if knows {
                    let _: () = msg_send![&*manager, requestWhenInUseAuthorization];
                } else {
                    locate(&manager);
                }
            }
            1 | 2 => finish(Fix::Denied),
            _ => locate(&manager),
        }
    }
}

/// Prüfhilfe: reicht einen von macOS selbst gebauten Standort durch dieselbe
/// Stelle, über die echte Ortungen ankommen – ohne die Ortung zu berühren.
pub fn deliver_for_test(latitude: f64, longitude: f64) {
    if !crate::dev::enabled() {
        return;
    }
    WAITING.set(true);
    unsafe {
        let allocated: Obj = msg_send![class!(CLLocation), alloc];
        let location: Obj = msg_send![allocated, initWithLatitude: latitude, longitude: longitude];
        if location.is_null() {
            finish(Fix::Failed);
            return;
        }
        let list: Retained<AnyObject> = msg_send![class!(NSArray), arrayWithObject: location];
        let _: () = msg_send![location, release];
        let delegate = delegate();
        let _: () = msg_send![&*delegate, locationManager: std::ptr::null_mut::<AnyObject>(), didUpdateLocations: &*list];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gestellter_standort() {
        assert_eq!(parse_mock("52.52,13.40"), Fix::At(52.52, 13.4));
        assert_eq!(parse_mock(" -33.87 , 151.21 "), Fix::At(-33.87, 151.21));
        for text in ["", "denied", "52.52", "52.52,13.4,7", "91,0", "0,181", "nan,0", "a,b"] {
            assert_eq!(parse_mock(text), Fix::Denied, "{text}");
        }
    }
}
