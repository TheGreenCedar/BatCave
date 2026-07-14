use base64::{engine::general_purpose::STANDARD, Engine as _};
use minisign_verify::{PublicKey, Signature};
use serde_json::Value;
use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
    process::ExitCode,
};

// This is intentionally far above BatCave's normal updater size while bounding
// the exact-byte buffer used by the release verifier.
const MAX_COMPRESSED_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;

fn decode_wrapped(value: &str, label: &str) -> Result<String, String> {
    let decoded = STANDARD
        .decode(value.trim())
        .map_err(|error| format!("invalid {label} base64: {error}"))?;
    String::from_utf8(decoded).map_err(|error| format!("invalid {label} UTF-8: {error}"))
}

fn read_archive_bounded(archive_path: &Path) -> Result<Vec<u8>, String> {
    let archive_file = File::open(archive_path)
        .map_err(|error| format!("failed to open {}: {error}", archive_path.display()))?;
    let archive_size = archive_file
        .metadata()
        .map_err(|error| format!("failed to inspect {}: {error}", archive_path.display()))?
        .len();
    if archive_size > MAX_COMPRESSED_ARCHIVE_BYTES {
        return Err(format!(
            "compressed updater archive exceeds the {MAX_COMPRESSED_ARCHIVE_BYTES}-byte limit: {}",
            archive_path.display()
        ));
    }

    let capacity = usize::try_from(archive_size)
        .map_err(|_| format!("updater archive is too large for this host: {archive_size} bytes"))?;
    let mut archive = Vec::with_capacity(capacity);
    let mut bounded_file = archive_file.take(archive_size);
    bounded_file
        .read_to_end(&mut archive)
        .map_err(|error| format!("failed to read {}: {error}", archive_path.display()))?;
    if archive.len() != capacity {
        return Err(format!(
            "updater archive size changed while reading {}",
            archive_path.display()
        ));
    }
    let final_size = bounded_file
        .into_inner()
        .metadata()
        .map_err(|error| format!("failed to re-inspect {}: {error}", archive_path.display()))?
        .len();
    if final_size != archive_size {
        return Err(format!(
            "updater archive size changed while reading {}",
            archive_path.display()
        ));
    }
    Ok(archive)
}

fn verify(
    archive_path: &Path,
    signature_path: &Path,
    tauri_config_path: &Path,
    verified_copy_path: &Path,
) -> Result<(), String> {
    let archive = read_archive_bounded(archive_path)?;
    let wrapped_signature = fs::read_to_string(signature_path)
        .map_err(|error| format!("failed to read {}: {error}", signature_path.display()))?;
    let config: Value = serde_json::from_slice(
        &fs::read(tauri_config_path)
            .map_err(|error| format!("failed to read {}: {error}", tauri_config_path.display()))?,
    )
    .map_err(|error| format!("invalid Tauri config JSON: {error}"))?;
    let wrapped_public_key = config
        .pointer("/plugins/updater/pubkey")
        .and_then(Value::as_str)
        .ok_or_else(|| "Tauri config is missing plugins.updater.pubkey".to_string())?;

    let public_key = PublicKey::decode(&decode_wrapped(wrapped_public_key, "updater public key")?)
        .map_err(|error| format!("invalid updater public key: {error}"))?;
    let signature = Signature::decode(&decode_wrapped(&wrapped_signature, "updater signature")?)
        .map_err(|error| format!("invalid updater signature: {error}"))?;
    public_key
        .verify(&archive, &signature, true)
        .map_err(|error| format!("updater signature verification failed: {error}"))?;

    let mut verified_copy = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(verified_copy_path)
        .map_err(|error| {
            format!(
                "failed to create verified archive copy {}: {error}",
                verified_copy_path.display()
            )
        })?;
    verified_copy.write_all(&archive).map_err(|error| {
        format!(
            "failed to write verified archive copy {}: {error}",
            verified_copy_path.display()
        )
    })
}

fn main() -> ExitCode {
    let args: Vec<_> = env::args_os().collect();
    if args.len() != 5 {
        eprintln!(
            "usage: batcave-verify-updater-signature <archive> <signature> <tauri-config> <verified-copy>"
        );
        return ExitCode::from(2);
    }

    match verify(
        Path::new(&args[1]),
        Path::new(&args[2]),
        Path::new(&args[3]),
        Path::new(&args[4]),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
