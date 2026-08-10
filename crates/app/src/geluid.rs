//! Korte tonen bij het komen en gaan van anderen, en bij een stream die aan of uit gaat.
//!
//! # Waarom de tonen hier gemaakt worden en niet meegeleverd
//!
//! Een wav-bestand naast de exe zou "losse exe in een zip" breken, en ze in de binary
//! bakken zou een handvol bestandjes in de repo betekenen die niemand kan nalezen of
//! aanpassen. Een geluidje is hier een parametertabel: je leest de noten, de partialen en
//! de uitdoving als getallen, en je kunt ze aanpassen zonder audiogereedschap.
//!
//! # Waarom niet via de voice-uitvoer
//!
//! Die bestaat alleen tijdens een gesprek, en het eerste geluidje dat je wilt horen is
//! precies dat van je eigen deelname. Dus langs de mixer heen, rechtstreeks naar het
//! standaardapparaat van het systeem: op Windows `PlaySound` met de bytes in het geheugen,
//! op macOS `afplay`. Zelfde afweging als bij `notify.rs`: nul afhankelijkheden.
//!
//! Gevolg van die keuze, en het is een echte beperking: **deze tonen gaan niet door de
//! volumeregeling van het gesprek**, dus hebben ze hun eigen volume nodig — de volumemixer
//! van Windows kan de app alleen als geheel zachter zetten, en dan gaat de stem van je
//! vriend mee. Dat volume staat in `config.toml` (`[sound] volume`), net als de gekozen set.
//!
//! Niet-storen onderdrukt ze; dat besluit staat in `engine.rs::geluid`, want alleen de
//! motor weet in welke stand hij staat.
//!
//! # Hoe een geluidje in elkaar zit
//!
//! Eén [`Geluid`] is een handvol [`Toon`]-en op een tijdlijn. Elke toon is een grondtoon
//! met [`Partiaal`]-en erboven — veelvouden die niet heel hoeven te zijn, want juist
//! ónhele veelvouden geven een klank die naar een aangeslagen voorwerp klinkt in plaats van
//! naar een pieper. De [`Omhulling`] bepaalt of een toon *staat* (vlak, met een korte in- en
//! uitregeling: de klassieke set) of *uitdooft* (een aanslag en daarna exponentieel weg,
//! zoals alles wat je aanslaat).
//!
//! Dat is het hele apparaat. Er is één [`samples`]-functie voor alle sets; een set is niets
//! anders dan een andere tabel.

use std::sync::OnceLock;

/// 48 kHz, zoals de rest van de app. Ruim: de hoogste partiaal hieronder zit rond 7 kHz en
/// dat is een factor drie onder Nyquist, dus er aliast niets.
const SAMPLERATE: u32 = 48_000;

/// Waar elk geluidje op wordt gezet: niet zijn piek maar zijn *luidheid*, gemeten als in
/// [`luidheid`]. `0.15` is wat de klassieke set sinds 1.0.0 heeft, dus die verandert hier
/// niet van niveau — en elke nieuwe set komt op datzelfde niveau uit.
///
/// **Dat het luidheid is en niet de piek, is gemeten en niet bedacht.** Eerst stond hier een
/// piekgrens van 0,22 voor alle sets. Dat klinkt eerlijk maar is het niet: een aangeslagen
/// klank van 380 ms zit maar de eerste honderd milliseconde in de buurt van zijn piek, en
/// kwam bij dezelfde piek 5 tot 9 dB zachter uit dan een staande sinus van 120 ms. Van set
/// wisselen zou dan voelen alsof de geluidjes bijna weg waren. Zie `docs/OVERDRACHT.md`,
/// beslissing 28.
const DOEL_LUIDHEID: f32 = 0.15;

/// Harde bovengrens op de piek, ná het normaliseren op luidheid. Een uitdovende klank heeft
/// een hogere piek nodig om even luid te klinken, en die ruimte is er — dit is er alleen om
/// te garanderen dat er nooit iets vervormt, wat er ook in een tabel gezet wordt.
///
/// Ruim onder vol bereik, en de tonen zijn hoe dan ook zacht: dit is de piek vóór het
/// volume van de gebruiker, dat standaard op 0,7 staat.
const PIEK_PLAFOND: f32 = 0.6;

/// Waarover de luidheid gemeten wordt: ongeveer de integratietijd van het oor. Korter en je
/// meet de aanslag in plaats van de klank; langer en een lange stille staart telt mee alsof
/// hij zachter maakt wat er aan het begin gebeurde.
const LUIDHEID_VENSTER_MS: u32 = 200;

// ---------------------------------------------------------------- de bouwstenen

/// Eén sinus binnen een toon.
///
/// `ratio` mag onheel zijn — dat is precies het verschil tussen een klok en een
/// orgelpijp: een aangeslagen voorwerp heeft modes die geen hele veelvouden zijn, en het
/// oor hoort dat als "voorwerp" in plaats van als "pieper".
#[derive(Debug, Clone, Copy)]
struct Partiaal {
    /// Veelvoud van de grondtoon.
    ratio: f32,
    /// Absolute bijstelling in Hz, bovenop `ratio`. Twee partialen op dezelfde ratio met
    /// een paar hertz ertussen geven een langzame zweving, en dat is wat een klank
    /// "levend" maakt in plaats van synthetisch. In hertz en niet als ratio, want dan
    /// zweeft elke noot van een set even snel; met een ratio zou een hoge noot sneller
    /// zweven dan een lage en valt de familie uit elkaar.
    offset_hz: f32,
    /// Sterkte ten opzichte van de grondtoon.
    amp: f32,
    /// Eigen aanslagtijd. Hoge partialen mogen sneller inzetten dan lage; dat is wat een
    /// aanslag zijn "tik" geeft zonder dat er een echte hoek in de golf komt.
    aanslag_ms: u32,
    /// De tijdconstante van deze partiaal, als deel van die van de toon. `1.0` is gelijk
    /// op, `0.3` is drie keer zo snel weg. Hoge partialen die sneller wegvallen dan lage
    /// is wat elk aangeslagen voorwerp doet, en waar hout van metaal te onderscheiden is.
    /// Genegeerd bij een vlakke omhulling.
    tau_deel: f32,
}

impl Partiaal {
    /// De grondtoon, als partiaal geschreven zodat de mengcode maar één geval kent.
    const GROND: Self = Self {
        ratio: 1.0,
        offset_hz: 0.0,
        amp: 1.0,
        aanslag_ms: 0,
        tau_deel: 1.0,
    };
}

/// Voor een toon die alleen zijn grondtoon is.
const ALLEEN_GROND: [Partiaal; 1] = [Partiaal::GROND];

/// Wat de toonhoogte tijdens de toon doet.
#[derive(Debug, Clone, Copy)]
enum Glijden {
    Geen,
    /// `f(t) = naar_hz + (hz − naar_hz)·e^(−t/tau)`: hij schiet erheen en valt dan op zijn
    /// plek. Klinkt als iets dat *aankomt*.
    ///
    /// Bewust exponentieel en niet lineair: gelijkmatig glijden over een hele noot klinkt
    /// als een sirene, terwijl een buiging die in de eerste tientallen milliseconden af is
    /// klinkt als een toon die ergens naartoe wíl. Er heeft hier ook een `Lineair` gestaan;
    /// die is eruit omdat geen enkele set hem koos.
    Naartoe {
        naar_hz: f32,
        tau_ms: u32,
    },
}

/// Frequentiemodulatie: de klank van een aangeslagen metalen tong.
///
/// Eén modulator op de golf van elke partiaal, met een modulatiediepte die zelf uitdooft.
/// Dat laatste is het hele recept: de eerste tientallen milliseconden staan er zijbanden
/// naast de grondtoon en klinkt het hol en metaalachtig, en daarna zakt de diepte weg en
/// blijft er een vrijwel zuivere sinus over. Een klank die *verkleurt* terwijl hij wegvalt,
/// met één sinus meer werk.
///
/// Bij `ratio: 1.0` vallen de zijbanden op hele veelvouden van de grondtoon, dus het blijft
/// een toonhoogte houden in plaats van te klinken als een belletje — precies het verschil
/// met [`Geluidset::Glas`], dat zijn kleur uit ónhele partialen haalt.
#[derive(Debug, Clone, Copy)]
struct Fm {
    /// Verhouding van de modulator tot de grondtoon.
    ratio: f32,
    /// Modulatiediepte aan het begin, in radialen.
    index: f32,
    /// Waarmee die diepte uitdooft.
    tau_ms: u32,
}

/// Een korte ruisstoot onder de toon: het geluid van het *contact*, niet van de toon.
///
/// Zonder dit klinkt een aangeslagen voorwerp alsof de toon uit niets opdoemt. Met een
/// paar milliseconde laagdoorlatend gefilterde ruis erbij hoor je de hamer het hout raken.
#[derive(Debug, Clone, Copy)]
struct Ruis {
    /// Piek van de ruisstoot ten opzichte van de piek van de toon zelf, vóórdat het hele
    /// geluidje genormaliseerd wordt. `0.3` is dus "het contact is een derde van de klank".
    amp: f32,
    /// Afsnijfrequentie van twee gecascadeerde eenpolers. Eén pool laat nog hoorbaar sissen
    /// (−6 dB/oct); twee (−12) klinkt als een dof contact.
    fc_hz: f32,
    aanslag_ms: u32,
    tau_ms: u32,
}

