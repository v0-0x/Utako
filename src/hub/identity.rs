//! Utako's side of the peer protocol mediaserver's sides speak (the Aniko and KumoBox apps'
//! "share" protocol): an X25519 key pair whose fingerprint is the device's ID; requests signed
//! with a secret only this device and the server can make; answers signed back; logins sealed
//! so only the device they're for can read them. Byte for byte as in mediaserver's
//! `anime/internal/share/identity.go`, which `tests` checks against vectors made by that code.

use std::path::Path;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};

type HmacSha256 = Hmac<Sha256>;

/// The headers a signed request carries (their names kept from before Aniko's rename).
pub const HEADER_ID: &str = "X-Kumo-Peer-Id";
pub const HEADER_KEY: &str = "X-Kumo-Peer-Key";
pub const HEADER_NAME: &str = "X-Kumo-Peer-Name";
pub const HEADER_PORT: &str = "X-Kumo-Peer-Port";
pub const HEADER_TOKEN: &str = "X-Kumo-Peer-Token";
pub const HEADER_USER: &str = "X-Kumo-Peer-User";
pub const HEADER_TIME: &str = "X-Kumo-Peer-Time";
pub const HEADER_NONCE: &str = "X-Kumo-Peer-Nonce";
/// In the server's answer: proves it comes from the server.
pub const HEADER_REPLY: &str = "X-Kumo-Peer-Reply";

/// This device's key pair.
pub struct Identity {
    pub id: String,
    secret: StaticSecret,
    public: PublicKey,
}

