//! Lengte-geprefixte control-frames over een QUIC-stream.
//!
//! `[u32 lengte big-endian][u16 tag][msgpack payload]`
//!
//! De lengteprefix zit hier en niet in `fitcom-proto`, omdat die crate bewust geen
//! kennis van transport heeft.

use anyhow::{bail, Context, Result};
use fitcom_proto::{ControlMsg, MAX_FRAME_LEN};
use quinn::{RecvStream, SendStream};

pub async fn write_frame(stream: &mut SendStream, msg: &ControlMsg) -> Result<()> {
    let body = msg.encode().context("control-bericht coderen")?;
    if body.len() > MAX_FRAME_LEN {
        bail!("frame van {} bytes is te groot om te versturen", body.len());
    }
    let mut buf = Vec::with_capacity(4 + body.len());
    buf.extend_from_slice(&(body.len() as u32).to_be_bytes());
    buf.extend_from_slice(&body);
    stream.write_all(&buf).await.context("frame schrijven")?;
    Ok(())
}

/// `Ok(None)` bij een net gesloten stream. `Ok(Some(None))` bij een bericht met een
/// onbekende tag — dat is een nieuwere peer, niet een fout: overslaan en doorlezen.
pub async fn read_frame(stream: &mut RecvStream) -> Result<Option<Option<ControlMsg>>> {
    let mut len_buf = [0u8; 4];
    match stream.read_exact(&mut len_buf).await {
        Ok(()) => {}
        Err(quinn::ReadExactError::FinishedEarly(0)) => return Ok(None),
        Err(e) => return Err(e).context("framelengte lezen"),
    }

    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME_LEN {
        bail!("peer kondigde een frame van {len} bytes aan; limiet is {MAX_FRAME_LEN}");
    }

    let mut body = vec![0u8; len];
    stream
        .read_exact(&mut body)
        .await
        .context("frame-inhoud lezen")?;

    match ControlMsg::decode(&body) {
        Ok(msg) => Ok(Some(msg)),
        // Een kapotte payload op één bericht mag de verbinding niet slopen; loggen en door.
        Err(e) => {
            tracing::warn!(error = %e, "onleesbaar control-frame overgeslagen");
            Ok(Some(None))
        }
    }
}
