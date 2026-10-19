//! Port mapping zolang de app draait (fase B2 van `docs/PLAN-ZONDER-TAILSCALE.md`).
//!
//! Twee `portmapper`-clients, één voor `control_port` (QUIC) en één voor `media_port` (de ene
//! mediasocket van [`crate::verdeler`]). De router zet de poorten zelf open met PCP, NAT-PMP
//! of UPnP en vertelt welk adres hij naar buiten toont; dat adres komt in de eigen
//! uitnodigingscode en in de adressen die peers uitwisselen (B4). Er praat hier niemand anders
//! dan de eigen router en, voor UPnP, het LAN (invariant 1). De meting van A0 zit erin: de
//! regel in Settings → Network komt nu van hier en is live.
//!
//! **Een eigen thread met een eigen kleine runtime**, zoals bij A0 (`bereik.rs`): portmapper
//! zoekt de gateway synchroon op (op Windows `SendARP` per netwerkkaart), en dat hoort niet op
//! de threads van de mesh en de motor, en zeker niet op een mediathread. Wat er draait is
//! gebeurtenisgestuurd: een lease wordt halverwege vernieuwd en verder slaapt alles, behalve
//! één tik per minuut die een mislukte mapping opnieuw vraagt (geen netwerk bij het opstarten,
//! een router die even weg was) en eens per vijf minuten opnieuw probet (portmapper vertrouwt
//! een probe tien minuten, en zonder verse probe kiest hij bij het vernieuwen van een lease geen
//! protocol meer). Spreekt de router PCP of NAT-PMP, dan staat UPnP uit. Zie de meting in
//! `docs/OVERDRACHT.md` beslissing 47.
//!
//! **Opruimen bij netjes afsluiten:** `deactivate()`, en dan een tweede probe als wachtpost
//! (de dienst handelt berichten op volgorde af; zie `bereik.rs`). Wie de app hard afsluit laat
//! de mapping staan tot zijn lease verloopt.

use crate::bereik::{self, Bereik};
use std::net::SocketAddrV4;
use std::num::NonZeroU16;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

/// Niet tijdens het opstarten zelf, net als A0.
const START_NA: Duration = Duration::from_secs(10);
/// Probe: UPnP zoekt 1 s, PCP en NAT-PMP wachten elk 500 ms. Ruim daarboven.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// Een mislukte of verlopen mapping opnieuw vragen. Zolang er een mapping is doet dit niets:
/// portmapper vraagt alleen als hij er geen heeft.
const OPNIEUW_ELKE: Duration = Duration::from_secs(60);
/// Om de zoveel tikken opnieuw proben als de router PCP, NAT-PMP of UPnP spreekt: binnen de
/// tien minuten dat portmapper een probe vertrouwt, anders kiest hij bij het vernieuwen van de
/// lease geen protocol meer.
const PROBE_ELKE_MET: u32 = 5;
/// En als hij niets spreekt: zelden, want elke probe zoekt dan ook een seconde naar UPnP.
const PROBE_ELKE_ZONDER: u32 = 10;
/// Hoe lang het vrijgeven de tijd krijgt voordat de thread toch stopt.
const OPRUIM_TIMEOUT: Duration = Duration::from_secs(2);

/// Wat de router nu over ons zegt.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Mapping {
    /// De probe van A0, maar live: welke protocollen, welk extern IPv4, IPv6 of niet.
    pub bereik: Bereik,
    /// Waar de control-poort en de mediapoort van buiten te bereiken zijn, als de router ze
    /// opende.
    pub control: Option<SocketAddrV4>,
    pub media: Option<SocketAddrV4>,
}

/// Het handvat op de mapper. Stoppen gebeurt bij drop van de mesh of met [`Mapper::sluit`].
pub(crate) struct Mapper {
    stop: watch::Sender<bool>,
    klaar: watch::Receiver<bool>,
}

impl Mapper {
    /// Start de mapper op een eigen thread. `meld` krijgt elke wijziging; hij draait op die
    /// thread en mag dus niet blokkeren.
    pub(crate) fn start(
        control: u16,
        media: u16,
        meld: impl Fn(Mapping) + Send + 'static,
    ) -> Arc<Mapper> {
        let (stop, stop_rx) = watch::channel(false);
        let (klaar_tx, klaar) = watch::channel(false);
        let gestart = std::thread::Builder::new()
            .name("fitcom-poorten".into())
            .spawn(move || {
                match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt.block_on(draai(control, media, stop_rx, meld)),
                    Err(e) => tracing::warn!(error = %e, "port mapping kon niet starten"),
                }
                let _ = klaar_tx.send(true);
            });
        if let Err(e) = gestart {
            tracing::warn!(error = %e, "port mapping kon niet starten");
        }
        Arc::new(Mapper { stop, klaar })
    }

    /// Geeft de mappings vrij en wacht daar hoogstens `max` op.
    pub(crate) async fn sluit(&self, max: Duration) {
        let _ = self.stop.send(true);
        let mut klaar = self.klaar.clone();
        let _ = tokio::time::timeout(max, klaar.wait_for(|k| *k)).await;
    }

    pub(crate) fn stop(&self) {
        let _ = self.stop.send(true);
    }
}