#[derive(Debug, Clone, Copy)]
enum Omhulling {
    /// Vlak, met een lineaire in- en uitregeling van `fade_ms`. Een toon die *staat*.
    /// Dit is wat de klassieke set doet.
    Vlak { fade_ms: u32 },
    /// Aanslag per partiaal (zie [`Partiaal::aanslag_ms`]), daarna exponentieel weg met
    /// `tau_ms`, en over de laatste `release_ms` met een raised cosine naar precies nul.
    /// Een toon die *wegvalt*.
    ///
    /// Die release is niet cosmetisch: een exponent bereikt nooit nul, en het verschil
    /// tussen "wat er nog staat" en stilte is een sprong, en een sprong is een tik.
    Aanslag { tau_ms: u32, release_ms: u32 },
}

/// Eén toon op de tijdlijn van een geluidje. Twee tonen mogen overlappen; dat is hoe een
/// interval samen gaat klinken in plaats van na elkaar.
#[derive(Debug, Clone, Copy)]
struct Toon {
    /// Vanaf het begin van het geluidje.
    begin_ms: u32,
    duur_ms: u32,
    hz: f32,
    glijden: Glijden,
    /// Sterkte ten opzichte van de andere tonen in hetzelfde geluidje. De absolute hoogte
    /// wordt aan het eind gezet, zie [`samples`] — hier gaat het alleen om de verhouding.
    amp: f32,
    omhulling: Omhulling,
    /// Het **volledige** spectrum, grondtoon inbegrepen — er wordt er geen bij verzonnen.
    ///
    /// Dat was eerst wel zo (de mengcode plakte er een vaste [`Partiaal::GROND`] voor), en
    /// dat is precies de reden dat het nu niet meer zo is: die vaste grondtoon had
    /// `aanslag_ms: 0`, dus bij een aanslag-omhulling sprong hij op de tweede sample van
    /// stil naar vol. Een sprong is een tik, en die had in elke set behalve de klassieke
    /// gezeten. De aanslag van de grondtoon hóórt bij de tabel, niet bij de mengcode.
    partialen: &'static [Partiaal],
    ruis: Option<Ruis>,
    fm: Option<Fm>,
}

impl Toon {
    /// Eén aangeslagen klank: een aanslag, exponentieel uitdoven, en over de laatste
    /// `release_ms` naar precies nul. De vorm die alles heeft wat je aanslaat.
    #[allow(clippy::too_many_arguments)]
    const fn aangeslagen(
        begin_ms: u32,
        duur_ms: u32,
        hz: f32,
        tau_ms: u32,
        release_ms: u32,
        amp: f32,
        partialen: &'static [Partiaal],
        ruis: Option<Ruis>,
    ) -> Self {
        Self {
            begin_ms,
            duur_ms,
            hz,
            glijden: Glijden::Geen,
            amp,
            omhulling: Omhulling::Aanslag { tau_ms, release_ms },
            partialen,
            ruis,
            fm: None,
        }
    }

    /// Een vlakke toon zonder partialen: de klassieke pieper.
    const fn vlak(begin_ms: u32, duur_ms: u32, hz: f32) -> Self {
        Self {
            begin_ms,
            duur_ms,
            hz,
            glijden: Glijden::Geen,
            amp: 1.0,
            omhulling: Omhulling::Vlak { fade_ms: 6 },
            partialen: &ALLEEN_GROND,
            ruis: None,
            fm: None,
        }
    }
}

// ---------------------------------------------------------------- de gebeurtenissen

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Geluid {
    /// Jij neemt deel aan het gesprek.
    EigenJoin,
    /// Jij verlaat het gesprek.
    EigenLeave,
    /// Iemand anders komt erbij.
    PeerJoin,
    /// Iemand anders gaat eruit.
    PeerLeave,
    /// Iemand zet een scherm of camera aan.
    StreamAan,
    /// Iemand zet dat weer uit.
    StreamUit,
}

impl Geluid {
    pub const ALLE: [Self; 6] = [
        Self::EigenJoin,
        Self::EigenLeave,
        Self::PeerJoin,
        Self::PeerLeave,
        Self::StreamAan,
        Self::StreamUit,
    ];

    /// De naam op de IPC-grens en in de log. Stabiel: hier hangt de proefknop aan.
    pub fn naam(self) -> &'static str {
        match self {
            Self::EigenJoin => "eigen-join",
            Self::EigenLeave => "eigen-leave",
            Self::PeerJoin => "peer-join",
            Self::PeerLeave => "peer-leave",
            Self::StreamAan => "stream-aan",
            Self::StreamUit => "stream-uit",
        }
    }

    /// Wat er op de proefknop staat. Engels, zoals alles wat de gebruiker leest.
    pub fn label(self) -> &'static str {
        match self {
            Self::EigenJoin => "You join",
            Self::EigenLeave => "You leave",
            Self::PeerJoin => "Someone joins",
            Self::PeerLeave => "Someone leaves",
            Self::StreamAan => "Sharing starts",
            Self::StreamUit => "Sharing stops",
        }
    }

    pub fn van_naam(s: &str) -> Option<Self> {
        Self::ALLE.into_iter().find(|g| g.naam() == s)
    }

    fn plek(self) -> usize {
        Self::ALLE.iter().position(|g| *g == self).unwrap_or(0)
    }
}

// ---------------------------------------------------------------- de sets

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Geluidset {
    /// De eerste set: kale sinustonen met een lineaire fade. Blijft bestaan omdat hij
    /// draait en omdat een gewijzigd geluid bij een update niemand hoort te verrassen.
    Klassiek,
    /// Aangeslagen glas: onhele partialen, een heldere aanzet die naar warm verkleurt.
    Glas,
    /// Aangeslagen hout: kort, droog, met het contact van de hamer eronder.
    Hout,
    /// Een elektrische piano: hol bij de aanslag, zuiver als hij wegvalt.
    Toets,
}

impl Geluidset {
    pub const ALLE: [Self; 4] = [Self::Klassiek, Self::Glas, Self::Hout, Self::Toets];

    /// Wat een config zonder `[sound]`-tabel krijgt, en waar een onbekende naam op
    /// terugvalt. Bewust de bestaande set: wie bijwerkt hoort te horen wat hij gewend is,
    /// en de nieuwe sets staan één klik verderop in de instellingen.
    pub const STANDAARD: Self = Self::Klassiek;

    /// De naam in `config.toml` en op de IPC-grens.
    pub fn naam(self) -> &'static str {
        match self {
            Self::Klassiek => "classic",
            Self::Glas => "glass",
            Self::Hout => "wood",
            Self::Toets => "keys",
        }
    }

    /// De naam op het kaartje in de instellingen. Engels, kort.
    pub fn label(self) -> &'static str {
        match self {
            Self::Klassiek => "Classic",
            Self::Glas => "Glass",
            Self::Hout => "Wood",
            Self::Toets => "Keys",
        }
    }

    /// De regel eronder. Eén zin, en hij moet iets zeggen over hoe het klinkt.
    pub fn beschrijving(self) -> &'static str {
        match self {
            Self::Klassiek => "Plain two-note beeps. What the app shipped with.",
            Self::Glas => "Struck glass. A brief shimmer that settles into a warm ring.",
            Self::Hout => "Wooden mallet. Dry and warm, over before you notice it.",
            Self::Toets => "Electric piano. Hollow on the strike, pure as it fades.",
        }
    }

    pub fn van_naam(s: &str) -> Option<Self> {
        Self::ALLE.into_iter().find(|g| g.naam() == s)
    }

    /// Hoe luid deze gebeurtenis mag klinken, als deel van [`DOEL_LUIDHEID`].
    ///
    /// Dit is dus geen correctie meer op de klankvorm — dat doet [`normaliseer`] al — maar
    /// een bewuste rangorde: een stream van iemand anders hoort zachter te zijn dan je eigen
    /// deelname. `1.0` betekent "even luid als de klassieke set", `0.7` betekent "hoorbaar
    /// op de achtergrond".
    fn gewicht(self, g: Geluid) -> f32 {
        match self {
            // De oorspronkelijke set: allemaal even hard, want dat was hij ook.
            Self::Klassiek => {
                let _ = g;
                1.0
            }
            // Wat iemand anders doet mag zachter, en een stream nog wat zachter — dat is
            // een rangorde in belang, niet in hoorbaarheid: de klankkleur doet het
            // onderscheid, dit doet de opdringerigheid. De streamtonen zitten bovendien
            // rond 1 kHz, waar het oor een paar dB gevoeliger is dan rond 400 Hz, dus
            // gelijke piek zou daar juist harder klinken.
            // Dezelfde rangorde voor alle drie de nieuwe sets: wat iemand anders doet mag
            // zachter, en een stream nog wat zachter. Dat is een keuze over belang, niet
            // over hoorbaarheid — het onderscheid zelf zit in de klankkleur, zodat het ook
            // op een laag volume overeind blijft.
            Self::Glas | Self::Hout | Self::Toets => match g {
                Geluid::EigenJoin | Geluid::EigenLeave => 1.00,
                Geluid::PeerJoin | Geluid::PeerLeave => 0.77,
                Geluid::StreamAan | Geluid::StreamUit => 0.68,
            },
        }
    }

    /// De noten van deze set voor deze gebeurtenis.
    ///
    /// Eigenaar in plaats van `&'static`: dit loopt precies één keer per (set,
    /// gebeurtenis) — [`gecachte_samples`] bewaart de uitkomst — dus een allocatie is hier
    /// gratis, en de tabellen mogen er rekenen in plaats van alleen constanten opsommen.
    fn tonen(self, g: Geluid) -> Vec<Toon> {
        match self {
            Self::Klassiek => klassiek(g),
            Self::Glas => glas(g),
            Self::Hout => hout(g),
            Self::Toets => toets(g),
        }
    }
}

