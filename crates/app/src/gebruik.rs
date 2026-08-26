//! Wat er de afgelopen week gebeurd is: tijd in het gesprek, tijd met iets gedeeld,
//! berichten, bestanden en Wordle-punten — per peer.
//!
//! # Waarom dit een lokaal bestand is en geen op
//!
//! Tijd in het gesprek is niets wat de log kan weten. `VoiceJoin`/`VoiceLeave` zijn
//! vluchtige control-berichten (net als `Typing`), dus na een herstart is er geen spoor
//! meer van. Er een op van maken zou drie peers elke minuut een regel in een
//! append-only log laten schrijven voor iets waar niemand het over eens hoeft te zijn.
//!
//! Dus meet elke peer wat hij **zelf gezien heeft**, en blijft dat op zijn eigen schijf
//! staan. Twee peers kunnen daardoor iets andere getallen tonen — wie een half uur
//! offline was, heeft dat half uur ook niet geteld. Dat is de eerlijke uitkomst en niet
//! een afwijking om weg te poetsen: het overzicht heet "zoals deze pc het zag".
//!
//! # De rest komt uit de oplog
//!
//! Berichten, bestanden en Wordle staan al in de log en worden hier alleen geteld — niets
//! extra's om bij te houden. De telling gaat over de rauwe ops en niet over de tijdlijn,
//! want [`fitcom_store::timeline`] klemt `wall_clock` op ±7 dagen rond nu (B-42) en alles
//! ouder komt daar dus op exact dezelfde tijdstempel uit. Voor een venster van een week
//! valt dat toevallig goed uit; voor een venster van een maand zou het stilzwijgend de
//! hele geschiedenis meetellen. Rauwe ops hebben die val niet.

use chrono::{Duration as ChronoDuration, Local, NaiveDate};
use fitcom_proto::{Op, OpKind, PeerId};
use fitcom_store::WordleEntry;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Hoeveel dagen het overzicht standaard terugkijkt.
pub const VENSTER_DAGEN: u32 = 7;

/// Dagen ouder dan dit vallen bij het inlezen af. Ruim boven het venster, zodat een
/// langere terugblik later niets hoeft te herstellen wat al weggegooid is.
const BEWAAR_DAGEN: i64 = 120;

/// Grootste stap die één tik mag bijtellen. De motor tikt elke 100 ms, dus alles hierboven
/// is geen verstreken gesprekstijd maar een pc die geslapen heeft of een thread die vastzat.
/// Zonder deze grens boekt één keer dichtklappen van de laptop acht uur "in gesprek".
const MAX_STAP: Duration = Duration::from_secs(5);

/// Wat er op één dag voor één peer gemeten is. In milliseconden, want de motor tikt tien
/// keer per seconde: in hele seconden bijtellen zou elke tik naar beneden afronden op nul.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tijd {
    /// Tijd dat deze peer in het gesprek zat, voor zover deze pc dat zag.
    #[serde(default)]
    pub voice_ms: u64,
    /// Tijd dat deze peer een scherm, venster of camera aankondigde. Bureaubladgeluid telt
    /// niet mee: dat is geen eigen bezigheid maar iets dat vanzelf met een scherm meekomt.
    #[serde(default)]
    pub deel_ms: u64,
}

/// De vorm op schijf. Eigen struct zodat er later een veld bij kan zonder dat een oud
/// bestand onleesbaar wordt.
#[derive(Debug, Default, Serialize, Deserialize)]
struct OpSchijf {
    /// Dagsleutel `YYYYMMDD` -> peer-UUID als tekst -> gemeten tijd. Tekst als sleutel en
    /// niet `PeerId`: JSON kent alleen tekstsleutels, en dan is het beter dat expliciet te
    /// doen dan erop te vertrouwen dat serde er hetzelfde van maakt.
    #[serde(default)]
    dagen: BTreeMap<u32, BTreeMap<String, Tijd>>,
}

pub struct Gebruik {
    /// `None` in tests: dan is er niets in te lezen en niets weg te schrijven.
    pad: Option<PathBuf>,
    dagen: BTreeMap<u32, BTreeMap<String, Tijd>>,
    /// Sinds de laatste [`Gebruik::tik`]. Los van de motorklok zodat een tik die te laat
    /// komt de echte verstreken tijd bijtelt en niet een aangenomen 100 ms.
    laatste: Instant,
    vuil: bool,
}

