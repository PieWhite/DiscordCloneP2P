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
//!
//! # Versleuteld (protocol 6)
//!
//! Elk pakket is AES-256-GCM met een sleutel per richting en per verbinding; zie
//! [`crate::mediasleutel`]. Een socket kent de sleuteltabel van zijn mesh en houdt zijn
//! eigen replayvensters bij, dus versturen gaat naar een [`PeerAdres`] en ontvangen levert
//! de bewezen afzender op in plaats van een bronadres.

use crate::mediasleutel::{MediaSleutels, Vensters};
use anyhow::{Context, Result};
use fitcom_proto::{MediaHeader, PeerId, MEDIA_HEADER_LEN, MEDIA_KOP_LEN, MEDIA_TAG_LEN};
use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

/// Wie een pakket krijgt: de peer, voor de sleutel, en waar zijn socket luistert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerAdres {
    pub id: PeerId,
    pub addr: SocketAddr,
}

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
/// 1 MB was ruim vier keyframes. Groter is geen gratis winst: wat hier in de rij staat is
/// beeld dat al te laat is, en de samensteller gooit oude frames toch weg.
///
/// Sinds protocol 6 is elk pakket 28 bytes groter (sessie-index, teller, tag). Gemeten op
/// loopback hield 1 MB daardoor 917 pakketten vast in plaats van 940; 1100 kB houdt er
/// weer minstens evenveel vast, zodat een stoot niet eerder sneuvelt dan vóór de AEAD.
const ONTVANGBUFFER: usize = 1100 * 1024;

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

/// Of deze fout betekent "het datagram paste niet in de buffer".
///
/// Windows meldt dat als `WSAEMSGSIZE`, macOS en de BSD's als `EMSGSIZE`, Linux gebruikt
/// een ander nummer voor diezelfde naam. `std::io::ErrorKind` heeft er geen variant voor,
/// dus het gaat via de rauwe code — en beide platforms bouwen deze binary, dus beide
/// codes horen hier te staan.
fn is_te_groot(e: &std::io::Error) -> bool {
    #[cfg(windows)]
    const CODES: [i32; 1] = [10040]; // WSAEMSGSIZE
    #[cfg(not(windows))]
    const CODES: [i32; 2] = [40, 90]; // EMSGSIZE op macOS/BSD, en op Linux
    e.raw_os_error().is_some_and(|c| CODES.contains(&c))
}

pub struct MediaSocket {
    sock: UdpSocket,
    sleutels: MediaSleutels,
    /// Per socket en niet per sleutel: zie de moduledoc van [`crate::mediasleutel`].
    vensters: Vensters,
}

