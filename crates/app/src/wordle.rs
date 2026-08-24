//! Het Wordle-spel van de dag, plus het scorebord over alle dagen.
//!
//! # Waarom hier een verbinding buiten het tailnet gelegd wordt
//!
//! Dit is de **derde** bewuste uitzondering op invariant 1 (nul servers), na de
//! release-feed (fase 13) en de YouTube-previews. De afweging staat in
//! `docs/OVERDRACHT.md` beslissing 31; kort: het echte woord van de dag kán alleen bij
//! NYT vandaan komen, en zonder dat woord is dit een ander spel dan het spel waar de drie
//! peers 's ochtends over praten. Rick heeft de keuze gemaakt met drie opties op tafel.
//!
//! Wat de uitzondering zo klein mogelijk houdt:
//!
//! - **Eén GET per dag per peer**, naar een vast eindpunt zonder sleutel of account:
//!   `https://www.nytimes.com/svc/wordle/v2/<datum>.json`. Daarna van schijf.
//! - **Iedere peer haalt het zelf op.** Geen peer die het voor de anderen doet — dat zou
//!   invariant 2 (geen host-peer) aantasten en de dag laten afhangen van wie er online
//!   was. Het antwoord staat daardoor ook nooit op de draad; over de mesh gaan alleen
//!   *uitslagen*.
//! - **Het ophalen zit in de motor**, net als bij `crate::youtube`: de CSP van het venster
//!   blijft dicht, er komt geen host bij in `connect-src`.
//! - **Mislukt het, dan is er die dag geen kaart** en werkt de rest van de app door.
//!   Invariant 7 (offline is normaal) geldt onverkort; dit is een spelletje.
//!
//! # Waarom het woord niet naar de webview gaat
//!
//! Zolang het spel loopt blijft de oplossing in de motor. De webview stuurt een gok en
//! krijgt vijf kleuren terug — hij weet het antwoord pas als het spel klaar is. Dat is
//! niet tegen een aanvaller (het is je eigen spel), maar tegen het te makkelijk maken van
//! iets waar de hele grap in zit.
//!
//! # De dag begint om 07:00
//!
//! Een Wordle-dag loopt van 07:00 tot 07:00 lokale tijd, want dat is het moment waarop de
//! kaart in de chat hoort te verschijnen. Wie om 00:30 nog zit te puzzelen speelt dus nog
//! aan het raadsel van "gisteren", en de uitslag wordt ook op die dag geboekt — de sleutel
//! is de `print_date` uit het antwoord van NYT en niet de stand van de eigen klok.

use anyhow::{bail, ensure, Context, Result};
use chrono::{DateTime, Local, NaiveDate, Timelike};
use fitcom_proto::PeerId;
use fitcom_store::WordleEntry;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// De toegestane gokwoorden: 14.855 woorden van vijf letters, precies de lijst waar het
/// echte Wordle een gok tegen afmeet (uit de gepubliceerde clientlijst, één keer
/// overgenomen en gesorteerd — de app haalt hem nooit op).
///
/// **Het formaat is strikt en de code leunt erop**: elke regel is vijf ASCII-kleine
/// letters plus een `\n`, dus zes bytes, en de regels staan gesorteerd. Daardoor kan er
/// binair op gezocht worden zonder de lijst eerst in een `Vec` te zetten: rij `i` staat op
/// `i * 6`. `de_lijst_heeft_het_verwachte_formaat` bewaakt dat.
const WOORDEN: &str = include_str!("wordle_woorden.txt");
const RIJ: usize = 6;

/// Het aantal pogingen, en daarmee ook het hoogste aantal rijen in een patroon.
pub const POGINGEN: u8 = 6;
/// Letters per woord.
pub const LETTERS: usize = 5;

/// Het uur waarop het raadsel van vandaag in de chat verschijnt, lokale tijd.
const OPENBAAR_UUR: u32 = 7;

/// Hoe lang we wachten voordat we het ophalen opnieuw proberen. Dit hoeft niet snel: de
/// kaart is een dag geldig en niemand mist iets als hij er een kwartier later staat.
const HERPROBEER: Duration = Duration::from_secs(15 * 60);

/// Een dag telt pas voor het scorebord als er minstens zoveel peers gespeeld hebben.
/// Twee, niet drie: je krijgt geen punt voor alleen spelen, maar één peer op vakantie legt
/// de competitie niet stil. Bewust een aantal en geen `peers.len()` — invariant 3 zegt dat
/// nergens een 3 hardgecodeerd staat, en "minstens één tegenstander" is de regel die Rick
/// bedoelde.
const MIN_SPELERS: usize = 2;

const ENDPOINT: &str = "https://www.nytimes.com/svc/wordle/v2/";
const MAX_ANTWOORD: u64 = 16 * 1024;
const TIMEOUT: Duration = Duration::from_secs(10);
const VERBIND_TIMEOUT: Duration = Duration::from_secs(5);

/// Wat één letter van een gok opleverde. De getalwaarden zijn het wire-formaat van
/// `OpKind::WordleResult::pattern` — niet veranderen zonder het patroon mee te verhuizen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Teken {
    /// Zit niet (meer) in het woord.
    Mis = 0,
    /// Zit erin, maar niet hier.
    Bijna = 1,
    /// Staat goed.
    Goed = 2,
}

