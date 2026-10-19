//! Versleuteling van het mediapad (protocol 6, beslissing 46).
//!
//! Tot protocol 5 was een mediapakket een kale header met payload, en de enige controle het
//! bronadres (B-18): wie dat kon vervalsen kon stem in de mix zetten of beeld laten
//! samenstellen uit eigen fragmenten (B-28). Nu is elk pakket AES-256-GCM:
//!
//! ```text
//! [sessie-index 4][teller 8][versleuteld: mediaheader 16 + payload][tag 16]
//! ```
//!
//! - **Sleutels per richting**, afgeleid van de QUIC-verbinding:
//!   `export_keying_material(32, MEDIA_SLEUTEL_LABEL, PeerId van de afzender)`. Beide kanten
//!   kunnen beide uitrekenen; niemand anders, want de exporter hangt aan de TLS-sessie die de
//!   handshake al aan een identiteit bond (`mesh.rs`).
//! - **De sessie-index kiest de ontvanger** (zoals WireGuard) en staat in zijn `Hello` of
//!   `HelloAck`. Een binnenkomend pakket wordt zo met één tabelopzoeking aan een sleutel
//!   gekoppeld: rommel op een open poort kost geen cryptografie (invariant 4).
//! - **De teller** is één `AtomicU64` per sleutel, gedeeld door alle threads die naar die
//!   peer sturen. Geen lock (werkafspraak). Hij is ook de nonce, dus hij mag nooit twee keer
//!   dezelfde waarde krijgen onder dezelfde sleutel: elke verbinding krijgt een nieuwe
//!   sleutel én een nieuwe teller, en een bestaande wordt nooit vervangen door een verse met
//!   dezelfde sleutel ([`MediaSleutels::zet`]).
//! - **Replayvenster van 2048** over die teller, per ontvangende socket. In fase A zijn er
//!   nog meerdere sockets (de voice-socket, één per kijker); een gedeeld venster zou een lock
//!   op het hete pad vragen. Hetzelfde pakket naar een *andere* socket van ons herhalen levert
//!   niets op: de `stream_id` zit in het versleutelde deel en elke consument filtert erop.
//! - **Plus een hoogste-tellermerk per sleutel**, gedeeld door alle sockets (`AtomicU64`,
//!   `fetch_max`). Een *nieuwe* socket voor dezelfde stream (opnieuw deelnemen aan het
//!   gesprek, opnieuw kijken) begint met een leeg venster, en zou zonder dit elk eerder
//!   opgevangen pakket van die verbinding nog één keer aannemen. Met het merk blijft dat
//!   beperkt tot de laatste 2048 tellers (ruim een seconde). Bij B5, één socket per proces,
//!   verdwijnt ook dat, zolang die socket en zijn vensters niet opnieuw opgebouwd worden bij
//!   opnieuw deelnemen of intekenen.
//!
//! De tabel is een `ArcSwap`: de mesh-actor schrijft hem bij verbinden en verbreken, de
//! mediathreads lezen hem zonder lock.
//!
//! # Roaming (fase B5)
//!
//! Sinds er één mediasocket per proces is, antwoorden we naar het **laatste geauthenticeerde
//! bronadres** van een peer, zoals WireGuard: dat dekt een NAT die de poort herschrijft en een
//! peer wiens adres wisselt. Elke verbinding heeft daarvoor één adresvak, gedeeld door haar
//! zend- en ontvangstsleutel. Het wordt alleen gevuld ná een geslaagde `open` (een adres is een
//! hint, nooit toegang) en pas nadat er van die verbinding een **keepalive** binnenkwam: alleen
//! een peer met één socket stuurt die. Een peer zonder B5 stuurt beeld vanaf losse poorten en
//! verwacht het op de poort uit zijn `StreamSubscribe`; voor hem blijft alles zoals het was.

use arc_swap::{ArcSwap, ArcSwapOption};
use fitcom_proto::{PayloadType, PeerId, MEDIA_KOP_LEN, MEDIA_TAG_LEN};
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// Zo ver mag een teller achterlopen op de hoogste die we van die sleutel zagen. Bij
/// 12 Mbit/s zijn dat ruim 1400 pakketten per seconde, dus ruim een seconde aan herordening.
pub const VENSTER: u64 = 2048;

/// Hoe lang de ontvangstsleutel van een verbroken verbinding nog geldt. Pakketten die rond
/// een herverbinding onderweg zijn mogen nog aankomen; daarna niet meer.
const NAWERKING: Duration = Duration::from_secs(5);