impl Gebruik {
    pub fn nieuw(data_dir: &Path) -> Self {
        let pad = data_dir.join("gebruik.json");
        let dagen = match std::fs::read_to_string(&pad) {
            Ok(tekst) => match serde_json::from_str::<OpSchijf>(&tekst) {
                Ok(s) => s.dagen,
                Err(e) => {
                    tracing::warn!(error = %e, "gebruik.json is onleesbaar; opnieuw beginnen");
                    BTreeMap::new()
                }
            },
            Err(_) => BTreeMap::new(),
        };
        let mut g = Self {
            pad: Some(pad),
            dagen,
            laatste: Instant::now(),
            vuil: false,
        };
        g.snoei(vandaag());
        g
    }

    #[cfg(test)]
    fn leeg() -> Self {
        Self {
            pad: None,
            dagen: BTreeMap::new(),
            laatste: Instant::now(),
            vuil: false,
        }
    }

    /// Tel de tijd sinds de vorige tik bij iedereen die nu meedoet. Roep dit op elke
    /// motortik aan; hoe vaak dat is maakt niet uit, alleen hoeveel tijd er verstreek.
    pub fn tik(&mut self, in_voice: &[PeerId], deelt: &[PeerId]) {
        let nu = Instant::now();
        let verstreken = nu.duration_since(self.laatste).min(MAX_STAP);
        self.laatste = nu;
        self.tel(vandaag(), verstreken, in_voice, deelt);
    }

    /// De rekenkant van [`Gebruik::tik`], zonder klok, zodat een test hem kan sturen.
    fn tel(&mut self, dag: u32, verstreken: Duration, in_voice: &[PeerId], deelt: &[PeerId]) {
        if in_voice.is_empty() && deelt.is_empty() {
            return;
        }
        let ms = verstreken.as_millis() as u64;
        if ms == 0 {
            return;
        }
        let vandaag = self.dagen.entry(dag).or_default();
        for p in in_voice {
            vandaag.entry(p.to_string()).or_default().voice_ms += ms;
        }
        for p in deelt {
            vandaag.entry(p.to_string()).or_default().deel_ms += ms;
        }
        self.vuil = true;
    }

    /// Wegschrijven als er iets veranderd is. Gaat het mis, dan is dat een logregel en
    /// geen fout in de UI: een kwijtgeraakt overzicht is vervelend, niet stuk.
    pub fn bewaar(&mut self) {
        if !self.vuil {
            return;
        }
        if self.pad.is_none() {
            self.vuil = false;
            return;
        }
        self.snoei(vandaag());
        let Some(pad) = &self.pad else { return };
        let inhoud = OpSchijf {
            dagen: self.dagen.clone(),
        };
        match serde_json::to_string_pretty(&inhoud) {
            Ok(json) => {
                if let Err(e) = std::fs::write(pad, json) {
                    tracing::warn!(error = %e, "gebruik.json niet weg te schrijven");
                    return;
                }
                self.vuil = false;
            }
            Err(e) => tracing::warn!(error = %e, "gebruik.json niet te serialiseren"),
        }
    }

    fn snoei(&mut self, vandaag: u32) {
        let Some(grens) = datum(vandaag).map(|d| d - ChronoDuration::days(BEWAAR_DAGEN)) else {
            return;
        };
        let grens = dagnummer(grens);
        self.dagen.retain(|dag, _| *dag >= grens);
    }

