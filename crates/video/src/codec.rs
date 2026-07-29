//! Hardware-encoder en -decoder via Media Foundation.
//!
//! # Waarom H.264 de standaard is en niet HEVC
//!
//! Zie `docs/SPEC.md`. Kort: encoden kan met beide, maar HEVC *decoderen* loopt op
//! Windows via een Store-uitbreiding die er niet standaard op zit. Bij 1 Gbit is de
//! bitrate-winst van HEVC irrelevant en een codec die misschien niet werkt bij de
//! ontvanger is dat niet.
//!
//! # Waarom er geen kleurconversie in zit
//!
//! De hardware-encoders op deze machines accepteren `ARGB32` rechtstreeks — precies
//! wat de schermopname levert. Er is dus geen omzetting naar NV12 nodig, en het beeld
//! gaat van de capture zonder tussenstap de encoder in.
//!
//! # Waarom de gebeurtenislus
//!
//! Hardware-MFT's zijn asynchroon: je duwt niet gewoon een frame erin en haalt er een
//! pakket uit. De transform meldt via gebeurtenissen wanneer hij invoer wil en wanneer
//! er uitvoer klaarstaat. Dat model moet je volgen, ook al ziet de API van deze module
//! er synchroon uit.

use crate::d3d::D3dContext;
use crate::mf;
use anyhow::{bail, Context, Result};
use windows::core::{Interface, GUID};
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Media::MediaFoundation::*;

/// 100-nanoseconden-eenheden: de tijdrekening van Media Foundation.
pub const HNS_PER_SEC: i64 = 10_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    H264,
    Hevc,
}

impl Codec {
    fn subtype(self) -> GUID {
        match self {
            Codec::H264 => MFVideoFormat_H264,
            Codec::Hevc => MFVideoFormat_HEVC,
        }
    }

    pub fn payload_type(self) -> fitcom_proto::PayloadType {
        match self {
            Codec::H264 => fitcom_proto::PayloadType::H264,
            Codec::Hevc => fitcom_proto::PayloadType::HEVC,
        }
    }

    pub fn naam(self) -> &'static str {
        match self {
            Codec::H264 => "h264",
            Codec::Hevc => "hevc",
        }
    }

    pub fn van_naam(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "h264" | "avc" => Some(Codec::H264),
            "hevc" | "h265" => Some(Codec::Hevc),
            _ => None,
        }
    }

    /// Of deze machine hem kan decoderen. HEVC hangt af van een Store-uitbreiding,
    /// dus dit is geen theoretische vraag.
    pub fn kan_decoderen(self) -> bool {
        let vlaggen = MFT_ENUM_FLAG(
            MFT_ENUM_FLAG_HARDWARE.0
                | MFT_ENUM_FLAG_SYNCMFT.0
                | MFT_ENUM_FLAG_ASYNCMFT.0
                | MFT_ENUM_FLAG_SORTANDFILTER.0,
        );
        mf::zoek_transform_met(false, self.subtype(), MFVideoFormat_NV12, vlaggen)
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    }
}

#[derive(Debug, Clone)]
pub struct EncoderConfig {
    pub codec: Codec,
    pub breedte: u32,
    pub hoogte: u32,
    pub fps: u32,
    pub bitrate: u32,
}

pub struct Encoder {
    transform: IMFTransform,
    events: IMFMediaEventGenerator,
    codec_api: Option<ICodecAPI>,
    frame_duur: i64,
    /// De encoder heeft om een beeld gevraagd en wacht erop.
    wacht_op_invoer: bool,
}

// SAFETY: de transform wordt uitsluitend vanaf de capture-thread gebruikt; MF-objecten
// zijn niet thread-affiene zolang je ze niet vanaf meerdere threads tegelijk aanroept.
unsafe impl Send for Encoder {}

impl Encoder {
    pub fn new(d3d: &D3dContext, cfg: &EncoderConfig) -> Result<Self> {
        mf::zorg_dat_mf_draait();
        let manager = mf::device_manager(&d3d.device)?;

        let activates = mf::zoek_transform(true, MFVideoFormat_ARGB32, cfg.codec.subtype())
            .context("encoder zoeken")?;
        // Voorkeur voor de NVIDIA-encoder: alle drie de peers hebben er een, en op een
        // machine met twee GPU's pakt de sortering anders soms de verkeerde.
        let activate = activates
            .iter()
            .find(|a| mf::naam_van(a).contains("NVIDIA"))
            .or_else(|| activates.first())
            .with_context(|| format!("geen hardware-encoder voor {} gevonden", cfg.codec.naam()))?;

        tracing::info!(encoder = %mf::naam_van(activate), codec = cfg.codec.naam(), "encoder gekozen");

        // SAFETY: de activate komt uit MFTEnumEx en is geldig.
        let transform: IMFTransform =
            unsafe { activate.ActivateObject().context("encoder activeren")? };

        // Asynchrone MFT's willen expliciet ontgrendeld worden voordat je ze mag
        // gebruiken; zonder dit weigert elke aanroep.
        // SAFETY: `transform` is net aangemaakt.
        unsafe {
            let attrs = transform.GetAttributes().context("encoder-attributen")?;
            attrs
                .SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1)
                .context("encoder ontgrendelen")?;
            // Weinig latency is belangrijker dan maximale compressie.
            let _ = attrs.SetUINT32(&MF_LOW_LATENCY, 1);

            transform
                .ProcessMessage(
                    MFT_MESSAGE_SET_D3D_MANAGER,
                    std::mem::transmute_copy::<IMFDXGIDeviceManager, usize>(&manager),
                )
                .context("D3D-apparaat aan encoder koppelen")?;
        }

