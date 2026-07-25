use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteStatus {
    pub name: String,
    pub host: String,
    pub reachable: bool,
    pub latency_ms: Option<u128>,
    pub status_code: Option<u16>,
    pub detail: String,
}

const SITES: &[(&str, &str)] = &[
    ("Discord", "discord.com"),
    ("YouTube", "youtube.com"),
    ("X", "x.com"),
    ("Twitch", "twitch.tv"),
    ("Instagram", "instagram.com"),
    ("Reddit", "reddit.com"),
    ("Spotify", "spotify.com"),
];

fn check_site(name: String, host: String) -> SiteStatus {
    let client = match Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(8))
        .redirect(Policy::limited(3))
        .user_agent("MavroDPI/0.3 connectivity-check")
        .no_proxy()
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            return SiteStatus {
                name,
                host,
                reachable: false,
                latency_ms: None,
                status_code: None,
                detail: format!("HTTPS istemcisi hazırlanamadı: {error}"),
            }
        }
    };

    let started = Instant::now();
    match client.get(format!("https://{host}/")).send() {
        Ok(response) => SiteStatus {
            name,
            host,
            reachable: true,
            latency_ms: Some(started.elapsed().as_millis()),
            status_code: Some(response.status().as_u16()),
            detail: format!("TLS/HTTPS yanıtı alındı: {}", response.status().as_u16()),
        },
        Err(error) => SiteStatus {
            name,
            host,
            reachable: false,
            latency_ms: Some(started.elapsed().as_millis()),
            status_code: None,
            detail: format!("TLS/HTTPS bağlantısı kurulamadı: {error}"),
        },
    }
}

#[tauri::command]
pub fn check_sites() -> Vec<SiteStatus> {
    let handles: Vec<_> = SITES
        .iter()
        .map(|(name, host)| {
            let name = name.to_string();
            let host = host.to_string();
            std::thread::spawn(move || check_site(name, host))
        })
        .collect();

    handles
        .into_iter()
        .map(|handle| {
            handle.join().unwrap_or_else(|_| SiteStatus {
                name: "?".into(),
                host: "?".into(),
                reachable: false,
                latency_ms: None,
                status_code: None,
                detail: "Bağlantı kontrolü beklenmedik biçimde sonlandı.".into(),
            })
        })
        .collect()
}
