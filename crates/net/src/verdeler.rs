//! Eén mediasocket per proces (fase B5 van `docs/PLAN-ZONDER-TAILSCALE.md`).
//!
//! Tot B5 had elk gesprek en elke kijker een eigen UDP-socket, en de deler verstuurde vanaf
//! weer een andere. Op een tailnet kan dat; achter een NAT is elke socket een eigen gat dat
//! iemand moet doorprikken, en de kijker weet niet eens naar welke poort. Nu is er één socket
//! op `media_port`, zoals BUNDLE in WebRTC: per peerpaar één gat, één keepalive en één port
//! mapping.
//!
//! Eén vaste ontvangthread ontsleutelt, leert het bronadres per peer (roaming, zie
//! [`crate::mediasleutel`]) en verdeelt:
//!
//! - Opus (stem, bureaubladgeluid, muziek) naar de audiosessie;
//! - beeld per `(afzender, stream_id)` naar de kijker die erop intekende;
//! - keepalives nergens heen: het leren gebeurde al bij het openen.
//!
//! **De thread wacht nooit op een consument.** Elke consument heeft een begrensde rij, en wie
//! achterloopt verliest wat er niet meer in past, zoals een volle socketbuffer dat vroeger
//! deed. Anders zou een kijker die even in `decode` zit de stem van iedereen ophouden.
//!
//! **De vensters horen bij deze ene socket** en worden niet opnieuw opgebouwd bij opnieuw
//! deelnemen of opnieuw intekenen. Daarmee is het gat uit beslissing 46 dicht: een opgevangen
//! pakket naar een verse socket herhalen kan niet meer, want er is geen verse socket.

use crate::media::{MediaSocket, PeerAdres, MAX_PAKKET};
use crate::mediasleutel::MediaSleutels;
use anyhow::{Context, Result};
use arc_swap::ArcSwap;
use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};
use fitcom_proto::{MediaHeader, PayloadType, PeerId, KEEPALIVE_STREAM_ID};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Eén ontvangen, ontsleuteld pakket. `van` is bewezen: het ging open met zijn sleutel.
#[derive(Debug)]
pub struct Pakket {
    pub van: PeerId,
    pub header: MediaHeader,
    pub payload: Vec<u8>,
}

/// Elke zoveel een keepalive naar elke verbonden peer, zodat het gat in de NAT openblijft
/// voordat iemand praat of deelt. Ruim onder de UDP-time-out van gangbare routers (30 s).
const KEEPALIVE_ELKE: Duration = Duration::from_secs(15);
/// Vlak na een nieuwe verbinding of een nieuwe hint elke seconde, tot dit voorbij is: dan
/// staat het pad er binnen een seconde in plaats van na een ronde van 15.
const SNEL_NA_WIJZIGING: Duration = Duration::from_secs(20);
const SNEL_ELKE: Duration = Duration::from_secs(1);
/// Hoe lang `ontvang` wacht als er niets komt. Bepaalt alleen hoe snel de thread een stop of
/// een keepalive-moment ziet; korter is wakker worden voor niets (invariant 4).
const WACHT: Duration = Duration::from_secs(1);
/// Een keyframe van 1080p is ruim 240 fragmenten; dit is ruim acht keyframes per kijker.
const BEELD_RIJ: usize = 2048;
/// Stem, bureaubladgeluid en muziek van iedereen samen: 50 pakketten per seconde per bron.
const GELUID_RIJ: usize = 512;
/// Niet vaker dan dit melden dat een consument pakketten liet liggen.
const MELD_PAUZE: Duration = Duration::from_secs(5);

/// Per route ook het id van het [`Abonnement`] dat hem maakte, zodat een oud abonnement bij
/// het opruimen niet de route van een nieuwer op dezelfde stream weghaalt.
#[derive(Clone, Default)]
struct Routes {
    geluid: Option<(u64, Sender<Pakket>)>,
    beeld: HashMap<(PeerId, u32), (u64, Sender<Pakket>)>,
}

/// Een rij van de verdeler: lees hem als een gewone crossbeam-`Receiver`. Valt hij weg, dan
/// verdwijnt zijn route meteen, ook als er voor die stream nooit meer iets binnenkomt.
pub struct Abonnement {
    rij: Receiver<Pakket>,
    /// `None` voor het geluid.
    stream: Option<(PeerId, u32)>,
    id: u64,
    routes: Arc<ArcSwap<Routes>>,
}

impl std::ops::Deref for Abonnement {
    type Target = Receiver<Pakket>;
    fn deref(&self) -> &Receiver<Pakket> {
        &self.rij
    }
}