        let frame_duur = HNS_PER_SEC / i64::from(cfg.fps.max(1));

        // Uitvoertype eerst: de encoder wil weten wat hij moet maken voordat hij zegt
        // welke invoer daarbij past.
        // SAFETY: alle types worden volledig ingevuld voordat ze gezet worden.
        unsafe {
            let uit = MFCreateMediaType().context("uitvoertype")?;
            mf::zet_video_type(&uit, cfg.codec.subtype())?;
            uit.SetUINT32(&MF_MT_AVG_BITRATE, cfg.bitrate)?;
            uit.SetUINT64(&MF_MT_FRAME_SIZE, mf::pak(cfg.breedte, cfg.hoogte))?;
            uit.SetUINT64(&MF_MT_FRAME_RATE, mf::pak(cfg.fps, 1))?;
            uit.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, mf::pak(1, 1))?;
            uit.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            uit.SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 0)?;
            transform
                .SetOutputType(0, &uit, 0)
                .context("uitvoertype instellen")?;

            let in_type = MFCreateMediaType().context("invoertype")?;
            mf::zet_video_type(&in_type, MFVideoFormat_ARGB32)?;
            in_type.SetUINT64(&MF_MT_FRAME_SIZE, mf::pak(cfg.breedte, cfg.hoogte))?;
            in_type.SetUINT64(&MF_MT_FRAME_RATE, mf::pak(cfg.fps, 1))?;
            in_type.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, mf::pak(1, 1))?;
            in_type.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            transform
                .SetInputType(0, &in_type, 0)
                .context("invoertype instellen")?;
        }

        let codec_api: Option<ICodecAPI> = transform.cast().ok();
        let events: IMFMediaEventGenerator = transform
            .cast()
            .context("encoder levert geen gebeurtenissen; asynchrone MFT verwacht")?;

        // SAFETY: types zijn gezet, dus de encoder mag beginnen.
        unsafe {
            transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
        }

        Ok(Self {
            transform,
            events,
            codec_api,
            frame_duur,
            wacht_op_invoer: false,
        })
    }

    /// Vraagt het volgende beeld als keyframe. Nodig als een ontvanger meldt dat hij
    /// de draad kwijt is; zonder keyframe blijft zijn beeld kapot tot de volgende
    /// periodieke IDR.
    pub fn vraag_keyframe(&self) {
        let Some(api) = &self.codec_api else { return };
        // SAFETY: de variant is een VT_UI4 zoals de codec-API verwacht.
        unsafe {
            let waarde = windows::Win32::System::Variant::VARIANT::from(1u32);
            let _ = api.SetValue(&CODECAPI_AVEncVideoForceKeyFrame, &waarde);
        }
    }

    /// Codeert één beeld. Levert nul of meer pakketten op: de encoder loopt een frame
    /// of twee achter, dus de eerste aanroepen kunnen leeg blijven.
    pub fn encode(&mut self, tex: &ID3D11Texture2D, tijd_hns: i64) -> Result<Vec<Vec<u8>>> {
        // SAFETY: `tex` blijft geldig zolang de aanroeper hem vasthoudt, en dat is
        // langer dan deze functie duurt.
        let sample = unsafe {
            let buffer = MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, tex, 0, false)
                .context("textuur in een MF-buffer verpakken")?;
            let lengte = buffer
                .cast::<IMF2DBuffer>()
                .and_then(|b| b.GetContiguousLength())
                .unwrap_or(0);
            buffer.SetCurrentLength(lengte)?;

            let sample = MFCreateSample().context("sample aanmaken")?;
            sample.AddBuffer(&buffer)?;
            sample.SetSampleTime(tijd_hns)?;
            sample.SetSampleDuration(self.frame_duur)?;
            sample
        };

        let mut uit = Vec::new();

        // Wachten tot de encoder om invoer vraagt. Dat moet blokkerend: met alleen
        // polsen mis je de gebeurtenis en dan blijft de encoder eeuwig stilstaan —
        // precies de fout die deze code eerder had.
        while !self.wacht_op_invoer {
            let Some(event) = self.volgend_event(true)? else {
                bail!("encoder leverde geen gebeurtenis meer");
            };
            self.verwerk_event(&event, &mut uit)?;
        }

        // SAFETY: de encoder heeft net om invoer gevraagd.
        unsafe { self.transform.ProcessInput(0, &sample, 0)? };
        self.wacht_op_invoer = false;

        // Alles wat al klaarstaat meenemen, maar niet wachten: de encoder loopt een
        // frame of twee achter en dat is normaal.
        while let Some(event) = self.volgend_event(false)? {
            self.verwerk_event(&event, &mut uit)?;
        }

        Ok(uit)
    }

    fn verwerk_event(&mut self, event: &IMFMediaEvent, uit: &mut Vec<Vec<u8>>) -> Result<()> {
        // SAFETY: het event komt net uit de wachtrij.
        let soort = unsafe { event.GetType()? };
        if soort == METransformNeedInput.0 as u32 {
            self.wacht_op_invoer = true;
        } else if soort == METransformHaveOutput.0 as u32 {
            if let Some(pakket) = self.haal_uitvoer()? {
                uit.push(pakket);
            }
        }
        Ok(())
    }

    fn volgend_event(&self, blokkeren: bool) -> Result<Option<IMFMediaEvent>> {
        let vlag = if blokkeren {
            MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS(0)
        } else {
            MF_EVENT_FLAG_NO_WAIT
        };
        // SAFETY: `events` is geldig; bij "geen event" geeft MF een bekende foutcode.
        match unsafe { self.events.GetEvent(vlag) } {
            Ok(e) => Ok(Some(e)),
            Err(e) if e.code() == MF_E_NO_EVENTS_AVAILABLE => Ok(None),
            Err(e) if e.code() == MF_E_MULTIPLE_SUBSCRIBERS => Ok(None),
            Err(e) => Err(e).context("encoder-gebeurtenis ophalen"),
        }
    }

    fn haal_uitvoer(&self) -> Result<Option<Vec<u8>>> {
        let mut buffers = [MFT_OUTPUT_DATA_BUFFER::default()];
        let mut status = 0u32;

        // SAFETY: de encoder levert zijn eigen samples aan (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES).
        unsafe {
            match self.transform.ProcessOutput(0, &mut buffers, &mut status) {
                Ok(()) => {}
                Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(None),
                Err(e) => return Err(e).context("uitvoer ophalen"),
            }
        }

        let Some(sample) = buffers[0].pSample.take() else {
            return Ok(None);
        };

        // SAFETY: het sample komt net uit de encoder en bevat één samenhangende buffer.
        let data = unsafe {
            let buffer = sample.ConvertToContiguousBuffer()?;
            let mut ptr = std::ptr::null_mut();
            let mut lengte = 0u32;
            buffer.Lock(&mut ptr, None, Some(&mut lengte))?;
            let data = std::slice::from_raw_parts(ptr, lengte as usize).to_vec();
            let _ = buffer.Unlock();
            data
        };

        Ok((!data.is_empty()).then_some(data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "vereist een GPU met hardware-encoder"]
    fn decoders_zijn_aanwezig_zoals_verwacht() {
        // Deze test legt de bevinding vast die de codeckeuze bepaalde. Slaat hij om,
        // dan is de aanname veranderd en verdient de keuze een heroverweging.
        assert!(
            Codec::H264.kan_decoderen(),
            "H.264 hoort altijd in Windows te zitten"
        );
        println!(
            "HEVC decodeerbaar op deze machine: {}",
            Codec::Hevc.kan_decoderen()
        );
    }

    #[test]
    #[ignore = "vereist een GPU met hardware-encoder"]
    fn encoder_levert_pakketten_voor_een_echt_beeld() {
        let d3d = D3dContext::new().expect("D3D11");
        let cfg = EncoderConfig {
            codec: Codec::H264,
            breedte: 1920,
            hoogte: 1080,
            fps: 60,
            bitrate: 25_000_000,
        };
        let mut enc = Encoder::new(&d3d, &cfg).expect("encoder");
        let tex = d3d.maak_textuur(1920, 1080).expect("textuur");

        let mut totaal = 0usize;
        let mut pakketten = 0usize;
        for i in 0..30 {
            let tijd = i as i64 * (HNS_PER_SEC / 60);
            for p in enc.encode(&tex, tijd).expect("encoderen") {
                totaal += p.len();
                pakketten += 1;
            }
        }
        println!("{pakketten} pakketten, {totaal} bytes uit 30 beelden");
        assert!(pakketten > 0, "encoder leverde helemaal niets op");
    }
}