impl Teken {
    fn cijfer(self) -> char {
        match self {
            Self::Mis => '0',
            Self::Bijna => '1',
            Self::Goed => '2',
        }
    }
}

/// Eén raadsel zoals NYT het uitgeeft.
#[derive(Debug, Clone)]
pub struct Raadsel {
    /// `print_date` als `YYYYMMDD` — de sleutel van alles hieronder en van de op.
    pub dag: u32,
    /// Het raadselnummer dat het echte Wordle erboven zet.
    pub nummer: u32,
    pub oplossing: String,
}

/// Wat er van één dag op schijf staat: het raadsel plus onze eigen gokken tot nu toe.
/// Puur lokaal, nooit een op, nooit op de draad — net als `bestandspaden.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Dag {
    nummer: u32,
    oplossing: String,
    #[serde(default)]
    gokken: Vec<String>,
}

impl Dag {
    fn gewonnen(&self) -> bool {
        self.gokken.last().is_some_and(|g| *g == self.oplossing)
    }

    fn klaar(&self) -> bool {
        self.gewonnen() || self.gokken.len() >= POGINGEN as usize
    }

    fn patroon(&self) -> String {
        self.gokken
            .iter()
            .flat_map(|g| beoordeel(&self.oplossing, g))
            .map(Teken::cijfer)
            .collect()
    }
}

/// De inhoud van `<data>/wordle.json`. Eigen struct met één veld, zodat er later iets bij
/// kan zonder dat een bestand van nu onleesbaar wordt.
#[derive(Debug, Default, Serialize, Deserialize)]
struct OpSchijf {
    #[serde(default)]
    dagen: BTreeMap<u32, Dag>,
}

/// Eén rij op het bord, zoals de UI hem tekent.
#[derive(Debug, Clone)]
pub struct Rij {
    pub woord: String,
    pub tekens: Vec<Teken>,
}

/// Het bord van de huidige dag.
#[derive(Debug, Clone)]
pub struct Bord {
    pub dag: u32,
    pub nummer: u32,
    pub rijen: Vec<Rij>,
    pub klaar: bool,
    pub gewonnen: bool,
    /// Alleen gevuld als het spel klaar is. Zie de moduledoc.
    pub oplossing: Option<String>,
}

/// Wat een gok opleverde.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gok {
    /// Aangenomen, het spel loopt door.
    Verder,
    /// Aangenomen en klaar. Dit moet de oplog in — de motor doet dat.
    ///
    /// `dag` reist mee en wordt door de aanroeper niet opnieuw opgevraagd: precies op 07:00
    /// zou een tweede `huidige_dag()` de volgende dag kunnen opleveren, en dan werd de
    /// uitslag van het ene raadsel op de dag van het andere geboekt.
    Klaar {
        dag: u32,
        pogingen: u8,
        gewonnen: bool,
        patroon: String,
    },
    /// Niet aangenomen. De tekst is Engels: hij gaat rechtstreeks het venster in.
    Geweigerd(&'static str),
}

/// Alles wat deze pc over Wordle weet.
pub struct Wordle {
    pad: PathBuf,
    dagen: BTreeMap<u32, Dag>,
    /// Wanneer we het laatst geprobeerd hebben op te halen, om niet elke tik opnieuw
    /// tegen een dichte deur te lopen.
    laatste_poging: Option<Instant>,
    /// De regel die het venster toont als er iets niet lukte: waarom de laatste gok niet
    /// aangenomen werd, of waarom een handmatig opvragen niets opleverde. Twee schrijvers,
    /// allebei via een methode hier ([`Wordle::gok`] en [`Wordle::nu_ophalen`]) plus de
    /// motor die de uitkomst van een handmatige poging neerlegt. Wordt gewist zodra er een
    /// gok wél doorkomt of een nieuwe poging begint, zodat er nooit een oude reden blijft
    /// staan bij een nieuwe situatie.
    pub fout: Option<String>,
}

impl Wordle {
    /// Leest wat er op schijf staat. Een onleesbaar of half bestand geeft geen fout: dan
    /// begint het spel vandaag opnieuw, en dat is beter dan een app die er niet van start.
    pub fn nieuw(data_dir: &Path) -> Self {
        let pad = data_dir.join("wordle.json");
        let mut dagen = match std::fs::read_to_string(&pad) {
            Ok(tekst) => match serde_json::from_str::<OpSchijf>(&tekst) {
                Ok(s) => s.dagen,
                Err(e) => {
                    tracing::warn!(error = %e, "wordle.json is onleesbaar; opnieuw beginnen");
                    BTreeMap::new()
                }
            },
            Err(_) => BTreeMap::new(),
        };
        // Een dag met een onbruikbare oplossing valt af in plaats van dat hij hierna
        // elke gok tegen onzin afmeet.
        dagen.retain(|dag, d| {
            let goed = geldige_oplossing(&d.oplossing);
            if !goed {
                tracing::warn!(dag, "dag met een onbruikbare oplossing overgeslagen");
            }
            goed
        });
        Self {
            pad,
            dagen,
            laatste_poging: None,
            fout: None,
        }
    }

