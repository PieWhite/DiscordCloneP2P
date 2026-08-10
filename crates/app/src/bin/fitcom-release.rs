//! Gereedschap om een release te tekenen (fase 13). Draait op de machine die de release
//! uitgeeft, nooit op een gebruikersmachine, en wordt niet meegeleverd in de zip.
//!
//! De app vertrouwt een update alleen als hij met de privésleutel getekend is; zie
//! `crates/app/src/release.rs`. Deze binary maakt die sleutel en het bijbehorende
//! `latest.json`.
//!
//! ```text
//! fitcom-release keygen --out release-key.pk8
//!     -> zet de publieke sleutel in PUBLIEKE_SLEUTEL_HEX in release.rs
//!
//! fitcom-release sign --key release-key.pk8 --exe target/release/fitcom.exe \
//!     --version 0.3.0 \
//!     --url https://github.com/PieWhite/fitcom/releases/download/v0.3.0/fitcom.exe
//!     -> schrijft latest.json; upload die samen met fitcom.exe naar de release
//! ```
//!
//! De `.pk8` staat buiten de repo en gaat nergens heen. Raakt hij kwijt, dan maak je een
//! nieuwe sleutel en moet iedereen één keer met de hand bijwerken — dat is de prijs van
//! niet-vervangbare vertrouwensankers, en de reden dat hij niet in de repo hoort.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use fitcom::release::{bytes_naar_hex, Release};
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::path::PathBuf;

#[derive(Parser)]
#[command(about = "Sleutels maken en releases tekenen voor de update-feed")]
struct Args {
    #[command(subcommand)]
    opdracht: Opdracht,
}

#[derive(Subcommand)]
enum Opdracht {
    /// Maakt een nieuw Ed25519-sleutelpaar en print de publieke helft.
    Keygen {
        #[arg(long, default_value = "release-key.pk8")]
        out: PathBuf,
    },
    /// Tekent een gebouwde exe en schrijft het manifest.
    Sign {
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        exe: PathBuf,
        /// Moet gelijk zijn aan de `version` in `Cargo.toml` van deze build.
        #[arg(long)]
        version: String,
        /// De volledige download-URL van deze exe in de release.
        #[arg(long)]
        url: String,
        #[arg(long, default_value = "latest.json")]
        out: PathBuf,
    },
    /// Legt een manifest langs de sleutel die *deze* build meedraagt — dezelfde controle
    /// die de app straks doet. Draai dit vóór het publiceren: het is het enige dat een
    /// verkeerd geplakte publieke sleutel aan het licht brengt vóórdat iedereen stilletjes
    /// geen updates meer krijgt.
    Verify {
        #[arg(long, default_value = "latest.json")]
        manifest: PathBuf,
    },
    /// Doet ná het publiceren precies wat een gebruikersmachine doet: het manifest bij
    /// `MANIFEST_URL` ophalen, de handtekening controleren, en kijken of de exe waar het
    /// naar wijst er ook werkelijk staat.
    ///
    /// Dat laatste is de reden dat dit bestaat. `sign --url` pint de download vast op zijn
    /// eigen tag, dus tekenen vóórdat die tag bestaat levert een manifest op dat perfect
    /// klopt boven een download die 404't — en de app meldt dan niets, want een
    /// onbereikbare feed is een normale toestand.
    Check,
}

fn main() -> Result<()> {
    match Args::parse().opdracht {
        Opdracht::Keygen { out } => keygen(&out),
        Opdracht::Sign {
            key,
            exe,
            version,
            url,
            out,
        } => sign(&key, &exe, &version, &url, &out),
        Opdracht::Verify { manifest } => verify(&manifest),
        Opdracht::Check => check(),
    }
}

fn check() -> Result<()> {
    println!("feed: {}", fitcom::release::MANIFEST_URL);
    let rel = fitcom::release::haal_manifest()?;
    println!("manifest: versie {} ({} bytes)", rel.version, rel.size);
    fitcom::release::controleer(&rel).context("het gepubliceerde manifest wordt geweigerd")?;
    println!("handtekening: goedgekeurd door de sleutel in deze build");

    let status = fitcom::release::bereikbaar(&rel)?;
    println!("{} -> HTTP {status}", rel.url);
    anyhow::ensure!(
        (200..300).contains(&status),
        "de exe uit het manifest staat er niet; \
         is er wel een release met die tag, met fitcom.exe erin?"
    );

    let eigen = env!("CARGO_PKG_VERSION");
    if fitcom_proto::is_newer(&rel.version, eigen) {
        println!("deze build ({eigen}) zou hem aanbieden.");
    } else {
        println!("deze build ({eigen}) is al even nieuw; hij biedt niets aan.");
    }
    Ok(())
}

fn verify(manifest: &std::path::Path) -> Result<()> {
    let tekst = std::fs::read_to_string(manifest).context("manifest lezen")?;
    let rel: Release = serde_json::from_str(&tekst).context("manifest is geen geldige JSON")?;
    fitcom::release::controleer(&rel)?;
    println!(
        "OK — versie {} ({} bytes) wordt door deze build geaccepteerd.",
        rel.version, rel.size
    );
    println!("{}", rel.url);
    Ok(())
}

fn keygen(out: &std::path::Path) -> Result<()> {
    anyhow::ensure!(
        !out.exists(),
        "{} bestaat al — een bestaande sleutel overschrijven zet iedereen buitenspel",
        out.display()
    );
    let rng = ring::rand::SystemRandom::new();
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng)
        .map_err(|_| anyhow::anyhow!("sleutel genereren mislukt"))?;
    let paar = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref())
        .map_err(|_| anyhow::anyhow!("verse sleutel niet in te lezen"))?;
    std::fs::write(out, pkcs8.as_ref()).context("privésleutel wegschrijven")?;

    println!("privésleutel: {}", out.display());
    println!("Zet deze regel in crates/app/src/release.rs:\n");
    println!(
        "const PUBLIEKE_SLEUTEL_HEX: &str =\n    \"{}\";",
        bytes_naar_hex(paar.public_key().as_ref())
    );
    Ok(())
}

fn sign(
    key: &std::path::Path,
    exe: &std::path::Path,
    version: &str,
    url: &str,
    out: &std::path::Path,
) -> Result<()> {
    let pkcs8 = std::fs::read(key).context("privésleutel lezen")?;
    let paar = Ed25519KeyPair::from_pkcs8(&pkcs8)
        .map_err(|_| anyhow::anyhow!("privésleutel is geen geldige PKCS#8-Ed25519-sleutel"))?;

    let bytes = std::fs::read(exe).context("exe lezen")?;
    let size = bytes.len() as u64;
    let hash = bytes_naar_hex(blake3::hash(&bytes).as_bytes());

    // Via `Release::ondertekend_bericht`, zodat wat hier getekend wordt per definitie is
    // wat de app straks verifieert. Zou dat uit elkaar lopen, dan keurt geen enkele
    // client de release nog goed.
    let rel = Release {
        version: version.to_string(),
        url: url.to_string(),
        size,
        hash,
        signature: String::new(),
    };
    let signature = bytes_naar_hex(paar.sign(rel.ondertekend_bericht().as_bytes()).as_ref());

    let manifest = serde_json::json!({
        "version": rel.version,
        "url": rel.url,
        "size": rel.size,
        "hash": rel.hash,
        "signature": signature,
    });
    std::fs::write(out, serde_json::to_vec_pretty(&manifest)?).context("manifest wegschrijven")?;

    println!(
        "{} geschreven voor versie {version} ({size} bytes)",
        out.display()
    );
    println!("Upload dit bestand én {} naar de release.", exe.display());
    Ok(())
}