impl Drop for Abonnement {
    fn drop(&mut self) {
        let (stream, id) = (self.stream, self.id);
        self.routes.rcu(|r| {
            let mut r = Routes::clone(r);
            match stream {
                None if r.geluid.as_ref().is_some_and(|(i, _)| *i == id) => r.geluid = None,
                Some(k) if r.beeld.get(&k).is_some_and(|(i, _)| *i == id) => {
                    r.beeld.remove(&k);
                }
                _ => {}
            }
            r
        });
    }
}

fn nieuw_id() -> u64 {
    static VOLGENDE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    VOLGENDE.fetch_add(1, Ordering::Relaxed)
}

/// Het handvat op de mediasocket van dit proces. Goedkoop te klonen. De ontvangthread stopt
/// als de laatste kloon weg is, of eerder met [`Verdeler::stop`].
#[derive(Clone)]
pub struct Verdeler(Arc<Binnen>);

struct Binnen {
    socket: MediaSocket,
    poort: u16,
    routes: Arc<ArcSwap<Routes>>,
    stop: Arc<AtomicBool>,
}

impl Drop for Binnen {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl std::fmt::Debug for Verdeler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Verdeler")
            .field("poort", &self.0.poort)
            .finish()
    }
}

impl Verdeler {
    /// Bindt de mediasocket en start de ontvangthread. Poort 0 laat het systeem kiezen.
    pub fn start(poort: u16, sleutels: MediaSleutels) -> Result<Self> {
        // Zonder fijne timer: die is pas nodig als er media loopt (`zender`, `luister_*`), en
        // deze socket bestaat vanaf het starten, ook in de tray.
        let socket = MediaSocket::bind_stil(poort, sleutels)?;
        let poort = socket.local_addr()?.port();
        let ontvanger = socket.probeer_clone()?;
        ontvanger.zet_timeout(WACHT)?;
        let routes = Arc::new(ArcSwap::from_pointee(Routes::default()));
        let stop = Arc::new(AtomicBool::new(false));
        {
            let (routes, stop) = (routes.clone(), stop.clone());
            std::thread::Builder::new()
                .name("fitcom-media".into())
                .spawn(move || ontvang_lus(ontvanger, &routes, &stop))
                .context("mediathread starten")?;
        }
        Ok(Self(Arc::new(Binnen {
            socket,
            poort,
            routes,
            stop,
        })))
    }

    /// De poort waarop alle media van dit proces binnenkomt; hoort in `Hello`, `VoiceJoin`
    /// en `StreamSubscribe`.
    pub fn poort(&self) -> u16 {
        self.0.poort
    }

    /// Een kloon van de socket om mee te versturen, zodat het bronadres van alles wat we
    /// sturen deze ene socket is. Elke verzendende thread zijn eigen kloon.
    pub fn zender(&self) -> Result<MediaSocket> {
        crate::media::zorg_voor_fijne_timer();
        self.0.socket.probeer_clone()
    }

    /// Alle Opus vanaf nu naar deze rij. Een vorige luisteraar krijgt niets meer.
    pub fn luister_geluid(&self) -> Abonnement {
        self.abonneer(None, GELUID_RIJ)
    }

    /// Het beeld van `stream_id` van `van` vanaf nu naar deze rij. Een vorige kijker op
    /// dezelfde stream krijgt niets meer.
    pub fn luister_beeld(&self, van: PeerId, stream_id: u32) -> Abonnement {
        self.abonneer(Some((van, stream_id)), BEELD_RIJ)
    }

    fn abonneer(&self, stream: Option<(PeerId, u32)>, plek: usize) -> Abonnement {
        crate::media::zorg_voor_fijne_timer();
        let (tx, rij) = bounded(plek);
        let id = nieuw_id();
        self.0.routes.rcu(|r| {
            let mut r = Routes::clone(r);
            match stream {
                None => r.geluid = Some((id, tx.clone())),
                Some(k) => {
                    r.beeld.insert(k, (id, tx.clone()));
                }
            }
            r
        });
        Abonnement {
            rij,
            stream,
            id,
            routes: self.0.routes.clone(),
        }
    }

    /// Stopt de ontvangthread nu, ook als er nog klonen zijn (bij het afsluiten van de mesh).
    pub fn stop(&self) {
        self.0.stop.store(true, Ordering::Relaxed);
    }
}

