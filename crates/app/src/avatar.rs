//! Profielfoto's (`OpKind::SetAvatar`, 2026-10-23).
//!
//! Twee kanten. [`maak`] maakt van een gekozen plaatje de bytes die de draad op gaan: een
//! vierkant uitgesneden JPEG van [`ZIJDE`] pixels, een paar KiB. [`op_schijf`] zet wat er
//! uit de log komt in `<data>/avatars/`, zodat het venster het van schijf kan tekenen —
//! het venster krijgt alleen die paden (`Snapshot::avatars`, `Ui::image_path_allowed`).
//!
//! De bytes van een ander komen van een andere pc. Daarom kijkt [`op_schijf`] zelf naar het
//! formaat en de afmeting voordat er iets op schijf komt, en is de bestandsnaam de hash van
//! de inhoud en nooit iets uit de op (B-03-klasse).

use anyhow::{ensure, Context, Result};
use fitcom_proto::op::MAX_AVATAR_LEN;
use image::{imageops::FilterType, ImageFormat, ImageReader, Limits};
use std::io::Cursor;
use std::path::{Path, PathBuf};

/// Breedte en hoogte van een profielfoto die wij maken. De grootste avatar in het venster
/// is 32 px, dus dit is scherp tot een schaal van 400%.
pub const ZIJDE: u32 = 128;

/// Wat we van een ander aannemen. Ruimer dan [`ZIJDE`], zodat een latere versie groter mag
/// maken, maar klein genoeg dat een paar KiB nooit een textuur van honderden MiB wordt.
const MAX_ZIJDE: u32 = 512;

/// Leest een plaatje van schijf, snijdt het midden vierkant uit en maakt er een JPEG van.
pub fn maak(pad: &Path) -> Result<Vec<u8>> {
    let mut lezer = ImageReader::open(pad)
        .with_context(|| format!("{} openen", pad.display()))?
        .with_guessed_format()?;
    // Een foto van een telefoon past ruim; een bestand dat 100k × 100k belooft niet.
    let mut grens = Limits::default();
    grens.max_image_width = Some(16_384);
    grens.max_image_height = Some(16_384);
    grens.max_alloc = Some(512 * 1024 * 1024);
    lezer.limits(grens);
    let beeld = lezer.decode().context("geen plaatje dat we kunnen lezen")?;
    let mut rgba = beeld
        .resize_to_fill(ZIJDE, ZIJDE, FilterType::Lanczos3)
        .to_rgba8();
    // JPEG kent geen doorzichtigheid: op zwart leggen, de grond van het venster.
    for p in rgba.pixels_mut() {
        let a = p[3] as u16;
        for k in 0..3 {
            p[k] = (p[k] as u16 * a / 255) as u8;
        }
    }
    let rgb = image::DynamicImage::ImageRgba8(rgba).to_rgb8();
    let mut uit = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut uit, 85).encode_image(&rgb)?;
    ensure!(uit.len() <= MAX_AVATAR_LEN, "profielfoto te groot");
    Ok(uit)
}

/// Zet een profielfoto uit de log in `dir`, als het een PNG of JPEG van een redelijke maat
/// is, en geeft het pad. Staat hij er al (de naam is de hash), dan alleen het pad.
pub fn op_schijf(dir: &Path, bytes: &[u8]) -> Option<PathBuf> {
    let formaat = image::guess_format(bytes).ok()?;
    let ext = match formaat {
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Png => "png",
        _ => return None,
    };
    let (b, h) = ImageReader::with_format(Cursor::new(bytes), formaat)
        .into_dimensions()
        .ok()?;
    if b == 0 || h == 0 || b > MAX_ZIJDE || h > MAX_ZIJDE {
        return None;
    }
    let pad = dir.join(format!("{}.{ext}", blake3::hash(bytes).to_hex()));
    if pad.is_file() {
        return Some(pad);
    }
    let schrijf = || -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let deel = pad.with_extension("part");
        std::fs::write(&deel, bytes)?;
        std::fs::rename(&deel, &pad)
    };
    match schrijf() {
        Ok(()) => Some(pad),
        Err(e) => {
            tracing::warn!(error = %e, "profielfoto wegschrijven mislukt");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eigen_foto_komt_klein_terug_en_wordt_aangenomen() {
        let dir = std::env::temp_dir().join(format!("fitcom-avatar-{}", std::process::id()));
        let bron = dir.join("bron.png");
        std::fs::create_dir_all(&dir).unwrap();
        image::RgbaImage::from_fn(900, 400, |x, y| image::Rgba([x as u8, y as u8, 7, 128]))
            .save(&bron)
            .unwrap();

        let bytes = maak(&bron).unwrap();
        assert!(bytes.len() <= MAX_AVATAR_LEN);
        let pad = op_schijf(&dir, &bytes).unwrap();
        assert_eq!(image::image_dimensions(&pad).unwrap(), (ZIJDE, ZIJDE));
        // Tweede keer: zelfde pad, niets nieuws.
        assert_eq!(op_schijf(&dir, &bytes), Some(pad));

        // Rommel, een ander formaat en een te groot plaatje komen er niet in.
        assert_eq!(op_schijf(&dir, b"geen plaatje"), None);
        let mut gif = Vec::new();
        image::RgbaImage::new(4, 4)
            .write_to(&mut Cursor::new(&mut gif), ImageFormat::Gif)
            .unwrap();
        assert_eq!(op_schijf(&dir, &gif), None);
        let mut groot = Vec::new();
        image::GrayImage::new(MAX_ZIJDE + 1, 1)
            .write_to(&mut Cursor::new(&mut groot), ImageFormat::Png)
            .unwrap();
        assert_eq!(op_schijf(&dir, &groot), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