impl Identity {
    /// The device's identity from `file`, made (and saved, readable by this user only) the
    /// first time.
    pub fn load_or_create(file: &Path) -> Result<Identity> {
        if let Ok(text) = std::fs::read_to_string(file) {
            if let Some(id) = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v.get("private").and_then(|p| p.as_str()).map(str::to_owned))
                .and_then(|p| Identity::from_private_b64(&p).ok())
            {
                return Ok(id);
            }
        }
        let raw: [u8; 32] = rand::random();
        let id = Identity::from_private(raw);
        let saved = serde_json::json!({ "private": B64.encode(raw) });
        crate::config::write_private(file, saved.to_string())
            .with_context(|| format!("couldn't save {}", file.display()))?;
        Ok(id)
    }

    pub fn from_private(raw: [u8; 32]) -> Identity {
        let secret = StaticSecret::from(raw);
        let public = PublicKey::from(&secret);
        Identity {
            id: fingerprint(public.as_bytes()),
            secret,
            public,
        }
    }

    pub fn from_private_b64(b64: &str) -> Result<Identity> {
        let raw: [u8; 32] = B64
            .decode(b64)
            .ok()
            .and_then(|b| b.try_into().ok())
            .ok_or_else(|| anyhow!("bad private key"))?;
        Ok(Identity::from_private(raw))
    }

    /// The public key, as sent to the others.
    pub fn public_key(&self) -> String {
        B64.encode(self.public.as_bytes())
    }

    /// What this device and the one with `peer` have in common, for a purpose.
    fn secret(&self, peer: &[u8; 32], purpose: &str) -> Result<[u8; 32]> {
        let shared = self.secret.diffie_hellman(&PublicKey::from(*peer));
        // Go's crypto/ecdh refuses a low-order key (an all-zero secret): so does this.
        if !shared.was_contributory() {
            bail!("bad public key");
        }
        let mut mac = <HmacSha256 as Mac>::new_from_slice(shared.as_bytes()).expect("any key length");
        mac.update(purpose.as_bytes());
        Ok(mac.finalize().into_bytes().into())
    }

    /// Proves a request (or the answer to it) between a guest and a host: hex HMAC-SHA256 of
    /// each part as `<length>:<part>`.
    pub fn mac(&self, peer: &[u8; 32], guest: &str, host: &str, parts: &[&str]) -> Result<String> {
        let key = self.secret(peer, &format!("kumo-share-v2|guest={guest}|host={host}"))?;
        let mut mac = <HmacSha256 as Mac>::new_from_slice(&key).expect("any key length");
        for part in parts {
            mac.update(format!("{}:{part}", part.len()).as_bytes());
        }
        Ok(hex::encode(mac.finalize().into_bytes()))
    }

    /// Signs a request to the host: the headers to send. `uri` is its path and query as
    /// sent; a `ticket` (a GET of a file mpv may repeat for a while) has no nonce.
    #[allow(clippy::too_many_arguments)]
    pub fn sign(
        &self,
        host: &Host,
        method: &str,
        uri: &str,
        body: &[u8],
        name: &str,
        port: u16,
        user: u64,
        at_unix: i64,
        ticket: bool,
    ) -> Result<Signed> {
        let nonce = if ticket {
            String::new()
        } else {
            hex::encode(rand::random::<[u8; 16]>())
        };
        let name = query_escape(name);
        let port = port.to_string();
        let user = if user > 0 { user.to_string() } else { String::new() };
        let time = at_unix.to_string();
        let body = hex::encode(Sha256::digest(body));
        let token = self.mac(
            &host.key,
            &self.id,
            &host.id,
            &["request", method, uri, &time, &nonce, &name, &port, &user, &body],
        )?;
        let mut headers = vec![
            (HEADER_ID, self.id.clone()),
            (HEADER_KEY, self.public_key()),
            (HEADER_NAME, name),
            (HEADER_PORT, port),
            (HEADER_TIME, time),
            (HEADER_NONCE, nonce.clone()),
            (HEADER_TOKEN, token),
        ];
        if !user.is_empty() {
            headers.push((HEADER_USER, user));
        }
        Ok(Signed { headers, nonce })
    }

    /// Checks that an answer comes from the host: its `X-Kumo-Peer-Reply`, made with the
    /// request's nonce and the answer's `Date`.
    pub fn check_reply(&self, host: &Host, nonce: &str, date: &str, reply: &str) -> Result<()> {
        let want = self.mac(&host.key, &self.id, &host.id, &["reply", nonce, date])?;
        if !constant_time_eq(want.as_bytes(), reply.as_bytes()) {
            bail!("the answer isn't signed by the server");
        }
        Ok(())
    }

    fn cipher(&self, peer: &[u8; 32], guest: &str, host: &str) -> Result<Aes256Gcm> {
        let key = self.secret(peer, &format!("kumo-share-v1|seal|guest={guest}|host={host}"))?;
        Ok(Aes256Gcm::new_from_slice(&key).expect("32 byte key"))
    }

    /// Reads what the host sealed for this device (a login): base64 of nonce + ciphertext.
    pub fn open_from(&self, host: &Host, sealed: &str) -> Result<Vec<u8>> {
        let raw = B64.decode(sealed).map_err(|_| anyhow!("bad sealed data"))?;
        if raw.len() < 12 {
            bail!("bad sealed data");
        }
        let (nonce, data) = raw.split_at(12);
        self.cipher(&host.key, &self.id, &host.id)?
            .decrypt(Nonce::from_slice(nonce), data)
            .map_err(|_| anyhow!("sealed data that isn't for this device"))
    }

    /// Seals `plain` (this device's logins) so only the server can read it, as an Aniko gives
    /// its AniList login to the hub (`share/hub.go`): the server is the one it's sealed *for*,
    /// so it stands in the "guest" place and this device in the "host" one.
    pub fn seal_for_host(&self, host: &Host, plain: &[u8]) -> Result<String> {
        let nonce: [u8; 12] = rand::random();
        let mut out = nonce.to_vec();
        out.extend(
            self.cipher(&host.key, &host.id, &self.id)?
                .encrypt(Nonce::from_slice(&nonce), plain)
                .map_err(|_| anyhow!("couldn't seal"))?,
        );
        Ok(B64.encode(out))
    }
}

/// A server (or any peer) as known: its ID and public key, checked against each other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Host {
    pub id: String,
    pub key: [u8; 32],
}

impl Host {
    pub fn new(id: &str, key_b64: &str) -> Result<Host> {
        let key: [u8; 32] = B64
            .decode(key_b64)
            .ok()
            .and_then(|b| b.try_into().ok())
            .ok_or_else(|| anyhow!("bad public key"))?;
        if fingerprint(&key) != id {
            bail!("the public key doesn't match the ID");
        }
        Ok(Host {
            id: id.to_string(),
            key,
        })
    }
}

/// A request's signature: the headers, and the nonce its answer is signed with.
#[derive(Debug, Clone)]
pub struct Signed {
    pub headers: Vec<(&'static str, String)>,
    pub nonce: String,
}

/// The ID of the device with this public key: 16 lowercase letters and digits.
pub fn fingerprint(public: &[u8]) -> String {
    let sum = Sha256::digest(public);
    data_encoding::BASE32_NOPAD.encode(&sum[..10]).to_lowercase()
}

/// Go's url.QueryEscape: letters, digits and `-_.~` stay, a space becomes `+`, the rest %XX.
pub fn query_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    /// Made by mediaserver's own code (a test printing `sign`, `verify`, `reply` and `sealFor`
    /// with fixed keys): Utako has to give the same answers.
    const VECTORS: &str = include_str!("testdata/mediaserver-vectors.json");

