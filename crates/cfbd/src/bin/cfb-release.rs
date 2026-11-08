// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary and confidential. See LICENSE.

//! cfb-release - release signing tool for the design organisation.
//!
//! Runs on the release workstation (ideally with the key on a token / HSM;
//! this file-based version is for development). Never ships on the box.
//!
//!   cfb-release keygen  <secret-key-file>          # prints public key hex
//!   cfb-release pubkey  <secret-key-file>
//!   cfb-release sign    <secret-key-file> <file>   # writes <file>.sig
//!   cfb-release verify  <pubkey-hex> <file>        # checks <file>.sig

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use std::fs;
use std::io::Read;
use std::process::exit;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

fn load_secret(path: &str) -> SigningKey {
    let b = fs::read(path).unwrap_or_else(|e| die(&format!("{path}: {e}")));
    let arr: [u8; 32] = b.as_slice().try_into().unwrap_or_else(|_| die("secret key must be 32 bytes"));
    SigningKey::from_bytes(&arr)
}

fn die(msg: &str) -> ! {
    eprintln!("cfb-release: {msg}");
    exit(1)
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match a.get(1).map(String::as_str) {
        Some("keygen") if a.len() == 3 => {
            if fs::metadata(&a[2]).is_ok() {
                die("refusing to overwrite an existing key file");
            }
            let mut seed = [0u8; 32];
            fs::File::open("/dev/urandom")
                .and_then(|mut f| f.read_exact(&mut seed))
                .unwrap_or_else(|e| die(&e.to_string()));
            fs::write(&a[2], seed).unwrap_or_else(|e| die(&e.to_string()));
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&a[2], fs::Permissions::from_mode(0o600));
            }
            println!("{}", hex(SigningKey::from_bytes(&seed).verifying_key().as_bytes()));
        }
        Some("pubkey") if a.len() == 3 => {
            println!("{}", hex(load_secret(&a[2]).verifying_key().as_bytes()));
        }
        Some("sign") if a.len() == 4 => {
            let k = load_secret(&a[2]);
            let data = fs::read(&a[3]).unwrap_or_else(|e| die(&e.to_string()));
            let sig = k.sign(&data);
            fs::write(format!("{}.sig", a[3]), sig.to_bytes()).unwrap_or_else(|e| die(&e.to_string()));
            println!("signed {} ({} bytes)", a[3], data.len());
        }
        Some("verify") if a.len() == 4 => {
            let pk = unhex(&a[2]).and_then(|b| <[u8; 32]>::try_from(b).ok()).unwrap_or_else(|| die("bad pubkey hex"));
            let vk = VerifyingKey::from_bytes(&pk).unwrap_or_else(|_| die("invalid public key"));
            let data = fs::read(&a[3]).unwrap_or_else(|e| die(&e.to_string()));
            let sig = fs::read(format!("{}.sig", a[3])).unwrap_or_else(|e| die(&e.to_string()));
            let sig = Signature::from_slice(&sig).unwrap_or_else(|_| die("bad signature file"));
            match vk.verify(&data, &sig) {
                Ok(()) => println!("OK"),
                Err(_) => die("SIGNATURE INVALID"),
            }
        }
        _ => {
            eprintln!("usage: cfb-release keygen|pubkey <sk> | sign <sk> <file> | verify <pkhex> <file>");
            exit(2);
        }
    }
}