    /// De dag waar het huidige raadsel bij hoort, als `YYYYMMDD`.
    pub fn huidige_dag(&self) -> u32 {
        dagnummer(datum_van(Local::now()))
    }

    /// De datum die opgehaald moet worden, of `None` als we hem al hebben of net geprobeerd
    /// hebben. De string gaat een URL-pad in, maar komt uit onze eigen klok en nooit van
    /// een peer — anders was dit de plek voor een controle zoals `youtube::geldig_id`.
    pub fn moet_ophalen(&mut self) -> Option<String> {
        let datum = datum_van(Local::now());
        if self.dagen.contains_key(&dagnummer(datum)) {
            return None;
        }
        if self
            .laatste_poging
            .is_some_and(|t| t.elapsed() < HERPROBEER)
        {
            return None;
        }
        self.laatste_poging = Some(Instant::now());
        Some(datum.format("%Y-%m-%d").to_string())
    }

    /// Hetzelfde, maar op verzoek van de gebruiker: het kwartier wachten telt niet mee.
    ///
    /// `None` betekent nog steeds "niets te doen", en op deze plek betekent dat maar één
    /// ding: het raadsel is er al. Dat is geen fout, dus dan blijft [`Self::fout`] leeg en
    /// laat het venster het bord zien.
    ///
    /// Let op wat dit *niet* doet: het haalt het raadsel van [`datum_van`], en dat is vóór
    /// 07:00 nog dat van gisteren. Eerder aan het raadsel van morgen komen kan hier niet,
    /// en dat is opzet — `OPENBAAR_UUR` bepaalt óók wanneer de kaart bij de anderen in de
    /// tijdlijn staat, dus dat vooruit halen zou het spel scheeftrekken.
    pub fn nu_ophalen(&mut self) -> Option<String> {
        self.laatste_poging = None;
        let datum = self.moet_ophalen();
        // Een oude reden mag niet blijven staan boven een poging die nog loopt.
        if datum.is_some() {
            self.fout = None;
        }
        datum
    }

    /// Een opgehaald raadsel opbergen. Overschrijft nooit een dag die we al hadden: daar
    /// zouden gedane gokken in zitten.
    pub fn neem_op(&mut self, r: Raadsel) {
        if self.dagen.contains_key(&r.dag) || !geldige_oplossing(&r.oplossing) {
            return;
        }
        self.dagen.insert(
            r.dag,
            Dag {
                nummer: r.nummer,
                oplossing: r.oplossing,
                gokken: Vec::new(),
            },
        );
        self.bewaar();
    }

    /// Het bord van de huidige dag, of `None` zolang het raadsel niet binnen is.
    pub fn bord(&self) -> Option<Bord> {
        let dag = self.huidige_dag();
        let d = self.dagen.get(&dag)?;
        let klaar = d.klaar();
        Some(Bord {
            dag,
            nummer: d.nummer,
            rijen: d
                .gokken
                .iter()
                .map(|g| Rij {
                    woord: g.to_uppercase(),
                    tekens: beoordeel(&d.oplossing, g).to_vec(),
                })
                .collect(),
            klaar,
            gewonnen: d.gewonnen(),
            oplossing: klaar.then(|| d.oplossing.to_uppercase()),
        })
    }

    /// Het raadselnummer van een dag, of `None` als deze pc die dag nooit ophaalde.
    pub fn nummer_van(&self, dag: u32) -> Option<u32> {
        self.dagen.get(&dag).map(|d| d.nummer)
    }

    /// De dagen waar we een raadsel van hebben met hun raadselnummer, oud naar nieuw.
    /// Eén doorloop: dit wordt bij elke momentopname opnieuw gelezen.
    pub fn bekende_dagen(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.dagen.iter().map(|(dag, d)| (*dag, d.nummer))
    }

    /// Een gok op het raadsel van vandaag. Alleen op vandaag: een oudere dag naspelen zou
    /// betekenen dat je een punt kunt halen op een dag waarop de anderen al klaar waren.
    pub fn gok(&mut self, ruw: &str) -> Gok {
        let uitkomst = self.probeer(ruw);
        // Eén plek waar `fout` gezet en gewist wordt: een geweigerde gok laat een regel in
        // het venster achter, een aangenomen gok haalt hem weg.
        self.fout = match &uitkomst {
            Gok::Geweigerd(reden) => Some((*reden).to_string()),
            _ => None,
        };
        uitkomst
    }

    fn probeer(&mut self, ruw: &str) -> Gok {
        let dag = self.huidige_dag();
        let woord = ruw.trim().to_ascii_lowercase();

        let Some(d) = self.dagen.get_mut(&dag) else {
            return Gok::Geweigerd("Today's puzzle has not arrived yet.");
        };
        if d.klaar() {
            return Gok::Geweigerd("Today's puzzle is already finished.");
        }
        if woord.len() != LETTERS || !woord.bytes().all(|b| b.is_ascii_lowercase()) {
            return Gok::Geweigerd("Five letters, A to Z.");
        }
        // De oplossing zelf is altijd toegestaan, ook als hij niet in onze lijst staat:
        // NYT kiest zijn woorden zelf en onze lijst is een afdruk. Anders zou het raadsel
        // op zo'n dag onoplosbaar zijn.
        if woord != d.oplossing && !is_woord(&woord) {
            return Gok::Geweigerd("Not in the word list.");
        }

        d.gokken.push(woord);
        let klaar = d.klaar();
        let uitkomst = if klaar {
            Gok::Klaar {
                dag,
                pogingen: d.gokken.len() as u8,
                gewonnen: d.gewonnen(),
                patroon: d.patroon(),
            }
        } else {
            Gok::Verder
        };
        self.bewaar();
        uitkomst
    }

