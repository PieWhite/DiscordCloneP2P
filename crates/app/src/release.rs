//! De release-feed: updates worden opgehaald bij een vaste HTTPS-URL, niet meer bij een
//! peer (fase 13).
//!
//! # Waarom niet meer P2P
//!
//! In fase 11 leverde de peer met de nieuwere versie zowel de bytes als de hash waartegen
//! die bytes gecontroleerd werden. Dat is geen controle: één besmette machine kon de
//! andere twee besmetten (`docs/BEVEILIGING.md`, B-01 en de wormketen daarboven). Hier
//! komt het aanbod van één plek en is de authenticiteit **niet** afhankelijk van wie de
//! bytes levert:
//!
//! - TLS bewijst dat we met de release-host praten en niet met iemand ertussen.
//! - De Ed25519-handtekening over `"{version}\n{hash}\n{size}"` bewijst dat de release
//!   met de privésleutel getekend is. Die sleutel staat niet op een van de drie PC's en
//!   niet bij de host. Een gekaapt hostaccount kan dus wel een bestand vervangen, maar
//!   geen geldige handtekening produceren.
//! - Pas daarna worden de bytes gehaald en tegen de *ondertekende* hash gelegd.
//!
//! Zonder ingebakken publieke sleutel weigert dit pad volledig — falen gaat dicht, niet
//! open. Sleutel maken en releases tekenen: `crates/app/src/bin/fitcom-release.rs`.
//!
//! Dit is de enige plek in de app die een verbinding buiten het tailnet opzet, en hij is
//! alleen actief tijdens een check. Zie `docs/OVERDRACHT.md` voor de afweging tegen
//! invariant 1 (nul servers).

use anyhow::{anyhow, bail, ensure, Context, Result};
use serde::Deserialize;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Waar het manifest staat. `releases/latest/download/...` volgt vanzelf de nieuwste
/// release, dus deze URL hoeft nooit mee te veranderen.
pub const MANIFEST_URL: &str =
    "https://github.com/PieWhite/DiscordCloneP2P/releases/latest/download/latest.json";

/// Waar de exe zelf vandaan mag komen. Het manifest noemt een volledige URL (vastgepind
/// op zijn eigen tag, zodat een release die er tussendoor bijkomt geen halve download
/// oplevert), maar hij moet wel binnen deze repo blijven.
const URL_VOORVOEGSEL: &str = "https://github.com/PieWhite/DiscordCloneP2P/releases/download/";

/// Ed25519-publieke sleutel, 32 bytes hex — de helft van het paar uit
/// `fitcom-release keygen`; de privéhelft staat buiten de repo.
///
/// Nullen hier betekent: nooit een update accepteren. Dat is expres — een build zonder
/// sleutel mag niet stilletjes op TLS alleen gaan vertrouwen. Bewaakt door
/// `tests::de_ingebakken_sleutel_is_bruikbaar`.
const PUBLIEKE_SLEUTEL_HEX: &str =
    "28cd4b5337be54dca2f512b60e48d62ddde872818065fceb9bf2fa244c8087c7";

/// Ruim boven een realistische buildgrootte, en een hard plafond tegen een host die
/// eindeloos blijft sturen (B-13-klasse).
const MAX_UPDATE: u64 = 200 * 1024 * 1024;
/// Het manifest is een handvol regels JSON.
const MAX_MANIFEST: u64 = 8 * 1024;

const MANIFEST_TIMEOUT: Duration = Duration::from_secs(20);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);
const VERBIND_TIMEOUT: Duration = Duration::from_secs(10);

/// Hoe vaak de voortgangsmelding hoogstens naar de UI gaat. Zelfde ritme als bij
/// bestandsoverdrachten.
const VOORTGANG_INTERVAL: Duration = Duration::from_millis(200);