    /// Het overzicht over de laatste `dagen` dagen, vandaag meegerekend.
    ///
    /// `ops` zijn alle ops uit de log (rauw, niet de tijdlijn — zie de moduledoc) en
    /// `wordle` is de al gevouwen uitslagenlijst uit de tijdlijn.
    pub fn overzicht(&self, ops: &[Op], wordle: &[WordleEntry], dagen: u32) -> Overzicht {
        let vandaag = vandaag();
        let vanaf_datum = datum(vandaag)
            .map(|d| d - ChronoDuration::days(dagen.saturating_sub(1) as i64))
            .unwrap_or_else(|| Local::now().date_naive());
        let vanaf_dag = dagnummer(vanaf_datum);
        let vanaf_ms = vanaf_datum
            .and_hms_opt(0, 0, 0)
            .and_then(|t| t.and_local_timezone(Local).earliest())
            .map(|t| t.timestamp_millis())
            .unwrap_or(i64::MIN);

        fn regel(regels: &mut BTreeMap<PeerId, Regel>, peer: PeerId) -> &mut Regel {
            regels.entry(peer).or_insert_with(|| Regel::nieuw(peer))
        }
        let mut regels: BTreeMap<PeerId, Regel> = BTreeMap::new();

        for (_, per_peer) in self.dagen.range(vanaf_dag..) {
            for (peer, tijd) in per_peer {
                // Een sleutel die geen peer meer is (met de hand bewerkt bestand) valt af
                // in plaats van dat het overzicht erop stukloopt.
                let Ok(uuid) = peer.parse() else { continue };
                let r = regel(&mut regels, PeerId(uuid));
                r.voice_ms += tijd.voice_ms;
                r.deel_ms += tijd.deel_ms;
            }
        }

        // Eerst op tijd filteren (een veldvergelijking), pas daarna de payload decoderen —
        // anders wordt elk bericht uit de hele geschiedenis uitgepakt om weggegooid te
        // worden.
        for op in ops.iter().filter(|o| o.wall_clock >= vanaf_ms) {
            match op.kind() {
                Ok(Some(OpKind::Post { .. })) => regel(&mut regels, op.author).berichten += 1,
                Ok(Some(OpKind::FileMeta { .. })) => regel(&mut regels, op.author).bestanden += 1,
                _ => {}
            }
        }

        // De puntenregel wordt niet nagebouwd maar hergebruikt: `standen` op alleen de
        // dagen in het venster. Een tweede telling hier zou stilletjes uit de pas gaan
        // lopen met het scorebord in de chat.
        let recent: Vec<WordleEntry> = wordle
            .iter()
            .filter(|e| e.day >= vanaf_dag)
            .cloned()
            .collect();
        for st in crate::wordle::standen(&recent) {
            let r = regel(&mut regels, st.peer);
            r.punten = st.punten;
            r.gespeeld = st.gespeeld;
            r.opgelost = st.opgelost;
        }

        let mut regels: Vec<Regel> = regels.into_values().filter(|r| !r.leeg()).collect();
        // Vaste volgorde, zodat het overzicht niet van plek wisselt bij het verversen.
        regels.sort_by(|a, b| {
            b.voice_ms
                .cmp(&a.voice_ms)
                .then(b.berichten.cmp(&a.berichten))
                .then(a.peer.cmp(&b.peer))
        });

        Overzicht {
            dagen,
            vanaf: vanaf_dag,
            regels,
        }
    }
}

/// Eén peer over het hele venster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Regel {
    pub peer: PeerId,
    pub voice_ms: u64,
    pub deel_ms: u64,
    pub berichten: u32,
    pub bestanden: u32,
    pub punten: u32,
    pub gespeeld: u32,
    pub opgelost: u32,
}

impl Regel {
    fn nieuw(peer: PeerId) -> Self {
        Self {
            peer,
            voice_ms: 0,
            deel_ms: 0,
            berichten: 0,
            bestanden: 0,
            punten: 0,
            gespeeld: 0,
            opgelost: 0,
        }
    }