fn keepalive_header() -> MediaHeader {
    MediaHeader {
        stream_id: KEEPALIVE_STREAM_ID,
        seq: 0,
        timestamp: 0,
        payload_type: PayloadType::KEEPALIVE,
        flags: 0,
        frag_index: 0,
    }
}

fn ontvang_lus(mut socket: MediaSocket, routes: &ArcSwap<Routes>, stop: &AtomicBool) {
    let mut buf = [0u8; MAX_PAKKET];
    let sleutels = socket.sleutels().clone();
    let mut versie = None;
    let mut snel_tot = Instant::now();
    let mut volgende_keepalive = Instant::now();
    let (mut verloren, mut gemeld) = (0u64, Instant::now());

    while !stop.load(Ordering::Relaxed) {
        // Per pakket alleen een klok en een getal lezen; de doelenlijst pas als er iets heen moet.
        let nu = Instant::now();
        let v = sleutels.versie();
        if versie != Some(v) {
            versie = Some(v);
            snel_tot = nu + SNEL_NA_WIJZIGING;
            volgende_keepalive = nu;
        }
        if nu >= volgende_keepalive {
            for (id, adressen) in sleutels.keepalive_doelen() {
                for addr in adressen {
                    // Een peer die net wegviel heeft geen sleutel meer: dan faalt dit, en
                    // dat is goed zo.
                    let _ = socket.stuur(PeerAdres { id, addr }, &keepalive_header(), &[]);
                }
            }
            volgende_keepalive = nu
                + if nu < snel_tot {
                    SNEL_ELKE
                } else {
                    KEEPALIVE_ELKE
                };
        }

        let (van, header, payload) = match socket.ontvang(&mut buf) {
            Ok(Some(p)) => p,
            Ok(None) => continue,
            Err(e) => {
                tracing::warn!(error = %format!("{e:#}"), "mediasocket gaf een fout");
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
        };
        if header.payload_type == PayloadType::KEEPALIVE {
            continue;
        }
        let r = routes.load();
        let geluid = header.payload_type == PayloadType::OPUS;
        let doel = if geluid {
            r.geluid.as_ref()
        } else {
            r.beeld.get(&(van, header.stream_id))
        };
        let Some((id, tx)) = doel else { continue };
        let pakket = Pakket {
            van,
            header,
            payload: payload.to_vec(),
        };
        match tx.try_send(pakket) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                verloren += 1;
                if gemeld.elapsed() >= MELD_PAUZE {
                    tracing::warn!(
                        verloren,
                        "een mediaconsument loopt achter; pakketten weggegooid"
                    );
                    (verloren, gemeld) = (0, Instant::now());
                }
            }
            // De kijker of de sessie is weg en zijn abonnement ruimde nog niet op: dan hier.
            Err(TrySendError::Disconnected(_)) => {
                let weg = *id;
                drop(r);
                routes.rcu(|r| {
                    let mut r = Routes::clone(r);
                    if r.geluid.as_ref().is_some_and(|(i, _)| *i == weg) {
                        r.geluid = None;
                    }
                    r.beeld.retain(|_, (i, _)| *i != weg);
                    r
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fitcom_proto::VOICE_STREAM_ID;
    use std::net::SocketAddr;

    fn header(stream_id: u32, seq: u32, payload_type: PayloadType) -> MediaHeader {
        MediaHeader {
            stream_id,
            seq,
            timestamp: 0,
            payload_type,
            flags: 0,
            frag_index: 0,
        }
    }

    /// Twee peers met elk één verdeler, sleutels gekoppeld in één tabel zoals in de andere
    /// tests van deze crate.
    fn paar() -> (Verdeler, Verdeler, PeerId, PeerId) {
        let sleutels = MediaSleutels::default();
        let (a, b) = (PeerId::new_random(), PeerId::new_random());
        MediaSleutels::koppel((&sleutels, a), (&sleutels, b));
        let va = Verdeler::start(0, sleutels.clone()).unwrap();
        let vb = Verdeler::start(0, sleutels).unwrap();
        (va, vb, a, b)
    }

    fn naar(v: &Verdeler, id: PeerId) -> PeerAdres {
        PeerAdres {
            id,
            addr: SocketAddr::from(([127, 0, 0, 1], v.poort())),
        }
    }

    #[test]
    fn stem_en_beeld_komen_elk_bij_hun_eigen_consument() {
        let (va, vb, a, b) = paar();
        let stem = vb.luister_geluid();
        let beeld = vb.luister_beeld(a, 3);
        let ander_beeld = vb.luister_beeld(a, 4);

        let z = va.zender().unwrap();
        z.stuur(
            naar(&vb, b),
            &header(VOICE_STREAM_ID, 1, PayloadType::OPUS),
            b"stem",
        )
        .unwrap();
        z.stuur(naar(&vb, b), &header(3, 1, PayloadType::H264), b"beeld")
            .unwrap();
        // Niemand kijkt naar stream 5: weg, zonder iemand op te houden.
        z.stuur(naar(&vb, b), &header(5, 1, PayloadType::H264), b"niemand")
            .unwrap();

        let wacht = Duration::from_secs(2);
        let p = stem.recv_timeout(wacht).unwrap();
        assert_eq!((p.van, &p.payload[..]), (a, &b"stem"[..]));
        let p = beeld.recv_timeout(wacht).unwrap();
        assert_eq!((p.header.stream_id, &p.payload[..]), (3, &b"beeld"[..]));
        assert!(ander_beeld
            .recv_timeout(Duration::from_millis(200))
            .is_err());
    }

    #[test]
    fn een_stoot_van_twee_delers_tegelijk_komt_heel_aan() {
        // Twee keyframes van 242 fragmenten op dezelfde socket, terwijl de kijker bezig is.
        // Zonder 2 MB ontvangbuffer en een rij per kijker sneuvelde de tweede.
        let (va, vb, a, b) = paar();
        let (k1, k2) = (vb.luister_beeld(a, 1), vb.luister_beeld(a, 2));
        let z = va.zender().unwrap();
        let payload = [7u8; fitcom_proto::MAX_MEDIA_PAYLOAD];
        for i in 0..242 {
            for stream in [1, 2] {
                z.stuur(
                    naar(&vb, b),
                    &header(stream, i, PayloadType::H264),
                    &payload,
                )
                .unwrap();
            }
        }
        std::thread::sleep(Duration::from_millis(100));
        let tel = |k: &Abonnement| {
            std::iter::from_fn(|| k.recv_timeout(Duration::from_millis(300)).ok()).count()
        };
        assert_eq!((tel(&k1), tel(&k2)), (242, 242));
    }

    #[test]
    fn na_een_keepalive_gaat_het_antwoord_naar_het_echte_bronadres() {
        // B stuurt vanaf zijn verdeler; A zag hem nog nooit en heeft een verkeerde hint
        // (een poort waar niemand luistert). Na B's keepalive weet A waar B zit.
        let (va, vb, a, b) = paar();
        let stem_bij_b = vb.luister_geluid();
        let verkeerd = PeerAdres {
            id: b,
            addr: SocketAddr::from(([127, 0, 0, 1], 9)),
        };
        let za = va.zender().unwrap();
        za.stuur(verkeerd, &header(0, 1, PayloadType::OPUS), b"weg")
            .unwrap();
        assert!(stem_bij_b.recv_timeout(Duration::from_millis(200)).is_err());

        let zb = vb.zender().unwrap();
        zb.stuur(naar(&va, a), &keepalive_header(), &[]).unwrap();
        let tot = Instant::now() + Duration::from_secs(2);
        while za.sleutels().geleerd(b).is_none() && Instant::now() < tot {
            std::thread::sleep(Duration::from_millis(10));
        }
        za.stuur(verkeerd, &header(0, 2, PayloadType::OPUS), b"aan")
            .unwrap();
        let p = stem_bij_b.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(&p.payload[..], b"aan");
    }

    #[test]
    fn een_vertrokken_kijker_houdt_niemand_op_en_ruimt_zijn_route_op() {
        let (va, vb, a, b) = paar();
        drop(vb.luister_beeld(a, 1));
        assert!(vb.0.routes.load().beeld.is_empty(), "weg zodra hij wegvalt");
        // Een oud abonnement dat na een nieuwer op dezelfde stream wegvalt, laat dat staan.
        let oud = vb.luister_beeld(a, 2);
        let nieuw = vb.luister_beeld(a, 2);
        drop(oud);
        assert_eq!(vb.0.routes.load().beeld.len(), 1);
        drop(nieuw);
        let stem = vb.luister_geluid();
        let z = va.zender().unwrap();
        for i in 0..10 {
            z.stuur(naar(&vb, b), &header(1, i, PayloadType::H264), b"x")
                .unwrap();
        }
        z.stuur(naar(&vb, b), &header(0, 1, PayloadType::OPUS), b"stem")
            .unwrap();
        assert!(stem.recv_timeout(Duration::from_secs(2)).is_ok());
        assert!(vb.0.routes.load().beeld.is_empty(), "dode route opgeruimd");
    }
}
