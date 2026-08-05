//! Wie deelt wat, en wie kijkt waarnaar.
//!
//! Alle beslissingen over screenshare staan hier, en niets in deze module raakt een
//! GPU, een scherm of een socket aan. Wat er moet gebeuren komt eruit als [`Actie`];
//! de motor voert die uit. Daardoor is het gedrag te testen zonder hardware — precies
//! zoals de chat-logica in `chat.rs`.
//!
//! # De regel die alles bij elkaar houdt
//!
//! **Er wordt pas opgenomen en gecodeerd als er iemand kijkt.** Een aangekondigde bron
//! kost niets: geen capture, geen encoder, geen verkeer. Pas bij de eerste
//! `StreamSubscribe` begint het werk, en bij de laatste `StreamUnsubscribe` stopt het
//! weer. Dat is de reden dat je een scherm gedeeld kunt laten staan terwijl je gamet.
//!
//! # Waarom stream-id's alleen binnen één peer uniek zijn
//!
//! De deler kent ze uit: 1, 2, 3. Twee peers delen dus allebei een stream 1. Dat mag,
//! want een stream wordt overal geïdentificeerd door het paar (eigenaar, id) — en op
//! de draad door de UDP-poort waarop de kijker luistert, die per stream apart is.

use fitcom_net::MeshCommand;
use fitcom_proto::control::{
    RequestKeyframe, StreamAnnounce, StreamKind, StreamRevoke, StreamSubscribe, StreamUnsubscribe,
};
use fitcom_proto::{ControlMsg, PeerId, VOICE_STREAM_ID};
use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};

/// Wat de motor moet doen. Alles wat hardware aanraakt staat hier als opdracht en
/// wordt buiten deze module uitgevoerd.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actie {
    /// Begin met opnemen en coderen: er kijkt nu iemand.
    StartDelen {
        stream_id: u32,
        kijkers: Vec<SocketAddr>,
    },
    /// Dezelfde stream, andere kijkers.
    ZetKijkers {
        stream_id: u32,
        kijkers: Vec<SocketAddr>,
    },
    /// Stop met opnemen en coderen.
    StopDelen { stream_id: u32, is_geluid: bool },
    /// Volgend beeld als keyframe versturen.
    StuurKeyframe { stream_id: u32 },
    /// Open een venster en ga luisteren.
    StartKijken {
        eigenaar: PeerId,
        stream_id: u32,
        titel: String,
        breedte: u32,
        hoogte: u32,
        /// Geluid in plaats van beeld: geen venster, geen eigen poort — dat komt binnen
        /// op de voice-poort en gaat de bestaande mixer in.
        is_geluid: bool,
    },
    /// Sluit het venster.
    StopKijken { eigenaar: PeerId, stream_id: u32 },
}

/// Een bron die wij delen.
#[derive(Debug, Clone)]
pub struct EigenStream {
    pub id: u32,
    pub kind: StreamKind,
    pub titel: String,
    pub breedte: u32,
    pub hoogte: u32,
    /// Wie er kijkt en waar zijn beeld heen moet.
    pub kijkers: BTreeMap<PeerId, SocketAddr>,
}

impl EigenStream {
    pub fn wordt_bekeken(&self) -> bool {
        !self.kijkers.is_empty()
    }

    fn doelen(&self) -> Vec<SocketAddr> {
        self.kijkers.values().copied().collect()
    }

    fn aankondiging(&self) -> ControlMsg {
        ControlMsg::StreamAnnounce(StreamAnnounce {
            stream_id: self.id,
            kind: self.kind,
            title: self.titel.clone(),
            width: self.breedte,
            height: self.hoogte,
        })
    }
}

/// Een bron die een ander deelt.
#[derive(Debug, Clone)]
pub struct VreemdeStream {
    pub eigenaar: PeerId,
    pub id: u32,
    pub kind: StreamKind,
    pub titel: String,
    pub breedte: u32,
    pub hoogte: u32,
    /// Kijken wij ernaar?
    pub kijken: bool,
}

