//! Zunder – ein schneller, schlichter Webbrowser für den Mac.
//!
//! Die gesamte Hülle (Fenster, Tabs, Adressleiste, Verlauf, Lesezeichen,
//! Downloads, Privatmodus, Werbeblocker, KI-Assistent, Passwort-Tresor) ist in
//! Rust geschrieben.
//! Die Seiten selbst stellt das in macOS eingebaute WebKit dar.

mod ai;
mod app;
mod assets;
mod assistant;
mod blocker;
mod bookmarks_io;
mod commands;
mod dev;
mod filters;
mod i18n;
mod logins;
mod menu;
mod native;
mod omnibox;
mod pages;
mod reader;
mod shield;
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
