//! Klein stukje handgeschreven framing voor de bulk-bytestream van een bestandsoverdracht.
//!
//! Los van `framing.rs`: dat gaat over `ControlMsg` op de ene control-stream die bij de
//! handshake wordt geopend. Dit gaat over een eigen uni-stream per bestandsoverdracht,
//! zodat een groot bestand nooit de chat of iets anders op de control-stream kan laten
//! wachten. Zie `docs/ARCHITECTURE.md`.
//!
//! De header is precies één ding — welke overdracht dit is — en dat verandert niet meer.
//! Een vaste 24-byte layout (`OpId` = 16-byte peer-uuid + 8-byte seq) is dan simpeler en
//! goedkoper dan er msgpack bij te halen.

use anyhow::{Context, Result};
use fitcom_proto::{OpId, PeerId};
use quinn::{RecvStream, SendStream};

const HEADER_LEN: usize = 24;

pub async fn write_header(stream: &mut SendStream, file: OpId) -> Result<()> {
    let mut buf = [0u8; HEADER_LEN];
    buf[..16].copy_from_slice(file.author.as_bytes());
    buf[16..].copy_from_slice(&file.seq.to_be_bytes());
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
    let seq = u64::from_be_bytes(buf[16..].try_into().expect("8 bytes"));
    Ok(OpId::new(PeerId::from_bytes(author), seq))
}
