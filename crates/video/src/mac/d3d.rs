//! De macOS-tegenhanger van `d3d.rs`: geen gedeeld GPU-apparaat, wel hetzelfde
//! frametype voor de gedeelde code.
//!
//! VideoToolbox en ScreenCaptureKit hebben geen equivalent van het ene D3D11-apparaat
//! dat capture, codec en weergave delen — IOSurface-gebackte `CVPixelBuffer`s reizen
//! daar vanzelf tussen de lagen. `D3dContext` blijft bestaan (lege struct) zodat
//! `engine.rs`, `deler.rs` en `kijker.rs` op beide platforms ongewijzigd compileren;
//! de naam is historisch en bewust niet hernoemd.

use anyhow::{bail, Context, Result};
use objc2_core_foundation::{CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_core_video::{
    kCVPixelBufferIOSurfacePropertiesKey, kCVPixelBufferPixelFormatTypeKey,
    kCVPixelFormatType_32BGRA, CVPixelBuffer, CVPixelBufferCreate, CVPixelBufferGetBaseAddress,
    CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight, CVPixelBufferGetWidth,
    CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
};
use std::ptr::NonNull;

/// Het frametype dat door de gedeelde code stroomt: een vastgehouden `CVPixelBuffer`
/// (BGRA, IOSurface-gebackt). Het binnenveld is bewust privé: methoden lenen daardoor
/// de hele struct, zodat een `move`-closure nooit per ongeluk alleen het niet-`Send`
/// binnenveld vangt en de `Send`-verklaring hieronder omzeilt.
#[derive(Clone)]
pub struct Beeld(CFRetained<CVPixelBuffer>);

// SAFETY: CoreFoundation-refcounts zijn atomair en een gedecodeerd of opgenomen beeld
// wordt na aanmaak alleen nog gelezen. Elk beeld wordt bovendien door één thread
// tegelijk gebruikt (de deel- of kijk-thread), net als de textuur op Windows.
unsafe impl Send for Beeld {}
unsafe impl Sync for Beeld {}

impl Beeld {
    pub(crate) fn nieuw(pb: CFRetained<CVPixelBuffer>) -> Self {
        Self(pb)
    }

    /// De onderliggende buffer, voor de mac-modules onderling (codec, venster).
    pub(crate) fn cv(&self) -> &CVPixelBuffer {
        &self.0
    }
}

/// Afmetingen van een bestaand beeld. Zelfde signatuur als de Windows-kant.
pub fn afmetingen(beeld: &Beeld) -> (u32, u32) {
    (
        CVPixelBufferGetWidth(beeld.cv()) as u32,
        CVPixelBufferGetHeight(beeld.cv()) as u32,
    )
}

/// De pixelbuffer-attributen die alle mac-modules gebruiken: BGRA op een IOSurface,
/// zodat encoder, decoder en weergavelaag zonder kopie bij het beeld kunnen.
pub(crate) fn bgra_attrs() -> CFRetained<CFDictionary<CFString, CFType>> {
    let sleutels: [&CFString; 2] = [unsafe { kCVPixelBufferPixelFormatTypeKey }, unsafe {
        kCVPixelBufferIOSurfacePropertiesKey
    }];
    let leeg = CFDictionary::<CFString, CFType>::from_slices(&[], &[]);
    let formaat = CFNumber::new_i64(kCVPixelFormatType_32BGRA as i64);
    let waarden: [&CFType; 2] = [formaat.as_ref(), leeg.as_ref()];
    CFDictionary::<CFString, CFType>::from_slices(&sleutels, &waarden)
}

#[derive(Clone)]
pub struct D3dContext;

impl D3dContext {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    /// Een BGRA-beeld met inhoud. `pixels` is `breedte * hoogte * 4` bytes.
    /// Alleen nodig om iets bekends de keten in te sturen, net als op Windows.
    pub fn maak_textuur_met(&self, breedte: u32, hoogte: u32, pixels: &[u8]) -> Result<Beeld> {
        let nodig = (breedte as usize) * (hoogte as usize) * 4;
        if pixels.len() != nodig {
            bail!("{} bytes aangeleverd, {nodig} verwacht", pixels.len());
        }

        let attrs = bgra_attrs();
        let mut pb: *mut CVPixelBuffer = std::ptr::null_mut();
        let status = unsafe {
            CVPixelBufferCreate(
                None,
                breedte as usize,
                hoogte as usize,
                kCVPixelFormatType_32BGRA,
                Some(attrs.as_opaque()),
                NonNull::from(&mut pb),
            )
        };
        if status != 0 {
            bail!("CVPixelBufferCreate gaf {status}");
        }
        let pb = unsafe {
            CFRetained::from_raw(NonNull::new(pb).context("CVPixelBufferCreate gaf geen buffer")?)
        };

        // SAFETY: de buffer is net aangemaakt met deze afmetingen; rijen kunnen breder
        // zijn dan `breedte * 4` (uitlijning), vandaar per rij kopiëren.
        unsafe {
            CVPixelBufferLockBaseAddress(&pb, CVPixelBufferLockFlags::empty());
            let basis = CVPixelBufferGetBaseAddress(&pb) as *mut u8;
            let stap = CVPixelBufferGetBytesPerRow(&pb);
            for y in 0..hoogte as usize {
                let bron = &pixels[y * breedte as usize * 4..][..breedte as usize * 4];
                std::ptr::copy_nonoverlapping(bron.as_ptr(), basis.add(y * stap), bron.len());
            }
            CVPixelBufferUnlockBaseAddress(&pb, CVPixelBufferLockFlags::empty());
        }
        Ok(Beeld::nieuw(pb))
    }

    /// Haalt een BGRA-beeld terug naar het werkgeheugen, rijen aaneengesloten.
    /// Traag pad, alleen voor controles — zelfde rol als op Windows.
    pub fn lees_bgra(&self, beeld: &Beeld) -> Result<(u32, u32, Vec<u8>)> {
        let (breedte, hoogte) = afmetingen(beeld);
        let pb = beeld.cv();
        // SAFETY: alleen-lezen vergrendeling; de rijen komen uit de buffer zelf.
        let uit = unsafe {
            CVPixelBufferLockBaseAddress(pb, CVPixelBufferLockFlags::ReadOnly);
            let basis = CVPixelBufferGetBaseAddress(pb) as *const u8;
            let stap = CVPixelBufferGetBytesPerRow(pb);
            let mut uit = Vec::with_capacity((breedte * hoogte * 4) as usize);
            for y in 0..hoogte as usize {
                uit.extend_from_slice(std::slice::from_raw_parts(
                    basis.add(y * stap),
                    breedte as usize * 4,
                ));
            }
            CVPixelBufferUnlockBaseAddress(pb, CVPixelBufferLockFlags::ReadOnly);
            uit
        };
        Ok((breedte, hoogte, uit))
    }

    /// Verkleint een BGRA-beeld naar `doel_breedte`×`doel_hoogte` met
    /// dichtstbijzijnde-pixel-bemonstering — de miniatuur voor het hoofdvenster,
    /// spiegelbeeldig aan de Windows-staging-uitlezing.
    pub fn lees_bgra_miniatuur(
        &self,
        beeld: &Beeld,
        doel_breedte: u32,
        doel_hoogte: u32,
    ) -> Result<Vec<u8>> {
        let (breedte, hoogte) = afmetingen(beeld);
        if breedte == 0 || hoogte == 0 {
            bail!("beeld zonder afmeting");
        }
        let pb = beeld.cv();
        // SAFETY: alleen-lezen vergrendeling; bemonsterde coördinaten blijven binnen
        // de afmetingen die de buffer zelf opgeeft.
        let uit = unsafe {
            CVPixelBufferLockBaseAddress(pb, CVPixelBufferLockFlags::ReadOnly);
            let basis = CVPixelBufferGetBaseAddress(pb) as *const u8;
            let stap = CVPixelBufferGetBytesPerRow(pb);
            let mut uit = vec![0u8; (doel_breedte * doel_hoogte * 4) as usize];
            for y in 0..doel_hoogte {
                let bron_y = (y * hoogte) / doel_hoogte.max(1);
                let rij = basis.add(bron_y as usize * stap);
                for x in 0..doel_breedte {
                    let bron_x = (x * breedte) / doel_breedte.max(1);
                    let bron = std::slice::from_raw_parts(rij.add(bron_x as usize * 4), 4);
                    let doel = ((y * doel_breedte + x) * 4) as usize;
                    uit[doel..doel + 4].copy_from_slice(bron);
                }
            }
            CVPixelBufferUnlockBaseAddress(pb, CVPixelBufferLockFlags::ReadOnly);
            uit
        };
        Ok(uit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beeld_is_te_maken_en_terug_te_lezen() {
        let d3d = D3dContext::new().expect("context");
        let pixels: Vec<u8> = (0..64u32 * 36 * 4).map(|i| (i % 251) as u8).collect();
        let beeld = d3d.maak_textuur_met(64, 36, &pixels).expect("beeld");
        assert_eq!(afmetingen(&beeld), (64, 36));

        let (b, h, terug) = d3d.lees_bgra(&beeld).expect("uitlezen");
        assert_eq!((b, h), (64, 36));
        assert_eq!(terug, pixels, "pixels veranderd onderweg");

        let mini = d3d.lees_bgra_miniatuur(&beeld, 16, 9).expect("miniatuur");
        assert_eq!(mini.len(), 16 * 9 * 4);
        // De eerste bemonsterde pixel is het origineel op (0,0).
        assert_eq!(&mini[..4], &pixels[..4]);
    }
}