#[derive(Debug, Default)]
pub struct Streams {
    /// Loopt alleen op. Een id hergebruiken terwijl er nog pakketten van de vorige
    /// stream onderweg zijn levert beeld op in het verkeerde venster.
    volgende_id: u32,
    eigen: Vec<EigenStream>,
    vreemd: Vec<VreemdeStream>,
}

impl Streams {
    pub fn new() -> Self {
        Self {
            volgende_id: 1,
            ..Default::default()
        }
    }

    pub fn eigen(&self) -> &[EigenStream] {
        &self.eigen
    }

    pub fn vreemd(&self) -> &[VreemdeStream] {
        &self.vreemd
    }

    // -- delen -------------------------------------------------------------

    /// Kondigt een bron aan. Levert het toegekende id op plus de melding aan de
    /// anderen. Er wordt nog niets opgenomen: dat begint pas als iemand intekent.
    pub fn deel(
        &mut self,
        kind: StreamKind,
        titel: String,
        breedte: u32,
        hoogte: u32,
    ) -> (u32, Vec<MeshCommand>) {
        let id = self.volgende_id.max(1);
        self.volgende_id = id + 1;

        let stream = EigenStream {
            id,
            kind,
            titel,
            breedte,
            hoogte,
            kijkers: BTreeMap::new(),
        };
        let melding = stream.aankondiging();
        self.eigen.push(stream);
        (id, vec![MeshCommand::Broadcast(melding)])
    }

    pub fn stop_delen(&mut self, stream_id: u32) -> (Vec<MeshCommand>, Vec<Actie>) {
        let Some(plek) = self.eigen.iter().position(|s| s.id == stream_id) else {
            return (Vec::new(), Vec::new());
        };
        let stream = self.eigen.remove(plek);

        let mut acties = Vec::new();
        if stream.wordt_bekeken() {
            acties.push(Actie::StopDelen {
                stream_id,
                is_geluid: stream.kind == StreamKind::DESKTOP_AUDIO,
            });
        }
        (
            vec![MeshCommand::Broadcast(ControlMsg::StreamRevoke(
                StreamRevoke { stream_id },
            ))],
            acties,
        )
    }

    // -- kijken ------------------------------------------------------------

    /// De gebruiker wil naar een stream van een ander kijken. Het venster moet er zijn
    /// vóór we intekenen, want in het abonnement staat de poort waarop het luistert.
    pub fn wil_kijken(&mut self, eigenaar: PeerId, stream_id: u32) -> Vec<Actie> {
        let Some(s) = self.vind_vreemd(eigenaar, stream_id) else {
            return Vec::new();
        };
        if s.kijken {
            return Vec::new();
        }
        s.kijken = true;
        vec![Actie::StartKijken {
            eigenaar,
            stream_id,
            titel: s.titel.clone(),
            breedte: s.breedte,
            hoogte: s.hoogte,
            is_geluid: s.kind == StreamKind::DESKTOP_AUDIO,
        }]
    }

    /// Bureaubladgeluid volgt het beeld: kijken naar het scherm van een peer betekent
    /// zijn geluid horen, en het laatste venster van die peer sluiten zet het weer uit.
    ///
    /// Deze regel stond tot fase 12 in de egui-UI (`stuur_kijk_toggle`) en verdween bij
    /// de wissel naar Tauri, waardoor niemand nog op een `DESKTOP_AUDIO`-stream intekende
    /// en er dus geen geluid meer bij een gedeeld scherm kwam. Hier hoort hij: nu geldt
    /// hij ook voor een aankondiging die later binnenkomt, een venster dat zichzelf sluit
    /// en een peer die wegvalt.
    ///
    /// `in_gesprek` is `false` zonder voice-sessie — dan is er geen socket om het op te
    /// ontvangen. Idempotent: tweemaal aanroepen levert de tweede keer niets op.
    pub fn stem_geluid_af_op_beeld(&mut self, in_gesprek: bool) -> (Vec<MeshCommand>, Vec<Actie>) {
        let met_beeld: Vec<PeerId> = self
            .vreemd
            .iter()
            .filter(|s| s.kijken && s.kind != StreamKind::DESKTOP_AUDIO)
            .map(|s| s.eigenaar)
            .collect();

        let wissels: Vec<(PeerId, u32, bool)> = self
            .vreemd
            .iter()
            .filter(|s| s.kind == StreamKind::DESKTOP_AUDIO)
            .filter_map(|s| {
                let hoort = in_gesprek && met_beeld.contains(&s.eigenaar);
                (hoort != s.kijken).then_some((s.eigenaar, s.id, hoort))
            })
            .collect();

        let mut cmds = Vec::new();
        let mut acties = Vec::new();
        for (eigenaar, id, aan) in wissels {
            if aan {
                acties.extend(self.wil_kijken(eigenaar, id));
            } else {
                let (c, a) = self.stop_kijken(eigenaar, id);
                cmds.extend(c);
                acties.extend(a);
            }
        }
        (cmds, acties)
    }

