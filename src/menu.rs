//! Menüleiste von macOS. Jeder Eintrag trägt eine Befehlskennung, die in
//! `commands.rs` ausgeführt wird – dieselben Kennungen nutzt auch das Menü
//! in der Symbolleiste.

use muda::accelerator::{Accelerator, Code, Key, KeyAccelerator, Modifiers};
use muda::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu};

use crate::i18n::t;

const CMD: Modifiers = Modifiers::META;
const SHIFT: Modifiers = Modifiers::SHIFT;
const ALT: Modifiers = Modifiers::ALT;
const CTRL: Modifiers = Modifiers::CONTROL;

fn item(id: &str, text_key: &str, accelerator: Option<Accelerator>) -> MenuItem {
    MenuItem::with_id(id, t(text_key), true, accelerator)
}

fn key(mods: Modifiers, code: Code) -> Option<Accelerator> {
    Some(Accelerator::new(mods, code))
}

fn sep() -> PredefinedMenuItem {
    PredefinedMenuItem::separator()
}

/// Baut die Menüleiste und hängt sie an die App. Das zurückgegebene Menü
/// muss am Leben bleiben.
pub fn build() -> Menu {
    let menu = Menu::new();

    let about = AboutMetadata {
        name: Some("Zunder".into()),
        version: Some(env!("CARGO_PKG_VERSION").into()),
        comments: Some(t("s_version").replace("{v}", env!("CARGO_PKG_VERSION"))),
        ..Default::default()
    };
    let app = Submenu::new("Zunder", true);
    let _ = app.append_items(&[
        &PredefinedMenuItem::about(Some(t("m_about")), Some(about)),
        &sep(),
        &item("settings", "m_settings", key(CMD, Code::Comma)),
        &item("passwords", "m_passwords", None),
        &sep(),
        &PredefinedMenuItem::services(Some(t("m_services"))),
        &sep(),
        &PredefinedMenuItem::hide(Some(t("m_hide"))),
        &PredefinedMenuItem::hide_others(Some(t("m_hide_others"))),
        &PredefinedMenuItem::show_all(Some(t("m_show_all"))),
        &sep(),
        &item("quit", "m_quit", key(CMD, Code::KeyQ)),
    ]);

    let file = Submenu::new(t("m_file"), true);
    let _ = file.append_items(&[
        &item("new_tab", "m_new_tab", key(CMD, Code::KeyT)),
        &item("new_window", "m_new_window", key(CMD, Code::KeyN)),
        &item("new_private", "m_new_private", key(CMD | SHIFT, Code::KeyN)),
        &item("new_stealth", "m_new_stealth", key(CMD | ALT, Code::KeyN)),
        &item("open_location", "m_open_location", key(CMD, Code::KeyL)),
        &sep(),
        &item("app_weather", "m_app_weather", None),
        &item("app_time", "m_app_time", key(CMD | ALT, Code::KeyZ)),
        &item("app_security", "m_app_security", None),
        &item("app_request", "m_app_request", None),
        &item("app_tasks", "m_app_tasks", key(CMD | ALT, Code::KeyK)),
        &item("app_inbox", "m_app_inbox", None),
        &item("app_notes", "m_app_notes", None),
        &item("app_read", "m_app_read", None),
        &item("app_memory", "m_app_memory", None),
        &sep(),
        &item("close_tab", "m_close_tab", key(CMD, Code::KeyW)),
        &item("close_window", "m_close_window", key(CMD | SHIFT, Code::KeyW)),
        &sep(),
        &item("print", "m_print", key(CMD, Code::KeyP)),
    ]);

    let edit = Submenu::new(t("m_edit"), true);
    let _ = edit.append_items(&[
        &PredefinedMenuItem::undo(Some(t("m_undo"))),
        &PredefinedMenuItem::redo(Some(t("m_redo"))),
        &sep(),
        &PredefinedMenuItem::cut(Some(t("m_cut"))),
        &PredefinedMenuItem::copy(Some(t("m_copy"))),
        &PredefinedMenuItem::paste(Some(t("m_paste"))),
        &PredefinedMenuItem::select_all(Some(t("m_select_all"))),
        &sep(),
        &item("find", "m_find", key(CMD, Code::KeyF)),
        &item("find_next", "m_find_next", key(CMD, Code::KeyG)),
        &item("find_prev", "m_find_prev", key(CMD | SHIFT, Code::KeyG)),
        &sep(),
        &item("fill_login", "m_fill_login", key(CMD | SHIFT, Code::KeyK)),
    ]);

    // „+“ und „-“ als Zeichen hinterlegen, damit die Kürzel auf jeder
    // Tastaturbelegung dort liegen, wo die Zeichen aufgedruckt sind.
    let zoom_in = item("zoom_in", "m_zoom_in", None);
    let _ = zoom_in.set_key_accelerator(Some(KeyAccelerator::new(CMD, Key::Character("+".into()))));
    let zoom_out = item("zoom_out", "m_zoom_out", None);
    let _ = zoom_out.set_key_accelerator(Some(KeyAccelerator::new(CMD, Key::Character("-".into()))));

    let view = Submenu::new(t("m_view"), true);
    let _ = view.append_items(&[
        &item("reload", "m_reload", key(CMD, Code::KeyR)),
        &item("hard_reload", "m_hard_reload", key(CMD | SHIFT, Code::KeyR)),
        &item("stop", "m_stop", key(CMD, Code::Period)),
        &sep(),
        &zoom_in,
        &zoom_out,
        &item("zoom_reset", "m_zoom_reset", key(CMD, Code::Digit0)),
        &sep(),
        &item("reader", "m_reader", key(CMD | ALT, Code::KeyR)),
        &item("translate_page", "m_translate", key(CMD | ALT, Code::KeyT)),
        &item("site_check", "m_site_check", key(CMD | ALT, Code::KeyS)),
        &sep(),
        &item("assistant", "m_assistant", key(CMD, Code::KeyJ)),
        &item("assistant_summary", "m_assistant_summary", key(CMD | SHIFT, Code::KeyJ)),
        &item("compare", "m_compare", key(CMD | ALT, Code::KeyJ)),
        &item("unsplit", "m_unsplit", None),
        &sep(),
        &PredefinedMenuItem::fullscreen(Some(t("m_fullscreen"))),
        &sep(),
        &item("mobile_view", "m_mobile_view", key(CMD | SHIFT, Code::KeyM)),
        &sep(),
        &item("devtools", "m_devtools", key(CMD | ALT, Code::KeyI)),
        &item("view_source", "m_view_source", key(CMD | ALT, Code::KeyU)),
    ]);

    let history = Submenu::new(t("m_history"), true);
    let _ = history.append_items(&[
        &item("back", "m_back", key(CMD, Code::BracketLeft)),
        &item("forward", "m_forward", key(CMD, Code::BracketRight)),
        &sep(),
        &item("reopen_tab", "m_reopen_tab", key(CMD | SHIFT, Code::KeyT)),
        &item("history", "m_show_history", key(CMD, Code::KeyY)),
        &sep(),
        &item("clear_data", "m_clear_data", key(CMD | SHIFT, Code::Backspace)),
    ]);

    let bookmarks = Submenu::new(t("m_bookmarks"), true);
    let _ = bookmarks.append_items(&[
        &item("bookmark_page", "m_bookmark_page", key(CMD, Code::KeyD)),
        &item("read_later", "m_read_later", None),
        &item("remember_page", "m_remember_page", None),
        &item("copy_clean_url", "m_copy_clean_url", None),
        &item("bookmarks", "m_show_bookmarks", key(CMD | ALT, Code::KeyB)),
        &item("bookmark_bar", "m_bookmark_bar", key(CMD | SHIFT, Code::KeyB)),
    ]);

    let window = Submenu::new(t("m_window"), true);
    let _ = window.append_items(&[
        &PredefinedMenuItem::minimize(Some(t("m_minimize"))),
        &PredefinedMenuItem::maximize(Some(t("m_zoom_window"))),
        &sep(),
        &item("next_tab", "m_next_tab", key(CTRL, Code::Tab)),
        &item("prev_tab", "m_prev_tab", key(CTRL | SHIFT, Code::Tab)),
        &item("command_palette", "m_command_palette", key(CMD, Code::KeyK)),
        &item("tab_search", "m_tab_search", key(CMD | SHIFT, Code::KeyA)),
        &item("tab_overview", "m_tab_overview", key(CMD | SHIFT, Code::Backslash)),
        &item("duplicate_tab", "m_duplicate_tab", None),
        &sep(),
        &item("downloads", "m_downloads", key(CMD | ALT, Code::KeyL)),
        &sep(),
    ]);
    let digits = [
        Code::Digit1,
        Code::Digit2,
        Code::Digit3,
        Code::Digit4,
        Code::Digit5,
        Code::Digit6,
        Code::Digit7,
        Code::Digit8,
        Code::Digit9,
    ];
    for (i, code) in digits.iter().enumerate() {
        let n = i + 1;
        let entry = MenuItem::with_id(
            format!("tab_{n}"),
            t("m_tab_n").replace("{n}", &n.to_string()),
            true,
            key(CMD, *code),
        );
        let _ = window.append(&entry);
    }

    let _ = menu.append_items(&[&app, &file, &edit, &view, &history, &bookmarks, &window]);
    menu.init_for_nsapp();
    window.set_as_windows_menu_for_nsapp();
    menu
}
