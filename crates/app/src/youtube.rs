//! Titel en miniatuur bij een YouTube-link in de chat.
//!
//! # Waarom dit hier staat en niet in de frontend
//!
//! Dit is de **tweede** bewuste uitzondering op invariant 1 (nul servers), na de
//! release-feed. De afweging staat in `docs/OVERDRACHT.md` beslissing 29; kort: een titel
//! en een plaatje bij een link kunnen alleen bij YouTube vandaan komen, dus de vraag is
//! niet *of* er een verbinding buiten het tailnet is maar *wie* hem legt en hoe vaak.
//!
//! Hier, in de motor, en niet met een `<img src="https://i.ytimg.com/...">` in de webview:
//!
//! - De CSP blijft dicht. Geen `img-src` naar een vreemde host betekent dat een bericht van
//!   een peer nooit een verbinding uit het venster kan laten vertrekken — de miniatuur komt
//!   straks over `asset:` van de eigen schijf.
//! - Geen cookies, geen referrer, geen tweede request bij elk hertekenen. Eén keer per
//!   video, ooit, en daarna van schijf.
//! - Werkt niets, dan blijft het een gewone link. Invariant 7 (offline is normaal) geldt
//!   ook hier: dit is versiering en mag nooit een foutmelding opleveren.
//!
//! # Waarom oEmbed
//!
//! `https://www.youtube.com/oembed` is publiek, gratis en zonder sleutel of account — dat
//! was de eis. De YouTube Data API zou een key vragen, en een key in een exe die bij drie
//! mensen op de pc staat is geen key.

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Het oEmbed-eindpunt, met de video-URL al percent-gecodeerd erin. `youtu.be/<id>` en niet
/// `watch?v=<id>`: dan zit er geen tweede `?` in de buitenste query, en hoeft niemand erop
/// te vertrouwen dat de juiste parser die als data en niet als parameter leest.
const OEMBED: &str = "https://www.youtube.com/oembed?format=json&url=https%3A%2F%2Fyoutu.be%2F";

/// De miniatuur wordt hier zelf samengesteld en komt **niet** uit `thumbnail_url` in het
/// antwoord. Dat veld is een URL uit een respons, en een URL uit een respons is een URL
/// die je moet gaan valideren; `hqdefault.jpg` bestaat voor elke video en is 480×360 —
/// ruim boven de kaartbreedte, dus scherp genoeg.
const THUMB: &str = "https://i.ytimg.com/vi/";

/// Een titel is een handvol woorden en een oEmbed-antwoord een handvol regels JSON.
const MAX_OEMBED: u64 = 16 * 1024;
/// `hqdefault.jpg` is in de praktijk 20-40 kB. Dit is het plafond tegen een host die
/// blijft sturen, niet een verwachting.
const MAX_THUMB: u64 = 4 * 1024 * 1024;
/// Wat er hoogstens in de kaart terechtkomt. De frontend `esc()`t hem, dit is tegen een
/// titel die de tijdlijn zou vullen.
const MAX_TITEL: usize = 200;

const TIMEOUT: Duration = Duration::from_secs(10);
const VERBIND_TIMEOUT: Duration = Duration::from_secs(5);

/// Wat er van een video bekend is, en waar de miniatuur op deze pc staat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preview {
    pub title: String,
    /// De kanaalnaam. Leeg als oEmbed hem niet meestuurde.
    pub author: String,
    /// Absoluut pad naar de gecachte JPEG. Alleen gezet als het bestand er echt is.
    #[serde(skip)]
    pub thumbnail: PathBuf,
}

/// Een video-id zoals YouTube ze uitgeeft: precies elf tekens uit `[A-Za-z0-9_-]`.
///
/// Dit is niet cosmetisch. Het id komt uit een bericht van een peer, via de webview, en
/// gaat hier twee kanten op die allebei een injectiepad zouden zijn zonder deze controle:
/// in een URL (queryparameter en padsegment) en in een bestandsnaam in de cachemap
/// (B-03 is precies deze klasse). Elf tekens uit een vaste verzameling kunnen geen van
/// beide iets anders worden.
pub fn geldig_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Titel, kanaal en miniatuur van één video. Uit de cache als het kan, anders van YouTube.
///
/// Blokkeert (ureq is synchroon), dus dit hoort op een blocking-thread — zie
/// `ui::commands::youtube_preview`.
pub fn preview(id: &str, cache_dir: &Path) -> Result<Preview> {
    ensure!(geldig_id(id), "geen geldig YouTube-video-id");

    if let Some(p) = uit_cache(id, cache_dir) {
        return Ok(p);
    }

    std::fs::create_dir_all(cache_dir).context("cachemap voor previews aanmaken")?;
    let preview = haal_op(id, cache_dir)?;
    // Pas wegschrijven als het plaatje er ook is: een halve cache-entry zou bij de
    // volgende start een kaart zonder miniatuur opleveren en nooit meer opnieuw halen.
    let json = serde_json::to_string(&preview).context("preview serialiseren")?;
    if let Err(e) = std::fs::write(meta_pad(id, cache_dir), json) {
        // Niet fataal: deze sessie heeft zijn kaart, de volgende haalt hem opnieuw op.
        tracing::warn!(error = %e, %id, "preview niet gecacht");
    }
    Ok(preview)
}

fn meta_pad(id: &str, cache_dir: &Path) -> PathBuf {
    cache_dir.join(format!("{id}.json"))
}

fn thumb_pad(id: &str, cache_dir: &Path) -> PathBuf {
    cache_dir.join(format!("{id}.jpg"))
}