    /// Het venster staat en luistert op `poort`; nu pas mag de deler beginnen.
    pub fn kijker_draait(&self, eigenaar: PeerId, stream_id: u32, poort: u16) -> Vec<MeshCommand> {
        vec![MeshCommand::Send {
            to: eigenaar,
            msg: ControlMsg::StreamSubscribe(StreamSubscribe {
                stream_id,
                media_port: poort,
            }),
        }]
    }

    pub fn stop_kijken(
        &mut self,
        eigenaar: PeerId,
        stream_id: u32,
    ) -> (Vec<MeshCommand>, Vec<Actie>) {
        let Some(s) = self.vind_vreemd(eigenaar, stream_id) else {
            return (Vec::new(), Vec::new());
        };
        if !s.kijken {
            return (Vec::new(), Vec::new());
        }
        s.kijken = false;
        (
            vec![MeshCommand::Send {
                to: eigenaar,
                msg: ControlMsg::StreamUnsubscribe(StreamUnsubscribe { stream_id }),
            }],
            vec![Actie::StopKijken {
                eigenaar,
                stream_id,
            }],
        )
    }

    /// De kijker is de draad kwijt en heeft een nieuw keyframe nodig.
    pub fn vraag_keyframe(&self, eigenaar: PeerId, stream_id: u32) -> Vec<MeshCommand> {
        vec![MeshCommand::Send {
            to: eigenaar,
            msg: ControlMsg::RequestKeyframe(RequestKeyframe { stream_id }),
        }]
    }

    // -- gebeurtenissen van de mesh ---------------------------------------

    /// Een peer is (opnieuw) verbonden. Hij heeft onze eerdere aankondigingen gemist,
    /// dus die sturen we alsnog — anders ziet hij nooit dat we iets delen.
    pub fn bij_verbinding(&self, peer: PeerId) -> Vec<MeshCommand> {
        self.eigen
            .iter()
            .map(|s| MeshCommand::Send {
                to: peer,
                msg: s.aankondiging(),
            })
            .collect()
    }

    /// Een peer is weg. Wat hij deelde bestaat voor ons niet meer, en waar hij naar
    /// keek hoeven we niet meer te versturen.
    pub fn bij_verbreking(&mut self, peer: PeerId) -> Vec<Actie> {
        let mut acties = Vec::new();

        for s in self
            .vreemd
            .iter()
            .filter(|s| s.eigenaar == peer && s.kijken)
        {
            acties.push(Actie::StopKijken {
                eigenaar: peer,
                stream_id: s.id,
            });
        }
        self.vreemd.retain(|s| s.eigenaar != peer);

        for s in &mut self.eigen {
            if s.kijkers.remove(&peer).is_some() {
                acties.push(if s.wordt_bekeken() {
                    Actie::ZetKijkers {
                        stream_id: s.id,
                        kijkers: s.doelen(),
                    }
                } else {
                    Actie::StopDelen {
                        stream_id: s.id,
                        is_geluid: s.kind == StreamKind::DESKTOP_AUDIO,
                    }
                });
            }
        }

        acties
    }

