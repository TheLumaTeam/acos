use acos_host::{api, qemu::QemuBackend};
use std::{path::PathBuf, sync::Arc};

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("--version" | "-V") => {
            println!("acos-host {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        Some("serve") => {
            let data = std::env::var_os("ACOS_DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| dirs_fallback().join("acos"));
            let images = std::env::var_os("ACOS_IMAGE_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| data.join("images"));
            let token = std::env::var("ACOS_API_TOKEN").unwrap_or_else(|_| {
                let token = uuid::Uuid::new_v4().to_string();
                eprintln!("ACOS_API_TOKEN not set; generated local API token: {token}");
                token
            });
            let backend = Arc::new(QemuBackend::new(images, data.join("computers")));
            if let Err(error) = api::serve(data.join("acos.db"), backend, token) {
                eprintln!("acos-host: {error}");
                std::process::exit(1);
            }
            return;
        }
        Some("--help" | "-h") => {
            println!(
                "ACOS Host — agent computer runtime\n\nUSAGE:\n    acos-host serve\n    acos-host [--version | --json | --help]\n\nCOMMANDS:\n    serve          Start the local HTTP API and QEMU backend\n\nOPTIONS:\n    --version, -V  Print version\n    --json         Print machine-readable host status\n    --help, -h     Print this help\n\nENVIRONMENT:\n    ACOS_API_TOKEN   Bearer token (generated if omitted)\n    ACOS_DATA_DIR    Host database and VM state directory\n    ACOS_IMAGE_DIR   Directory containing base QCOW2 images"
            );
            return;
        }
        Some("--json") => {
            println!(
                "{{\"name\":\"ACOS Host\",\"version\":\"{}\",\"status\":\"idle\",\"api_enabled\":false}}",
                env!("CARGO_PKG_VERSION")
            );
            return;
        }
        Some(argument) => {
            eprintln!("unknown argument: {argument}; use --help");
            std::process::exit(2);
        }
        None => {}
    }

    println!("ACOS Host foundation v{}", env!("CARGO_PKG_VERSION"));
    println!("Run `acos-host serve` to start the local API.");
}

fn dirs_fallback() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(".local/share")
}
