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

/// Windows geeft een UDP-socket standaard 64 kB ontvangbuffer. Een keyframe van 1080p is
/// 100 tot 260 kB en gaat in ruim tweehonderd fragmenten achter elkaar de deur uit; de
/// ontvanger zit op dat moment in `decode` en `toon` en leest dus even niet. Met 64 kB
/// overleeft daar ongeveer een kwart van — gemeten in `tests/burst.rs`, op loopback, waar
/// onderweg niets kwijt kan raken. Eén gemist fragment maakt het hele keyframe
/// onbruikbaar, de kijker vraagt een nieuw keyframe, en de volgende stoot sneuvelt net zo.
///
/// 1 MB is ruim vier keyframes. Groter is geen gratis winst: wat hier in de rij staat is
/// beeld dat al te laat is, en de samensteller gooit oude frames toch weg.
const ONTVANGBUFFER: usize = 1024 * 1024;

/// Zet de timerresolutie van dit proces op één milliseconde.
///
/// Zonder dit tikt Windows op 15,6 ms, en dat is precies de resolutie die je terugkrijgt
/// bij `set_read_timeout` en `thread::sleep` — ongeacht wat je vraagt. Gemeten op
/// 2026-08-02: een gevraagde leestimeout van 1, 2 én 8 ms duurde alle drie 15,6 ms.
///
/// Dat sloopt de weergaveklok van de kijker. Die plant elk beeld op het moment waarop het
/// is opgenomen, maar de lus die dat moet uitvoeren kwam alleen wakker als er een pakket
/// binnenviel of als de grove tik afliep — en dus werd elk beeld getoond op het moment dat
/// het *volgende* beeld binnenkwam. Gemeten gevolg: gemiddeld 5,8 ms te laat, gelijkmatig
/// verdeeld over een hele beeldtijd. De hele klok deed daardoor niets.
///
/// Sinds Windows 10 2004 werkt dit per proces en niet meer systeembreed, dus een game
/// naast ons merkt er niets van. Er staat geen `timeEndPeriod` tegenover: Windows ruimt
/// dat bij het afsluiten van het proces zelf op, en de enige plek waar dit vandaan komt is
/// het openen van een mediasocket — dus zodra er beeld of geluid loopt.
#[cfg(windows)]
fn zorg_voor_fijne_timer() {
    static EEN_KEER: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    EEN_KEER.get_or_init(|| {
        // SAFETY: `timeBeginPeriod` neemt alleen een getal aan en heeft geen
        // voorwaarden; falen meldt hij met een foutcode die we hier niet kunnen
        // repareren en die alleen betekent dat de tik grof blijft.
        let uitkomst = unsafe { windows::Win32::Media::timeBeginPeriod(1) };
        if uitkomst != 0 {
            tracing::warn!(
                code = uitkomst,
                "timerresolutie niet op 1 ms te zetten; beeld zal onregelmatiger tonen"
            );
        }
    });
}

/// Op macOS tikt de klok al fijner dan een milliseconde; `recv_timeout` en
/// `thread::sleep` doen daar gewoon wat er gevraagd wordt. Niets te repareren.
#[cfg(not(windows))]
fn zorg_voor_fijne_timer() {}