async fn probe(client: &portmapper::Client) -> Option<portmapper::ProbeOutput> {
    match tokio::time::timeout(PROBE_TIMEOUT, client.probe()).await {
        Ok(Ok(Ok(p))) => Some(p),
        _ => None,
    }
}

async fn draai(control: u16, media: u16, mut stop: watch::Receiver<bool>, meld: impl Fn(Mapping)) {
    tokio::select! {
        _ = stop.wait_for(|s| *s) => return,
        _ = tokio::time::sleep(START_NA) => {}
    }
    let mut stand = Mapping::default();
    let peil = |p: Option<portmapper::ProbeOutput>, stand: &mut Mapping| {
        let (gateway, upnp, pcp, nat_pmp) = match p {
            Some(p) => (true, p.upnp, p.pcp, p.nat_pmp),
            None => (false, false, false, false),
        };
        stand.bereik = Bereik {
            gateway,
            upnp,
            pcp,
            nat_pmp,
            extern_v4: stand.control.map(|a| *a.ip()),
            ipv6: bereik::heeft_globaal_ipv6(),
            gemapt: stand.control,
        };
    };

    // Eerst proben, dan pas mappen: zonder probe begint portmapper met UPnP, en op een router
    // die alleen PCP en NAT-PMP doet (die van Rick) mislukt die eerste poging dan. Een stop
    // tijdens de probe: er is dan nog niets gemapt, dus ook niets vrij te geven.
    let verken = portmapper::Client::new(portmapper::Config::default());
    tokio::select! {
        _ = stop.wait_for(|s| *s) => return,
        p = probe(&verken) => peil(p, &mut stand),
    }
    drop(verken);
    // Spreekt de router PCP of NAT-PMP, dan UPnP uit. portmapper vertrouwt een probe maar
    // tien minuten, en kiest bij het vernieuwen van een lease opnieuw; zonder dit viel hij na
    // een uur terug op UPnP, en op een router zonder UPnP was de mapping dan weg (gevonden door
    // de reviewer).
    let config = portmapper::Config {
        enable_upnp: !(stand.bereik.pcp || stand.bereik.nat_pmp),
        ..portmapper::Config::default()
    };
    let upnp_gezien = stand.bereik.upnp;
    let upnp_aan = config.enable_upnp;
    let c = portmapper::Client::new(config.clone());
    let m = portmapper::Client::new(config);
    let _ = tokio::join!(probe(&c), probe(&m));
    tracing::info!(bereik = %stand.bereik, "port mapping gestart");
    meld(stand.clone());
    for (client, poort) in [(&c, control), (&m, media)] {
        if let Some(p) = NonZeroU16::new(poort) {
            client.update_local_port(p);
        }
    }

    let (mut wc, mut wm) = (c.watch_external_address(), m.watch_external_address());
    let mut opnieuw = tokio::time::interval(OPNIEUW_ELKE);
    opnieuw.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    opnieuw.tick().await;
    let mut tikken: u32 = 0;
    loop {
        tokio::select! {
            _ = stop.wait_for(|s| *s) => break,
            r = wc.changed() => {
                if r.is_err() { break }
                stand.control = *wc.borrow_and_update();
                stand.bereik.extern_v4 = stand.control.map(|a| *a.ip());
                stand.bereik.gemapt = stand.control;
                tracing::info!(extern = ?stand.control, "control-poort gemapt");
                meld(stand.clone());
            }
            r = wm.changed() => {
                if r.is_err() { break }
                stand.media = *wm.borrow_and_update();
                tracing::info!(extern = ?stand.media, "mediapoort gemapt");
                meld(stand.clone());
            }
            _ = opnieuw.tick() => {
                tikken = tikken.wrapping_add(1);
                let protocol = stand.bereik.pcp || stand.bereik.nat_pmp || stand.bereik.upnp;
                // Opnieuw proben: zonder router elke minuut (autostart vóór de wifi); met een
                // router die iets spreekt binnen de tien minuten dat portmapper een probe
                // vertrouwt; met een router die niets spreekt maar eens per tien minuten.
                let proben = !stand.bereik.gateway
                    || (protocol && tikken.is_multiple_of(PROBE_ELKE_MET))
                    || (!protocol && tikken.is_multiple_of(PROBE_ELKE_ZONDER));
                if proben {
                    let (pc, _) = tokio::join!(probe(&c), probe(&m));
                    peil(pc, &mut stand);
                    if !upnp_aan {
                        // Niet meer gevraagd; de regel in Settings houdt wat de eerste probe zag.
                        stand.bereik.upnp = upnp_gezien;
                    }
                    // Ook ongewijzigd opnieuw melden: een melding die ooit niet aankwam (een
                    // volle rij) staat zo binnen een paar minuten toch goed.
                    meld(stand.clone());
                }
                // Zolang er een mapping is doet dit niets; zonder protocol zou het alleen
                // UPnP zoeken, en dat hoort dan bij het proben.
                if protocol || proben {
                    c.procure_mapping();
                    m.procure_mapping();
                }
            }
        }
    }

    c.deactivate();
    m.deactivate();
    let _ = tokio::time::timeout(OPRUIM_TIMEOUT, async {
        let _ = tokio::join!(c.probe(), m.probe());
    })
    .await;
    tracing::info!("port mapping vrijgegeven");
}