/// Het laatste geauthenticeerde bronadres van een verbinding (roaming). Leeg tot er een
/// keepalive van die verbinding binnenkwam.
type Adresvak = Arc<ArcSwapOption<SocketAddr>>;

/// Plek van `payload_type` in de ontsleutelde mediaheader (`fitcom_proto::MediaHeader`).
const PAYLOAD_TYPE_PLEK: usize = 12;

struct Zend {
    sleutel: LessSafeKey,
    /// De index die de ontvanger koos.
    index: u32,
    teller: AtomicU64,
    conn_id: u64,
    adres: Adresvak,
}

struct Ontvang {
    peer: PeerId,
    sleutel: LessSafeKey,
    conn_id: u64,
    vervalt: Option<Instant>,
    /// De hoogste teller die met deze sleutel ooit opengegaan is, over alle sockets heen.
    hoogste: Arc<AtomicU64>,
    /// Hetzelfde vak als in de [`Zend`] van deze verbinding.
    adres: Adresvak,
}

#[derive(Clone, Default)]
struct Tabel {
    zend: HashMap<PeerId, Arc<Zend>>,
    ontvang: HashMap<u32, Arc<Ontvang>>,
    /// Waar keepalives heen gaan zolang er van een peer nog niets geleerd is: zijn IP met
    /// zijn mediapoort uit de handshake, plus wat hij zelf als kandidaat opgaf (B4).
    hints: HashMap<PeerId, Arc<Vec<SocketAddr>>>,
    /// Gaat omhoog als er een verbinding of een hint bij komt; de mediapost stuurt dan meteen
    /// een keepalive in plaats van op zijn volgende ronde te wachten.
    versie: u64,
}

/// De mediasleutels van deze mesh. Goedkoop te klonen; elke kloon kijkt naar dezelfde tabel.
#[derive(Clone, Default)]
pub struct MediaSleutels(Arc<ArcSwap<Tabel>>);

/// Geen sleutels in de log.
impl std::fmt::Debug for MediaSleutels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let t = self.0.load();
        f.debug_struct("MediaSleutels")
            .field("zend", &t.zend.len())
            .field("ontvang", &t.ontvang.len())
            .finish()
    }
}

/// Een sessie-index die in dit proces nog niet uitgedeeld is. Willekeurige start, zodat een
/// herstarte app niet dezelfde indices uitdeelt als vlak ervoor (dan zouden pakketten van de
/// vorige sessie op een verse sleutel stuklopen in plaats van op de tabel).
pub fn nieuwe_index() -> u32 {
    static VOLGENDE: OnceLock<AtomicU32> = OnceLock::new();
    let volgende = VOLGENDE.get_or_init(|| {
        let mut start = [0u8; 4];
        let _ = ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut start);
        AtomicU32::new(u32::from_be_bytes(start))
    });
    loop {
        // 0 betekent op de draad "niet gezet" (een `Hello` zonder dit veld).
        let i = volgende.fetch_add(1, Ordering::Relaxed);
        if i != 0 {
            return i;
        }
    }
}

fn aead(sleutel: &[u8; 32]) -> LessSafeKey {
    LessSafeKey::new(UnboundKey::new(&AES_256_GCM, sleutel).expect("32 bytes is AES-256"))
}

fn nonce(teller: u64) -> Nonce {
    let mut n = [0u8; 12];
    n[4..].copy_from_slice(&teller.to_be_bytes());
    Nonce::assume_unique_for_key(n)
}

