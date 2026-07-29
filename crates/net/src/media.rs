//! UDP-transport voor audio en video.
//!
//! # Waarom hier geen tokio aan te pas komt
//!
//! De rest van de netwerklaag is async, deze niet. Media loopt op eigen threads die
//! door de geluidskaart worden aangedreven: die vraagt elke 20 ms om een frame en
//! wacht nergens op. Een async runtime ertussen voegt planning en dus jitter toe aan
//! precies het pad waar dat het meest opvalt.
//!
//! Blokkerende sockets met een korte timeout zijn hier eenvoudiger én voorspelbaarder.

use anyhow::{Context, Result};
use fitcom_proto::{MediaHeader, MEDIA_HEADER_LEN};
use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

/// Ruim boven het grootste pakket dat we versturen; een Opus-frame is enkele honderden
/// bytes en videofragmenten zitten onder de MTU.
pub const MAX_PAKKET: usize = 1500;

pub struct MediaSocket {
    sock: UdpSocket,
}

impl MediaSocket {
    /// Bindt op alle interfaces. Poort 0 laat het besturingssysteem kiezen, wat handig
    /// is voor tests.
    pub fn bind(port: u16) -> Result<Self> {
        let sock = UdpSocket::bind(("0.0.0.0", port))
            .with_context(|| format!("UDP-poort {port} binden"))?;
        // Zonder timeout kan een wachtende thread niet meer worden afgesloten.
        sock.set_read_timeout(Some(Duration::from_millis(200)))?;
        Ok(Self { sock })
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.sock.local_addr()?)
    }

    /// Hoe lang [`MediaSocket::ontvang`] blijft wachten voordat hij `None` teruggeeft.
    ///
    /// Video wil hier korter zitten dan audio: die thread bedient ook een venster, en
    /// een venster dat pas na 200 ms op een muisklik reageert voelt kapot.
    pub fn zet_timeout(&self, timeout: Duration) -> Result<()> {
        self.sock
            .set_read_timeout(Some(timeout))
            .context("leestimeout instellen")?;
        Ok(())
    }

    /// Kloont de socket zodat verzenden en ontvangen op eigen threads kunnen draaien.
    pub fn probeer_clone(&self) -> Result<Self> {
        Ok(Self {
            sock: self.sock.try_clone().context("mediasocket klonen")?,
        })
    }

    pub fn stuur(&self, naar: SocketAddr, header: &MediaHeader, payload: &[u8]) -> Result<()> {
        let mut buf = [0u8; MAX_PAKKET];
        let einde = MEDIA_HEADER_LEN + payload.len();
        if einde > MAX_PAKKET {
            anyhow::bail!("mediapakket van {einde} bytes is te groot");
        }
        header.write_to(
            (&mut buf[..MEDIA_HEADER_LEN])
                .try_into()
                .expect("vaste lengte"),
        );
        buf[MEDIA_HEADER_LEN..einde].copy_from_slice(payload);
        self.sock.send_to(&buf[..einde], naar)?;
        Ok(())
    }

    /// `Ok(None)` bij een timeout — dat is de normale gang van zaken als niemand praat,
    /// geen fout.
    ///
    /// Pakketten die te kort zijn voor een header worden stil genegeerd: op een open
    /// UDP-poort komt vroeg of laat rommel binnen en daar mag niets van omvallen.
    pub fn ontvang<'a>(
        &self,
        buf: &'a mut [u8; MAX_PAKKET],
    ) -> Result<Option<(SocketAddr, MediaHeader, &'a [u8])>> {
        match self.sock.recv_from(buf) {
            Ok((n, van)) => match MediaHeader::parse(&buf[..n]) {
                Some((header, payload)) => Ok(Some((van, header, payload))),
                None => Ok(None),
            },
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                Ok(None)
            }
            // Windows meldt een niet-bereikbare ontvanger op een verbindingsloze socket
            // met deze fout. Een peer die net weg is mag onze ontvangstlus niet stoppen.
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => Ok(None),
            Err(e) => Err(e).context("mediapakket ontvangen"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fitcom_proto::PayloadType;

    #[test]
    fn pakket_komt_heelhuids_aan() {
        let a = MediaSocket::bind(0).unwrap();
        let b = MediaSocket::bind(0).unwrap();
        let doel = SocketAddr::from(([127, 0, 0, 1], b.local_addr().unwrap().port()));

        let header = MediaHeader {
            stream_id: 0,
            seq: 42,
            timestamp: 960,
            payload_type: PayloadType::OPUS,
            flags: 0,
            frag_index: 0,
        };
        a.stuur(doel, &header, b"spraak").unwrap();

        let mut buf = [0u8; MAX_PAKKET];
        let (_, terug, payload) = b.ontvang(&mut buf).unwrap().expect("pakket verwacht");
        assert_eq!(terug, header);
        assert_eq!(payload, b"spraak");
    }

    #[test]
    fn timeout_is_geen_fout() {
        // Niemand praat. Dat is de normale toestand, niet iets om op te struikelen.
        let s = MediaSocket::bind(0).unwrap();
        let mut buf = [0u8; MAX_PAKKET];
        assert!(s.ontvang(&mut buf).unwrap().is_none());
    }

    #[test]
    fn rommel_op_de_poort_wordt_genegeerd() {
        let a = MediaSocket::bind(0).unwrap();
        let b = MediaSocket::bind(0).unwrap();
        let doel = SocketAddr::from(([127, 0, 0, 1], b.local_addr().unwrap().port()));
        a.sock.send_to(b"te kort", doel).unwrap();

        let mut buf = [0u8; MAX_PAKKET];
        assert!(b.ontvang(&mut buf).unwrap().is_none(), "mag niet omvallen");
    }

    #[test]
    fn te_groot_pakket_wordt_geweigerd_in_plaats_van_afgekapt() {
        let a = MediaSocket::bind(0).unwrap();
        let doel = SocketAddr::from(([127, 0, 0, 1], 9));
        let header = MediaHeader {
            stream_id: 0,
            seq: 0,
            timestamp: 0,
            payload_type: PayloadType::OPUS,
            flags: 0,
            frag_index: 0,
        };
        assert!(a.stuur(doel, &header, &[0u8; MAX_PAKKET]).is_err());
    }
}