    /// Een control-bericht dat over screenshare gaat. Alles wat er niet over gaat komt
    /// er ongemoeid weer uit als een leeg antwoord.
    pub fn bij_bericht(
        &mut self,
        van: PeerId,
        van_ip: IpAddr,
        msg: &ControlMsg,
    ) -> (Vec<MeshCommand>, Vec<Actie>) {
        match msg {
            ControlMsg::StreamAnnounce(a) => (self.aangekondigd(van, a), Vec::new()),
            ControlMsg::StreamRevoke(r) => (Vec::new(), self.ingetrokken(van, r.stream_id)),
            ControlMsg::StreamSubscribe(s) => (
                Vec::new(),
                self.ingetekend(van, SocketAddr::new(van_ip, s.media_port), s.stream_id),
            ),
            ControlMsg::StreamUnsubscribe(u) => (Vec::new(), self.uitgetekend(van, u.stream_id)),
            ControlMsg::RequestKeyframe(k) => {
                tracing::debug!(peer = ?van, stream = k.stream_id, "keyframe gevraagd door kijker");
                (Vec::new(), self.keyframe_gevraagd(van, k.stream_id))
            }
            _ => (Vec::new(), Vec::new()),
        }
    }

    fn aangekondigd(&mut self, van: PeerId, a: &StreamAnnounce) -> Vec<MeshCommand> {
        if a.stream_id == VOICE_STREAM_ID {
            tracing::warn!(peer = ?van, "aankondiging op de voice-stream genegeerd");
            return Vec::new();
        }
        if !a.kind.is_known() {
            // Van een nieuwere peer. We weten niet wat het is, dus we kunnen er niets
            // zinnigs mee; loggen en verder gaan, niet als fout behandelen.
            tracing::info!(peer = ?van, kind = ?a.kind, "onbekende soort stream genegeerd");
            return Vec::new();
        }

        // Opnieuw aankondigen is normaal: het gebeurt bij elke herverbinding. Alleen
        // de gegevens bijwerken, de kijkstatus laten staan.
        if let Some(s) = self.vind_vreemd(van, a.stream_id) {
            s.kind = a.kind;
            s.titel = a.title.clone();
            s.breedte = a.width;
            s.hoogte = a.height;
            return Vec::new();
        }

        tracing::info!(peer = ?van, stream = a.stream_id, titel = %a.title, "peer deelt iets");
        self.vreemd.push(VreemdeStream {
            eigenaar: van,
            id: a.stream_id,
            kind: a.kind,
            titel: a.title.clone(),
            breedte: a.width,
            hoogte: a.height,
            kijken: false,
        });
        Vec::new()
    }

    fn ingetrokken(&mut self, van: PeerId, stream_id: u32) -> Vec<Actie> {
        let keek = self
            .vind_vreemd(van, stream_id)
            .map(|s| s.kijken)
            .unwrap_or(false);
        self.vreemd
            .retain(|s| !(s.eigenaar == van && s.id == stream_id));

        if keek {
            vec![Actie::StopKijken {
                eigenaar: van,
                stream_id,
            }]
        } else {
            Vec::new()
        }
    }

    fn ingetekend(&mut self, van: PeerId, adres: SocketAddr, stream_id: u32) -> Vec<Actie> {
        let Some(s) = self.eigen.iter_mut().find(|s| s.id == stream_id) else {
            tracing::warn!(peer = ?van, stream = stream_id, "intekening op een stream die we niet delen");
            return Vec::new();
        };

        let eerste = !s.wordt_bekeken();
        s.kijkers.insert(van, adres);
        let kijkers = s.doelen();

        if eerste {
            tracing::info!(stream = stream_id, "eerste kijker; opnemen begint");
            vec![Actie::StartDelen { stream_id, kijkers }]
        } else {
            vec![Actie::ZetKijkers { stream_id, kijkers }]
        }
    }