#[cfg(test)]
thread_local! {
    /// Hoe vaak deze thread een pakket probeerde te ontsleutelen. Alleen om te bewijzen dat
    /// een onbekende index geen cryptografie kost.
    static POGINGEN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

impl MediaSleutels {
    /// Legt de sleutels van een nieuwe verbinding vast. Alleen de mesh-actor roept dit aan,
    /// voor de verbinding die de botsing won.
    ///
    /// `hint` is waar zijn mediasocket waarschijnlijk zit (zijn IP met de mediapoort uit de
    /// handshake): daar gaan keepalives heen tot er iets van hem binnen is.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn zet(
        &self,
        conn_id: u64,
        peer: PeerId,
        zend: [u8; 32],
        hun_index: u32,
        ontvang: [u8; 32],
        mijn_index: u32,
        hint: SocketAddr,
    ) {
        let mut t = Tabel::clone(&self.0.load());
        // Nooit een verse teller op dezelfde verbinding: dezelfde sleutel met een teller die
        // opnieuw bij 1 begint hergebruikt nonces, en dan is AES-GCM stuk.
        let vak = match t.zend.get(&peer) {
            Some(z) if z.conn_id == conn_id => z.adres.clone(),
            _ => {
                let vak = Adresvak::default();
                t.zend.insert(
                    peer,
                    Arc::new(Zend {
                        sleutel: aead(&zend),
                        index: hun_index,
                        teller: AtomicU64::new(0),
                        conn_id,
                        adres: vak.clone(),
                    }),
                );
                // Een nieuwe verbinding begint opnieuw met leren: zijn adres kan veranderd
                // zijn, en wat er van de vorige geleerd was hoorde bij die NAT-toestand.
                t.hints.insert(peer, Arc::new(vec![hint]));
                vak
            }
        };
        t.ontvang.entry(mijn_index).or_insert_with(|| {
            Arc::new(Ontvang {
                peer,
                sleutel: aead(&ontvang),
                conn_id,
                vervalt: None,
                hoogste: Arc::default(),
                adres: vak,
            })
        });
        t.versie += 1;
        self.0.store(Arc::new(t));
    }

    /// Waar keepalives naar `peer` heen mogen zolang er van hem niets geleerd is. Doet niets
    /// voor een peer zonder verbinding. De aanroeper begrenst: dit komt deels van de draad (B4).
    pub(crate) fn zet_hints(&self, peer: PeerId, hints: Vec<SocketAddr>) {
        let oud = self.0.load();
        let zelfde = |h: &[SocketAddr]| {
            let (mut a, mut b) = (h.to_vec(), hints.clone());
            a.sort_unstable();
            b.sort_unstable();
            a == b
        };
        if !oud.zend.contains_key(&peer) || oud.hints.get(&peer).is_some_and(|h| zelfde(h)) {
            return;
        }
        let mut t = Tabel::clone(&oud);
        t.hints.insert(peer, Arc::new(hints));
        t.versie += 1;
        self.0.store(Arc::new(t));
    }

    /// Het laatst geleerde bronadres van de mediasocket van `peer`, als hij één socket heeft.
    pub fn geleerd(&self, peer: PeerId) -> Option<SocketAddr> {
        self.0
            .load()
            .zend
            .get(&peer)?
            .adres
            .load()
            .as_deref()
            .copied()
    }

    /// Zie [`Tabel::versie`].
    pub(crate) fn versie(&self) -> u64 {
        self.0.load().versie
    }

    /// Per verbonden peer: waar een keepalive heen moet. Het geleerde adres als dat er is,
    /// anders elke hint.
    pub(crate) fn keepalive_doelen(&self) -> Vec<(PeerId, Vec<SocketAddr>)> {
        let t = self.0.load();
        t.zend
            .iter()
            .map(|(&peer, z)| {
                let naar = match z.adres.load().as_deref() {
                    Some(&a) => vec![a],
                    None => t.hints.get(&peer).map(|h| h.to_vec()).unwrap_or_default(),
                };
                (peer, naar)
            })
            .collect()
    }

    /// Een verbinding is weg. Verzenden met haar sleutel stopt meteen; ontvangen mag nog
    /// [`NAWERKING`] lang, voor wat er onderweg was.
    pub(crate) fn vergeet(&self, conn_id: u64) {
        let oud = self.0.load();
        if !oud.zend.values().any(|z| z.conn_id == conn_id)
            && !oud
                .ontvang
                .values()
                .any(|o| o.conn_id == conn_id && o.vervalt.is_none())
        {
            return;
        }
        let mut t = Tabel::clone(&oud);
        t.zend.retain(|_, z| z.conn_id != conn_id);
        let Tabel { zend, hints, .. } = &mut t;
        hints.retain(|peer, _| zend.contains_key(peer));
        let tot = Instant::now() + NAWERKING;
        for o in t.ontvang.values_mut() {
            if o.conn_id == conn_id && o.vervalt.is_none() {
                *o = Arc::new(Ontvang {
                    peer: o.peer,
                    sleutel: LessSafeKey::clone(&o.sleutel),
                    conn_id,
                    vervalt: Some(tot),
                    hoogste: o.hoogste.clone(),
                    adres: o.adres.clone(),
                });
            }
        }
        self.0.store(Arc::new(t));
    }

    /// Haalt ontvangstsleutels weg waarvan de nawerking voorbij is. Op de tik van de actor.
    pub(crate) fn ruim_op(&self) {
        let nu = Instant::now();
        let verlopen = |o: &Arc<Ontvang>| o.vervalt.is_some_and(|v| v <= nu);
        let oud = self.0.load();
        if !oud.ontvang.values().any(verlopen) {
            return;
        }
        let mut t = Tabel::clone(&oud);
        t.ontvang.retain(|_, o| !verlopen(o));
        self.0.store(Arc::new(t));
    }

    /// Twee kanten met verse, willekeurige sleutels aan elkaar koppelen, zonder QUIC. Alleen
    /// voor tests en voorbeelden die media over loopback sturen; mag dezelfde tabel twee keer
    /// zijn. Nooit op de tabel van een draaiende mesh: alleen de mesh-actor mag die schrijven,
    /// en dit leest-en-schrijft hem zonder te kijken of iemand anders tussendoor schreef.
    #[doc(hidden)]
    pub fn koppel(a: (&MediaSleutels, PeerId), b: (&MediaSleutels, PeerId)) {
        let willekeur = ring::rand::SystemRandom::new();
        let sleutel = || {
            let mut k = [0u8; 32];
            ring::rand::SecureRandom::fill(&willekeur, &mut k).expect("willekeur");
            k
        };
        let (ab, ba) = (sleutel(), sleutel());
        let (index_a, index_b) = (nieuwe_index(), nieuwe_index());
        // Van a naar b: a verzegelt met `ab` op de index van b, b opent met `ab`. Eerst
        // beide zendkanten, zodat elke ontvangstkant het adresvak van zijn verbinding vindt.
        a.0.zet_kant(b.1, ab, index_b);
        b.0.zet_kant(a.1, ba, index_a);
        b.0.zet_open(index_b, a.1, ab);
        a.0.zet_open(index_a, b.1, ba);
    }

    fn zet_kant(&self, naar: PeerId, sleutel: [u8; 32], index: u32) {
        let mut t = Tabel::clone(&self.0.load());
        t.zend.insert(
            naar,
            Arc::new(Zend {
                sleutel: aead(&sleutel),
                index,
                teller: AtomicU64::new(0),
                conn_id: 0,
                adres: Adresvak::default(),
            }),
        );
        self.0.store(Arc::new(t));
    }

    fn zet_open(&self, index: u32, van: PeerId, sleutel: [u8; 32]) {
        let mut t = Tabel::clone(&self.0.load());
        let adres = t
            .zend
            .get(&van)
            .map_or_else(Adresvak::default, |z| z.adres.clone());
        t.ontvang.insert(
            index,
            Arc::new(Ontvang {
                peer: van,
                sleutel: aead(&sleutel),
                conn_id: 0,
                vervalt: None,
                hoogste: Arc::default(),
                adres,
            }),
        );
        self.0.store(Arc::new(t));
    }

    /// Verzegelt `buf[MEDIA_KOP_LEN..MEDIA_KOP_LEN + lengte]` voor `naar`, op zijn plek, en
    /// levert de lengte van het hele pakket plus zijn geleerde adres (roaming), als dat er
    /// is. `None` als er geen sleutel voor die peer is.
    pub(crate) fn verzegel(
        &self,
        naar: PeerId,
        buf: &mut [u8],
        lengte: usize,
    ) -> Option<(usize, Option<SocketAddr>)> {
        let tabel = self.0.load();
        let z = tabel.zend.get(&naar)?;
        let totaal = MEDIA_KOP_LEN + lengte + MEDIA_TAG_LEN;
        if totaal > buf.len() {
            return None;
        }
        // Begint bij 1: een teller van 0 neemt de ontvanger nooit aan.
        let teller = z.teller.fetch_add(1, Ordering::Relaxed) + 1;
        buf[..4].copy_from_slice(&z.index.to_be_bytes());
        buf[4..MEDIA_KOP_LEN].copy_from_slice(&teller.to_be_bytes());
        let (kop, rest) = buf.split_at_mut(MEDIA_KOP_LEN);
        let tag = z
            .sleutel
            .seal_in_place_separate_tag(nonce(teller), Aad::from(&*kop), &mut rest[..lengte])
            .ok()?;
        rest[lengte..lengte + MEDIA_TAG_LEN].copy_from_slice(tag.as_ref());
        Some((totaal, z.adres.load().as_deref().copied()))
    }

    /// Opent een pakket op zijn plek: de afzender en de klare bytes (mediaheader plus
    /// payload), of `None` voor alles wat niet van een verbonden peer komt, vervalst is,
    /// herhaald wordt of te oud is. `bron` is waar het vandaan kwam; dat wordt het adres van
    /// die peer als hij één socket heeft (zie "Roaming" bovenaan).
    pub(crate) fn open<'a>(
        &self,
        vensters: &mut Vensters,
        bron: SocketAddr,
        pakket: &'a mut [u8],
    ) -> Option<(PeerId, &'a [u8])> {
        if pakket.len() < MEDIA_KOP_LEN + MEDIA_TAG_LEN {
            return None;
        }
        let index = u32::from_be_bytes(pakket[..4].try_into().ok()?);
        let teller = u64::from_be_bytes(pakket[4..MEDIA_KOP_LEN].try_into().ok()?);
        let tabel = self.0.load();
        // Een onbekende index is rommel of een verbinding die er niet meer is: één
        // tabelopzoeking, geen cryptografie.
        let o = tabel.ontvang.get(&index)?;
        // Ouder dan het venster onder het hoogste dat deze sleutel ooit liet zien, op welke
        // socket ook: weg, nog vóór de cryptografie.
        if teller.saturating_add(VENSTER) <= o.hoogste.load(Ordering::Relaxed) {
            return None;
        }
        let venster = vensters.voor(index, &tabel);
        if !venster.mag(teller) {
            return None;
        }
        #[cfg(test)]
        POGINGEN.with(|p| p.set(p.get() + 1));
        let (kop, rest) = pakket.split_at_mut(MEDIA_KOP_LEN);
        let klaar = o
            .sleutel
            .open_in_place(nonce(teller), Aad::from(&*kop), rest)
            .ok()?;
        // Pas markeren ná het ontsleutelen: anders schuift een vervalst pakket met een hoge
        // teller het venster op en valt al het echte verkeer erbuiten.
        venster.markeer(teller);
        o.hoogste.fetch_max(teller, Ordering::Relaxed);
        // Pas hier, ná de tag en het venster: een adres leren van een pakket dat niet echt
        // of niet nieuw is, zou iedereen het verkeer laten omleiden. Vergelijken vóór
        // schrijven, zodat het gewone pad alleen leest.
        let keepalive = klaar.get(PAYLOAD_TYPE_PLEK) == Some(&PayloadType::KEEPALIVE.0);
        let nu = o.adres.load();
        if (keepalive || nu.is_some()) && nu.as_deref() != Some(&bron) {
            o.adres.store(Some(Arc::new(bron)));
        }
        Some((o.peer, klaar))
    }
}

