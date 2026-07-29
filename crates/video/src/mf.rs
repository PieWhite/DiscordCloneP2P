//! Gedeelde Media Foundation-plumbing voor encoder en decoder.
//!
//! Media Foundation moet één keer per proces opgestart worden en verwacht dat de
//! aanroepende thread een COM-apartment heeft. Beide gebeuren hier, precies één keer,
//! want tweemaal opstarten of een verkeerd apartment levert fouten op die verderop
//! nergens meer op te herleiden zijn.

use anyhow::{Context, Result};
use std::sync::Once;
use windows::core::GUID;
use windows::Win32::Media::MediaFoundation::{
    IMFActivate, IMFAttributes, IMFDXGIDeviceManager, IMFMediaType, IMFTransform,
    MFCreateDXGIDeviceManager, MFMediaType_Video, MFStartup, MFTEnumEx, MFVideoFormat_ARGB32,
    MFVideoFormat_HEVC, MFVideoFormat_NV12, MFSTARTUP_NOSOCKET, MFT_CATEGORY_VIDEO_DECODER,
    MFT_CATEGORY_VIDEO_ENCODER, MFT_ENUM_FLAG, MFT_ENUM_FLAG_HARDWARE, MFT_ENUM_FLAG_SORTANDFILTER,
    MFT_REGISTER_TYPE_INFO, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_VERSION,
};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

static START: Once = Once::new();

/// Idempotent: elke thread die Media Foundation aanraakt roept dit eerst aan.
pub fn zorg_dat_mf_draait() {
    START.call_once(|| {
        // SAFETY: eenmalig per proces, vóór elk ander MF-gebruik.
        unsafe {
            // Multithreaded apartment: onze media-threads hebben geen message pump.
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            if let Err(e) = MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET) {
                tracing::error!(error = %e, "Media Foundation kon niet starten");
            }
        }
    });
}

/// Koppelt het D3D11-apparaat aan Media Foundation, zodat encoder en decoder met
/// texturen kunnen werken in plaats van met buffers in het werkgeheugen.
pub fn device_manager(
    device: &windows::Win32::Graphics::Direct3D11::ID3D11Device,
) -> Result<IMFDXGIDeviceManager> {
    zorg_dat_mf_draait();
    let mut token = 0u32;
    let mut manager = None;
    // SAFETY: beide uitvoerparameters zijn geldige lokale variabelen.
    unsafe {
        MFCreateDXGIDeviceManager(&mut token, &mut manager).context("DXGI device manager")?;
    }
    let manager = manager.context("MF gaf geen device manager")?;
    // SAFETY: `device` is geldig en `token` komt uit de aanmaak hierboven.
    unsafe {
        manager
            .ResetDevice(device, token)
            .context("apparaat aan MF koppelen")?;
    }
    Ok(manager)
}

/// Zoekt de hardware-MFT die van `invoer` naar `uitvoer` kan.
pub fn zoek_transform(encoder: bool, invoer: GUID, uitvoer: GUID) -> Result<Vec<IMFActivate>> {
    zoek_transform_met(
        encoder,
        invoer,
        uitvoer,
        MFT_ENUM_FLAG(MFT_ENUM_FLAG_HARDWARE.0 | MFT_ENUM_FLAG_SORTANDFILTER.0),
    )
}

/// Zelfde, maar met zelfgekozen zoekvlaggen.
pub fn zoek_transform_met(
    encoder: bool,
    invoer: GUID,
    uitvoer: GUID,
    vlaggen: MFT_ENUM_FLAG,
) -> Result<Vec<IMFActivate>> {
    zorg_dat_mf_draait();

    let in_info = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: invoer,
    };
    let uit_info = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: uitvoer,
    };

    let categorie = if encoder {
        MFT_CATEGORY_VIDEO_ENCODER
    } else {
        MFT_CATEGORY_VIDEO_DECODER
    };

    let mut activates: *mut Option<IMFActivate> = std::ptr::null_mut();
    let mut aantal = 0u32;

    // SAFETY: uitvoerparameters zijn geldig; het geretourneerde blok wordt hieronder
    // met CoTaskMemFree vrijgegeven nadat we de interfaces overgenomen hebben.
    unsafe {
        MFTEnumEx(
            categorie,
            vlaggen,
            Some(&in_info),
            Some(&uit_info),
            &mut activates,
            &mut aantal,
        )
        .context("MFT's opzoeken")?;
    }

    let mut uit = Vec::new();
    // SAFETY: MFTEnumEx vulde `aantal` items op `activates`.
    unsafe {
        for i in 0..aantal as usize {
            if let Some(a) = (*activates.add(i)).take() {
                uit.push(a);
            }
        }
        windows::Win32::System::Com::CoTaskMemFree(Some(activates as *const _));
    }
    Ok(uit)
}

