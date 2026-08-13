//! Lengte-geprefixte control-frames over een QUIC-stream.
//!
//! `[u32 lengte big-endian][u16 tag][msgpack payload]`
//!
//! De lengteprefix zit hier en niet in `fitcom-proto`, omdat die crate bewust geen
//! kennis van transport heeft.

use anyhow::{bail, Context, Result};
use fitcom_proto::ControlMsg;
use quinn::{RecvStream, SendStream};
use tokio::io::{AsyncRead, AsyncReadExt};

/// Bovengrens voor een control-frame in déze crate (B-17).
///
/// `fitcom_proto::MAX_FRAME_LEN` staat op 16 MiB en die grens mogen we hier niet
/// verlagen — dat is een andere crate. Wat we wél kunnen is er aan de netwerkkant een
/// strengere bovenop leggen, en dat is nodig: 16 MiB is wat een nog niet
/// geauthenticeerde peer met vier bytes kan laten reserveren.
///
/// Waarom 2 MiB en niet de 256 KiB uit `docs/BEVEILIGING.md`: de sync-kant knipt sinds
/// B-15 op een **bytebudget** van 1 MiB (`fitcom_store::SYNC_BATCH_BYTES`) en levert
/// daarnaast altijd minstens één op af, ook als die zelf al over dat budget gaat
/// (`MAX_OP_LEN`, 256 KiB). Een legitiem `SyncResponse` haalt daarmee ruim boven de
/// 256 KiB, dus die waarde zou gewone sync breken. 2 MiB = budget + één te grote op +
/// msgpack-overhead, en is nog altijd een factor acht onder wat een vreemde kon
/// aanvragen.
pub const MAX_FRAME_LEN: usize = 2 * 1024 * 1024;

/// Hoeveel we per keer bijreserveren en inlezen (B-17). Klein genoeg dat een gelogen
/// lengte niets kost, groot genoeg dat een echt frame van 2 MiB in 128 stappen binnen is.
const LEES_BROK: usize = 16 * 1024;

/// De bytes van één frame, inclusief lengteprefix.
///
/// Staat los van [`write_frame`] omdat de schrijflus in `mesh.rs` het verschil moet
/// kunnen zien tussen "dit bericht past niet" (overslaan, B-15) en "de stream is stuk"
/// (stoppen). Zolang beide in één functie zaten, sloopte één te groot bericht de hele
/// control-verbinding permanent.
pub fn frame_bytes(msg: &ControlMsg) -> Result<Vec<u8>> {
    let body = msg.encode().context("control-bericht coderen")?;
    if body.len() > MAX_FRAME_LEN {
        bail!("frame van {} bytes is te groot om te versturen", body.len());
    }
    let mut buf = Vec::with_capacity(4 + body.len());
    buf.extend_from_slice(&(body.len() as u32).to_be_bytes());
    buf.extend_from_slice(&body);
    Ok(buf)
}

pub async fn write_frame(stream: &mut SendStream, msg: &ControlMsg) -> Result<()> {
    let buf = frame_bytes(msg)?;
    stream.write_all(&buf).await.context("frame schrijven")?;
    Ok(())
}

/// `Ok(None)` bij een net gesloten stream. `Ok(Some(None))` bij een bericht met een
/// onbekende tag — dat is een nieuwere peer, niet een fout: overslaan en doorlezen.
pub async fn read_frame(stream: &mut RecvStream) -> Result<Option<Option<ControlMsg>>> {
    lees_frame(stream).await
}

/// Generiek over de bron zodat de grenzen uit B-17 met een plakje bytes te testen zijn;
/// een `RecvStream` maak je niet zonder een echte QUIC-verbinding.
async fn lees_frame<R: AsyncRead + Unpin>(bron: &mut R) -> Result<Option<Option<ControlMsg>>> {
    let mut len_buf = [0u8; 4];
    if !lees_precies(bron, &mut len_buf).await? {
        return Ok(None);
    }

    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME_LEN {
        bail!("peer kondigde een frame van {len} bytes aan; limiet is {MAX_FRAME_LEN}");
    }

    // B-17: groeien op wat er écht binnenkomt, niet op wat er aangekondigd is. Hiervoor
    // stond hier `vec![0u8; len]` — een genulde en dus echt vastgelegde allocatie, in
    // precies de functie die ook de eerste, nog niet geauthenticeerde `Hello` leest. Met
    // vier bytes kon een vreemde daarmee geheugen claimen zonder ooit een body te sturen,
    // en OOM is in Rust een `abort` en geen fout die je kunt opvangen.
    let mut body: Vec<u8> = Vec::new();
    while body.len() < len {
        let brok = LEES_BROK.min(len - body.len());
        body.try_reserve(brok)
            .map_err(|_| anyhow::anyhow!("geen geheugen voor {brok} bytes frame-inhoud"))?;
        let vanaf = body.len();
        body.resize(vanaf + brok, 0);
        if !lees_precies(bron, &mut body[vanaf..]).await? {
            bail!("stream eindigde midden in een frame van {len} bytes");
        }
    }

    match ControlMsg::decode(&body) {
        Ok(msg) => Ok(Some(msg)),
        // Een kapotte payload op één bericht mag de verbinding niet slopen; loggen en door.
        Err(e) => {
            tracing::warn!(error = %e, "onleesbaar control-frame overgeslagen");
            Ok(Some(None))
        }
    }
}