// ---------------------------------------------------------------- Glas

/// De partialen van een aangeslagen glazen staaf.
///
/// **2,760 en 5,404 zijn geen willekeurige getallen**: het zijn de modeverhoudingen van een
/// aangeslagen buis. Precies omdat ze onheel zijn hoort het oor een voorwerp in plaats van
/// een toongenerator — bij hele veelvouden (1, 2, 3) klinkt het als een orgel.
///
/// De tweede regel is de belangrijkste: de partiaal op 2,760 dooft ruim twee keer zo snel
/// weg als de grondtoon (`tau_deel` 0,45) en die op 5,404 bijna zes keer zo snel (0,17).
/// Daardoor is de klank de eerste tientallen milliseconden helder en daarna warm — een
/// aanslag die *verkleurt* in plaats van alleen zachter worden. Dat is wat elk aangeslagen
/// voorwerp doet, en zonder dat klinkt additieve synthese als een orgelpijp.
///
/// De twee partialen op ratio 1,000 met 2,5 Hz ertussen zweven met een periode van 400 ms,
/// dus je hoort binnen één noot de eerste helft van één langzame golf: levend, en met
/// 2,5 Hz ver onder de ruwheidsgrens van ~20 Hz.
const GLAS_HELDER: [Partiaal; 4] = [
    Partiaal {
        ratio: 1.0,
        offset_hz: 0.0,
        amp: 1.0,
        aanslag_ms: 9,
        tau_deel: 1.0,
    },
    Partiaal {
        ratio: 1.0,
        offset_hz: 2.5,
        amp: 0.55,
        aanslag_ms: 9,
        tau_deel: 1.0,
    },
    Partiaal {
        ratio: 2.760,
        offset_hz: 0.0,
        amp: 0.32,
        aanslag_ms: 5,
        tau_deel: 0.45,
    },
    Partiaal {
        ratio: 5.404,
        offset_hz: 0.0,
        amp: 0.11,
        aanslag_ms: 3,
        tau_deel: 0.17,
    },
];

/// Dezelfde staaf, maar donkerder: de hoogste partiaal weg en de middelste zwakker.
///
/// Voor wat *iemand anders* doet. Het oor leest een dovere, minder heldere klank als verder
/// weg, en dat is precies de betekenis van "niet jij". Onderscheid via klankkleur en niet via
/// volume, want dan blijft het herkenbaar als je het volume laag zet.
const GLAS_DONKER: [Partiaal; 3] = [
    GLAS_HELDER[0],
    GLAS_HELDER[1],
    Partiaal {
        amp: 0.20,
        tau_deel: 0.35,
        ..GLAS_HELDER[2]
    },
];

/// En droger, voor het aan- en uitzetten van een stream: kortere aanslag, minder boventoon.
/// Een ander voorwerp, niet dezelfde klank een terts hoger.
const GLAS_STREAM: [Partiaal; 3] = [
    Partiaal {
        aanslag_ms: 6,
        ..GLAS_HELDER[0]
    },
    Partiaal {
        aanslag_ms: 6,
        amp: 0.50,
        ..GLAS_HELDER[1]
    },
    Partiaal {
        amp: 0.18,
        aanslag_ms: 4,
        tau_deel: 0.30,
        ..GLAS_HELDER[2]
    },
];

/// Aangeslagen glas. Twee overlappende klanken voor je eigen gebeurtenissen, één voor die
/// van iemand anders — het *aantal* klanken is daarmee de eerste aanwijzing, nog vóór de
/// toonhoogte.
///
/// De intervallen zijn zuiver gestemd (3:2 voor de kwint, 4:3 voor de kwart) in plaats van
/// gelijkzwevend: bij twee tonen die samen klinken hoor je het verschil, en zuiver klinkt
/// rustiger.
fn glas(g: Geluid) -> Vec<Toon> {
    // 440 en 660 zijn een zuivere kwint; de tweede noot valt in de nagalm van de eerste.
    const A4: f32 = 440.0;
    const E5: f32 = 660.0;
    // Een kwart hoger, en een register erboven: het gaat over iets anders.
    const G5: f32 = 783.99;
    const C6: f32 = 1045.32;

    let bel = |begin, duur, hz, tau, release, amp, partialen| {
        Toon::aangeslagen(begin, duur, hz, tau, release, amp, partialen, None)
    };
    match g {
        // Stijgend, en de tweede noot iets sterker: het gebaar wijst omhoog.
        Geluid::EigenJoin => vec![
            bel(0, 380, A4, 130, 80, 1.00, &GLAS_HELDER),
            bel(105, 275, E5, 165, 80, 1.10, &GLAS_HELDER),
        ],
        // De spiegel, maar hij *ontspant*: de laatste noot klinkt langer na (185 tegen 165)
        // en is zachter. Aflopend in toonhoogte én in sterkte leest als "klaar".
        Geluid::EigenLeave => vec![
            bel(0, 380, E5, 130, 80, 1.00, &GLAS_HELDER),
            bel(105, 275, A4, 185, 80, 0.90, &GLAS_HELDER),
        ],
        // Eén donkere klank op de aankomsttoon van je eigen join: dezelfde familie, maar
        // onmiskenbaar niet jij.
        Geluid::PeerJoin => vec![bel(0, 260, E5, 140, 70, 1.00, &GLAS_DONKER)],
        Geluid::PeerLeave => vec![bel(0, 260, A4, 150, 70, 1.00, &GLAS_DONKER)],
        Geluid::StreamAan => vec![
            bel(0, 300, G5, 85, 60, 1.00, &GLAS_STREAM),
            bel(80, 220, C6, 110, 60, 1.08, &GLAS_STREAM),
        ],
        Geluid::StreamUit => vec![
            bel(0, 300, C6, 85, 60, 1.00, &GLAS_STREAM),
            bel(80, 220, G5, 125, 60, 0.92, &GLAS_STREAM),
        ],
    }
}

// ---------------------------------------------------------------- Hout

/// De partialen van een aangeslagen houten staaf, per toonhoogte.
///
/// **1 : 3,93 : 9,55 zijn de modeverhoudingen van een vrij opgelegde balk** — de stemming
/// van een marimba of kalimba. Dat is een heel andere reeks dan die van glas hierboven
/// (1 : 2,76 : 5,40, een aangeslagen buis), en dát is waarom deze twee sets niet op elkaar
/// lijken: het zijn twee verschillende voorwerpen, niet twee kleurtjes.
///
/// Waarom er per toonhoogte een eigen tabel staat in plaats van één: een hogere balk klinkt
/// mínder helder, niet meer. De sterkte van de tweede mode gaat daarom omlaag met de
/// grondtoon (`0,30 × 700 / f0`, begrensd op 0,10–0,30), en de derde mode doet alleen mee
/// zolang hij onder ~5 kHz blijft — daarboven is hij scherp in plaats van helder, en zou hij
/// bij honderd keer per avond gaan irriteren.
const HOUT_C5: [Partiaal; 3] = [
    Partiaal {
        ratio: 1.0,
        offset_hz: 0.0,
        amp: 1.0,
        aanslag_ms: 4,
        tau_deel: 1.0,
    },
    Partiaal {
        ratio: 3.93,
        offset_hz: 0.0,
        amp: 0.30,
        aanslag_ms: 4,
        tau_deel: 0.38,
    },
    Partiaal {
        ratio: 9.55,
        offset_hz: 0.0,
        amp: 0.06,
        aanslag_ms: 4,
        tau_deel: 0.13,
    },
];

/// G5: de derde mode zou hier op 7,5 kHz komen en valt dus weg.
const HOUT_G5: [Partiaal; 2] = [
    HOUT_C5[0],
    Partiaal {
        amp: 0.268,
        ..HOUT_C5[1]
    },
];

/// De gebogen staaf van het aan- en uitzetten van een stream, en die is met opzet níet
/// hetzelfde voorwerp: een *heel* veelvoud (2,0) in plaats van de 3,93 van een houten balk.
/// Dat maakt de klank zuiverder en gladder, en dat is precies de bedoeling — een stream is
/// geen persoon die binnenkomt, dus hij mag ook niet als dezelfde slag klinken.
const HOUT_GEBOGEN: [Partiaal; 2] = [
    Partiaal {
        ratio: 1.0,
        offset_hz: 0.0,
        amp: 1.0,
        aanslag_ms: 12,
        tau_deel: 1.0,
    },
    Partiaal {
        ratio: 2.0,
        offset_hz: 0.0,
        amp: 0.42,
        aanslag_ms: 12,
        tau_deel: 0.63,
    },
];

/// Het contact van de hamer: een paar milliseconde dof gefilterde ruis onder de aanslag.
///
/// Dit is wat "aangeslagen hout" van "een synthesizer die hout nadoet" scheidt. Zonder deze
/// laag doemt de toon uit niets op; met een tik van elf milliseconde eronder hoor je iets
/// wat érgens tegenaan komt. Op 900 Hz afgesneden en twee polen diep, dus het is een bonk en
/// geen sis.
const HOUT_TIK: Ruis = Ruis {
    amp: 0.28,
    fc_hz: 900.0,
    aanslag_ms: 3,
    tau_ms: 11,
};