/// De replayvensters van één ontvangende socket, per sessie-index.
#[derive(Default)]
pub(crate) struct Vensters(HashMap<u32, Venster>);

impl Vensters {
    fn voor(&mut self, index: u32, tabel: &Tabel) -> &mut Venster {
        if !self.0.contains_key(&index) {
            // Alleen bij een nieuwe index, dus zelden: vergeet vensters van verbindingen die
            // niet meer in de tabel staan. Anders groeit dit met elke herverbinding.
            self.0.retain(|i, _| tabel.ontvang.contains_key(i));
        }
        self.0.entry(index).or_default()
    }
}

/// Welke tellers van één sleutel we al zagen, binnen [`VENSTER`] onder de hoogste.
#[derive(Default)]
pub(crate) struct Venster {
    hoogste: u64,
    gezien: [u64; (VENSTER / 64) as usize],
}

impl Venster {
    fn plek(teller: u64) -> (usize, u64) {
        let p = teller % VENSTER;
        ((p / 64) as usize, 1 << (p % 64))
    }

    fn mag(&self, teller: u64) -> bool {
        if teller == 0 {
            return false;
        }
        if teller > self.hoogste {
            return true;
        }
        if self.hoogste - teller >= VENSTER {
            return false;
        }
        let (woord, bit) = Self::plek(teller);
        self.gezien[woord] & bit == 0
    }

