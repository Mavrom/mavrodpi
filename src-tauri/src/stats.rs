// Anlık ağ trafiği istatistikleri (tüm arabirimlerin toplamı).
// Frontend bunu saniyede bir örnekleyip hız ve oturum toplamını hesaplar.

use std::sync::Mutex;

use serde::Serialize;
use sysinfo::Networks;

pub struct NetState(pub Mutex<Networks>);

impl Default for NetState {
    fn default() -> Self {
        NetState(Mutex::new(Networks::new_with_refreshed_list()))
    }
}

#[derive(Serialize)]
pub struct NetSample {
    pub rx: u64,
    pub tx: u64,
}

#[tauri::command]
pub fn net_stats(state: tauri::State<NetState>) -> NetSample {
    let mut nets = match state.0.lock() {
        Ok(n) => n,
        Err(_) => return NetSample { rx: 0, tx: 0 },
    };
    nets.refresh();
    let mut rx = 0u64;
    let mut tx = 0u64;
    for (_name, data) in nets.iter() {
        rx += data.total_received();
        tx += data.total_transmitted();
    }
    NetSample { rx, tx }
}
