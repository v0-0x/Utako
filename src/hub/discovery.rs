//! Finding a media server on the home network: like the Aniko and KumoBox apps, by the small
//! message (a beacon) it sends every few seconds to a multicast group; the music side on its
//! own port.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

use serde::Deserialize;
use tokio::sync::mpsc::UnboundedSender;

/// The music side's beacons (anime 43212, movies & TV 43312).
pub const DISCOVERY_PORT: u16 = 43412;
/// The group every side uses.
pub const GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 75, 77);

/// A beacon, as mediaserver's sides send them.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Beacon {
    /// Protocol version (under its key from before Aniko's rename).
    #[serde(rename = "kumo", default)]
    pub protocol: u32,
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub port: u16,
    pub key: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub reply: bool,
}

/// A server heard on the network.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub beacon: Beacon,
    pub address: String,
}

/// Listens for beacons in a thread of its own; each one heard goes to `found`. Stops when
/// `found` is closed.
pub fn listen(found: UnboundedSender<Found>) -> std::io::Result<()> {
    let socket = bind()?;
    std::thread::Builder::new()
        .name("server-discovery".into())
        .spawn(move || {
            let mut buf = [0u8; 2048];
            loop {
                if found.is_closed() {
                    return;
                }
                let Ok((n, from)) = socket.recv_from(&mut buf) else {
                    continue;
                };
                let Some(beacon) = parse(&buf[..n]) else { continue };
                let SocketAddr::V4(from) = from else { continue };
                let address = format!("{}:{}", from.ip(), beacon.port);
                if found.send(Found { beacon, address }).is_err() {
                    return;
                }
            }
        })?;
    Ok(())
}

pub fn parse(bytes: &[u8]) -> Option<Beacon> {
    let b: Beacon = serde_json::from_slice(bytes).ok()?;
    (b.port > 0 && b.id.len() == 16 && !b.key.is_empty()).then_some(b)
}

fn bind() -> std::io::Result<UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    // Other apps on this computer listen there too (an Aniko, another Utako).
    socket.set_reuse_address(true)?;
    #[cfg(all(unix, not(target_os = "solaris")))]
    socket.set_reuse_port(true)?;
    socket.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT)).into())?;
    socket.join_multicast_v4(&GROUP, &Ipv4Addr::UNSPECIFIED)?;
    let socket: UdpSocket = socket.into();
    // Wakes now and then, to notice when nobody listens any more.
    socket.set_read_timeout(Some(Duration::from_secs(5)))?;
    Ok(socket)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beacons() {
        let b = parse(
            br#"{"kumo":1,"id":"ov2btdx64snn4ynu","name":"Pi","port":43411,"key":"fZwk","version":"1.0","role":"hub"}"#,
        )
        .unwrap();
        assert_eq!((b.protocol, b.port, b.role.as_str()), (1, 43411, "hub"));
        assert!(parse(br#"{"kumo":1,"id":"short","port":1,"key":"k"}"#).is_none());
        assert!(parse(b"not json").is_none());
    }
}
