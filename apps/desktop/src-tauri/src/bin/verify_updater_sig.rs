//! CI helper: verify a Tauri updater archive signature matches a minisign pubkey.
//!
//! Mirrors `tauri-plugin-updater::verify_signature` exactly:
//!   1. Base64-decode the pubkey string → minisign public-key file body
//!   2. Base64-decode the `.sig` file → minisign signature body
//!   3. `PublicKey::verify(data, signature, allow_legacy=true)`
//!
//! Usage: verify-updater-sig <archive-path> <pubkey-base64>

use std::env;
use std::fs;
use std::process::ExitCode;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use minisign_verify::{PublicKey, Signature};

fn base64_to_utf8(label: &str, b64: &str) -> Result<String, String> {
    let raw = STANDARD
        .decode(b64.trim().as_bytes())
        .map_err(|e| format!("base64-decode {label}: {e}"))?;
    String::from_utf8(raw).map_err(|_| format!("{label} is not valid UTF-8 after base64 decode"))
}

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let Some(archive) = args.next() else {
        eprintln!("usage: verify-updater-sig <archive-path> <pubkey-base64>");
        return ExitCode::from(2);
    };
    let Some(pubkey_b64) = args.next() else {
        eprintln!("usage: verify-updater-sig <archive-path> <pubkey-base64>");
        return ExitCode::from(2);
    };

    let sig_path = format!("{archive}.sig");
    let data = match fs::read(&archive) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("read {archive}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let sig_b64 = match fs::read_to_string(&sig_path) {
        Ok(s) => s.trim().to_owned(),
        Err(e) => {
            eprintln!("read {sig_path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let pub_key_body = match base64_to_utf8("pubkey", &pubkey_b64) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let signature_body = match base64_to_utf8("signature", &sig_b64) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    let public_key = match PublicKey::decode(&pub_key_body) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("decode pubkey: {e}");
            return ExitCode::FAILURE;
        }
    };
    let signature = match Signature::decode(&signature_body) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("decode signature: {e}");
            return ExitCode::FAILURE;
        }
    };

    match public_key.verify(&data, &signature, true) {
        Ok(()) => {
            println!("OK: signature matches pubkey for {archive}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("VERIFY FAILED for {archive}: {e}");
            ExitCode::FAILURE
        }
    }
}