impl MediaSocket {
    /// Bindt op alle interfaces. Poort 0 laat het besturingssysteem kiezen, wat handig
    /// is voor tests. `sleutels` is de tabel van de mesh (`MeshHandle::media`).
    pub fn bind(port: u16, sleutels: MediaSleutels) -> Result<Self> {
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

        Ok(Self {
            sock,
            sleutels,
            vensters: Vensters::default(),
        })
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
    /// Een kloon heeft eigen replayvensters: gebruik er één per ontvangende thread.
    pub fn probeer_clone(&self) -> Result<Self> {
        let sock = self.sock.try_clone().context("mediasocket klonen")?;
        niet_doorgeven_aan_kindproces(&sock);
        Ok(Self {
            sock,
            sleutels: self.sleutels.clone(),
            vensters: Vensters::default(),
        })
    }

    /// Versleutelt en verstuurt. Een fout als er voor die peer (nog of niet meer) geen
    /// sleutel is: dan is hij niet verbonden, en wat er dan nog gestuurd wordt kan weg.
    pub fn stuur(&self, naar: PeerAdres, header: &MediaHeader, payload: &[u8]) -> Result<()> {
        let mut buf = [0u8; MAX_PAKKET];
        let klaar = MEDIA_HEADER_LEN + payload.len();
        if MEDIA_KOP_LEN + klaar + MEDIA_TAG_LEN > MAX_PAKKET {
            anyhow::bail!("mediapakket van {klaar} bytes is te groot");
        }
        header.write_to(
            (&mut buf[MEDIA_KOP_LEN..MEDIA_KOP_LEN + MEDIA_HEADER_LEN])
                .try_into()
                .expect("vaste lengte"),
        );
        buf[MEDIA_KOP_LEN + MEDIA_HEADER_LEN..MEDIA_KOP_LEN + klaar].copy_from_slice(payload);
        let lengte = self
            .sleutels
            .verzegel(naar.id, &mut buf, klaar)
            .context("geen mediasleutel voor deze peer")?;
        self.sock.send_to(&buf[..lengte], naar.addr)?;
        Ok(())
    }

    /// `Ok(None)` bij een timeout — dat is de normale gang van zaken als niemand praat,
    /// geen fout.
    ///
    /// Alles wat niet opengaat wordt stil genegeerd: te kort, een onbekende sessie-index,
    /// een vervalste tag, een herhaling. Op een open UDP-poort komt vroeg of laat rommel
    /// binnen en daar mag niets van omvallen. De afzender is wie de sleutel had, niet een
    /// bronadres.
    pub fn ontvang<'a>(
        &mut self,
        buf: &'a mut [u8; MAX_PAKKET],
    ) -> Result<Option<(PeerId, MediaHeader, &'a [u8])>> {
        match self.sock.recv_from(buf) {
            Ok((n, _)) => match self.sleutels.open(&mut self.vensters, &mut buf[..n]) {
                Some((van, klaar)) => {
                    Ok(MediaHeader::parse(klaar).map(|(header, payload)| (van, header, payload)))
                }
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
            // Een datagram dat groter is dan onze buffer is precies zo goed rommel als
            // een datagram dat te kort is: onze eigen pakketten passen altijd, dus dit
            // komt van iemand anders. Het als fout teruggeven kostte per pakket één
            // synchrone schrijfactie naar het logbestand — bij de twee consumenten die
            // erop loggen en doorgaan — en dat op precies de audio- en videothreads die
            // invariant 4 beschermt (B-27).
            Err(e) if is_te_groot(&e) => Ok(None),
            Err(e) => Err(e).context("mediapakket ontvangen"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fitcom_proto::PayloadType;

    /// Twee sockets met gekoppelde sleutels, zoals twee verbonden peers ze hebben. Levert
    /// ook het adres op waaronder `b` voor `a` bereikbaar is.
    fn paar() -> (MediaSocket, MediaSocket, PeerAdres, PeerId) {
        let sleutels = MediaSleutels::default();
        let (ida, idb) = (PeerId::new_random(), PeerId::new_random());
        MediaSleutels::koppel((&sleutels, ida), (&sleutels, idb));
        let a = MediaSocket::bind(0, sleutels.clone()).unwrap();
        let b = MediaSocket::bind(0, sleutels).unwrap();
        let naar_b = PeerAdres {
            id: idb,
            addr: SocketAddr::from(([127, 0, 0, 1], b.local_addr().unwrap().port())),
        };
        (a, b, naar_b, ida)
    }

    fn header(seq: u32) -> MediaHeader {
        MediaHeader {
            stream_id: 0,
            seq,
            timestamp: 960,
            payload_type: PayloadType::OPUS,
            flags: 0,
            frag_index: 0,
        }
    }

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

        let sock = MediaSocket::bind(0, MediaSleutels::default()).unwrap();
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
    fn pakket_komt_heelhuids_aan_met_de_afzender_erbij() {
        let (a, mut b, naar_b, ida) = paar();
        a.stuur(naar_b, &header(42), b"spraak").unwrap();

        let mut buf = [0u8; MAX_PAKKET];
        let (van, terug, payload) = b.ontvang(&mut buf).unwrap().expect("pakket verwacht");
        assert_eq!(van, ida, "de afzender is wie de sleutel had");
        assert_eq!(terug, header(42));
        assert_eq!(payload, b"spraak");
    }

    #[test]
    fn een_opgevangen_of_vervalst_pakket_komt_er_niet_door() {
        // Wat B-18 en B-28 open lieten: wie een pakket op de draad zag of een bronadres
        // kon vervalsen, kon stem of beeld injecteren. Nu niet meer, ook niet over een
        // echte socket: een herhaling, een omgedraaid bit en kale bytes gaan niet open.
        let (a, mut b, naar_b, _) = paar();
        let afluisteraar = UdpSocket::bind("127.0.0.1:0").unwrap();
        afluisteraar
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let tap = PeerAdres {
            addr: afluisteraar.local_addr().unwrap(),
            ..naar_b
        };
        a.stuur(tap, &header(7), b"echt").unwrap();
        let mut opgevangen = [0u8; MAX_PAKKET];
        let n = afluisteraar.recv(&mut opgevangen).unwrap();
        let opgevangen = &opgevangen[..n];

        let mut buf = [0u8; MAX_PAKKET];
        afluisteraar.send_to(opgevangen, naar_b.addr).unwrap();
        assert!(
            b.ontvang(&mut buf).unwrap().is_some(),
            "het origineel gaat open"
        );
        afluisteraar.send_to(opgevangen, naar_b.addr).unwrap();
        assert!(b.ontvang(&mut buf).unwrap().is_none(), "de herhaling niet");

        let mut omgedraaid = opgevangen.to_vec();
        omgedraaid[MEDIA_KOP_LEN + 2] ^= 0x40;
        afluisteraar.send_to(&omgedraaid, naar_b.addr).unwrap();
        assert!(
            b.ontvang(&mut buf).unwrap().is_none(),
            "een omgedraaid bit niet"
        );

        // Een pakket in de vorm van protocol 5: een kale header met payload.
        let mut oud = [0u8; MEDIA_HEADER_LEN + 6];
        header(8).write_to((&mut oud[..MEDIA_HEADER_LEN]).try_into().unwrap());
        afluisteraar.send_to(&oud, naar_b.addr).unwrap();
        assert!(b.ontvang(&mut buf).unwrap().is_none(), "onversleuteld niet");
    }

    #[test]
    fn zonder_sleutel_gaat_er_niets_de_deur_uit() {
        let a = MediaSocket::bind(0, MediaSleutels::default()).unwrap();
        let doel = PeerAdres {
            id: PeerId::new_random(),
            addr: SocketAddr::from(([127, 0, 0, 1], 9)),
        };
        assert!(a.stuur(doel, &header(1), b"x").is_err());
    }

    #[test]
    fn timeout_is_geen_fout() {
        // Niemand praat. Dat is de normale toestand, niet iets om op te struikelen.
        let mut s = MediaSocket::bind(0, MediaSleutels::default()).unwrap();
        let mut buf = [0u8; MAX_PAKKET];
        assert!(s.ontvang(&mut buf).unwrap().is_none());
    }

    #[test]
    fn rommel_op_de_poort_wordt_genegeerd() {
        let (a, mut b, naar_b, _) = paar();
        a.sock.send_to(b"te kort", naar_b.addr).unwrap();

        let mut buf = [0u8; MAX_PAKKET];
        assert!(b.ontvang(&mut buf).unwrap().is_none(), "mag niet omvallen");
    }

    #[test]
    fn b27_een_te_groot_datagram_is_rommel_en_geen_fout() {
        // Niet met een echt datagram te testen: op macOS kapt de kernel een te groot
        // datagram stil af en komt er helemaal geen fout: dit pad is er voor Windows,
        // waar `WSAEMSGSIZE` terugkomt. Wat hier te bewaken valt is dat de code herkend
        // wordt op het platform waar hij vandaan komt — anders is elk zo'n pakket een
        // logregel op de audio- of videothread.
        let code = if cfg!(windows) { 10040 } else { 40 };
        assert!(is_te_groot(&std::io::Error::from_raw_os_error(code)));
        assert!(
            !is_te_groot(&std::io::Error::from_raw_os_error(1)),
            "een gewone fout mag niet stil weggeslikt worden"
        );
    }

    #[test]
    fn te_groot_pakket_wordt_geweigerd_in_plaats_van_afgekapt() {
        let (a, _b, naar_b, _) = paar();
        assert!(a.stuur(naar_b, &header(0), &[0u8; MAX_PAKKET]).is_err());
        // Het grootste pakket dat we echt versturen (een pariteitsfragment) past wel.
        let pariteit = [0u8; fitcom_proto::PARITEIT_PAYLOAD_LEN];
        assert!(a.stuur(naar_b, &header(1), &pariteit).is_ok());
    }
}