/// Hoe vaak een GET in totaal geprobeerd wordt, en de pauze vóór elke herhaling (die
/// oploopt: één keer, dan twee keer deze duur).
///
/// Gemeten op 2026-08-12: ongeveer de helft van de aanroepen strandde met
/// `io: Peer disconnected` terwijl `curl` naar dezelfde URL op datzelfde moment tien van
/// de tien haalde en een rauwe TCP+TLS-handshake naar de CDN evengoed. Het netwerk is dus
/// niet stuk — de verbinding gaat dicht tijdens de redirectketen van `releases/latest`
/// naar `release-assets.githubusercontent.com`. Elke `agent()` bouwt een verse Agent met
/// een lege verbindingspool, dus een nieuwe poging is per definitie een schone verbinding.
///
/// Dit is invariant 7 (offline is normaal) toegepast op het updatepad: een haperende
/// verbinding hoort geen foutmelding op te leveren zolang de volgende poging hem wel haalt.
const POGINGEN: u32 = 3;
const HERHAALPAUZE: Duration = Duration::from_millis(500);

/// Eén release zoals het manifest hem beschrijft. Alle velden zijn verdacht tot
/// `controleer` ze goedkeurt.
#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub version: String,
    /// Volledige download-URL van de exe, binnen `URL_VOORVOEGSEL`.
    pub url: String,
    pub size: u64,
    /// BLAKE3 over de exe, hex.
    pub hash: String,
    /// Ed25519 over `ondertekend_bericht()`, hex.
    pub signature: String,
}