/// Aangeslagen hout: de rustigste van de vier. Kort, droog, warm, en zonder de nagalm van
/// glas — hij is weg voordat je erop gelet hebt.
fn hout(g: Geluid) -> Vec<Toon> {
    const C5: f32 = 523.25;
    const G5: f32 = 784.00;
    const A5: f32 = 880.00;
    const C6: f32 = 1046.50;

    let bar = |begin, duur, hz, tau, amp, partialen| {
        Toon::aangeslagen(begin, duur, hz, tau, 25, amp, partialen, Some(HOUT_TIK))
    };
    match g {
        // Stijgende kwint. De tweede noot klinkt langer na dan de eerste: het gebaar staat
        // open aan het eind, en dat leest als "erbij gekomen".
        Geluid::EigenJoin => vec![
            bar(0, 150, C5, 85, 1.00, &HOUT_C5),
            bar(95, 185, G5, 120, 1.00, &HOUT_G5),
        ],
        // Dalende kwint, en de laatste noot juist *korter* — gedempt, dus "dicht".
        Geluid::EigenLeave => vec![
            bar(0, 170, G5, 100, 1.00, &HOUT_G5),
            bar(95, 130, C5, 75, 1.00, &HOUT_C5),
        ],
        // Eén slag in plaats van twee: dat is de eerste aanwijzing dat het iemand anders is.
        Geluid::PeerJoin => vec![bar(0, 200, G5, 90, 1.00, &HOUT_G5)],
        Geluid::PeerLeave => vec![bar(0, 210, C5, 95, 1.00, &HOUT_C5)],
        // Geen twee slagen maar één gebogen toon, en zonder hamertik: een ander sóort
        // gebaar, niet dezelfde klank getransponeerd. De buiging is binnen veertig
        // milliseconde af, dus je hoort hem ergens naartoe gaan en niet glijden.
        Geluid::StreamAan => vec![Toon {
            glijden: Glijden::Naartoe {
                naar_hz: C6,
                tau_ms: 40,
            },
            ..Toon::aangeslagen(0, 190, A5, 95, 14, 1.00, &HOUT_GEBOGEN, None)
        }],
        Geluid::StreamUit => vec![Toon {
            glijden: Glijden::Naartoe {
                naar_hz: A5,
                tau_ms: 40,
            },
            ..Toon::aangeslagen(0, 170, C6, 80, 14, 1.00, &HOUT_GEBOGEN, None)
        }],
    }
}

// ---------------------------------------------------------------- Toets

/// De modulator van een elektrische piano: even diep bij de aanslag, weg binnen vijftig
/// milliseconde.
///
/// `ratio: 1.0` houdt de zijbanden op hele veelvouden, dus dit blijft een noot met een
/// duidelijke toonhoogte — anders dan glas, dat juist onheel is. Index 2,3 is diep genoeg
/// voor de holle "tine" aan het begin en laag genoeg om niet te gaan rammelen.
const TOETS_FM: Fm = Fm {
    ratio: 1.0,
    index: 2.3,
    tau_ms: 48,
};

/// Dezelfde tong, maar dover: de modulatie is minder diep en is sneller weg, dus de holle
/// metaalklank aan het begin is er nauwelijks.
///
/// Voor wat *iemand anders* doet, en dat is precies het middel dat [`GLAS_DONKER`] ook
/// gebruikt: het oor leest een dovere klank als verder weg. Zonder dit was Keys de enige van
/// de vier sets zonder verschil vanaf de eerste sample — de eerste 120 ms van "iemand komt
/// erbij" was meetbaar dezelfde golf als die van je eigen deelname (correlatie 0,9999), en
/// dan hangt het onderscheid er volledig aan dat de tweede noot niet komt. Dat weet je pas
/// 120 ms later. Gevonden in de review, met de meting erbij.
const TOETS_FM_DONKER: Fm = Fm {
    ratio: 1.0,
    index: 1.5,
    tau_ms: 26,
};

/// Eén partiaal is genoeg: alle kleur komt van de modulator hierboven. De goedkoopste van de
/// vier sets, en klanklijk de warmste.
const TOETS_PARTIALEN: [Partiaal; 2] = [
    Partiaal {
        ratio: 1.0,
        offset_hz: 0.0,
        amp: 1.0,
        aanslag_ms: 5,
        tau_deel: 1.0,
    },
    // Een tweede, 1,8 Hz ernaast: een zweving van ruim een halve seconde, dus je hoort er
    // net het begin van. Hetzelfde middel als bij glas, maar langzamer.
    Partiaal {
        ratio: 1.0,
        offset_hz: 1.8,
        amp: 0.45,
        aanslag_ms: 5,
        tau_deel: 1.0,
    },
];

/// Een elektrische piano. Hol en warm bij de aanslag, zuiver als hij wegvalt — de klank die
/// het verst van de andere drie af staat: niet kaal zoals klassiek, niet onheel zoals glas,
/// en met een veel langere nagalm dan hout.
fn toets(g: Geluid) -> Vec<Toon> {
    const C4: f32 = 261.63;
    const F4: f32 = 349.23;
    const A4: f32 = 440.00;
    const C5: f32 = 523.25;

    let tine = |begin, duur, hz, tau, amp| Toon {
        fm: Some(TOETS_FM),
        ..Toon::aangeslagen(begin, duur, hz, tau, 60, amp, &TOETS_PARTIALEN, None)
    };
    // Dezelfde noot, dovere klank. Zie `TOETS_FM_DONKER` voor waarom dit er is.
    let dof = |begin, duur, hz, tau, amp| Toon {
        fm: Some(TOETS_FM_DONKER),
        ..Toon::aangeslagen(begin, duur, hz, tau, 60, amp, &TOETS_PARTIALEN, None)
    };
    match g {
        // Een kwart omhoog, laag in het register: warm en niet opdringerig.
        Geluid::EigenJoin => vec![tine(0, 420, C4, 150, 1.00), tine(120, 300, F4, 190, 1.05)],
        Geluid::EigenLeave => vec![tine(0, 420, F4, 150, 1.00), tine(120, 300, C4, 210, 0.90)],
        // Eén dovere noot op de *aankomsttoon* van het eigen gebaar — precies wat
        // [`Geluidset::Glas`] ook doet. Drie aanwijzingen die alle drie vanaf de eerste
        // sample gelden: één noot in plaats van twee, een dovere klank, en een andere
        // toonhoogte dan waar je eigen gebeurtenis begint.
        //
        // Dat laatste stond er eerst niet — peer-join begon op C4, de grondtoon van
        // eigen-join — en toen waren de eerste 120 ms meetbaar dezelfde golf (correlatie
        // 0,9999 tegen onder 0,15 bij de andere drie sets). Een octaaf naar beneden was de
        // verleiding en zou fout zijn geweest: `luidheid` weegt niet naar frequentie, dus
        // dezelfde RMS een octaaf lager klinkt merkbaar zachter dan de rest van de set.
        // De aankomsttoon lost het op zonder het register te verlaten.
        Geluid::PeerJoin => vec![dof(0, 300, F4, 160, 1.00)],
        Geluid::PeerLeave => vec![dof(0, 300, C4, 170, 1.00)],
        // Hoger en korter, en met een kleine terts: het gaat over beeld, niet over stemmen.
        Geluid::StreamAan => vec![tine(0, 300, A4, 100, 1.00), tine(85, 215, C5, 120, 1.05)],
        Geluid::StreamUit => vec![tine(0, 300, C5, 100, 1.00), tine(85, 215, A4, 130, 0.92)],
    }
}

/// De oorspronkelijke set, letterlijk dezelfde frequenties en duren als in 1.0.0: twee
/// vlakke sinustonen achter elkaar, oplopend voor "erbij" en aflopend voor "eraf".
fn klassiek(g: Geluid) -> Vec<Toon> {
    match g {
        Geluid::EigenJoin => vec![Toon::vlak(0, 90, 587.0), Toon::vlak(90, 130, 880.0)],
        Geluid::EigenLeave => vec![Toon::vlak(0, 90, 880.0), Toon::vlak(90, 130, 587.0)],
        Geluid::PeerJoin => vec![Toon::vlak(0, 120, 880.0)],
        Geluid::PeerLeave => vec![Toon::vlak(0, 120, 587.0)],
        Geluid::StreamAan => vec![Toon::vlak(0, 60, 1046.0), Toon::vlak(60, 90, 1318.0)],
        Geluid::StreamUit => vec![Toon::vlak(0, 60, 1318.0), Toon::vlak(60, 90, 1046.0)],
    }
}

// ---------------------------------------------------------------- synthese

/// Rekent één geluidje uit, op de doelluidheid. Het volume van de
/// gebruiker komt er pas in [`wav`] bij, zodat de tests over de klank gaan en niet over de
/// stand van de schuif.
fn samples(set: Geluidset, g: Geluid) -> Vec<f32> {
    let tonen = set.tonen(g);
    let einde_ms = tonen
        .iter()
        .map(|t| t.begin_ms + t.duur_ms)
        .max()
        .unwrap_or(0);
    let totaal = ms_naar_samples(einde_ms);
    let mut uit = vec![0.0f32; totaal];

    for toon in &tonen {
        meng(&mut uit, toon);
    }
    normaliseer(&mut uit, DOEL_LUIDHEID * set.gewicht(g));
    uit
}

fn ms_naar_samples(ms: u32) -> usize {
    (u64::from(SAMPLERATE) * u64::from(ms) / 1000) as usize
}

