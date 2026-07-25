use sha2::{Digest, Sha256};
use std::io::Read;

fn main() {
    let service_path = std::path::Path::new("resources/mavrodpi-svc.exe");
    println!("cargo:rerun-if-changed={}", service_path.display());

    let mut service_file =
        std::fs::File::open(service_path).expect("mavrodpi-svc.exe kaynağı bulunamadı");
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = service_file
            .read(&mut buffer)
            .expect("mavrodpi-svc.exe hash doğrulaması okunamadı");
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    println!(
        "cargo:rustc-env=MAVRODPI_SERVICE_SHA256={:x}",
        hasher.finalize()
    );

    tauri_build::build()
}
