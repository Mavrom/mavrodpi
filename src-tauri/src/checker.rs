use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde::Serialize;

#[derive(Serialize)]
pub struct SiteStatus {
    pub name: String,
    pub host: String,
    pub reachable: bool,
}

const SITES: &[(&str, &str)] = &[
    ("Discord",    "discord.com"),
    ("YouTube",    "youtube.com"),
    ("Twitter / X","x.com"),
    ("Twitch",     "twitch.tv"),
    ("Instagram",  "instagram.com"),
    ("Reddit",     "reddit.com"),
    ("Spotify",    "spotify.com"),
    ("OnlyFans",   "onlyfans.com"),
];

#[tauri::command]
pub fn check_sites() -> Vec<SiteStatus> {
    let handles: Vec<_> = SITES
        .iter()
        .map(|(name, host)| {
            let name = name.to_string();
            let host = host.to_string();
            std::thread::spawn(move || {
                let addr = format!("{host}:443");
                let reachable = addr
                    .to_socket_addrs()
                    .ok()
                    .and_then(|mut a| a.next())
                    .map(|a| TcpStream::connect_timeout(&a, Duration::from_secs(5)).is_ok())
                    .unwrap_or(false);
                SiteStatus { name, host, reachable }
            })
        })
        .collect();

    handles
        .into_iter()
        .map(|h| {
            h.join().unwrap_or(SiteStatus {
                name: "?".into(),
                host: "?".into(),
                reachable: false,
            })
        })
        .collect()
}