    /// Een peer die deze week niets deed hoort niet in het overzicht. Kan ontstaan door
    /// een dag met een tik van nul milliseconden of een Wordle-stand zonder punten.
    fn leeg(&self) -> bool {
        self.voice_ms == 0
            && self.deel_ms == 0
            && self.berichten == 0
            && self.bestanden == 0
            && self.gespeeld == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overzicht {
    /// Lengte van het venster in dagen.
    pub dagen: u32,
    /// Eerste dag in het venster, `YYYYMMDD`.
    pub vanaf: u32,
    /// Alleen peers die iets deden, op aflopende gesprekstijd.
    pub regels: Vec<Regel>,
}

impl Default for Overzicht {
    fn default() -> Self {
        Self {
            dagen: VENSTER_DAGEN,
            vanaf: 0,
            regels: Vec::new(),
        }
    }
}

/// De kalenderdag van nu, `YYYYMMDD`. Bewust de gewone lokale datum en niet de
/// Wordle-dag: die loopt van 07:00 tot 07:00, en dat is een regel van dat spel en niet
/// van "wat deden we deze week".
fn vandaag() -> u32 {
    dagnummer(Local::now().date_naive())
}

fn dagnummer(d: NaiveDate) -> u32 {
    crate::wordle::dagnummer(d)
}

fn datum(dag: u32) -> Option<NaiveDate> {
    crate::wordle::datum_van_dagnummer(dag)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer() -> PeerId {
        PeerId::new_random()
    }

    #[test]
    fn tel_boekt_op_de_juiste_peer_en_de_juiste_dag() {
        let (a, b) = (peer(), peer());
        let mut g = Gebruik::leeg();

        g.tel(20260824, Duration::from_millis(500), &[a, b], &[a]);
        g.tel(20260824, Duration::from_millis(500), &[a], &[]);
        g.tel(20260825, Duration::from_millis(200), &[b], &[]);

        let dag = &g.dagen[&20260824];
        assert_eq!(dag[&a.to_string()].voice_ms, 1000);
        assert_eq!(dag[&a.to_string()].deel_ms, 500);
        assert_eq!(dag[&b.to_string()].voice_ms, 500);
        assert_eq!(dag[&b.to_string()].deel_ms, 0, "b deelde niets");
        assert_eq!(g.dagen[&20260825][&b.to_string()].voice_ms, 200);
    }

    #[test]
    fn een_tik_na_een_slaapstand_boekt_hoogstens_max_stap() {
        let a = peer();
        let mut g = Gebruik::leeg();
        g.laatste = Instant::now() - Duration::from_secs(8 * 3600);

        g.tik(&[a], &[]);

        let geteld = g.dagen[&vandaag()][&a.to_string()].voice_ms;
        assert_eq!(geteld, MAX_STAP.as_millis() as u64);
    }

    #[test]
    fn overzicht_telt_alleen_dagen_binnen_het_venster() {
        let a = peer();
        let vandaag_dag = vandaag();
        let oud = dagnummer(datum(vandaag_dag).unwrap() - ChronoDuration::days(30));
        let gisteren = dagnummer(datum(vandaag_dag).unwrap() - ChronoDuration::days(1));

        let mut g = Gebruik::leeg();
        g.tel(oud, Duration::from_secs(60), &[a], &[]);
        g.tel(gisteren, Duration::from_secs(30), &[a], &[]);
        g.tel(vandaag_dag, Duration::from_secs(10), &[a], &[a]);

        let o = g.overzicht(&[], &[], VENSTER_DAGEN);
        assert_eq!(o.regels.len(), 1);
        assert_eq!(o.regels[0].peer, a);
        assert_eq!(
            o.regels[0].voice_ms, 40_000,
            "de dag van 30 dagen terug valt buiten het venster"
        );
        assert_eq!(o.regels[0].deel_ms, 10_000);
    }

    #[test]
    fn wat_bewaard_is_komt_terug_en_te_oude_dagen_niet() {
        let a = peer();
        let dir = std::env::temp_dir().join(format!("fitcom-gebruik-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let vandaag_dag = vandaag();
        let stokoud =
            dagnummer(datum(vandaag_dag).unwrap() - ChronoDuration::days(BEWAAR_DAGEN + 1));

        {
            let mut g = Gebruik::nieuw(&dir);
            g.tel(stokoud, Duration::from_secs(99), &[a], &[]);
            g.tel(vandaag_dag, Duration::from_secs(12), &[a], &[a]);
            g.bewaar();
        }

        let terug = Gebruik::nieuw(&dir);
        assert!(
            !terug.dagen.contains_key(&stokoud),
            "een dag ouder dan het bewaarvenster hoort bij het inlezen af te vallen"
        );
        let tijd = terug.dagen[&vandaag_dag][&a.to_string()];
        assert_eq!(tijd.voice_ms, 12_000);
        assert_eq!(tijd.deel_ms, 12_000);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn overzicht_zet_de_meeste_gesprekstijd_bovenaan() {
        let (stil, luid) = (peer(), peer());
        let mut g = Gebruik::leeg();
        g.tel(vandaag(), Duration::from_secs(5), &[stil], &[]);
        g.tel(vandaag(), Duration::from_secs(50), &[luid], &[]);

        let o = g.overzicht(&[], &[], VENSTER_DAGEN);
        assert_eq!(o.regels[0].peer, luid);
        assert_eq!(o.regels[1].peer, stil);
    }
}