    /// Wat er van de huidige dag in de oplog hoort te staan, als het spel klaar is.
    /// Gebruikt bij het starten: mislukte het vastleggen ooit, dan komt het er alsnog in.
    pub fn te_melden(&self) -> Option<(u32, u8, bool, String)> {
        let dag = self.huidige_dag();
        let d = self.dagen.get(&dag)?;
        d.klaar()
            .then(|| (dag, d.gokken.len() as u8, d.gewonnen(), d.patroon()))
    }

    fn bewaar(&self) {
        let inhoud = OpSchijf {
            dagen: self.dagen.clone(),
        };
        match serde_json::to_string_pretty(&inhoud) {
            // Niet fataal: deze sessie speelt door, alleen een herstart vergeet de
            // gedane gokken van vandaag.
            Ok(json) => {
                if let Err(e) = std::fs::write(&self.pad, json) {
                    tracing::warn!(error = %e, pad = %self.pad.display(), "wordle.json niet weggeschreven");
                }
            }
            Err(e) => tracing::warn!(error = %e, "wordle-stand niet te serialiseren"),
        }
    }
}

/// De dag waar het raadsel bij hoort op moment `nu`: vóór 07:00 loopt dat van gisteren nog.
fn datum_van(nu: DateTime<Local>) -> NaiveDate {
    let d = nu.date_naive();
    if nu.hour() < OPENBAAR_UUR {
        d.pred_opt().unwrap_or(d)
    } else {
        d
    }
}

/// `YYYYMMDD`. Eén getal in plaats van een string, zodat de op geen lengtegrens nodig
/// heeft en de sortering vanzelf chronologisch is.
pub fn dagnummer(d: NaiveDate) -> u32 {
    use chrono::Datelike;
    d.year().max(0) as u32 * 10_000 + d.month() * 100 + d.day()
}

/// `YYYYMMDD` terug naar een datum, voor de kop van de kaart. `None` bij onzin uit een op
/// van een peer.
pub fn datum_van_dagnummer(dag: u32) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt((dag / 10_000) as i32, (dag / 100) % 100, dag % 100)
}

/// Het moment waarop de kaart van deze dag in de chat hoort te staan: 07:00 lokaal.
/// Millis sinds epoch, zodat de tijdlijn hem tussen de berichten kan zetten.
pub fn openbaar_op(dag: u32) -> i64 {
    datum_van_dagnummer(dag)
        .and_then(|d| d.and_hms_opt(OPENBAAR_UUR, 0, 0))
        .and_then(|t| t.and_local_timezone(Local).earliest())
        .map(|t| t.timestamp_millis())
        .unwrap_or(0)
}

fn geldige_oplossing(w: &str) -> bool {
    w.len() == LETTERS && w.bytes().all(|b| b.is_ascii_lowercase())
}

/// Staat dit woord in de toegestane lijst? Binair, op de vaste rijlengte — zie [`WOORDEN`].
pub fn is_woord(woord: &str) -> bool {
    if woord.len() != LETTERS {
        return false;
    }
    let rijen = WOORDEN.len() / RIJ;
    let bytes = WOORDEN.as_bytes();
    let mut laag = 0usize;
    let mut hoog = rijen;
    while laag < hoog {
        let mid = (laag + hoog) / 2;
        match bytes[mid * RIJ..mid * RIJ + LETTERS].cmp(woord.as_bytes()) {
            std::cmp::Ordering::Less => laag = mid + 1,
            std::cmp::Ordering::Greater => hoog = mid,
            std::cmp::Ordering::Equal => return true,
        }
    }
    false
}

/// De vijf kleuren van één gok.
///
/// Dubbele letters zijn de hele subtiliteit: `Goed` gaat vóór `Bijna`, en `Bijna` mag
/// een letter niet vaker uitdelen dan hij nog in de oplossing over is. Zonder die twee
/// rondes zou `ALLEE` tegen `ANGEL` drie gele L's opleveren waar er één L over is.
pub fn beoordeel(oplossing: &str, gok: &str) -> [Teken; LETTERS] {
    let mut uit = [Teken::Mis; LETTERS];
    let o: Vec<u8> = oplossing.bytes().collect();
    let g: Vec<u8> = gok.bytes().collect();
    if o.len() != LETTERS || g.len() != LETTERS {
        return uit;
    }

    // Eerste ronde: wat goed staat, en wat er per letter overblijft.
    let mut over: HashMap<u8, u8> = HashMap::new();
    for i in 0..LETTERS {
        if g[i] == o[i] {
            uit[i] = Teken::Goed;
        } else {
            *over.entry(o[i]).or_insert(0) += 1;
        }
    }
    // Tweede ronde: de rest, zolang de voorraad strekt.
    for i in 0..LETTERS {
        if uit[i] == Teken::Goed {
            continue;
        }
        if let Some(n) = over.get_mut(&g[i]).filter(|n| **n > 0) {
            *n -= 1;
            uit[i] = Teken::Bijna;
        }
    }
    uit
}