/// Telt één toon — zijn partialen en zijn eventuele ruisstoot — bij het geluidje op.
///
/// De fase wordt per partiaal doorgeteld in plaats van uit `sin(2π f t)` berekend. Met een
/// glijdende frequentie is `t` niet meer de juiste hoek: dan springt de fase op het moment
/// dat de frequentie verandert, en dat hoor je als een tik. Doortellen geeft een continue
/// fase, en dat is wat glijden vloeiend maakt.
fn meng(uit: &mut [f32], toon: &Toon) {
    let begin = ms_naar_samples(toon.begin_ms);
    let lengte = ms_naar_samples(toon.duur_ms);
    if lengte == 0 {
        return;
    }
    let alle = toon.partialen;
    // Door de som gedeeld, zodat een klank met meer partialen niet ook automatisch harder
    // is: een partiaal erbij hoort de kléur te veranderen, niet het niveau.
    let som: f32 = alle.iter().map(|p| p.amp).sum::<f32>().max(1e-6);
    let mut fase = vec![0.0f32; alle.len()];

    let ruis = toon.ruis.map(|r| ruisstoot(&r, lengte));
    let mut mod_fase = 0.0f32;

    for i in 0..lengte {
        let plek = begin + i;
        if plek >= uit.len() {
            break;
        }
        let hz = toonhoogte(toon, i);
        let t = i as f32 / SAMPLERATE as f32;

        // De modulator, als deze toon er een heeft. Zijn diepte dooft uit, dus de klank
        // begint hol en eindigt zuiver.
        let buiging = match toon.fm {
            None => 0.0,
            Some(fm) => {
                mod_fase += std::f32::consts::TAU * hz * fm.ratio / SAMPLERATE as f32;
                let tau = (fm.tau_ms as f32 / 1000.0).max(0.001);
                fm.index * (-t / tau).exp() * mod_fase.sin()
            }
        };

        let mut waarde = 0.0f32;
        for (p, f) in alle.iter().zip(fase.iter_mut()) {
            *f += std::f32::consts::TAU * (hz * p.ratio + p.offset_hz) / SAMPLERATE as f32;
            waarde += (*f + buiging).sin() * p.amp * omhulling(toon.omhulling, i, lengte, p);
        }
        let mut sample = waarde / som;
        if let Some(r) = &ruis {
            sample += r[i];
        }
        uit[plek] += sample * toon.amp;
    }
}

/// De frequentie op sample `i`.
fn toonhoogte(toon: &Toon, i: usize) -> f32 {
    match toon.glijden {
        Glijden::Geen => toon.hz,
        Glijden::Naartoe { naar_hz, tau_ms } => {
            let t = i as f32 / SAMPLERATE as f32;
            let tau = (tau_ms as f32 / 1000.0).max(0.001);
            naar_hz + (toon.hz - naar_hz) * (-t / tau).exp()
        }
    }
}

/// De ruisstoot van één toon, al genormaliseerd en van zijn omhulling voorzien.
///
/// De volgorde is dwingend: filteren, dán op piek 1 normaliseren, dán de omhulling erover.
/// Andersom is `amp` niet meer te lezen als "hoe hard het contact is ten opzichte van de
/// toon" (de filterversterking hangt van `fc_hz` af), en zou de eerste sample niet nul zijn
/// — de filterstaat begint op nul maar zijn eerste uitvoer niet.
fn ruisstoot(r: &Ruis, lengte: usize) -> Vec<f32> {
    // Vaste zaadwaarde: dezelfde ruis bij elke start. Anders klinkt hetzelfde geluidje elke
    // keer een beetje anders — bij een tikje van tien milliseconde onhoorbaar, maar het zou
    // ook de tests van run tot run laten verschillen, en een test die soms faalt is erger
    // dan geen test.
    let mut staat: u32 = 0x2545_F491;
    let a = 1.0 - (-std::f32::consts::TAU * r.fc_hz / SAMPLERATE as f32).exp();
    let (mut y1, mut y2) = (0.0f32, 0.0f32);

    let mut ruw = Vec::with_capacity(lengte);
    for _ in 0..lengte {
        staat = staat.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let x = (staat >> 8) as f32 / 8_388_608.0 - 1.0;
        y1 += a * (x - y1);
        y2 += a * (y1 - y2);
        ruw.push(y2);
    }

    let piek = ruw.iter().fold(0.0f32, |m, s| m.max(s.abs())).max(1e-9);
    let aanslag = ms_naar_samples(r.aanslag_ms).max(1);
    let tau = (r.tau_ms as f32 / 1000.0).max(0.001);
    for (i, s) in ruw.iter_mut().enumerate() {
        let t = i as f32 / SAMPLERATE as f32;
        *s = *s / piek * r.amp * raised_cosine(i, aanslag) * (-t / tau).exp();
    }
    ruw
}

/// Een raised cosine van 0 naar 1 over `lengte` samples, en daarna 1. Precies 0 op `i = 0`,
/// en zonder hoek — een rechte lijn heeft er een aan het begin, en een hoek is een tik.
fn raised_cosine(i: usize, lengte: usize) -> f32 {
    if i >= lengte {
        return 1.0;
    }
    0.5 - 0.5 * (std::f32::consts::PI * i as f32 / lengte as f32).cos()
}

/// De sterkte van de omhulling op sample `i` van `lengte`, voor deze partiaal.
fn omhulling(env: Omhulling, i: usize, lengte: usize, p: &Partiaal) -> f32 {
    match env {
        Omhulling::Vlak { fade_ms } => {
            let fade = ms_naar_samples(fade_ms).clamp(1, lengte.max(2) / 2);
            let omhoog = (i as f32 / fade as f32).min(1.0);
            let omlaag = ((lengte - i) as f32 / fade as f32).min(1.0);
            omhoog * omlaag
        }
        Omhulling::Aanslag { tau_ms, release_ms } => {
            let aanslag = ms_naar_samples(p.aanslag_ms).max(1);
            let tau = (tau_ms as f32 / 1000.0 * p.tau_deel.max(0.01)).max(0.001);
            let t = i as f32 / SAMPLERATE as f32;
            let weg = (-t / tau).exp();
            // De staart naar precies nul dwingen, met dezelfde vorm als de aanslag: een
            // exponent komt er nooit, en van "wat er nog staat" naar stilte is een sprong.
            let release = ms_naar_samples(release_ms).max(1).min(lengte);
            let over = lengte - i;
            let uit = if over < release {
                raised_cosine(over, release)
            } else {
                1.0
            };
            raised_cosine(i, aanslag) * weg * uit
        }
    }
}

/// De luidheid van een geluidje: de hoogste RMS over een schuivend venster van
/// [`LUIDHEID_VENSTER_MS`].
///
/// Waarom niet de piek en waarom niet de RMS over het hele bestand: de piek zegt niets over
/// hoe hard iets *klinkt* (één sample kan de piek zetten), en RMS over het hele bestand
/// rekent een lange stille nagalm mee alsof die het begin zachter maakt. Het oor integreert
/// over ongeveer tweehonderd milliseconde, dus dat is wat hier gemeten wordt.
///
/// Voor een geluidje dat korter is dan het venster wordt over zijn eigen lengte gemeten. Dat
/// is met opzet: anders zou een korte klank als "zachter" gelden en zou de klassieke set —
/// waarvan het kortste geluidje 120 ms is — bij het normaliseren harder worden dan hij in
/// 1.0.0 was, en dat is precies wat niet mag veranderen.
fn luidheid(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let venster = ms_naar_samples(LUIDHEID_VENSTER_MS)
        .max(1)
        .min(samples.len());
    let mut som: f32 = samples[..venster].iter().map(|s| s * s).sum();
    let mut beste = som;
    for i in venster..samples.len() {
        som += samples[i] * samples[i] - samples[i - venster] * samples[i - venster];
        beste = beste.max(som);
    }
    (beste / venster as f32).sqrt()
}

/// Schaalt het hele geluidje naar luidheid `doel`, en houdt de piek onder
/// [`PIEK_PLAFOND`].
///
/// Twee redenen om altijd te schalen in plaats van alleen te begrenzen:
///
/// 1. **Van set wisselen verandert het volume niet.** Zonder dit klinkt een uitdovende klank
///    bij dezelfde piek 5 tot 9 dB zachter dan een staande toon — gemeten, zie
///    [`DOEL_LUIDHEID`] — en dan moet je bij elke wissel ook aan de schuif.
/// 2. **Vervormen kan niet.** Partialen tellen op en tonen mogen overlappen, dus de piek van
///    een tabel is niet uit de losse getallen af te lezen. Het plafond hieronder maakt de
///    bovengrens een eigenschap van de code in plaats van een belofte van de tabel.
///
/// Slaat dat plafond aan, dan is het geluidje zachter dan bedoeld en staat er dus een
/// parameter uit de hand gelopen. Dat mag niet stil gebeuren, dus er is een test die
/// vastlegt dat geen enkele tabel het plafond raakt.
fn normaliseer(samples: &mut [f32], doel: f32) {
    let nu = luidheid(samples);
    if nu <= 0.0 {
        return;
    }
    let mut factor = doel / nu;
    let piek = samples.iter().fold(0.0f32, |m, s| m.max(s.abs())) * factor;
    if piek > PIEK_PLAFOND {
        factor *= PIEK_PLAFOND / piek;
    }
    for s in samples.iter_mut() {
        *s *= factor;
    }
}