/// Een titel verandert bijna nooit en een miniatuur nog minder; er is dus geen reden om
/// ooit te verversen. Ontbreekt een van de twee helften, dan is dit geen cache-treffer.
fn uit_cache(id: &str, cache_dir: &Path) -> Option<Preview> {
    let thumbnail = thumb_pad(id, cache_dir);
    if !thumbnail.exists() {
        return None;
    }
    let tekst = std::fs::read_to_string(meta_pad(id, cache_dir)).ok()?;
    let mut preview: Preview = serde_json::from_str(&tekst).ok()?;
    preview.thumbnail = thumbnail;
    Some(preview)
}

/// Het antwoord van oEmbed. Alles behalve `title` en `author_name` interesseert ons niet;
/// serde negeert de rest.
#[derive(Deserialize)]
struct OEmbed {
    title: String,
    #[serde(default)]
    author_name: String,
}

fn haal_op(id: &str, cache_dir: &Path) -> Result<Preview> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .timeout_connect(Some(VERBIND_TIMEOUT))
        .user_agent(concat!("fitcom/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();

    let antwoord: OEmbed = agent
        .get(format!("{OEMBED}{id}"))
        .call()
        .context("YouTube niet bereikbaar")?
        .body_mut()
        .with_config()
        .limit(MAX_OEMBED)
        .read_json()
        .context("oEmbed-antwoord is geen bruikbare JSON")?;

    let bytes = agent
        .get(format!("{THUMB}{id}/hqdefault.jpg"))
        .call()
        .context("miniatuur niet op te halen")?
        .body_mut()
        .with_config()
        .limit(MAX_THUMB)
        .read_to_vec()
        .context("miniatuur niet te lezen")?;
    // Geen JPEG betekent: dit is niet wat we vroegen. Twee bytes zijn genoeg om dat te
    // zien, en het scheelt een plaatje dat de webview toch niet kan tekenen.
    if bytes.len() < 2 || bytes[0..2] != [0xFF, 0xD8] {
        bail!("miniatuur is geen JPEG");
    }

    let thumbnail = thumb_pad(id, cache_dir);
    std::fs::write(&thumbnail, &bytes).context("miniatuur wegschrijven")?;

    Ok(Preview {
        title: kort(&antwoord.title),
        author: kort(&antwoord.author_name),
        thumbnail,
    })
}

/// Kapt af op chars en niet op bytes: een titel is vaak niet-ASCII en mag geen paniek geven.
fn kort(s: &str) -> String {
    s.chars().take(MAX_TITEL).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn een_echt_video_id_wordt_geaccepteerd() {
        assert!(geldig_id("dQw4w9WgXcQ"));
        assert!(geldig_id("_-aBcDeFgH1"));
    }

    #[test]
    fn alles_wat_een_pad_of_een_url_kan_worden_valt_af() {
        for aanval in [
            "",
            "dQw4w9WgXc",        // tien
            "dQw4w9WgXcQQ",      // twaalf
            "../../../etc/pass", // padtraversal
            "dQw4w9WgXc/",       // padscheiding
            "dQw4w9WgXc.",       // extensie eraan plakken
            "dQw4w9WgX Q",       // spatie
            "dQw4w9WgXc&",       // tweede queryparameter
            "dQw4w9WgXc?",
            "dQw4w9WgXc#",
            "dQw4w9WgXcé",
        ] {
            assert!(!geldig_id(aanval), "geaccepteerd: {aanval:?}");
        }
    }

    #[test]
    fn een_titel_wordt_afgekapt_zonder_te_panieken() {
        let lang = "é".repeat(500);
        assert_eq!(kort(&lang).chars().count(), MAX_TITEL);
        assert_eq!(kort("kort"), "kort");
    }

    /// Praat met YouTube, dus `--ignored` — net als de rooktest op de echte geluidskaart.
    /// Dit is de test die bewijst dat de twee URL-vormen hierboven kloppen; die kun je
    /// niet offline nakijken, en fout betekent hier "de kaart verschijnt nooit" zonder
    /// dat iemand ziet waarom.
    ///
    /// `cargo test -p fitcom youtube -- --ignored --nocapture`
    #[test]
    #[ignore = "praat met youtube.com"]
    fn haalt_een_echte_video_op() {
        let map = std::env::temp_dir().join("fitcom-youtube-test");
        let _ = std::fs::remove_dir_all(&map);
        let p = preview("dQw4w9WgXcQ", &map).expect("preview ophalen");
        println!("titel: {:?} kanaal: {:?}", p.title, p.author);
        assert!(!p.title.is_empty());
        assert!(p.thumbnail.exists());
        let bytes = std::fs::metadata(&p.thumbnail).unwrap().len();
        assert!(bytes > 1000, "miniatuur van {bytes} bytes");

        // Tweede keer: uit de cache, zonder netwerk.
        let uit_cache = uit_cache("dQw4w9WgXcQ", &map).expect("cache-treffer");
        assert_eq!(uit_cache.title, p.title);
        let _ = std::fs::remove_dir_all(&map);
    }

    /// Het id zit zowel in een queryparameter als in een padsegment; deze test legt vast
    /// dat er niets *tussen* `geldig_id` en de opbouw van die URL's zit.
    #[test]
    fn de_urls_worden_uit_het_gecontroleerde_id_opgebouwd() {
        let id = "dQw4w9WgXcQ";
        assert_eq!(
            format!("{OEMBED}{id}"),
            "https://www.youtube.com/oembed?format=json&url=https%3A%2F%2Fyoutu.be%2FdQw4w9WgXcQ"
        );
        assert_eq!(
            format!("{THUMB}{id}/hqdefault.jpg"),
            "https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg"
        );
    }
}