// -- scorebord -------------------------------------------------------------

/// Wat één peer in het scorebord staat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stand {
    pub peer: PeerId,
    /// Dagen waarop deze peer een punt kreeg: gewonnen, of gelijkgespeeld op het laagste
    /// aantal pogingen.
    pub punten: u32,
    /// Dagen waarop hij meedeed, ook de dagen die niet meetelden.
    pub gespeeld: u32,
    /// Dagen die hij oploste, ook als hij er niet mee won.
    pub opgelost: u32,
}

/// De peers die op deze dag een punt krijgen.
///
/// Leeg als de dag niet meetelt: minder dan [`MIN_SPELERS`] deelnemers (je krijgt geen
/// punt voor alleen spelen), of niemand die het woord vond. Meer dan één naam betekent een
/// gelijkspel, en dan krijgen ze allemaal hun punt — zo heeft Rick het gevraagd.
///
/// `dag` is een aaneengesloten stuk uitslagen van één dag; `standen` snijdt die eruit.
pub fn winnaars(dag: &[WordleEntry]) -> Vec<PeerId> {
    if dag.len() < MIN_SPELERS {
        return Vec::new();
    }
    let Some(beste) = dag.iter().filter(|e| e.solved).map(|e| e.guesses).min() else {
        return Vec::new();
    };
    dag.iter()
        .filter(|e| e.solved && e.guesses == beste)
        .map(|e| e.author)
        .collect()
}

/// Telt deze dag mee voor het scorebord? Zie [`MIN_SPELERS`]: met één speler is er geen
/// wedstrijd, dus valt er ook niets te winnen.
pub fn telt_mee(dag: &[WordleEntry]) -> bool {
    dag.len() >= MIN_SPELERS
}