impl Release {
    /// Wat er getekend is. Versie, hash én grootte zitten erin, zodat geen van drieën
    /// los te verwisselen is met die uit een andere (wel geldig getekende) release.
    pub fn ondertekend_bericht(&self) -> String {
        format!("{}\n{}\n{}", self.version, self.hash, self.size)
    }
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .timeout_connect(Some(VERBIND_TIMEOUT))
        .user_agent(concat!("fitcom/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// Voert `poging` uit en probeert het bij een fout opnieuw, hoogstens `POGINGEN` keer.
///
/// De laatste poging staat bewust buiten de lus: zijn fout is degene die de gebruiker te
/// zien krijgt, en zo klopt het ook nog als `POGINGEN` ooit op 1 gezet wordt.
fn met_herhaling<T>(mut poging: impl FnMut() -> Result<T>) -> Result<T> {
    for nr in 1..POGINGEN {
        match poging() {
            Ok(t) => return Ok(t),
            Err(e) => {
                tracing::debug!("poging {nr} van {POGINGEN} mislukt, opnieuw: {e:#}");
                std::thread::sleep(HERHAALPAUZE * nr);
            }
        }
    }
    poging()
}

/// Haalt het manifest op. Doet geen enkele uitspraak over of het te vertrouwen is —
/// dat is `controleer`.
pub fn haal_manifest() -> Result<Release> {
    met_herhaling(|| {
        agent(MANIFEST_TIMEOUT)
            .get(MANIFEST_URL)
            .call()
            .context("release-feed niet bereikbaar")?
            .body_mut()
            .with_config()
            .limit(MAX_MANIFEST)
            .read_json()
            .context("release-feed staat vol met iets anders dan een manifest")
    })
}

/// Keurt een manifest goed en levert de hash waartegen de download straks moet kloppen.
///
/// Volgorde is met opzet: eerst de goedkope vormcontroles, dan pas de handtekening.
/// `version` gaat in een bestandsnaam, dus die wordt hier begrensd tot cijfers en punten
/// — ook al is hij getekend (B-02 kwam langs precies deze weg binnen).
pub fn controleer(rel: &Release) -> Result<[u8; 32]> {
    let sleutel = hex_naar_bytes::<32>(PUBLIEKE_SLEUTEL_HEX)
        .context("de ingebakken release-sleutel is geen 32 hex-bytes")?;
    ensure!(
        sleutel != [0u8; 32],
        "deze build heeft geen release-sleutel; updates staan daarmee uit"
    );
    controleer_met_sleutel(rel, sleutel)
}

/// De inhoud van `controleer`, met de sleutel als parameter zodat het geslaagde pad
/// testbaar is zonder een sleutel in de broncode te zetten.
fn controleer_met_sleutel(rel: &Release, sleutel: [u8; 32]) -> Result<[u8; 32]> {
    ensure!(
        !rel.version.is_empty() && rel.version.len() <= 32,
        "versienummer heeft een onwerkbare lengte"
    );
    ensure!(
        rel.version.bytes().all(|b| b.is_ascii_digit() || b == b'.'),
        "versienummer mag alleen cijfers en punten bevatten"
    );
    ensure!(
        rel.url.starts_with(URL_VOORVOEGSEL),
        "download-URL hoort niet bij de ingebakken release-repo"
    );
    ensure!(
        rel.size > 0 && rel.size <= MAX_UPDATE,
        "aangekondigde grootte ({} bytes) valt buiten wat een build kan zijn",
        rel.size
    );

    let hash = hex_naar_bytes::<32>(&rel.hash).context("hash-veld is geen 32 hex-bytes")?;
    let handtekening =
        hex_naar_bytes::<64>(&rel.signature).context("signature-veld is geen 64 hex-bytes")?;

    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, sleutel)
        .verify(rel.ondertekend_bericht().as_bytes(), &handtekening)
        .map_err(|_| anyhow!("handtekening van de release klopt niet"))?;

    Ok(hash)
}

/// Kijkt of de URL uit het manifest daadwerkelijk bestaat, zonder de bytes te halen.
///
/// Levert de HTTP-status op. Bestaat alleen voor `fitcom-release check`: het manifest
/// noemt een URL die op zijn eigen tag vastgepind is, en een release publiceren met een
/// tag die er (nog) niet is levert een handtekening op die perfect klopt boven een
/// download die 404't. Dat is precies één keer gebeurd — zie `docs/OVERDRACHT.md`,
/// beslissing 24.
pub fn bereikbaar(rel: &Release) -> Result<u16> {
    met_herhaling(|| {
        let antwoord = agent(MANIFEST_TIMEOUT)
            .get(&rel.url)
            .call()
            .context("download-URL niet op te vragen")?;
        // Body niet lezen: ureq breekt de verbinding af als hij ongelezen weggegooid wordt,
        // dus dit kost geen 20 MB.
        Ok(antwoord.status().as_u16())
    })
}

/// Haalt de exe op, schrijft hem weg en legt hem tegen `verwachte_hash`. Blokkerend:
/// hoort in een `spawn_blocking`. `voortgang` wordt hoogstens elke
/// `VOORTGANG_INTERVAL` aangeroepen, plus één keer aan het eind.
pub fn download(
    rel: &Release,
    verwachte_hash: [u8; 32],
    updates_dir: &Path,
    mut voortgang: impl FnMut(u64),
) -> Result<PathBuf> {
    let deelpad = updates_dir.join(format!("update-{}.exe.part", rel.version));

    // Ophalen en wegschrijven zitten samen in de herhaling, want een verbinding die
    // halverwege dichtgaat is dezelfde hapering als een die meteen dichtgaat. Elke poging
    // begint het deelbestand opnieuw (`File::create` kapt hem af) en de voortgang springt
    // dus terug naar nul — eerlijker dan een balk die blijft staan.
    //
    // Geen hervatten: zonder Range-verzoek is dit één rechte lijn, en een build van
    // enkele tientallen MB's opnieuw halen is goedkoper dan het hervatpunt bewaken.
    let uitkomst = met_herhaling(|| {
        let mut bestand = std::fs::File::create(&deelpad).context("deelbestand aanmaken")?;

        let mut antwoord = agent(DOWNLOAD_TIMEOUT)
            .get(&rel.url)
            .call()
            .context("update niet op te halen")?;
        // `limit` op de aangekondigde grootte is het tweede net tegen een host die meer stuurt
        // dan hij aankondigde. Het eerste is `lees_precies` zelf, die nooit meer opvraagt dan
        // het restant — lees daar waarom die grens niet als lus-einde te gebruiken is.
        let lezer = antwoord.body_mut().with_config().limit(rel.size).reader();

        let hash = lees_precies(lezer, rel.size, &mut bestand, &mut voortgang)?;
        bestand.flush().context("deelbestand doorschrijven")?;
        Ok(hash)
    })
    // De hashcontrole staat buiten de herhaling: bytes die netjes zijn aangekomen en tóch
    // niet kloppen zijn geen hapering, en dat mag geen drie downloads kosten.
    .and_then(|hash| {
        ensure!(
            hash == verwachte_hash,
            "de opgehaalde update komt niet overeen met de ondertekende hash"
        );
        Ok(())
    });
    if let Err(e) = uitkomst {
        let _ = std::fs::remove_file(&deelpad);
        return Err(e);
    }

    let definitief = updates_dir.join(format!("update-{}.exe", rel.version));
    std::fs::rename(&deelpad, &definitief).context("update hernoemen naar definitieve naam")?;
    Ok(definitief)
}

/// Leest precies `grootte` bytes, schrijft ze weg en levert de BLAKE3 eroverheen.
///
/// **De lus telt zelf af en wacht niet op einde-bestand.** Dat is geen stijlkeuze: de
/// begrensde lezer van ureq geeft bij een `read` ná de laatste byte geen `Ok(0)` maar
/// `Err(BodyExceedsLimit)` (`ureq::body::limit::LimitReader` — de teller staat dan op nul
/// en dat is niet te onderscheiden van "er komt meer dan aangekondigd"). Een lus die tot
/// `n == 0` doorleest doet dus altijd één leesbeurt te veel en faalt *elke* download, ook
/// een volstrekt correcte. Dat was de reden dat het bijwerken nooit werkte; zie
/// `tests::een_body_van_exact_de_aangekondigde_grootte_komt_er_heel_door`.
///
/// Een afgebroken verbinding is dan wél gewoon `Ok(0)` — daar is einde-bestand precies wat
/// het lijkt — en dat is hier een fout met het aantal bytes erbij, niet stilzwijgend een
/// half bestand dat de hashcontrole later afkeurt.
fn lees_precies(
    mut lezer: impl Read,
    grootte: u64,
    uit: &mut impl Write,
    mut voortgang: impl FnMut(u64),
) -> Result<[u8; 32]> {
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut ontvangen = 0u64;
    let mut laatste_melding = Instant::now();

    while ontvangen < grootte {
        let ruimte = ((grootte - ontvangen) as usize).min(buf.len());
        let n = lezer.read(&mut buf[..ruimte]).context("bytes ontvangen")?;
        if n == 0 {
            bail!("de verbinding brak af na {ontvangen} van {grootte} bytes");
        }
        uit.write_all(&buf[..n]).context("bytes wegschrijven")?;
        hasher.update(&buf[..n]);
        ontvangen += n as u64;
        if laatste_melding.elapsed() >= VOORTGANG_INTERVAL {
            laatste_melding = Instant::now();
            voortgang(ontvangen);
        }
    }
    voortgang(ontvangen);
    Ok(*hasher.finalize().as_bytes())
}

pub fn hex_naar_bytes<const N: usize>(s: &str) -> Result<[u8; N]> {
    ensure!(
        s.len() == 2 * N,
        "verwacht {} hex-tekens, kreeg er {}",
        2 * N,
        s.len()
    );
    let mut uit = [0u8; N];
    for (i, paar) in s.as_bytes().chunks(2).enumerate() {
        let tekst = std::str::from_utf8(paar).context("niet-ascii in een hex-veld")?;
        uit[i] = u8::from_str_radix(tekst, 16).context("geen geldig hex-teken")?;
    }
    Ok(uit)
}

pub fn bytes_naar_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::KeyPair;

    fn sleutelpaar() -> ring::signature::Ed25519KeyPair {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
    }

    fn publiek(paar: &ring::signature::Ed25519KeyPair) -> [u8; 32] {
        paar.public_key().as_ref().try_into().unwrap()
    }

    /// Doet wat `fitcom-release sign` doet: tekenen over `ondertekend_bericht`.
    fn getekend(paar: &ring::signature::Ed25519KeyPair) -> Release {
        let mut rel = Release {
            version: "0.3.0".into(),
            url: format!("{URL_VOORVOEGSEL}v0.3.0/fitcom.exe"),
            size: 1000,
            hash: "aa".repeat(32),
            signature: String::new(),
        };
        rel.signature = bytes_naar_hex(paar.sign(rel.ondertekend_bericht().as_bytes()).as_ref());
        rel
    }

    /// De hapering waarvoor `met_herhaling` bestaat: de eerste poging strandt, de tweede
    /// haalt hem. Zonder herhaling zag de gebruiker hier "Update failed".
    #[test]
    fn een_hapering_wordt_bij_de_volgende_poging_alsnog_gehaald() {
        let mut keer = 0;
        let uit = met_herhaling(|| {
            keer += 1;
            if keer < 2 {
                bail!("io: Peer disconnected");
            }
            Ok(keer)
        });
        assert_eq!(uit.unwrap(), 2, "de tweede poging hoort te slagen");
    }

    /// Blijft het mislukken, dan is de fout van de láátste poging wat de gebruiker ziet —
    /// en er wordt niet eindeloos doorgeprobeerd.
    #[test]
    fn blijvend_kapot_stopt_na_pogingen_met_de_laatste_fout() {
        let mut keer = 0;
        let uit: Result<()> = met_herhaling(|| {
            keer += 1;
            bail!("poging {keer} stuk");
        });
        assert_eq!(
            keer, POGINGEN,
            "er hoort precies POGINGEN keer geprobeerd te worden"
        );
        assert!(uit
            .unwrap_err()
            .to_string()
            .contains(&format!("poging {POGINGEN}")));
    }

    /// Eén geslaagde poging blijft één poging: geen verkeer erbij op het gewone pad.
    #[test]
    fn meteen_goed_probeert_niet_nog_een_keer() {
        let mut keer = 0;
        let uit = met_herhaling(|| {
            keer += 1;
            Ok(keer)
        });
        assert_eq!((uit.unwrap(), keer), (1, 1));
    }

    #[test]
    fn een_correct_getekende_release_komt_erdoor_met_zijn_hash() {
        let paar = sleutelpaar();
        let rel = getekend(&paar);
        assert_eq!(
            controleer_met_sleutel(&rel, publiek(&paar)).unwrap(),
            [0xaa; 32]
        );
    }

    #[test]
    fn gewijzigde_hash_wordt_geweigerd() {
        let paar = sleutelpaar();
        let mut rel = getekend(&paar);
        rel.hash = "bb".repeat(32);
        assert!(
            controleer_met_sleutel(&rel, publiek(&paar)).is_err(),
            "een omgewisselde hash mag niet door de handtekening komen"
        );
    }

    #[test]
    fn gewijzigde_grootte_wordt_geweigerd() {
        let paar = sleutelpaar();
        let mut rel = getekend(&paar);
        rel.size = 2000;
        assert!(controleer_met_sleutel(&rel, publiek(&paar)).is_err());
    }

    #[test]
    fn gewijzigde_versie_wordt_geweigerd() {
        let paar = sleutelpaar();
        let mut rel = getekend(&paar);
        rel.version = "9.9.9".into();
        assert!(controleer_met_sleutel(&rel, publiek(&paar)).is_err());
    }

    #[test]
    fn andere_sleutel_komt_er_niet_door() {
        let rel = getekend(&sleutelpaar());
        assert!(controleer_met_sleutel(&rel, publiek(&sleutelpaar())).is_err());
    }

    /// Bewaakt de sleutel die deze build meedraagt. Wordt hij per ongeluk geleegd,
    /// afgekapt of vervangen door iets dat geen 32 hex-bytes is, dan faalt dit —
    /// en niet pas op de machine van iemand die zich afvraagt waarom hij nooit
    /// een update krijgt.
    #[test]
    fn de_ingebakken_sleutel_is_bruikbaar() {
        let rel = getekend(&sleutelpaar());
        // Met een vreemde sleutel getekend, dus dit hóórt te falen — maar op de
        // handtekening, niet op de sleutel zelf. Dat onderscheid is de test.
        let fout = controleer(&rel).unwrap_err().to_string();
        assert!(
            fout.contains("handtekening"),
            "verwachtte een handtekeningfout, kreeg: {fout}"
        );
    }

    #[test]
    fn versie_met_padtekens_wordt_geweigerd() {
        let paar = sleutelpaar();
        let mut rel = getekend(&paar);
        rel.version = "../../evil".into();
        let fout = controleer_met_sleutel(&rel, publiek(&paar))
            .unwrap_err()
            .to_string();
        assert!(fout.contains("cijfers en punten"), "kreeg: {fout}");
    }

    #[test]
    fn url_buiten_de_ingebakken_repo_wordt_geweigerd() {
        let paar = sleutelpaar();
        let mut rel = getekend(&paar);
        rel.url = "https://elders.example/fitcom.exe".into();
        let fout = controleer_met_sleutel(&rel, publiek(&paar))
            .unwrap_err()
            .to_string();
        assert!(fout.contains("release-repo"), "kreeg: {fout}");
    }

    #[test]
    fn te_grote_aangekondigde_release_wordt_geweigerd() {
        let paar = sleutelpaar();
        let mut rel = getekend(&paar);
        rel.size = MAX_UPDATE + 1;
        assert!(controleer_met_sleutel(&rel, publiek(&paar)).is_err());
    }

    /// Doet na wat `ureq::body::limit::LimitReader` doet: hij levert de bytes tot aan de
    /// grens en geeft daarna geen einde-bestand maar een fout. Zonder deze nabootsing is
    /// de bug hieronder alleen met een echte HTTP-server te zien.
    struct BegrensdeLezer<'a> {
        bron: &'a [u8],
        over: usize,
    }