/// Vult `buf` helemaal. `Ok(false)` als de stream sloot vóórdat er ook maar één byte
/// van dit stuk binnen was — op een framegrens is dat het nette einde, geen fout.
async fn lees_precies<R: AsyncRead + Unpin>(bron: &mut R, buf: &mut [u8]) -> Result<bool> {
    let mut gelezen = 0;
    while gelezen < buf.len() {
        match bron
            .read(&mut buf[gelezen..])
            .await
            .context("frame lezen")?
        {
            0 if gelezen == 0 => return Ok(false),
            0 => bail!("stream eindigde na {gelezen} van {} bytes", buf.len()),
            n => gelezen += n,
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fitcom_proto::control::Ping;

    fn aankondiging(len: u32) -> Vec<u8> {
        len.to_be_bytes().to_vec()
    }

    #[tokio::test]
    async fn frame_gaat_er_heen_en_weer_doorheen() {
        let msg = ControlMsg::Ping(Ping { nonce: 42 });
        let bytes = frame_bytes(&msg).unwrap();
        let terug = lees_frame(&mut bytes.as_slice()).await.unwrap();
        assert_eq!(terug, Some(Some(msg)));
    }

    #[tokio::test]
    async fn lege_stream_is_geen_fout() {
        // Een peer die netjes op een framegrens ophangt is het normale einde.
        assert_eq!(lees_frame(&mut [].as_slice()).await.unwrap(), None);
    }

    #[tokio::test]
    async fn b17_aangekondigde_lengte_boven_de_limiet_wordt_geweigerd() {
        let bron = aankondiging(MAX_FRAME_LEN as u32 + 1);
        let fout = lees_frame(&mut bron.as_slice()).await.unwrap_err();
        assert!(
            fout.to_string().contains("kondigde"),
            "onverwachte fout: {fout:#}"
        );
    }

    /// De grens van deze crate ligt écht onder die van proto, anders haalt B-17 niets uit —
    /// en écht boven het bytebudget van een sync-antwoord plus één te grote op, want anders
    /// sneuvelt gewone chat-sync er juist op. Beide kant en klaar bij het compileren.
    const _: () = assert!(MAX_FRAME_LEN < fitcom_proto::MAX_FRAME_LEN);
    const _: () = assert!(MAX_FRAME_LEN >= 1024 * 1024 + 256 * 1024);

    #[tokio::test]
    async fn b17_gelogen_lengte_kost_geen_geheugen_vooraf() {
        // Twee megabyte aangekondigd, drie bytes geleverd, dan einde stream. Vroeger
        // stond er op dat moment al een genulde buffer van 2 MiB; nu groeit hij mee met
        // wat er binnenkomt en stopt het bij de eerste brok.
        let mut bron = aankondiging(MAX_FRAME_LEN as u32);
        bron.extend_from_slice(&[1, 2, 3]);
        let fout = lees_frame(&mut bron.as_slice()).await.unwrap_err();
        assert!(
            fout.to_string().contains("stream eindigde"),
            "onverwachte fout: {fout:#}"
        );
    }

    #[tokio::test]
    async fn b15_te_groot_bericht_geeft_een_fout_in_plaats_van_een_kapotte_stream() {
        // `frame_bytes` bestaat zodat de schrijflus dit geval kan overslaan zonder de
        // verbinding op te geven; hier alleen dat hij het inderdaad weigert.
        let ruim_te_groot = "x".repeat(MAX_FRAME_LEN + 1);
        let msg = ControlMsg::Hello(fitcom_proto::control::Hello {
            protocol_version: fitcom_proto::PROTOCOL_VERSION,
            peer_id: fitcom_proto::PeerId::new_random(),
            display_name: ruim_te_groot,
            media_port: 41700,
            app_version: "1.0.0".into(),
        });
        assert!(frame_bytes(&msg).is_err());
    }
}