    fn uitgetekend(&mut self, van: PeerId, stream_id: u32) -> Vec<Actie> {
        let Some(s) = self.eigen.iter_mut().find(|s| s.id == stream_id) else {
            return Vec::new();
        };
        if s.kijkers.remove(&van).is_none() {
            return Vec::new();
        }

        if s.wordt_bekeken() {
            vec![Actie::ZetKijkers {
                stream_id,
                kijkers: s.doelen(),
            }]
        } else {
            tracing::info!(stream = stream_id, "laatste kijker weg; opnemen stopt");
            vec![Actie::StopDelen {
                stream_id,
                is_geluid: s.kind == StreamKind::DESKTOP_AUDIO,
            }]
        }
    }

    fn keyframe_gevraagd(&self, van: PeerId, stream_id: u32) -> Vec<Actie> {
        // Alleen honoreren voor wie daadwerkelijk kijkt: anders kan iedereen die het
        // adres kent de bitrate laten ontploffen door keyframes te blijven vragen.
        let mag = self
            .eigen
            .iter()
            .any(|s| s.id == stream_id && s.kijkers.contains_key(&van));
        if mag {
            vec![Actie::StuurKeyframe { stream_id }]
        } else {
            Vec::new()
        }
    }

    fn vind_vreemd(&mut self, eigenaar: PeerId, stream_id: u32) -> Option<&mut VreemdeStream> {
        self.vreemd
            .iter_mut()
            .find(|s| s.eigenaar == eigenaar && s.id == stream_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip() -> IpAddr {
        IpAddr::from([100, 64, 0, 7])
    }

    fn aankondiging(stream_id: u32) -> ControlMsg {
        ControlMsg::StreamAnnounce(StreamAnnounce {
            stream_id,
            kind: StreamKind::MONITOR,
            title: "Scherm 1920×1080".into(),
            width: 1920,
            height: 1080,
        })
    }

    fn intekening(stream_id: u32, poort: u16) -> ControlMsg {
        ControlMsg::StreamSubscribe(StreamSubscribe {
            stream_id,
            media_port: poort,
        })
    }

    #[test]
    fn delen_kost_niets_tot_iemand_kijkt() {
        // Dit is de belangrijkste eigenschap van de hele fase: een gedeeld scherm mag
        // geen enkele opdracht opleveren zolang niemand meekijkt.
        let mut s = Streams::new();
        let (id, cmds) = s.deel(StreamKind::MONITOR, "Scherm".into(), 1920, 1080);
        assert_eq!(id, 1);
        assert_eq!(cmds.len(), 1, "alleen de aankondiging");
        assert!(!s.eigen()[0].wordt_bekeken());
    }

    #[test]
    fn eerste_kijker_start_het_opnemen_en_de_tweede_niet() {
        let mut s = Streams::new();
        let (id, _) = s.deel(StreamKind::MONITOR, "Scherm".into(), 1920, 1080);
        let a = PeerId::new_random();
        let b = PeerId::new_random();

        let (_, acties) = s.bij_bericht(a, ip(), &intekening(id, 41000));
        assert_eq!(
            acties,
            vec![Actie::StartDelen {
                stream_id: id,
                kijkers: vec![SocketAddr::new(ip(), 41000)],
            }]
        );

        let (_, acties) = s.bij_bericht(b, ip(), &intekening(id, 41001));
        assert!(
            matches!(acties.as_slice(), [Actie::ZetKijkers { .. }]),
            "de tweede kijker mag de encoder niet opnieuw starten: {acties:?}"
        );
        assert_eq!(s.eigen()[0].kijkers.len(), 2);
    }

    #[test]
    fn opnemen_stopt_pas_als_de_laatste_kijker_weg_is() {
        let mut s = Streams::new();
        let (id, _) = s.deel(StreamKind::MONITOR, "Scherm".into(), 1920, 1080);
        let a = PeerId::new_random();
        let b = PeerId::new_random();
        s.bij_bericht(a, ip(), &intekening(id, 41000));
        s.bij_bericht(b, ip(), &intekening(id, 41001));

        let (_, acties) = s.bij_bericht(
            a,
            ip(),
            &ControlMsg::StreamUnsubscribe(StreamUnsubscribe { stream_id: id }),
        );
        assert!(
            matches!(acties.as_slice(), [Actie::ZetKijkers { .. }]),
            "b kijkt nog: {acties:?}"
        );

        let (_, acties) = s.bij_bericht(
            b,
            ip(),
            &ControlMsg::StreamUnsubscribe(StreamUnsubscribe { stream_id: id }),
        );
        assert_eq!(
            acties,
            vec![Actie::StopDelen {
                stream_id: id,
                is_geluid: false
            }]
        );
    }

    #[test]
    fn een_kijker_die_wegvalt_stopt_het_opnemen_ook() {
        // Zonder dit blijven we coderen en versturen naar een dood adres, en dat is
        // precies het geval waarin de app in de weg gaat zitten tijdens het gamen.
        let mut s = Streams::new();
        let (id, _) = s.deel(StreamKind::MONITOR, "Scherm".into(), 1920, 1080);
        let a = PeerId::new_random();
        s.bij_bericht(a, ip(), &intekening(id, 41000));

        assert_eq!(
            s.bij_verbreking(a),
            vec![Actie::StopDelen {
                stream_id: id,
                is_geluid: false
            }]
        );
        assert!(!s.eigen()[0].wordt_bekeken());
    }

    #[test]
    fn een_peer_die_wegvalt_neemt_zijn_streams_mee() {
        let mut s = Streams::new();
        let a = PeerId::new_random();
        s.bij_bericht(a, ip(), &aankondiging(1));
        s.wil_kijken(a, 1);

        assert_eq!(
            s.bij_verbreking(a),
            vec![Actie::StopKijken {
                eigenaar: a,
                stream_id: 1
            }]
        );
        assert!(s.vreemd().is_empty(), "zijn streams horen weg te zijn");
    }

    #[test]
    fn opnieuw_aankondigen_gooit_de_kijkstatus_niet_om() {
        // Gebeurt bij elke herverbinding. Zou dit het kijken resetten, dan sprong het
        // venster dicht zodra de verbinding even hikte.
        let mut s = Streams::new();
        let a = PeerId::new_random();
        s.bij_bericht(a, ip(), &aankondiging(1));
        s.wil_kijken(a, 1);

        s.bij_bericht(a, ip(), &aankondiging(1));
        assert_eq!(s.vreemd().len(), 1, "geen duplicaat");
        assert!(s.vreemd()[0].kijken, "we keken al");
    }

    #[test]
    fn een_verbonden_peer_krijgt_onze_lopende_aankondigingen() {
        let mut s = Streams::new();
        s.deel(StreamKind::MONITOR, "Scherm".into(), 1920, 1080);
        s.deel(StreamKind::WINDOW, "Kladblok".into(), 800, 600);

        let a = PeerId::new_random();
        let cmds = s.bij_verbinding(a);
        assert_eq!(cmds.len(), 2);
        assert!(cmds
            .iter()
            .all(|c| matches!(c, MeshCommand::Send { to, .. } if *to == a)));
    }

    #[test]
    fn intrekken_sluit_het_venster_van_de_kijker() {
        let mut s = Streams::new();
        let a = PeerId::new_random();
        s.bij_bericht(a, ip(), &aankondiging(3));
        s.wil_kijken(a, 3);

        let (_, acties) = s.bij_bericht(
            a,
            ip(),
            &ControlMsg::StreamRevoke(StreamRevoke { stream_id: 3 }),
        );
        assert_eq!(
            acties,
            vec![Actie::StopKijken {
                eigenaar: a,
                stream_id: 3
            }]
        );
        assert!(s.vreemd().is_empty());
    }

    #[test]
    fn keyframe_alleen_voor_wie_daadwerkelijk_kijkt() {
        // Anders kan iedereen die het adres kent de bitrate laten ontploffen.
        let mut s = Streams::new();
        let (id, _) = s.deel(StreamKind::MONITOR, "Scherm".into(), 1920, 1080);
        let kijker = PeerId::new_random();
        let vreemde = PeerId::new_random();
        s.bij_bericht(kijker, ip(), &intekening(id, 41000));

        let verzoek = ControlMsg::RequestKeyframe(RequestKeyframe { stream_id: id });
        let (_, acties) = s.bij_bericht(vreemde, ip(), &verzoek);
        assert!(acties.is_empty(), "die kijkt helemaal niet");

        let (_, acties) = s.bij_bericht(kijker, ip(), &verzoek);
        assert_eq!(acties, vec![Actie::StuurKeyframe { stream_id: id }]);
    }

    #[test]
    fn stream_ids_worden_nooit_hergebruikt() {
        // Een hergebruikt id zou beeld in het verkeerde venster kunnen zetten zolang
        // er nog pakketten van de vorige stream onderweg zijn.
        let mut s = Streams::new();
        let (eerste, _) = s.deel(StreamKind::MONITOR, "A".into(), 1920, 1080);
        s.stop_delen(eerste);
        let (tweede, _) = s.deel(StreamKind::MONITOR, "B".into(), 1920, 1080);
        assert_ne!(eerste, tweede);
    }

    #[test]
    fn aankondiging_op_de_voice_stream_wordt_geweigerd() {
        // Stream 0 is voice. Een peer die daar screenshare op aankondigt is stuk of
        // kwaadaardig; in beide gevallen willen we hem niet volgen.
        let mut s = Streams::new();
        let a = PeerId::new_random();
        s.bij_bericht(a, ip(), &aankondiging(VOICE_STREAM_ID));
        assert!(s.vreemd().is_empty());
    }

    #[test]
    fn onbekende_soort_stream_wordt_genegeerd_niet_getoond() {
        let mut s = Streams::new();
        let a = PeerId::new_random();
        s.bij_bericht(
            a,
            ip(),
            &ControlMsg::StreamAnnounce(StreamAnnounce {
                stream_id: 1,
                kind: StreamKind(200),
                title: "iets van later".into(),
                width: 1920,
                height: 1080,
            }),
        );
        assert!(s.vreemd().is_empty());
    }

    #[test]
    fn intekenen_op_iets_wat_we_niet_delen_doet_niets() {
        let mut s = Streams::new();
        let a = PeerId::new_random();
        let (_, acties) = s.bij_bericht(a, ip(), &intekening(42, 41000));
        assert!(acties.is_empty());
    }

    #[test]
    fn tweemaal_kijken_opent_maar_een_venster() {
        let mut s = Streams::new();
        let a = PeerId::new_random();
        s.bij_bericht(a, ip(), &aankondiging(1));
        assert_eq!(s.wil_kijken(a, 1).len(), 1);
        assert!(s.wil_kijken(a, 1).is_empty(), "we keken al");
    }

    #[test]
    fn expliciet_stoppen_met_bureaubladgeluid_is_gemarkeerd_als_geluid() {
        // Bug (fase 10): stop_delen haalt de stream al uit `eigen` vóórdat de motor
        // met `is_geluid` zou kunnen navragen wat voor stream het was. Daarom draagt
        // de actie het zelf, in plaats van dat de motor het achteraf opzoekt.
        let mut s = Streams::new();
        let (id, _) = s.deel(StreamKind::DESKTOP_AUDIO, "Bureaubladgeluid".into(), 0, 0);
        let a = PeerId::new_random();
        s.bij_bericht(a, ip(), &intekening(id, 41000));

        let (_, acties) = s.stop_delen(id);
        assert_eq!(
            acties,
            vec![Actie::StopDelen {
                stream_id: id,
                is_geluid: true
            }]
        );
    }

    #[test]
    fn bureaubladgeluid_is_gemarkeerd_als_geluid() {
        // De motor moet aan de actie kunnen zien dat er geen venster open hoeft maar
        // dat het de mixer in gaat. Zonder deze vlag zou hij een decoder opstarten
        // voor iets wat geen beeld is.
        let mut s = Streams::new();
        let a = PeerId::new_random();
        s.bij_bericht(
            a,
            ip(),
            &ControlMsg::StreamAnnounce(StreamAnnounce {
                stream_id: 4,
                kind: StreamKind::DESKTOP_AUDIO,
                title: "Bureaubladgeluid".into(),
                width: 0,
                height: 0,
            }),
        );

        match s.wil_kijken(a, 4).as_slice() {
            [Actie::StartKijken { is_geluid, .. }] => assert!(*is_geluid),
            andere => panic!("verwachtte één StartKijken, kreeg {andere:?}"),
        }
    }

    #[test]
    fn geluid_en_beeld_van_dezelfde_peer_staan_los_van_elkaar() {
        // Je wilt iemands scherm kunnen bekijken zonder zijn spelgeluid, en andersom.
        let mut s = Streams::new();
        let a = PeerId::new_random();
        s.bij_bericht(a, ip(), &aankondiging(1));
        s.bij_bericht(
            a,
            ip(),
            &ControlMsg::StreamAnnounce(StreamAnnounce {
                stream_id: 2,
                kind: StreamKind::DESKTOP_AUDIO,
                title: "Bureaubladgeluid".into(),
                width: 0,
                height: 0,
            }),
        );

        s.wil_kijken(a, 1);
        assert!(s.vreemd().iter().any(|v| v.id == 1 && v.kijken));
        assert!(
            s.vreemd().iter().any(|v| v.id == 2 && !v.kijken),
            "het geluid hoort niet mee te liften op het beeld"
        );
    }

    #[test]
    fn geluid_volgt_het_beeld_aan_en_uit() {
        // De regressie van fase 12: de Tauri-UI tekent alleen op beeld in, dus zonder
        // deze afstemming hoort niemand nog geluid bij een gedeeld scherm.
        let mut s = Streams::new();
        let a = PeerId::new_random();
        s.bij_bericht(a, ip(), &aankondiging(1));
        s.bij_bericht(
            a,
            ip(),
            &ControlMsg::StreamAnnounce(StreamAnnounce {
                stream_id: 2,
                kind: StreamKind::DESKTOP_AUDIO,
                title: "Bureaubladgeluid".into(),
                width: 0,
                height: 0,
            }),
        );

        // Niet in het gesprek: geen socket om het op te ontvangen.
        s.wil_kijken(a, 1);
        assert_eq!(s.stem_geluid_af_op_beeld(false).1, Vec::new());

        let (_, acties) = s.stem_geluid_af_op_beeld(true);
        assert!(
            matches!(
                acties.as_slice(),
                [Actie::StartKijken {
                    stream_id: 2,
                    is_geluid: true,
                    ..
                }]
            ),
            "geluid hoort mee te gaan met het beeld: {acties:?}"
        );
        assert!(s.stem_geluid_af_op_beeld(true).1.is_empty(), "idempotent");

        // Laatste beeld dicht: het geluid gaat mee uit, mét afmelding bij de deler.
        s.stop_kijken(a, 1);
        let (cmds, acties) = s.stem_geluid_af_op_beeld(true);
        assert_eq!(
            acties,
            vec![Actie::StopKijken {
                eigenaar: a,
                stream_id: 2
            }]
        );
        assert_eq!(cmds.len(), 1, "de deler moet stoppen met versturen");
    }

    #[test]
    fn opnieuw_intekenen_met_een_andere_poort_verhuist_de_kijker() {
        // Gebeurt als iemand zijn venster sluit en meteen opnieuw opent.
        let mut s = Streams::new();
        let (id, _) = s.deel(StreamKind::MONITOR, "Scherm".into(), 1920, 1080);
        let a = PeerId::new_random();
        s.bij_bericht(a, ip(), &intekening(id, 41000));
        let (_, acties) = s.bij_bericht(a, ip(), &intekening(id, 41005));

        assert_eq!(
            acties,
            vec![Actie::ZetKijkers {
                stream_id: id,
                kijkers: vec![SocketAddr::new(ip(), 41005)],
            }],
            "de oude poort mag geen beeld meer krijgen"
        );
    }
}