    impl Read for BegrensdeLezer<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.over == 0 {
                return Err(std::io::Error::other("body exceeds limit"));
            }
            let n = self.bron.len().min(buf.len()).min(self.over);
            buf[..n].copy_from_slice(&self.bron[..n]);
            self.bron = &self.bron[n..];
            self.over -= n;
            Ok(n)
        }
    }

    /// De bug die het bijwerken vanaf GitHub onbruikbaar maakte: de lus las door tot
    /// `n == 0`, en die leesbeurt komt bij een begrensde body nooit — hij geeft een fout.
    /// Elke geslaagde download eindigde dus in "bytes ontvangen: body exceeds limit".
    #[test]
    fn een_body_van_exact_de_aangekondigde_grootte_komt_er_heel_door() {
        let bytes: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let lezer = BegrensdeLezer {
            bron: &bytes,
            over: bytes.len(),
        };
        let mut uit = Vec::new();
        let hash = lees_precies(lezer, bytes.len() as u64, &mut uit, |_| {})
            .expect("een body die precies zo groot is als aangekondigd hoort geen fout te zijn");
        assert_eq!(uit, bytes);
        assert_eq!(hash, *blake3::hash(&bytes).as_bytes());
    }

    #[test]
    fn een_afgebroken_download_is_een_fout_en_geen_half_bestand() {
        let bytes = vec![7u8; 1000];
        let lezer = BegrensdeLezer {
            bron: &bytes[..400],
            over: 1000,
        };
        let mut uit = Vec::new();
        let fout = lees_precies(lezer, 1000, &mut uit, |_| {})
            .unwrap_err()
            .to_string();
        assert!(fout.contains("brak af"), "kreeg: {fout}");
    }

    #[test]
    fn de_voortgang_eindigt_op_het_totaal() {
        let bytes = vec![1u8; 300_000];
        let lezer = BegrensdeLezer {
            bron: &bytes,
            over: bytes.len(),
        };
        let mut laatste = 0u64;
        let mut uit = Vec::new();
        lees_precies(lezer, bytes.len() as u64, &mut uit, |n| laatste = n).unwrap();
        assert_eq!(laatste, 300_000, "de balk moet op vol eindigen");
    }

    #[test]
    fn hex_heen_en_weer() {
        let bytes: [u8; 4] = [0x00, 0x0f, 0xa5, 0xff];
        assert_eq!(bytes_naar_hex(&bytes), "000fa5ff");
        assert_eq!(hex_naar_bytes::<4>("000fa5ff").unwrap(), bytes);
        assert!(hex_naar_bytes::<4>("000fa5").is_err());
        assert!(hex_naar_bytes::<4>("000fa5zz").is_err());
    }
}
