//! Zunder – ein schneller, schlichter Webbrowser für den Mac.
//!
//! Die gesamte Hülle (Fenster, Tabs, Adressleiste, Verlauf, Lesezeichen,
//! Downloads, Privatmodus, Werbeblocker, KI-Assistent, Passwort-Tresor) ist in
//! Rust geschrieben.
//! Die Seiten selbst stellt das in macOS eingebaute WebKit dar.

mod ai;
mod app;
mod apps;
mod assets;
mod assistant;
mod blocker;
mod bookmarks_io;
mod commands;
mod dev;
mod filters;
mod geo;
mod i18n;
mod logins;
mod media;
mod menu;
mod native;
mod omnibox;
mod pages;
mod pdfs;
mod reader;
mod shield;
mod inbox;
mod inspect;
mod security;
mod sitecheck;
mod sites;
mod stealth;
mod store;
mod translate;
mod vault;
mod wallpapers;
mod webviews;

fn main() {
    app::run();
}