    fn markeer(&mut self, teller: u64) {
        if teller > self.hoogste {
            if teller - self.hoogste >= VENSTER {
                self.gezien = [0; (VENSTER / 64) as usize];
            } else {
                // De plekken die nu vrijkomen hoorden bij tellers die uit het venster vallen.
                for t in self.hoogste + 1..teller {
                    let (woord, bit) = Self::plek(t);
                    self.gezien[woord] &= !bit;
                }
            }
            self.hoogste = teller;
        }
        let (woord, bit) = Self::plek(teller);
        self.gezien[woord] |= bit;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BRON: SocketAddr = SocketAddr::new(
        std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1)),
        9,
    );

    fn paar() -> (MediaSleutels, PeerId, PeerId) {
        let t = MediaSleutels::default();
        let (a, b) = (PeerId::new_random(), PeerId::new_random());
        MediaSleutels::koppel((&t, a), (&t, b));
        (t, a, b)
    }

    /// Verzegelt `inhoud` van wie `b` heet voor `naar`.
    fn pakket(t: &MediaSleutels, naar: PeerId, inhoud: &[u8]) -> Vec<u8> {
        let mut buf = vec![0u8; 1500];
        buf[MEDIA_KOP_LEN..MEDIA_KOP_LEN + inhoud.len()].copy_from_slice(inhoud);
        let (n, _) = t.verzegel(naar, &mut buf, inhoud.len()).expect("sleutel");
        buf.truncate(n);
        buf
    }

    fn pogingen() -> u64 {
        POGINGEN.with(|p| p.get())
    }

    #[test]
    fn heen_en_weer() {
        let (t, a, b) = paar();
        let mut v = Vensters::default();
        let mut p = pakket(&t, b, b"stem van a");
        assert_eq!(p.len(), MEDIA_KOP_LEN + 10 + MEDIA_TAG_LEN);
        let (van, klaar) = t.open(&mut v, BRON, &mut p).expect("hoort open te gaan");
        assert_eq!((van, klaar), (a, &b"stem van a"[..]));

        let mut terug = pakket(&t, a, b"en van b");
        assert_eq!(t.open(&mut v, BRON, &mut terug).map(|(p, _)| p), Some(b));
    }

    #[test]
    fn een_vervalste_tag_of_inhoud_gaat_niet_open() {
        let (t, _, b) = paar();
        let mut v = Vensters::default();
        let mut p = pakket(&t, b, b"echt");
        let laatste = p.len() - 1;
        p[laatste] ^= 1;
        assert!(t.open(&mut v, BRON, &mut p).is_none());

        let mut p = pakket(&t, b, b"echt");
        p[MEDIA_KOP_LEN] ^= 1;
        assert!(t.open(&mut v, BRON, &mut p).is_none());

        // En de teller zit in de AAD: ophogen om het venster te omzeilen helpt niet.
        let mut p = pakket(&t, b, b"echt");
        p[11] ^= 1;
        assert!(t.open(&mut v, BRON, &mut p).is_none());
    }

    #[test]
    fn een_herhaald_pakket_gaat_een_tweede_keer_niet_open() {
        let (t, _, b) = paar();
        let mut v = Vensters::default();
        let p = pakket(&t, b, b"een keer");
        assert!(t.open(&mut v, BRON, &mut p.clone()).is_some());
        assert!(t.open(&mut v, BRON, &mut p.clone()).is_none(), "replay");

        // Een vervalsing met dezelfde teller mag het echte pakket niet blokkeren.
        let echt = pakket(&t, b, b"daarna");
        let mut nep = echt.clone();
        let laatste = nep.len() - 1;
        nep[laatste] ^= 1;
        assert!(t.open(&mut v, BRON, &mut nep).is_none());
        assert!(t.open(&mut v, BRON, &mut echt.clone()).is_some());
    }

    #[test]
    fn herordening_binnen_het_venster_mag_daarbuiten_niet() {
        let (t, _, b) = paar();
        let mut v = Vensters::default();
        let eerste = pakket(&t, b, b"oud");
        let mut tussen: Vec<Vec<u8>> = (0..VENSTER - 2).map(|_| pakket(&t, b, b"x")).collect();
        let laat = tussen.pop().unwrap();
        for mut p in tussen {
            assert!(t.open(&mut v, BRON, &mut p).is_some());
        }
        // Verwisseld onderweg, maar nog binnen 2048: welkom.
        assert!(t.open(&mut v, BRON, &mut laat.clone()).is_some());
        assert!(t.open(&mut v, BRON, &mut eerste.clone()).is_some());

        // Nu ver vooruit: het eerste pakket valt buiten het venster, ook ongezien.
        let oud = pakket(&t, b, b"te oud");
        for _ in 0..VENSTER {
            assert!(t.open(&mut v, BRON, &mut pakket(&t, b, b"y")).is_some());
        }
        assert!(
            t.open(&mut v, BRON, &mut oud.clone()).is_none(),
            "buiten het venster"
        );
    }

    #[test]
    fn een_onbekende_index_kost_geen_cryptografie() {
        let (t, _, b) = paar();
        let mut v = Vensters::default();
        let mut p = pakket(&t, b, b"inhoud");
        p[..4].copy_from_slice(&0xdead_beefu32.to_be_bytes());
        let voor = pogingen();
        assert!(t.open(&mut v, BRON, &mut p).is_none());
        assert_eq!(
            pogingen(),
            voor,
            "onbekende index mag niet ontsleuteld worden"
        );

        // Te kort om iets te zijn: ook niets.
        assert!(t.open(&mut v, BRON, &mut [0u8; 20]).is_none());
        assert_eq!(pogingen(), voor);
    }

    #[test]
    fn een_vergeten_verbinding_ontvangt_nog_even_en_verstuurt_niet_meer() {
        let t = MediaSleutels::default();
        let (ik, hij) = (PeerId::new_random(), PeerId::new_random());
        let (mijn, zijn) = (nieuwe_index(), nieuwe_index());
        t.zet(7, hij, [1; 32], zijn, [2; 32], mijn, BRON);
        // Wat hij ons stuurt: zijn zendsleutel is onze ontvangstsleutel.
        let zijn_kant = MediaSleutels::default();
        zijn_kant.zet(7, ik, [2; 32], mijn, [1; 32], zijn, BRON);

        let mut v = Vensters::default();
        assert!(t
            .open(&mut v, BRON, &mut pakket(&zijn_kant, ik, b"a"))
            .is_some());

        t.vergeet(7);
        assert!(
            t.verzegel(hij, &mut [0u8; 64], 4).is_none(),
            "niet meer naar een dode verbinding"
        );
        assert!(
            t.open(&mut v, BRON, &mut pakket(&zijn_kant, ik, b"b"))
                .is_some(),
            "nawerking"
        );
        t.ruim_op();
        assert!(
            t.open(&mut v, BRON, &mut pakket(&zijn_kant, ik, b"c"))
                .is_some(),
            "nog binnen 5 s"
        );
    }

    #[test]
    fn dezelfde_verbinding_twee_keer_zetten_begint_de_teller_niet_opnieuw() {
        let t = MediaSleutels::default();
        let hij = PeerId::new_random();
        t.zet(3, hij, [1; 32], 10, [2; 32], 11, BRON);
        let eerste = pakket(&t, hij, b"a");
        t.zet(3, hij, [1; 32], 10, [2; 32], 11, BRON);
        let tweede = pakket(&t, hij, b"a");
        assert_ne!(
            eerste[4..12],
            tweede[4..12],
            "dezelfde nonce onder dezelfde sleutel"
        );
    }

    #[test]
    fn een_nieuwe_socket_neemt_geen_oude_pakketten_aan() {
        // Opnieuw deelnemen aan het gesprek of opnieuw kijken geeft een verse socket, met een
        // leeg venster, op dezelfde verbinding en dus dezelfde sleutel. Wat eerder op die
        // verbinding opgevangen is mag daar niet nog eens binnenkomen.
        let (t, _, b) = paar();
        let oud = pakket(&t, b, b"oud");
        let mut eerste_socket = Vensters::default();
        assert!(t.open(&mut eerste_socket, BRON, &mut oud.clone()).is_some());
        for _ in 0..VENSTER {
            assert!(t
                .open(&mut eerste_socket, BRON, &mut pakket(&t, b, b"x"))
                .is_some());
        }

        let mut nieuwe_socket = Vensters::default();
        let voor = pogingen();
        assert!(t.open(&mut nieuwe_socket, BRON, &mut oud.clone()).is_none());
        assert_eq!(pogingen(), voor, "te oud: weg vóór de cryptografie");
        // Wat nieuw is komt op de nieuwe socket gewoon binnen.
        assert!(t
            .open(&mut nieuwe_socket, BRON, &mut pakket(&t, b, b"nieuw"))
            .is_some());
    }

    /// Het venster tegen een triviaal juist model (een verzameling van alles wat gezien is),
    /// met sprongen precies rond de rand van het venster, terugkijkers en teller 0.
    #[test]
    fn venster_komt_overeen_met_een_referentiemodel() {
        use std::collections::HashSet;
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut rnd = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for ronde in 0..20 {
            let mut v = Venster::default();
            let mut gezien: HashSet<u64> = HashSet::new();
            let mut h: u64 = 0;
            for _ in 0..20_000 {
                let t = match rnd() % 6 {
                    0 => h + 1 + (rnd() % 3),
                    1 => h.saturating_sub(rnd() % 2100),
                    2 => h + (rnd() % 4200),
                    3 => h + VENSTER - 2 + (rnd() % 5),
                    4 => h.saturating_sub(VENSTER - 3 + (rnd() % 6)),
                    _ => rnd() % (h + 5),
                };
                let verwacht = t > 0 && !gezien.contains(&t) && (t > h || h - t < VENSTER);
                assert_eq!(v.mag(t), verwacht, "ronde {ronde} t={t} h={h}");
                if verwacht {
                    v.markeer(t);
                    gezien.insert(t);
                    h = h.max(t);
                }
            }
        }
    }

    /// Een mediaheader van 16 bytes met dit payload-type, als inhoud van een pakket.
    fn kop(payload_type: PayloadType) -> [u8; 16] {
        let mut k = [0u8; 16];
        k[PAYLOAD_TYPE_PLEK] = payload_type.0;
        k
    }

    #[test]
    fn een_adres_wordt_pas_geleerd_na_een_keepalive_en_alleen_van_echte_pakketten() {
        let (t, a, b) = paar();
        let mut v = Vensters::default();
        let adres = |p: u16| SocketAddr::from(([198, 51, 100, 7], p));

        // Een peer zonder B5 stuurt nooit een keepalive: van hem leren we niets, ook niet van
        // zijn stem. Zijn beeld komt van een andere poort dan zijn stem.
        let mut stem = pakket(&t, b, &kop(PayloadType::OPUS));
        assert!(t.open(&mut v, adres(1), &mut stem).is_some());
        assert_eq!(t.geleerd(a), None);

        let mut ka = pakket(&t, b, &kop(PayloadType::KEEPALIVE));
        assert!(t.open(&mut v, adres(2), &mut ka).is_some());
        assert_eq!(t.geleerd(a), Some(adres(2)), "na een keepalive: één socket");

        // Daarna volgt het adres elk echt pakket (de NAT herschreef de poort).
        let mut beeld = pakket(&t, b, &kop(PayloadType::H264));
        assert!(t.open(&mut v, adres(3), &mut beeld.clone()).is_some());
        assert_eq!(t.geleerd(a), Some(adres(3)));
        let (_, naar_a) = t.verzegel(a, &mut [0u8; 64], 4).unwrap();
        assert_eq!(
            naar_a,
            Some(adres(3)),
            "antwoorden gaan naar het geleerde adres"
        );

        // Een herhaling of een vervalsing vanaf elders verplaatst niets.
        assert!(t.open(&mut v, adres(4), &mut beeld).is_none());
        let mut nep = pakket(&t, b, &kop(PayloadType::H264));
        let laatste = nep.len() - 1;
        nep[laatste] ^= 1;
        assert!(t.open(&mut v, adres(5), &mut nep).is_none());
        assert_eq!(t.geleerd(a), Some(adres(3)));
    }

    #[test]
    fn een_nieuwe_verbinding_begint_opnieuw_met_leren() {
        let t = MediaSleutels::default();
        let (ik, hij) = (PeerId::new_random(), PeerId::new_random());
        let hint = SocketAddr::from(([198, 51, 100, 7], 41651));
        t.zet(1, hij, [1; 32], 10, [2; 32], 11, hint);
        let zijn_kant = MediaSleutels::default();
        zijn_kant.zet(1, ik, [2; 32], 11, [1; 32], 10, hint);

        let mut v = Vensters::default();
        let ergens = SocketAddr::from(([203, 0, 113, 1], 5000));
        let mut ka = pakket(&zijn_kant, ik, &kop(PayloadType::KEEPALIVE));
        assert!(t.open(&mut v, ergens, &mut ka).is_some());
        assert_eq!(t.keepalive_doelen(), vec![(hij, vec![ergens])]);

        // Opnieuw verbonden (zijn IP kan intussen anders zijn): terug naar de hint.
        t.zet(2, hij, [3; 32], 12, [4; 32], 13, hint);
        assert_eq!(t.geleerd(hij), None);
        assert_eq!(t.keepalive_doelen(), vec![(hij, vec![hint])]);
        t.vergeet(2);
        assert!(t.keepalive_doelen().is_empty());
    }

    #[test]
    fn indices_zijn_uniek_en_nooit_nul() {
        let mut gezien = std::collections::HashSet::new();
        for _ in 0..1000 {
            let i = nieuwe_index();
            assert_ne!(i, 0);
            assert!(gezien.insert(i));
        }
    }
}