/// Een compleet wav-bestand (16 bits PCM, mono) uit samples in −1..1, met `volume`
/// erover.
fn wav(samples: &[f32], volume: f32) -> Vec<u8> {
    let volume = volume.clamp(0.0, 1.0);
    let data_bytes = (samples.len() * 2) as u32;
    let mut uit = Vec::with_capacity(44 + samples.len() * 2);
    uit.extend_from_slice(b"RIFF");
    uit.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    uit.extend_from_slice(b"WAVEfmt ");
    uit.extend_from_slice(&16u32.to_le_bytes()); // lengte van dit blok
    uit.extend_from_slice(&1u16.to_le_bytes()); // PCM
    uit.extend_from_slice(&1u16.to_le_bytes()); // mono
    uit.extend_from_slice(&SAMPLERATE.to_le_bytes());
    uit.extend_from_slice(&(SAMPLERATE * 2).to_le_bytes()); // bytes per seconde
    uit.extend_from_slice(&2u16.to_le_bytes()); // bytes per blok
    uit.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    uit.extend_from_slice(b"data");
    uit.extend_from_slice(&data_bytes.to_le_bytes());
    for s in samples {
        let waarde = (s * volume * i16::MAX as f32).clamp(i16::MIN as f32, i16::MAX as f32);
        uit.extend_from_slice(&(waarde as i16).to_le_bytes());
    }
    uit
}

/// De samples per (set, gebeurtenis), één keer gerekend en daarna blijvend.
///
/// Het volume zit er niet in, dus aan de schuif draaien maakt deze cache niet ongeldig —
/// dat is de reden dat het volume pas in [`wav`] wordt toegepast.
///
/// Eén slot per combinatie in plaats van één `OnceLock` om de hele tabel: dan rekent de
/// eerste toon van de avond alleen zichzelf uit en niet ook de eenentwintig die je die
/// avond misschien nooit hoort. Op de motorthread, dus dat verschil is de moeite.
fn gecachte_samples(set: Geluidset, g: Geluid) -> &'static [f32] {
    const N: usize = Geluidset::ALLE.len() * Geluid::ALLE.len();
    static CACHE: [OnceLock<Vec<f32>>; N] = [const { OnceLock::new() }; N];
    CACHE[set.plek() * Geluid::ALLE.len() + g.plek()].get_or_init(|| samples(set, g))
}

impl Geluidset {
    fn plek(self) -> usize {
        Self::ALLE.iter().position(|s| *s == self).unwrap_or(0)
    }
}

// ---------------------------------------------------------------- afspelen

/// Speelt het geluidje. Lukt dat niet, dan is dat geen fout die iemand hoeft te zien: een
/// gesprek zonder piepje werkt nog steeds.
///
/// `volume` is 0..1. Op nul wordt er niets afgespeeld — dat is goedkoper dan stilte
/// afspelen en het scheelt een apparaat dat uit stand-by komt voor niets.
#[cfg(windows)]
pub fn speel(set: Geluidset, g: Geluid, volume: f32) {
    use windows::core::PCWSTR;
    use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};

    if volume <= 0.0 {
        return;
    }
    let bytes = std::sync::Arc::new(wav(gecachte_samples(set, g), volume));
    let pointer = bytes.as_ptr();
    onthoud(bytes);

    // SAFETY: met `SND_MEMORY` is de eerste parameter een verwijzing naar wav-bytes in
    // plaats van naar een bestandsnaam. `SND_ASYNC` betekent dat Windows er ná deze
    // aanroep nog uit leest, dus die bytes moeten blijven leven — daar is `onthoud` voor.
    // `SND_NODEFAULT` voorkomt dat Windows er een standaardpiep van maakt als er iets
    // niet klopt.
    let ok = unsafe {
        PlaySoundW(
            PCWSTR(pointer as *const u16),
            None,
            SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
        )
    };
    if !ok.as_bool() {
        tracing::debug!(
            geluid = g.naam(),
            set = set.naam(),
            "geluidje niet afgespeeld"
        );
    }
}

/// Houdt de laatste paar wav-buffers in leven.
///
/// Nodig omdat `PlaySound` met `SND_ASYNC` ná de aanroep nog uit het geheugen leest dat we
/// hem gaven. Een ring van vier is ruim: `PlaySound` speelt er hoogstens één tegelijk — een
/// nieuwe aanroep breekt de vorige af — dus er zijn er nooit meer dan één in gebruik, en de
/// drie erachter zijn de marge.
///
/// Diezelfde eigenschap is ook een beperking, en een bewuste: twee gebeurtenissen binnen
/// een paar honderd milliseconde geven één toon in plaats van twee door elkaar. Overlappen
/// zou `waveOut` of XAudio2 vragen, en dat is veel machinerie voor iets waar niemand op
/// wacht.
#[cfg(windows)]
fn onthoud(bytes: std::sync::Arc<Vec<u8>>) {
    use std::sync::Mutex;
    static KLINKEND: Mutex<Vec<std::sync::Arc<Vec<u8>>>> = Mutex::new(Vec::new());
    const RING: usize = 4;

    // **Een vergiftigd slot mag deze buffer niet kosten.** De aanroeper heeft er al een
    // rauwe verwijzing van genomen en geeft die zo aan Windows; zou `bytes` hier weggegooid
    // worden, dan leest `PlaySound` uit vrijgegeven geheugen. Vandaar `into_inner` in plaats
    // van `if let Ok(..)`: een ring van bytebuffers heeft geen invariant die een paniek kán
    // beschadigen, dus er is niets om tegen te beschermen en alles om te verliezen.
    //
    // Vergiftigen kan hier trouwens niet — er wordt niets gedaan dan pushen en verwijderen —
    // maar "onmogelijk" is geen reden om een use-after-free open te laten staan.
    let mut ring = KLINKEND.lock().unwrap_or_else(|e| e.into_inner());
    ring.push(bytes);
    if ring.len() > RING {
        ring.remove(0);
    }
}