/// Zorgt dat een socket **niet** meegaat naar een kindproces.
///
/// `UdpSocket::try_clone` dupliceert op Windows met `bInheritHandle = TRUE`: de kloon is
/// erfelijk, het origineel niet (gemeten). Elk kindproces dat daarna start erft die kloon
/// en houdt de poort bezet zolang het leeft. Dat is de fout die na een update opdook: de
/// app start de updater, de updater start de nieuwe app, en die nieuwe app kan zijn eigen
/// mediapoort niet meer binden — "Only one usage of each socket address (os error 10048)"
/// — tot iemand hem met de hand opnieuw start. De controlepoort had het niet, want de
/// QUIC-socket wordt nooit gekloond.
///
/// Unix zet hier `CLOEXEC` voor; Windows heeft daar in std geen equivalent voor.
///
/// Mislukken mag het opstarten niet tegenhouden: dan werkt alles gewoon, en is alleen
/// bijwerken zonder herstart weer stuk.
#[cfg(windows)]
fn niet_doorgeven_aan_kindproces(sock: &UdpSocket) {
    use std::os::windows::io::AsRawSocket;
    use windows::Win32::Foundation::{
        SetHandleInformation, HANDLE, HANDLE_FLAGS, HANDLE_FLAG_INHERIT,
    };
    let handle = HANDLE(sock.as_raw_socket() as *mut std::ffi::c_void);
    // SAFETY: het handle komt uit de socket die we hier vasthouden en leeft dus nog.
    if let Err(e) = unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0)) }
    {
        tracing::warn!(error = %e, "mediasocket blijft erfelijk; bijwerken kan een herstart vragen");
    }
}

#[cfg(not(windows))]
fn niet_doorgeven_aan_kindproces(_sock: &UdpSocket) {}

pub struct MediaSocket {
    sock: UdpSocket,
}

impl MediaSocket {
    /// Bindt op alle interfaces. Poort 0 laat het besturingssysteem kiezen, wat handig
    /// is voor tests.
    pub fn bind(port: u16) -> Result<Self> {
        // Vóór de timeout hieronder: anders is die 15,6 ms in plaats van 200.
        zorg_voor_fijne_timer();

        let sock = UdpSocket::bind(("0.0.0.0", port))
            .with_context(|| format!("UDP-poort {port} binden"))?;
        // Zonder timeout kan een wachtende thread niet meer worden afgesloten.
        sock.set_read_timeout(Some(Duration::from_millis(200)))?;

        // Mislukken mag: een kleinere buffer is trager, geen fout. Wél melden, want dan
        // is dit de eerste verdachte zodra beeld begint te haperen.
        let s2 = socket2::SockRef::from(&sock);
        if let Err(e) = s2.set_recv_buffer_size(ONTVANGBUFFER) {
            tracing::warn!(error = %e, "ontvangbuffer vergroten mislukt");
        }

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
    ///
    /// De kloon is op Windows erfelijk waar het origineel dat niet is; zie
    /// [`niet_doorgeven_aan_kindproces`] voor wat dat kost als je het laat staan.
    pub fn probeer_clone(&self) -> Result<Self> {
        let sock = self.sock.try_clone().context("mediasocket klonen")?;
        niet_doorgeven_aan_kindproces(&sock);
        Ok(Self { sock })
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

    /// De kloon van een mediasocket mag niet meegaan naar een kindproces.
    ///
    /// Zonder dit hield de app die de updater startte zijn eigen mediapoort vast via de
    /// geërfde kloon, en kon de bijgewerkte app na de herstart geen voice meer openen —
    /// gemeten op 2026-08-12, `os error 10048`. Het origineel is al niet erfelijk; de
    /// kloon was dat wél, en dat is wat deze test bewaakt.
    #[cfg(windows)]
    #[test]
    fn een_gekloonde_mediasocket_is_niet_erfelijk() {
        use std::os::windows::io::AsRawSocket;
        use windows::Win32::Foundation::{GetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT};

        let sock = MediaSocket::bind(0).unwrap();
        let kloon = sock.probeer_clone().unwrap();

        for (wat, s) in [("origineel", &sock), ("kloon", &kloon)] {
            let handle = HANDLE(s.sock.as_raw_socket() as *mut std::ffi::c_void);
            let mut vlaggen = 0u32;
            unsafe { GetHandleInformation(handle, &mut vlaggen) }.expect("handle-vlaggen");
            assert_eq!(
                vlaggen & HANDLE_FLAG_INHERIT.0,
                0,
                "{wat} gaat mee naar kindprocessen"
            );
        }
    }

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