/// Het scorebord over alle dagen, hoogste punten eerst.
///
/// `uitslagen` komt uit `Timeline::wordle` en staat daar al op `(dag, auteur)` gesorteerd,
/// dus de dagen liggen als aaneengesloten stukken naast elkaar en dit is één doorloop —
/// belangrijk, want dit wordt bij elke momentopname opnieuw gerekend.
pub fn standen(uitslagen: &[WordleEntry]) -> Vec<Stand> {
    fn regel(per_peer: &mut HashMap<PeerId, Stand>, peer: PeerId) -> &mut Stand {
        per_peer.entry(peer).or_insert(Stand {
            peer,
            punten: 0,
            gespeeld: 0,
            opgelost: 0,
        })
    }

    let mut per_peer: HashMap<PeerId, Stand> = HashMap::new();
    for e in uitslagen {
        let s = regel(&mut per_peer, e.author);
        s.gespeeld += 1;
        if e.solved {
            s.opgelost += 1;
        }
    }
    for groep in per_dag(uitslagen) {
        for peer in winnaars(groep) {
            regel(&mut per_peer, peer).punten += 1;
        }
    }

    let mut uit: Vec<Stand> = per_peer.into_values().collect();
    // Punten omlaag, dan opgelost omlaag, dan op peer-id zodat elke pc dezelfde volgorde
    // toont bij een gelijke stand.
    uit.sort_by(|a, b| {
        (b.punten, b.opgelost, a.peer)
            .partial_cmp(&(a.punten, a.opgelost, b.peer))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    uit
}

/// De uitslagen opgedeeld in aaneengesloten stukken van één dag. Leunt erop dat
/// `Timeline::wordle` op `(dag, auteur)` gesorteerd staat.
pub fn per_dag(uitslagen: &[WordleEntry]) -> Vec<&[WordleEntry]> {
    let mut uit = Vec::new();
    let mut begin = 0;
    for i in 1..=uitslagen.len() {
        if i == uitslagen.len() || uitslagen[i].day != uitslagen[begin].day {
            uit.push(&uitslagen[begin..i]);
            begin = i;
        }
    }
    uit
}

// -- ophalen ---------------------------------------------------------------

/// Het antwoord van NYT. De rest van de velden (`id`, `editor`) interesseert ons niet;
/// serde negeert ze.
#[derive(Deserialize)]
struct Antwoord {
    solution: String,
    print_date: String,
    #[serde(default)]
    days_since_launch: u32,
}

/// Het raadsel van één datum (`YYYY-MM-DD`) ophalen. Blokkeert (ureq is synchroon), dus
/// dit hoort op een blocking-thread — zie `engine::start_wordle_ophalen`.
pub fn haal_op(datum: &str) -> Result<Raadsel> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .timeout_connect(Some(VERBIND_TIMEOUT))
        .user_agent(concat!("fitcom/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();

    let antwoord: Antwoord = agent
        .get(format!("{ENDPOINT}{datum}.json"))
        .call()
        .context("het woord van de dag is niet op te halen")?
        .body_mut()
        .with_config()
        .limit(MAX_ANTWOORD)
        .read_json()
        .context("antwoord is geen bruikbare JSON")?;

    let oplossing = antwoord.solution.trim().to_ascii_lowercase();
    ensure!(
        geldige_oplossing(&oplossing),
        "onbruikbare oplossing in het antwoord"
    );
    // De datum uit het antwoord is de sleutel, niet de datum die wij vroegen — maar ze
    // moeten wel gelijk zijn. Zo niet, dan hebben we een ander raadsel te pakken dan we
    // bedoelden, en dat zou de uitslag op de verkeerde dag boeken.
    if antwoord.print_date != datum {
        bail!(
            "antwoord is van {} en niet van {datum}",
            antwoord.print_date
        );
    }
    let dag = NaiveDate::parse_from_str(datum, "%Y-%m-%d")
        .map(dagnummer)
        .context("datum uit het antwoord is niet te lezen")?;

    Ok(Raadsel {
        dag,
        nummer: antwoord.days_since_launch,
        oplossing,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(n: u8) -> PeerId {
        let mut b = [0u8; 16];
        b[0] = n;
        PeerId::from_bytes(b)
    }

    fn uitslag(day: u32, author: PeerId, guesses: u8, solved: bool) -> WordleEntry {
        WordleEntry {
            day,
            author,
            guesses,
            solved,
            pattern: String::new(),
        }
    }

    /// De binaire zoekopdracht in `is_woord` rekent met een vaste rijlengte in plaats van
    /// regels te splitsen. Deze test is wat dat mag: zes bytes per rij, gesorteerd, en
    /// niets anders dan kleine letters erin.
    #[test]
    fn de_lijst_heeft_het_verwachte_formaat() {
        assert_eq!(
            WOORDEN.len() % RIJ,
            0,
            "geen heel aantal rijen van zes bytes"
        );
        let b = WOORDEN.as_bytes();
        let rijen = WOORDEN.len() / RIJ;
        assert!(rijen > 10_000, "maar {rijen} woorden");
        let mut vorige: &[u8] = b"";
        for i in 0..rijen {
            let woord = &b[i * RIJ..i * RIJ + LETTERS];
            assert_eq!(
                b[i * RIJ + LETTERS],
                b'\n',
                "rij {i} eindigt niet op newline"
            );
            assert!(
                woord.iter().all(|c| c.is_ascii_lowercase()),
                "rij {i} is geen kleine letters"
            );
            assert!(vorige < woord, "rij {i} staat niet gesorteerd");
            vorige = woord;
        }
    }

    #[test]
    fn de_woordenlijst_kent_gewone_woorden_en_geen_onzin() {
        for goed in ["crane", "slate", "audio", "murky", "aahed", "zymic"] {
            assert!(is_woord(goed), "{goed} zou erin moeten staan");
        }
        for fout in ["", "abc", "abcdef", "zzzzz", "qwert", "CRANE"] {
            assert!(!is_woord(fout), "{fout} zou eruit moeten vallen");
        }
    }

    fn tekens(oplossing: &str, gok: &str) -> String {
        beoordeel(oplossing, gok)
            .iter()
            .map(|t| t.cijfer())
            .collect()
    }

    #[test]
    fn een_gok_zonder_dubbele_letters() {
        assert_eq!(tekens("murky", "murky"), "22222");
        // MURKY / CRANE: alleen de R zit erin, en niet op die plek.
        assert_eq!(tekens("murky", "crane"), "01000");
        // MURKY / BRICK: R staat in de oplossing op plek 3 en in de gok op plek 2, dus
        // geel en niet groen; de K net zo.
        assert_eq!(tekens("murky", "brick"), "01001");
    }

    /// De hele subtiliteit van Wordle. Een letter mag niet vaker geel worden dan hij nog
    /// in de oplossing over is, en groen gaat voor.
    #[test]
    fn dubbele_letters_delen_niet_meer_uit_dan_er_is() {
        // ANGEL / ALLEE: A en E staan goed. Er is één L over, dus van de twee L's in de
        // gok wordt alleen de eerste geel; de tweede blijft grijs. De tweede E van de gok
        // krijgt niets, want de enige E is al groen vergeven.
        assert_eq!(tekens("angel", "allee"), "21020");
        // LLAMA / LOLLY: twee L's in de oplossing, waarvan er één groen staat. Dus is er
        // nog één L over voor de drie andere L-plekken in de gok, en die gaat naar de
        // eerste ervan.
        assert_eq!(tekens("llama", "lolly"), "20100");
        // WEEDS / EERIE: de tweede E staat groen. Van de twee E's in de oplossing is er
        // dan nog één over, en die gaat naar de eerste E van de gok — niet naar de derde.
        assert_eq!(tekens("weeds", "eerie"), "12000");
        // Niets van de gok zit erin.
        assert_eq!(tekens("murky", "poets"), "00000");
    }

    #[test]
    fn de_dag_wisselt_om_zeven_uur_en_niet_om_middernacht() {
        let dag_van = |s: &str| {
            let t = DateTime::parse_from_rfc3339(s)
                .unwrap()
                .with_timezone(&Local);
            dagnummer(datum_van(t))
        };
        // 06:59 hoort nog bij gisteren, 07:00 bij vandaag. Getoetst in de lokale zone van
        // de machine, want dat is ook waar `datum_van` mee rekent.
        let offset = Local::now().offset().to_string();
        let met = |tijd: &str| format!("2026-08-20T{tijd}{offset}");
        assert_eq!(dag_van(&met("06:59:59")), 20_260_819);
        assert_eq!(dag_van(&met("07:00:00")), 20_260_820);
        assert_eq!(dag_van(&met("23:59:59")), 20_260_820);
        assert_eq!(dag_van(&met("00:30:00")), 20_260_819);
    }

    #[test]
    fn een_dagnummer_is_terug_te_lezen() {
        let d = NaiveDate::from_ymd_opt(2026, 8, 20).unwrap();
        assert_eq!(dagnummer(d), 20_260_820);
        assert_eq!(datum_van_dagnummer(20_260_820), Some(d));
        assert_eq!(datum_van_dagnummer(20_261_332), None); // maand 13, dag 32
        assert!(openbaar_op(20_260_820) > 0);
        assert_eq!(openbaar_op(0), 0);
    }

    #[test]
    fn alleen_spelen_levert_geen_punt_op() {
        let dag = [uitslag(20_260_820, peer(1), 2, true)];
        assert!(winnaars(&dag).is_empty(), "één speler is geen wedstrijd");
    }

    #[test]
    fn de_minste_pogingen_wint_en_gelijk_is_allebei_een_punt() {
        let dag = [
            uitslag(20_260_820, peer(1), 3, true),
            uitslag(20_260_820, peer(2), 5, true),
        ];
        assert_eq!(winnaars(&dag), vec![peer(1)]);

        let gelijk = [
            uitslag(20_260_820, peer(1), 4, true),
            uitslag(20_260_820, peer(2), 4, true),
            uitslag(20_260_820, peer(3), 6, false),
        ];
        assert_eq!(winnaars(&gelijk), vec![peer(1), peer(2)]);
    }

    #[test]
    fn niemand_opgelost_is_niemand_een_punt() {
        let dag = [
            uitslag(20_260_820, peer(1), 6, false),
            uitslag(20_260_820, peer(2), 6, false),
        ];
        assert!(winnaars(&dag).is_empty());
    }

    #[test]
    fn een_verliezer_kan_de_dag_wel_laten_meetellen() {
        // Twee spelers, één opgelost: de dag telt (er was een tegenstander) en de
        // oplosser krijgt zijn punt.
        let dag = [
            uitslag(20_260_820, peer(1), 4, true),
            uitslag(20_260_820, peer(2), 6, false),
        ];
        assert_eq!(winnaars(&dag), vec![peer(1)]);
    }

    #[test]
    fn het_scorebord_telt_per_dag_en_niet_per_uitslag() {
        // Twee dagen. Dag 1: peer 1 wint met 3. Dag 2: gelijkspel op 4. Dag 3: peer 2
        // speelde alleen en scoort dus niets.
        let uitslagen = [
            uitslag(20_260_818, peer(1), 3, true),
            uitslag(20_260_818, peer(2), 4, true),
            uitslag(20_260_819, peer(1), 4, true),
            uitslag(20_260_819, peer(2), 4, true),
            uitslag(20_260_820, peer(2), 2, true),
        ];
        let stand = standen(&uitslagen);
        assert_eq!(stand.len(), 2);
        assert_eq!(stand[0].peer, peer(1));
        assert_eq!(stand[0].punten, 2); // dag 1 gewonnen, dag 2 gelijk
        assert_eq!(stand[0].gespeeld, 2);
        assert_eq!(stand[1].peer, peer(2));
        assert_eq!(stand[1].punten, 1); // alleen dag 2
        assert_eq!(stand[1].gespeeld, 3);
        assert_eq!(stand[1].opgelost, 3);
    }

    #[test]
    fn per_dag_snijdt_op_de_dagovergang() {
        let uitslagen = [
            uitslag(20_260_819, peer(1), 3, true),
            uitslag(20_260_820, peer(1), 3, true),
            uitslag(20_260_820, peer(2), 3, true),
        ];
        let groepen = per_dag(&uitslagen);
        assert_eq!(groepen.len(), 2);
        assert_eq!(groepen[0].len(), 1);
        assert_eq!(groepen[1].len(), 2);
        assert!(per_dag(&[]).is_empty());
    }

    fn met_raadsel(map: &Path, oplossing: &str) -> Wordle {
        let mut w = Wordle::nieuw(map);
        let dag = w.huidige_dag();
        w.neem_op(Raadsel {
            dag,
            nummer: 1888,
            oplossing: oplossing.to_string(),
        });
        w
    }

    #[test]
    fn een_spel_van_begin_tot_eind_overleeft_een_herstart() {
        let map = std::env::temp_dir().join("fitcom-wordle-test-spel");
        let _ = std::fs::remove_dir_all(&map);
        std::fs::create_dir_all(&map).unwrap();

        let mut w = met_raadsel(&map, "murky");
        assert_eq!(w.gok("crane"), Gok::Verder);
        assert_eq!(w.gok("qwert"), Gok::Geweigerd("Not in the word list."));
        assert_eq!(w.gok("abc"), Gok::Geweigerd("Five letters, A to Z."));

        // Herstart: de gedane gok staat er nog, en het raadsel wordt niet overschreven.
        let mut w2 = met_raadsel(&map, "slate");
        let bord = w2.bord().expect("bord");
        assert_eq!(bord.rijen.len(), 1);
        assert_eq!(bord.rijen[0].woord, "CRANE");
        assert_eq!(bord.nummer, 1888);
        assert!(!bord.klaar);
        assert_eq!(
            bord.oplossing, None,
            "de oplossing lekt niet tijdens het spel"
        );

        assert_eq!(
            w2.gok("MURKY"),
            Gok::Klaar {
                dag: w2.huidige_dag(),
                pogingen: 2,
                gewonnen: true,
                patroon: "0100022222".into(),
            }
        );
        let bord = w2.bord().unwrap();
        assert!(bord.klaar && bord.gewonnen);
        assert_eq!(bord.oplossing.as_deref(), Some("MURKY"));
        assert_eq!(
            w2.gok("slate"),
            Gok::Geweigerd("Today's puzzle is already finished.")
        );
        assert_eq!(w2.te_melden().map(|t| (t.1, t.2)), Some((2, true)));

        let _ = std::fs::remove_dir_all(&map);
    }

    #[test]
    fn zes_keer_mis_is_ook_klaar() {
        let map = std::env::temp_dir().join("fitcom-wordle-test-verloren");
        let _ = std::fs::remove_dir_all(&map);
        std::fs::create_dir_all(&map).unwrap();

        let mut w = met_raadsel(&map, "murky");
        for _ in 0..5 {
            assert_eq!(w.gok("poets"), Gok::Verder);
        }
        assert_eq!(
            w.gok("poets"),
            Gok::Klaar {
                dag: w.huidige_dag(),
                pogingen: 6,
                gewonnen: false,
                patroon: "0".repeat(30),
            }
        );
        let _ = std::fs::remove_dir_all(&map);
    }

    #[test]
    fn zonder_raadsel_valt_er_niets_te_gokken() {
        let map = std::env::temp_dir().join("fitcom-wordle-test-leeg");
        let _ = std::fs::remove_dir_all(&map);
        std::fs::create_dir_all(&map).unwrap();
        let mut w = Wordle::nieuw(&map);
        assert!(w.bord().is_none());
        assert_eq!(
            w.gok("crane"),
            Gok::Geweigerd("Today's puzzle has not arrived yet.")
        );
        // En het ophaalverzoek komt precies één keer per kwartier langs.
        assert!(w.moet_ophalen().is_some());
        assert!(w.moet_ophalen().is_none());
        let _ = std::fs::remove_dir_all(&map);
    }

    /// De `+`-knop: die moet dwars door het kwartier heen, want hij bestaat juist voor het
    /// moment waarop je niet nóg een kwartier wilt wachten.
    #[test]
    fn handmatig_ophalen_wacht_het_kwartier_niet_af() {
        let map = std::env::temp_dir().join("fitcom-wordle-test-handmatig");
        let _ = std::fs::remove_dir_all(&map);
        std::fs::create_dir_all(&map).unwrap();
        let mut w = Wordle::nieuw(&map);

        assert!(w.moet_ophalen().is_some());
        assert!(w.moet_ophalen().is_none(), "de tik zit nu in zijn kwartier");
        assert!(
            w.nu_ophalen().is_some(),
            "de knop trekt zich daar niets van aan"
        );

        // Een oude reden mag niet blijven staan boven een poging die net begonnen is.
        w.fout = Some("iets van hiervoor".into());
        assert!(w.nu_ophalen().is_some());
        assert_eq!(w.fout, None);

        // Staat het raadsel er al, dan valt er niets te halen en is dat geen fout.
        w.neem_op(Raadsel {
            dag: w.huidige_dag(),
            nummer: 1,
            oplossing: "murky".into(),
        });
        w.fout = Some("iets van hiervoor".into());
        assert_eq!(w.nu_ophalen(), None);
        assert_eq!(
            w.fout.as_deref(),
            Some("iets van hiervoor"),
            "niets geprobeerd, dus ook niets om te wissen"
        );

        let _ = std::fs::remove_dir_all(&map);
    }

    /// Praat met nytimes.com, dus `--ignored` — zelfde patroon als de YouTube-test en de
    /// rooktest op de echte geluidskaart. Dit is de test die bewijst dat het eindpunt en
    /// de veldnamen kloppen; dat kun je niet offline nakijken, en fout betekent hier "er
    /// verschijnt nooit een kaart" zonder dat iemand ziet waarom.
    ///
    /// `cargo test -p fitcom --lib wordle -- --ignored --nocapture`
    #[test]
    #[ignore = "praat met nytimes.com"]
    fn haalt_het_echte_woord_van_de_dag_op() {
        let datum = datum_van(Local::now()).format("%Y-%m-%d").to_string();
        let r = haal_op(&datum).expect("raadsel ophalen");
        println!("dag {} nummer {} woord {:?}", r.dag, r.nummer, r.oplossing);
        assert!(geldige_oplossing(&r.oplossing));
        assert_eq!(r.dag, dagnummer(datum_van(Local::now())));
        assert!(r.nummer > 1000, "raadselnummer {}", r.nummer);
        // Een echt Wordle-woord staat ook in de gokwoordenlijst; is dat niet zo, dan is
        // dat geen fout (de lijst is een afdruk) maar wel iets om te weten.
        if !is_woord(&r.oplossing) {
            println!("let op: {:?} staat niet in onze woordenlijst", r.oplossing);
        }
    }
}