/// Op macOS via `afplay`, dezelfde soort keuze als de `osascript`-melding in `notify.rs`:
/// nul afhankelijkheden en het werkt zowel gebundeld als als losse binary.
///
/// `afplay` wil een bestand, dus de bytes gaan één keer naar de tijdelijke map — op volle
/// sterkte, want het volume gaat hier via `-v` in plaats van in de samples. Dat scheelt een
/// bestand per volumestand.
#[cfg(target_os = "macos")]
pub fn speel(set: Geluidset, g: Geluid, volume: f32) {
    if volume <= 0.0 {
        return;
    }
    let bytes = wav(gecachte_samples(set, g), 1.0);
    // **De naam komt uit de inhoud, niet uit de gebeurtenis.** Met een vaste naam per
    // gebeurtenis gold "het bestand bestaat" als bewijs dat het het juiste bestand was, en
    // dat is het niet: na een wijziging in een tonentabel blijft de vorige build klinken
    // (dat is precies het soort uur dat je kwijt bent aan "waarom hoor ik mijn wijziging
    // niet"), en een half weggeschreven bestand uit een afgebroken run is niet van een heel
    // bestand te onderscheiden. Met de hash erin kán het bestand alleen kloppen.
    let naam = format!("fitcom-{}.wav", &blake3::hash(&bytes).to_hex()[..16]);
    let pad = std::env::temp_dir().join(naam);
    // Schrijven onder een andere naam en dan hernoemen: hernoemen is atomair, dus een
    // tweede instantie ziet het bestand nooit halfaf.
    if !pad.exists() {
        let deel = pad.with_extension("part");
        if std::fs::write(&deel, &bytes).is_err() || std::fs::rename(&deel, &pad).is_err() {
            tracing::debug!(geluid = g.naam(), "geluidje niet weg te schrijven");
            let _ = std::fs::remove_file(&deel);
            return;
        }
    }
    let gestart = std::process::Command::new("afplay")
        .arg("-v")
        .arg(format!("{:.3}", volume.clamp(0.0, 1.0)))
        .arg(&pad)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    match gestart {
        // Niet op afronden wachten en de zombie niet laten hangen: afplay is klaar binnen
        // een kwart seconde en de motor mag daar niet op staan wachten.
        Ok(mut kind) => {
            std::thread::spawn(move || {
                let _ = kind.wait();
            });
        }
        Err(e) => tracing::debug!(error = %e, geluid = g.naam(), "afplay niet te starten"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lees_u32(b: &[u8], op: usize) -> u32 {
        u32::from_le_bytes(b[op..op + 4].try_into().unwrap())
    }

    fn alle() -> impl Iterator<Item = (Geluidset, Geluid)> {
        Geluidset::ALLE
            .into_iter()
            .flat_map(|s| Geluid::ALLE.into_iter().map(move |g| (s, g)))
    }

    /// Een verkeerd samengestelde header levert op Windows geen fout op maar stilte, en
    /// dat is precies het soort bug dat je nooit vindt. Dus hier vastgelegd.
    #[test]
    fn elke_wav_heeft_een_kloppende_header() {
        for (s, g) in alle() {
            let w = wav(gecachte_samples(s, g), 1.0);
            let wat = format!("{}/{}", s.naam(), g.naam());
            assert_eq!(&w[0..4], b"RIFF", "{wat}");
            assert_eq!(&w[8..12], b"WAVE", "{wat}");
            assert_eq!(&w[12..16], b"fmt ", "{wat}");
            assert_eq!(&w[36..40], b"data", "{wat}");
            assert_eq!(lees_u32(&w, 4) as usize, w.len() - 8, "RIFF-lengte {wat}");
            assert_eq!(lees_u32(&w, 40) as usize, w.len() - 44, "data-lengte {wat}");
            assert_eq!(lees_u32(&w, 24), SAMPLERATE, "samplerate {wat}");
        }
    }

    /// Een sinus die op volle amplitude begint of ophoudt klikt hoorbaar, en een geluidje
    /// dat klikt klinkt als een fout in de app.
    #[test]
    fn elk_geluidje_begint_en_eindigt_op_stil() {
        for (s, g) in alle() {
            let sm = gecachte_samples(s, g);
            let wat = format!("{}/{}", s.naam(), g.naam());
            assert_eq!(sm[0], 0.0, "{wat} begint met een klik");
            assert!(
                sm[sm.len() - 1].abs() < 0.005,
                "{wat} eindigt op {} en dat is een klik",
                sm[sm.len() - 1]
            );
        }
    }

    #[test]
    fn geen_geluidje_gaat_over_het_piekplafond() {
        for (s, g) in alle() {
            let piek = gecachte_samples(s, g)
                .iter()
                .fold(0.0f32, |m, x| m.max(x.abs()));
            assert!(
                piek <= PIEK_PLAFOND + 1e-6,
                "{}/{} piekt op {piek}",
                s.naam(),
                g.naam()
            );
        }
    }

    /// Het plafond is een vangnet, geen ontwerpmiddel. Raakt een tabel het, dan klinkt die
    /// gebeurtenis zachter dan bedoeld en is er dus een parameter uit de hand gelopen — dat
    /// hoort een gefaalde test te zijn en geen stilletjes zachter geluidje.
    #[test]
    fn geen_enkele_tabel_raakt_het_piekplafond() {
        for (s, g) in alle() {
            let piek = gecachte_samples(s, g)
                .iter()
                .fold(0.0f32, |m, x| m.max(x.abs()));
            assert!(
                piek < PIEK_PLAFOND * 0.95,
                "{}/{} piekt op {piek} en zit daarmee tegen het plafond van {PIEK_PLAFOND} aan; \
                 de luidheid is dan teruggeschroefd en klinkt zachter dan bedoeld",
                s.naam(),
                g.naam()
            );
        }
    }

    // De bouwstenen los, zodat een fout daarin niet pas in een set opduikt en dan aan de
    // parametertabel geweten wordt.

    #[test]
    fn een_aanslag_begint_op_nul_stijgt_dooft_uit_en_eindigt_op_nul() {
        let env = Omhulling::Aanslag {
            tau_ms: 100,
            release_ms: 20,
        };
        let p = Partiaal {
            aanslag_ms: 8,
            ..Partiaal::GROND
        };
        let n = ms_naar_samples(300);
        let waarde = |i| omhulling(env, i, n, &p);

        assert_eq!(waarde(0), 0.0, "een aanslag hoort op stil te beginnen");
        let top = ms_naar_samples(8);
        assert!(
            waarde(top) > 0.9,
            "na de aanslagtijd hoort hij vol te staan"
        );
        assert!(
            waarde(top) > waarde(ms_naar_samples(150)),
            "daarna hoort hij te dooien"
        );
        assert!(waarde(n - 1) < 1e-3, "en op stil te eindigen");
    }

    /// De hele reden dat `tau_deel` bestaat: een hoge partiaal die sneller wegvalt dan de
    /// grondtoon is wat een klank "voorwerp" laat klinken. Staat die factor de verkeerde
    /// kant op, dan klinkt elke set metaalachtiger in plaats van warmer, en dat is aan de
    /// tabellen niet te zien.
    #[test]
    fn een_partiaal_met_een_kleiner_tau_deel_valt_sneller_weg() {
        let env = Omhulling::Aanslag {
            tau_ms: 100,
            release_ms: 20,
        };
        let langzaam = Partiaal {
            aanslag_ms: 4,
            tau_deel: 1.0,
            ..Partiaal::GROND
        };
        let snel = Partiaal {
            aanslag_ms: 4,
            tau_deel: 0.25,
            ..Partiaal::GROND
        };
        let n = ms_naar_samples(300);
        let i = ms_naar_samples(120);
        assert!(
            omhulling(env, i, n, &snel) < omhulling(env, i, n, &langzaam) * 0.5,
            "tau_deel 0.25 hoort na 120 ms ruim onder tau_deel 1.0 te zitten"
        );
    }

    #[test]
    fn een_ruisstoot_begint_op_nul_en_blijft_onder_zijn_amplitude() {
        let r = Ruis {
            amp: 0.3,
            fc_hz: 800.0,
            aanslag_ms: 6,
            tau_ms: 12,
        };
        let n = ms_naar_samples(120);
        let stoot = ruisstoot(&r, n);
        assert_eq!(stoot[0], 0.0);
        let piek = stoot.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(piek <= 0.3 + 1e-6, "ruis piekt op {piek}, boven amp 0.3");
        assert!(piek > 0.05, "ruis is helemaal niet aangekomen: piek {piek}");
        // Na tien tijdconstanten hoort er niets meer te staan.
        let laat = ms_naar_samples(110);
        assert!(stoot[laat].abs() < 1e-3, "de stoot dooft niet uit");
    }

    /// Twee opeenvolgende ruisstoten met dezelfde parameters horen identiek te zijn. Anders
    /// klinkt hetzelfde geluidje elke keer een beetje anders én verschillen de tests van run
    /// tot run, en een test die soms faalt is erger dan geen test.
    #[test]
    fn ruis_is_reproduceerbaar() {
        let r = Ruis {
            amp: 0.25,
            fc_hz: 700.0,
            aanslag_ms: 5,
            tau_ms: 10,
        };
        let n = ms_naar_samples(50);
        assert_eq!(ruisstoot(&r, n), ruisstoot(&r, n));
    }

    #[test]
    fn glijden_gaat_de_kant_op_die_er_staat() {
        // Naartoe: schiet erheen en valt op zijn plek, dus halverwege is hij er al bijna.
        let naartoe = Toon {
            glijden: Glijden::Naartoe {
                naar_hz: 800.0,
                tau_ms: 20,
            },
            ..Toon::vlak(0, 200, 400.0)
        };
        assert!((toonhoogte(&naartoe, 0) - 400.0).abs() < 1.0);
        // Na 100 ms zijn er vijf tijdconstanten om: e^-5 is 0,0067, dus er staat nog
        // 0,0067 × 400 Hz = 2,7 Hz van de sprong open. Dat is de exacte verwachting, niet
        // een marge — een exponent komt er nooit helemaal.
        let na_100 = toonhoogte(&naartoe, ms_naar_samples(100));
        assert!(
            (795.0..=800.0).contains(&na_100),
            "met tau 20 ms hoort hij na 100 ms vrijwel aangekomen te zijn, niet {na_100}"
        );
    }

    /// Structurele tegenhanger van de klik-test hierboven, en hij bestaat om een echte fout
    /// die hier gezeten heeft: bij een aanslag-omhulling wordt de in-regeling per partiaal
    /// gezet, dus een partiaal zonder aanslagtijd springt op zijn tweede sample van stil
    /// naar vol. Dat is niet aan sample nul te zien — die is dan nog netjes nul — en het
    /// zou in elke set behalve de klassieke een tik hebben opgeleverd.
    #[test]
    fn elke_aangeslagen_partiaal_heeft_een_aanslagtijd() {
        for (s, g) in alle() {
            for t in s.tonen(g) {
                if let Omhulling::Aanslag { .. } = t.omhulling {
                    for p in t.partialen {
                        assert!(
                            p.aanslag_ms >= 2,
                            "{}/{}: partiaal op ratio {} heeft {} ms aanslag",
                            s.naam(),
                            g.naam(),
                            p.ratio,
                            p.aanslag_ms
                        );
                    }
                    if let Some(r) = t.ruis {
                        assert!(
                            r.aanslag_ms >= 2,
                            "{}/{}: de ruislaag heeft {} ms aanslag",
                            s.naam(),
                            g.naam(),
                            r.aanslag_ms
                        );
                    }
                }
            }
        }
    }

    /// Geen enkele toon mag zonder partialen zijn: dan is er niets te horen, en dat is een
    /// tabel die stilletjes verkeerd staat in plaats van een fout.
    #[test]
    fn elke_toon_heeft_minstens_een_partiaal() {
        for (s, g) in alle() {
            for t in s.tonen(g) {
                assert!(
                    !t.partialen.is_empty(),
                    "{}/{} heeft een toon zonder partialen",
                    s.naam(),
                    g.naam()
                );
            }
        }
    }

    /// De luidheid is geen benadering maar een afspraak: precies `DOEL_LUIDHEID × gewicht`.
    /// Zo is aan de tabel met gewichten te zien hoe de gebeurtenissen zich tot elkaar
    /// verhouden, en niet alleen aan hoe de partialen toevallig uitkwamen.
    #[test]
    fn elke_luidheid_staat_precies_op_zijn_gewicht() {
        for (s, g) in alle() {
            let gemeten = luidheid(gecachte_samples(s, g));
            let verwacht = DOEL_LUIDHEID * s.gewicht(g);
            assert!(
                (gemeten - verwacht).abs() < 1e-4,
                "{}/{} is {gemeten} luid en niet {verwacht}",
                s.naam(),
                g.naam()
            );
        }
    }

    /// **De hele reden dat er op luidheid genormaliseerd wordt.** Twee sets naast elkaar op
    /// hetzelfde gewicht moeten even luid zijn; zo niet, dan moet je bij het wisselen van set
    /// ook aan de volumeschuif, en dat is precies de klacht die dit moest oplossen.
    ///
    /// Op de piek genormaliseerd was dit verschil 5 tot 9 dB.
    #[test]
    fn twee_sets_met_hetzelfde_gewicht_klinken_even_luid() {
        for g in Geluid::ALLE {
            let referentie = luidheid(gecachte_samples(Geluidset::Klassiek, g));
            for s in Geluidset::ALLE {
                if s.gewicht(g) != Geluidset::Klassiek.gewicht(g) {
                    continue;
                }
                let gemeten = luidheid(gecachte_samples(s, g));
                let db = 20.0 * (gemeten / referentie).log10();
                assert!(
                    db.abs() < 0.5,
                    "{} zit {db:.1} dB naast klassiek op {}",
                    s.naam(),
                    g.naam()
                );
            }
        }
    }

    /// De klassieke set is de set die Rick al goedgekeurd heeft; die mag door al dit
    /// normaliseren niet van niveau veranderen. Zijn piek stond in 1.0.0 op 0,22.
    #[test]
    fn de_klassieke_set_houdt_het_niveau_van_1_0_0() {
        for g in Geluid::ALLE {
            let piek = gecachte_samples(Geluidset::Klassiek, g)
                .iter()
                .fold(0.0f32, |m, x| m.max(x.abs()));
            assert!(
                (0.21..=0.23).contains(&piek),
                "klassiek/{} piekt op {piek} in plaats van rond 0,22",
                g.naam()
            );
        }
    }

    /// Geen enkel gewicht boven 1: dat zou een geluidje luider maken dan de set die Rick
    /// al goedgekeurd heeft, en daar is de volumeschuif voor.
    #[test]
    fn geen_gewicht_gaat_boven_een() {
        for (s, g) in alle() {
            let w = s.gewicht(g);
            assert!(
                w > 0.0 && w <= 1.0,
                "{}/{} heeft gewicht {w}",
                s.naam(),
                g.naam()
            );
        }
    }

    /// Ze moeten alle zes van elkaar te onderscheiden zijn, en het makkelijkste manier
    /// waarop dat stilletjes stukgaat is een kopieerfout in de tabel: twee gebeurtenissen
    /// die per ongeluk dezelfde noten dragen.
    #[test]
    fn binnen_een_set_klinkt_geen_enkel_geluidje_hetzelfde() {
        for s in Geluidset::ALLE {
            for (i, a) in Geluid::ALLE.iter().enumerate() {
                for b in &Geluid::ALLE[i + 1..] {
                    assert_ne!(
                        gecachte_samples(s, *a),
                        gecachte_samples(s, *b),
                        "{}: {} en {} klinken identiek",
                        s.naam(),
                        a.naam(),
                        b.naam()
                    );
                }
            }
        }
    }

    /// Genormaliseerde correlatie over de eerste `ms` van twee geluidjes: 1,0 betekent
    /// dezelfde golf, 0 betekent niets met elkaar te maken. Op sterkte genormaliseerd, dus
    /// een verschil in volume telt hier niet als verschil.
    fn onset_correlatie(a: &[f32], b: &[f32], ms: u32) -> f32 {
        let n = ms_naar_samples(ms).min(a.len()).min(b.len());
        let (x, y) = (&a[..n], &b[..n]);
        let na = x.iter().map(|v| v * v).sum::<f32>().sqrt();
        let nb = y.iter().map(|v| v * v).sum::<f32>().sqrt();
        if na <= 0.0 || nb <= 0.0 {
            return 0.0;
        }
        x.iter().zip(y).map(|(p, q)| p * q).sum::<f32>() / (na * nb)
    }

    /// **Je eigen gebeurtenis en die van iemand anders moeten vanaf de eerste sample
    /// verschillen**, niet pas als de tweede noot uitblijft.
    ///
    /// Dit is een echte fout die de review eruit haalde. Keys gebruikte voor alle zes
    /// dezelfde partialen en dezelfde modulator, en peer-join begint op de grondtoon van
    /// eigen-join — dus de eerste 120 ms waren meetbaar dezelfde golf (correlatie 0,9999) en
    /// hing het hele onderscheid aan een noot die 120 ms later niet komt. De andere drie sets
    /// zaten allemaal onder 0,15. Aan de tabel is dat niet te zien en te horen is het hier
    /// niet, dus staat het nu als getal vast.
    #[test]
    fn een_eigen_gebeurtenis_klinkt_vanaf_het_begin_anders_dan_die_van_een_ander() {
        for s in Geluidset::ALLE {
            for (eigen, peer) in [
                (Geluid::EigenJoin, Geluid::PeerJoin),
                (Geluid::EigenLeave, Geluid::PeerLeave),
            ] {
                let c =
                    onset_correlatie(gecachte_samples(s, eigen), gecachte_samples(s, peer), 120);
                assert!(
                    c < 0.9,
                    "{}: {} en {} beginnen met vrijwel dezelfde golf (correlatie {c:.4})",
                    s.naam(),
                    eigen.naam(),
                    peer.naam()
                );
            }
        }
    }

    #[test]
    fn elke_set_en_elk_geluid_is_op_naam_terug_te_vinden() {
        for s in Geluidset::ALLE {
            assert_eq!(Geluidset::van_naam(s.naam()), Some(s));
            assert!(!s.label().is_empty());
            assert!(!s.beschrijving().is_empty());
        }
        for g in Geluid::ALLE {
            assert_eq!(Geluid::van_naam(g.naam()), Some(g));
            assert!(!g.label().is_empty());
        }
        assert_eq!(Geluidset::van_naam("bestaat-niet"), None);
        assert_eq!(Geluid::van_naam(""), None);
    }

    #[test]
    fn volume_schaalt_de_samples_en_nul_is_echt_stil() {
        let sm = gecachte_samples(Geluidset::STANDAARD, Geluid::PeerJoin);
        let piek = |w: &[u8]| {
            w[44..]
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes(c.try_into().unwrap()).saturating_abs())
                .max()
                .unwrap_or(0) as f32
        };
        let vol = piek(&wav(sm, 1.0));
        let half = piek(&wav(sm, 0.5));
        assert!(
            (half / vol - 0.5).abs() < 0.02,
            "halve schuif hoort halve uitslag te geven, kreeg {}",
            half / vol
        );
        assert_eq!(piek(&wav(sm, 0.0)), 0.0, "nul hoort echt stil te zijn");
    }

    /// Schrijft alle sets naar wav-bestanden om ze te kunnen beluisteren en nameten.
    ///
    /// De reden dat dit bestaat: aan een parametertabel is niet te horen of hij klopt, en
    /// de tests hierboven meten alleen wat in getallen te vatten is (geen klik, geen
    /// overschrijding, niet twee keer hetzelfde). Wat overblijft — klinkt het prettig,
    /// hoor je welke gebeurtenis het is — kan alleen een mens, en die heeft er bestanden
    /// voor nodig.
    ///
    /// ```text
    /// cargo test -p fitcom --lib geluid -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "schrijft bestanden; voor beluisteren en nameten"]
    fn schrijf_alle_geluidjes_weg() {
        let map = std::env::temp_dir().join("fitcom-geluidjes");
        std::fs::create_dir_all(&map).expect("map aanmaken");
        for (s, g) in alle() {
            let pad = map.join(format!("{}-{}.wav", s.naam(), g.naam()));
            std::fs::write(&pad, wav(gecachte_samples(s, g), 1.0)).expect("wegschrijven");
        }
        println!(
            "{} bestanden in {}",
            Geluidset::ALLE.len() * 6,
            map.display()
        );
    }

    /// Het macOS-afspeelpad echt aflopen, tot en met het bestand dat `afplay` krijgt.
    ///
    /// Dat pad had een echte fout: de naam hing aan de gebeurtenis, dus "het bestand bestaat"
    /// gold als bewijs dat het het júiste bestand was. Na een wijziging in een tonentabel
    /// bleef daardoor de vorige build klinken. De naam komt nu uit de inhoud, en dat is hier
    /// nagelopen op de enige machine waar dit pad draait.
    #[test]
    #[cfg(target_os = "macos")]
    #[ignore = "speelt geluid af"]
    fn het_mac_pad_schrijft_een_bestand_dat_bij_de_inhoud_hoort() {
        let set = Geluidset::Glas;
        let g = Geluid::EigenJoin;
        let bytes = wav(gecachte_samples(set, g), 1.0);
        let verwacht = std::env::temp_dir().join(format!(
            "fitcom-{}.wav",
            &blake3::hash(&bytes).to_hex()[..16]
        ));
        let _ = std::fs::remove_file(&verwacht);

        speel(set, g, 0.4);

        assert!(verwacht.exists(), "geen bestand op {}", verwacht.display());
        assert_eq!(
            std::fs::read(&verwacht).unwrap(),
            bytes,
            "het bestand hoort byte voor byte de inhoud te zijn waar zijn naam uit komt"
        );
        assert!(
            !verwacht.with_extension("part").exists(),
            "het deelbestand hoort na het hernoemen weg te zijn"
        );
        println!("{} ({} bytes)", verwacht.display(), bytes.len());
    }

    /// Buiten 0..1 komt uit de webview, niet uit deze code. Vertrouwen is hier niet nodig:
    /// afkappen kost één regel.
    #[test]
    fn een_onmogelijk_volume_vervormt_niet() {
        let sm = gecachte_samples(Geluidset::STANDAARD, Geluid::EigenJoin);
        let w = wav(sm, 9.0);
        let piek = w[44..]
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes(c.try_into().unwrap()).saturating_abs())
            .max()
            .unwrap();
        assert!(
            (piek as f32) <= PIEK_PLAFOND * i16::MAX as f32 + 2.0,
            "volume 9.0 werd niet afgekapt: piek {piek}"
        );
    }
}
