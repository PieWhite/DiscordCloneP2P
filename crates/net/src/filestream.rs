//! Klein stukje handgeschreven framing voor de bulk-bytestream van een bestandsoverdracht.
//!
//! Los van `framing.rs`: dat gaat over `ControlMsg` op de ene control-stream die bij de
//! handshake wordt geopend. Dit gaat over een eigen uni-stream per bestandsoverdracht,
//! zodat een groot bestand nooit de chat of iets anders op de control-stream kan laten
//! wachten. Zie `docs/ARCHITECTURE.md`.
//!
//! De header is precies één ding — welke overdracht dit is — en dat verandert niet meer.
//! Een vaste layout (`OpId` = 16-byte peer-uuid + kanaal + 8-byte seq) is dan simpeler en
//! goedkoper dan er msgpack bij te halen.
//!
//! Het kanaal zit erin omdat `OpId` sinds de kanalen-uitbreiding niet meer globaal uniek
//! is op `(author, seq)` alleen — zie `crates/proto/src/ids.rs`. Zonder het kanaal in de
//! header zou een download van een algemeen bestand en een DM-bestand met toevallig
//! dezelfde `(author, seq)` niet te onderscheiden zijn.

use anyhow::{Context, Result};
use fitcom_proto::{Channel, OpId, PeerId};
use quinn::{RecvStream, SendStream};

/// 16-byte peer-uuid + 1-byte kanaal-tag + 16-byte kanaal-peer (nullen als afwezig) + 8-byte seq.
const HEADER_LEN: usize = 16 + 1 + 16 + 8;

pub async fn write_header(stream: &mut SendStream, file: OpId) -> Result<()> {
    let mut buf = [0u8; HEADER_LEN];
    buf[..16].copy_from_slice(file.author.as_bytes());
    let (tag, kanaal_peer) = encode_channel(file.channel);
    buf[16] = tag;
    buf[17..33].copy_from_slice(&kanaal_peer);
    buf[33..].copy_from_slice(&file.seq.to_be_bytes());
    stream
        .write_all(&buf)
        .await
        .context("header van bestandsoverdracht schrijven")?;
    Ok(())
}

pub async fn read_header(stream: &mut RecvStream) -> Result<OpId> {
    let mut buf = [0u8; HEADER_LEN];
    stream
        .read_exact(&mut buf)
        .await
        .context("header van bestandsoverdracht lezen")?;
    let mut author = [0u8; 16];
    author.copy_from_slice(&buf[..16]);
    let mut kanaal_peer = [0u8; 16];
    kanaal_peer.copy_from_slice(&buf[17..33]);
    let channel = decode_channel(buf[16], kanaal_peer);
    let seq = u64::from_be_bytes(buf[33..].try_into().expect("8 bytes"));
    Ok(OpId::new(PeerId::from_bytes(author), channel, seq))
}

fn encode_channel(channel: Channel) -> (u8, [u8; 16]) {
    match channel.dm_peer() {
        Some(p) => (1, *p.as_bytes()),
        None => (0, [0u8; 16]),
    }
}

fn decode_channel(tag: u8, peer: [u8; 16]) -> Channel {
    match tag {
        1 => Channel::dm(PeerId::from_bytes(peer)),
        _ => Channel::GENERAL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kanaal_roundtrip() {
        let algemeen = Channel::GENERAL;
        let (tag, peer) = encode_channel(algemeen);
        assert_eq!(decode_channel(tag, peer), algemeen);

        let ander = PeerId::new_random();
        let dm = Channel::dm(ander);
        let (tag, peer) = encode_channel(dm);
        assert_eq!(decode_channel(tag, peer), dm);
    }
}
