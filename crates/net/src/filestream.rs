//! Klein stukje handgeschreven framing voor de bulk-bytestream van een bestandsoverdracht.
//!
//! Los van `framing.rs`: dat gaat over `ControlMsg` op de ene control-stream die bij de
//! handshake wordt geopend. Dit gaat over een eigen uni-stream per overdracht, zodat een
//! groot bestand nooit de chat of iets anders op de control-stream kan laten wachten. Zie
//! `docs/ARCHITECTURE.md`.
//!
//! Sinds fase 11 (automatische updates) begint elke uni-stream met een 1-byte kind: een
//! bestandsoverdracht (`OpId` = 16-byte peer-uuid + kanaal + 8-byte seq, zoals hiervoor)
//! of een update-overdracht (geen verdere body — de identiteit is impliciet: "de huidige
//! exe van de peer die zojuist `UpdateResponse::READY` stuurde"). Dat kind-byte is
//! precies de reden voor de `PROTOCOL_VERSION`-bump 3 → 4: een oudere peer zou het anders
//! als eerste byte van de oude header lezen. Zie `crates/proto/src/lib.rs`.
//!
//! Het kanaal zit in de bestandsheader omdat `OpId` sinds de kanalen-uitbreiding niet meer
//! globaal uniek is op `(author, seq)` alleen — zie `crates/proto/src/ids.rs`. Zonder het
//! kanaal in de header zou een download van een algemeen bestand en een DM-bestand met
//! toevallig dezelfde `(author, seq)` niet te onderscheiden zijn.

use anyhow::{Context, Result};
use fitcom_proto::{Channel, OpId, PeerId, TopicId};
use quinn::{RecvStream, SendStream};

const KIND_FILE: u8 = 0;
const KIND_UPDATE: u8 = 1;

/// 16-byte peer-uuid + 1-byte kanaal-tag + 16-byte kanaal-peer-of-subkanaal-id (nullen als
/// afwezig) + 8-byte seq.
const FILE_BODY_LEN: usize = 16 + 1 + 16 + 8;

/// Wat voor overdracht een inkomende uni-stream is, na het lezen van het kind-byte.
pub enum Inkomend {
    Bestand(OpId),
    Update,
}

pub async fn write_header(stream: &mut SendStream, file: OpId) -> Result<()> {
    let mut buf = [0u8; 1 + FILE_BODY_LEN];
    buf[0] = KIND_FILE;
    buf[1..17].copy_from_slice(file.author.as_bytes());
    let (tag, kanaal_peer) = encode_channel(file.channel);
    buf[17] = tag;
    buf[18..34].copy_from_slice(&kanaal_peer);
    buf[34..].copy_from_slice(&file.seq.to_be_bytes());
    stream
        .write_all(&buf)
        .await
        .context("header van bestandsoverdracht schrijven")?;
    Ok(())
}

/// Geen body nodig: de aanvrager kent de update al uit de voorafgaande `UpdateResponse`.
pub async fn write_update_header(stream: &mut SendStream) -> Result<()> {
    stream
        .write_all(&[KIND_UPDATE])
        .await
        .context("kind-byte van update-overdracht schrijven")?;
    Ok(())
}

/// Leest het kind-byte en, voor een bestand, meteen de rest van de header erachteraan.
pub async fn read_kind(stream: &mut RecvStream) -> Result<Inkomend> {
    let mut kind = [0u8; 1];
    stream
        .read_exact(&mut kind)
        .await
        .context("kind-byte van inkomende overdracht lezen")?;
    if kind[0] == KIND_UPDATE {
        return Ok(Inkomend::Update);
    }

    let mut buf = [0u8; FILE_BODY_LEN];
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
    Ok(Inkomend::Bestand(OpId::new(
        PeerId::from_bytes(author),
        channel,
        seq,
    )))
}

fn encode_channel(channel: Channel) -> (u8, [u8; 16]) {
    if let Some(p) = channel.dm_peer() {
        return (1, *p.as_bytes());
    }
    if let Some(t) = channel.topic_id() {
        return (2, *t.as_bytes());
    }
    (0, [0u8; 16])
}

fn decode_channel(tag: u8, id: [u8; 16]) -> Channel {
    match tag {
        1 => Channel::dm(PeerId::from_bytes(id)),
        2 => Channel::topic(TopicId::from_bytes(id)),
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

        let subkanaal = Channel::topic(TopicId::new_random());
        let (tag, id) = encode_channel(subkanaal);
        assert_eq!(decode_channel(tag, id), subkanaal);
    }
}