    fn header<'a>(v: &'a Value, set: &str, name: &str) -> &'a str {
        v[set][name][0].as_str().unwrap()
    }

    #[test]
    fn same_keys_and_ids_as_mediaserver() {
        let v: Value = serde_json::from_str(VECTORS).unwrap();
        let guest = Identity::from_private_b64(v["guest_private"].as_str().unwrap()).unwrap();
        let host = Identity::from_private_b64(v["host_private"].as_str().unwrap()).unwrap();
        assert_eq!(guest.id, v["guest_id"]);
        assert_eq!(guest.public_key(), v["guest_key"]);
        assert_eq!(host.id, v["host_id"]);
        assert_eq!(host.public_key(), v["host_key"]);
        assert!(Host::new(&host.id, &host.public_key()).is_ok());
        assert!(
            Host::new(&guest.id, &host.public_key()).is_err(),
            "a key that isn't the ID's"
        );
    }

    #[test]
    fn same_signatures_as_mediaserver() {
        let v: Value = serde_json::from_str(VECTORS).unwrap();
        let guest = Identity::from_private_b64(v["guest_private"].as_str().unwrap()).unwrap();
        let host = Host::new(v["host_id"].as_str().unwrap(), v["host_key"].as_str().unwrap()).unwrap();
        // The signed request: POST /api/peer/hub/prefs?x=1&y=two%20words, its body, its nonce.
        let nonce = header(&v, "headers", "X-Kumo-Peer-Nonce");
        let body = hex::encode(Sha256::digest(v["body"].as_str().unwrap().as_bytes()));
        assert_eq!(
            query_escape("Utako on iPhone, 2"),
            header(&v, "headers", "X-Kumo-Peer-Name")
        );
        let token = guest
            .mac(
                &host.key,
                &guest.id,
                &host.id,
                &[
                    "request",
                    "POST",
                    "/api/peer/hub/prefs?x=1&y=two%20words",
                    "1793500000",
                    nonce,
                    "Utako+on+iPhone%2C+2",
                    "43411",
                    "7",
                    &body,
                ],
            )
            .unwrap();
        assert_eq!(token, header(&v, "headers", "X-Kumo-Peer-Token"));
        // A ticket (no nonce, no user).
        let ticket = guest
            .sign(
                &host,
                "GET",
                "/api/peer/local/file?path=%2Fmusic%2Fa%20b.flac",
                b"",
                "Utako",
                0,
                0,
                1_793_500_000,
                true,
            )
            .unwrap();
        let get = |name: &str| ticket.headers.iter().find(|(h, _)| *h == name).map(|(_, v)| v.as_str());
        assert_eq!(
            get(HEADER_TOKEN),
            Some(header(&v, "ticket_headers", "X-Kumo-Peer-Token"))
        );
        assert_eq!(get(HEADER_NONCE), Some(""));
        assert_eq!(get(HEADER_USER), None);
        // The server's signed answer, and a login it sealed for this device.
        guest
            .check_reply(
                &host,
                nonce,
                "Sun, 01 Nov 2026 12:00:00 GMT",
                v["reply"].as_str().unwrap(),
            )
            .unwrap();
        assert!(guest
            .check_reply(&host, nonce, "another date", v["reply"].as_str().unwrap())
            .is_err());
        assert_eq!(
            guest.open_from(&host, v["sealed"].as_str().unwrap()).unwrap(),
            b"secret login"
        );
    }

    #[test]
    fn sealed_for_the_host_opens_there() {
        let v: Value = serde_json::from_str(VECTORS).unwrap();
        let guest = Identity::from_private_b64(v["guest_private"].as_str().unwrap()).unwrap();
        let host_id = Identity::from_private_b64(v["host_private"].as_str().unwrap()).unwrap();
        let host = Host::new(&host_id.id, &host_id.public_key()).unwrap();
        let sealed = guest.seal_for_host(&host, b"my logins").unwrap();
        // The server opens it as the hub does a device's login: openFrom(device key,
        // guest = itself, host = the device).
        let device = Host::new(&guest.id, &guest.public_key()).unwrap();
        let cipher = host_id.cipher(&device.key, &host.id, &guest.id).unwrap();
        let raw = B64.decode(sealed).unwrap();
        let (nonce, data) = raw.split_at(12);
        assert_eq!(cipher.decrypt(Nonce::from_slice(nonce), data).unwrap(), b"my logins");
    }

    #[test]
    fn identity_is_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("id.json");
        let a = Identity::load_or_create(&file).unwrap();
        let b = Identity::load_or_create(&file).unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(a.id.len(), 16);
    }
}