/// Naam van een MFT, voor in de logs. Handig om te zien of je de hardware-encoder te
/// pakken hebt of per ongeluk de software-variant.
pub fn naam_van(activate: &IMFActivate) -> String {
    use windows::Win32::Media::MediaFoundation::MFT_FRIENDLY_NAME_Attribute;
    let attrs: &IMFAttributes = activate.into();
    // SAFETY: het attribuut hoeft niet te bestaan; dat vangen we af.
    unsafe {
        let Ok(lengte) = attrs.GetStringLength(&MFT_FRIENDLY_NAME_Attribute) else {
            return "onbekend".into();
        };
        let mut buf = vec![0u16; lengte as usize + 1];
        if attrs
            .GetString(&MFT_FRIENDLY_NAME_Attribute, &mut buf, None)
            .is_err()
        {
            return "onbekend".into();
        }
        String::from_utf16_lossy(&buf[..lengte as usize])
    }
}

/// Alle invoerformaten die deze transform accepteert. Gebruikt om te bepalen of we
/// zelf van BGRA naar NV12 moeten omzetten of dat de encoder dat al aankan.
pub fn invoerformaten(transform: &IMFTransform) -> Vec<GUID> {
    let mut uit = Vec::new();
    // SAFETY: we lopen door tot de API zegt dat er niets meer is.
    unsafe {
        for i in 0..64 {
            let Ok(mt) = transform.GetInputAvailableType(0, i) else {
                break;
            };
            if let Ok(sub) = mt.GetGUID(&MF_MT_SUBTYPE) {
                uit.push(sub);
            }
        }
    }
    uit
}

/// Leesbare naam van de bekende formaten, alleen voor logs en tests.
pub fn formaatnaam(g: GUID) -> String {
    if g == MFVideoFormat_NV12 {
        "NV12".into()
    } else if g == MFVideoFormat_ARGB32 {
        "ARGB32".into()
    } else if g == MFVideoFormat_HEVC {
        "HEVC".into()
    } else {
        format!("{g:?}")
    }
}

/// Zet het hoofdtype en subtype in één keer; dat komt in elke type-opbouw terug.
pub fn zet_video_type(mt: &IMFMediaType, subtype: GUID) -> Result<()> {
    // SAFETY: `mt` is geldig en beide GUID's zijn constanten.
    unsafe {
        mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        mt.SetGUID(&MF_MT_SUBTYPE, &subtype)?;
    }
    Ok(())
}

/// Breedte en hoogte zitten samengepakt in één 64-bits attribuut.
pub fn pak(hoog: u32, laag: u32) -> u64 {
    (u64::from(hoog) << 32) | u64::from(laag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "vereist een GPU met hardware-encoder"]
    fn welke_hevc_encoder_heeft_deze_machine() {
        let encoders = zoek_transform(true, MFVideoFormat_NV12, MFVideoFormat_HEVC).unwrap();
        println!("HEVC-encoders die NV12 aannemen: {}", encoders.len());
        for a in &encoders {
            println!("  {}", naam_van(a));
        }
        assert!(!encoders.is_empty(), "geen hardware HEVC-encoder gevonden");

        // Neemt hij ook BGRA rechtstreeks aan? Zo ja, dan hoeven we zelf niet om te
        // zetten en scheelt dat een hele stap in het hot path.
        let direct = zoek_transform(true, MFVideoFormat_ARGB32, MFVideoFormat_HEVC).unwrap();
        println!("HEVC-encoders die ARGB32 aannemen: {}", direct.len());
        for a in &direct {
            println!("  {}", naam_van(a));
        }

        // En wat zegt de encoder zelf als we het hem vragen?
        // SAFETY: `ActivateObject` levert de transform waar de activate voor staat.
        let transform: IMFTransform = unsafe { encoders[0].ActivateObject().unwrap() };
        let formaten = invoerformaten(&transform);
        println!("invoerformaten volgens de encoder:");
        for f in &formaten {
            println!("  {}", formaatnaam(*f));
        }
    }

    #[test]
    #[ignore = "vereist een GPU"]
    fn welke_decoders_heeft_deze_machine() {
        use windows::Win32::Media::MediaFoundation::{
            MFVideoFormat_H264, MFT_ENUM_FLAG_ASYNCMFT, MFT_ENUM_FLAG_SYNCMFT,
        };

        // HEVC-decoding op Windows zit soms achter de "HEVC Video Extensions" uit de
        // Store. Alleen op hardware-MFT's zoeken zegt dus te weinig; we kijken breed.
        let alle = MFT_ENUM_FLAG(
            MFT_ENUM_FLAG_HARDWARE.0
                | MFT_ENUM_FLAG_SYNCMFT.0
                | MFT_ENUM_FLAG_ASYNCMFT.0
                | MFT_ENUM_FLAG_SORTANDFILTER.0,
        );

        for (naam, formaat) in [("HEVC", MFVideoFormat_HEVC), ("H.264", MFVideoFormat_H264)] {
            let gevonden = zoek_transform_met(false, formaat, MFVideoFormat_NV12, alle).unwrap();
            println!("{naam}-decoders: {}", gevonden.len());
            for a in &gevonden {
                println!("  {}", naam_van(a));
            }
        }
    }

    #[test]
    #[ignore = "vereist een GPU"]
    fn welke_h264_encoder_heeft_deze_machine() {
        use windows::Win32::Media::MediaFoundation::MFVideoFormat_H264;
        let encoders = zoek_transform(true, MFVideoFormat_ARGB32, MFVideoFormat_H264).unwrap();
        println!("H.264-encoders die ARGB32 aannemen: {}", encoders.len());
        for a in &encoders {
            println!("  {}", naam_van(a));
        }
    }
}
