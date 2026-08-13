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
use crate::kleur::{Kleuromzetter, Kleurruimte};
use crate::mf;
use anyhow::{bail, Context, Result};
use windows::core::{Interface, GUID};
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Media::MediaFoundation::*;

/// 100-nanoseconden-eenheden: de tijdrekening van Media Foundation.
pub const HNS_PER_SEC: i64 = 10_000_000;

/// Zoveel seconden tussen twee periodieke keyframes.
///
/// Dit is een vangnet, geen hersteltijd: wie aanhaakt of de draad kwijt is vraagt zelf om
/// een keyframe ([`Encoder::vraag_keyframe`]), over een betrouwbare QUIC-verbinding, en
/// wacht hier dus niet op.
///
/// **Stond op 2, en dat was duur.** Gemeten op 2026-08-02 bij 1080p op een budget van
/// 8 Mbit/s: een keyframe is 371 kB tegen 6 kB voor een gewoon beeld — 57×. Elke twee
/// seconden was dat 1,5 Mbit/s, bijna een vijfde van het hele budget, plus een stoot van
/// 346 datagrammen die op een echt internetpad zelf verlies veroorzaakt. Die piek is niet
/// te temmen: `CODECAPI_AVEncCommonRateControlMode`, `-MeanBitRate`, `-BufferSize` en
/// `-MaxBitRate` geven allemaal `S_OK` op de NVIDIA-MFT en veranderen geen byte, zowel
/// vóór als ná `SetOutputType`. Dus blijft alleen: minder vaak.
const GOP_SECONDEN: u32 = 10;

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

    /// De omgekeerde richting: welke codec er bij een payload-type op de draad hoort.
    /// `None` voor alles wat geen video is (Opus) of wat we nog niet kennen.
    pub fn van_payload(pt: fitcom_proto::PayloadType) -> Option<Self> {
        match pt {
            fitcom_proto::PayloadType::H264 => Some(Codec::H264),
            fitcom_proto::PayloadType::HEVC => Some(Codec::Hevc),
            _ => None,
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

/// Eén gecodeerd beeld zoals het de encoder verlaat.
#[derive(Debug, Clone)]
pub struct Pakket {
    /// Tijd van de encoder in 100-nanoseconden-eenheden. Hoort ongewijzigd door naar
    /// de tijdstempel op de draad: daaraan herkent de ontvanger één beeld.
    pub tijd_hns: i64,
    pub keyframe: bool,
    pub data: Vec<u8>,
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

        // De GOP-lengte moet vóór `SetOutputType`: daarna staat de encoder vast en neemt
        // de NVIDIA-MFT hem niet meer aan. Zie het bestandscommentaar van
        // `tests/encoder_gedrag.rs` voor de meting.
        let codec_api: Option<ICodecAPI> = transform.cast().ok();
        if let Some(api) = &codec_api {
            // SAFETY: de variant is een VT_UI4 zoals de codec-API verwacht.
            unsafe {
                let waarde =
                    windows::Win32::System::Variant::VARIANT::from(cfg.fps.max(1) * GOP_SECONDEN);
                if let Err(e) = api.SetValue(&CODECAPI_AVEncMPVGOPSize, &waarde) {
                    tracing::warn!(error = %e, "GOP-lengte instellen mislukt; de driver kiest zelf");
                }
            }
        }

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
            // De gedocumenteerde MF-manier om de keyframe-afstand te zetten. De
            // NVIDIA-MFT doet er niets mee — daarvoor staat de codec-API hieronder — maar
            // een andere encoder kijkt er wel naar, en fout kan het nooit staan.
            uit.SetUINT32(&MF_MT_MAX_KEYFRAME_SPACING, cfg.fps.max(1) * GOP_SECONDEN)?;
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
    pub fn encode(&mut self, tex: &ID3D11Texture2D, tijd_hns: i64) -> Result<Vec<Pakket>> {
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

    fn verwerk_event(&mut self, event: &IMFMediaEvent, uit: &mut Vec<Pakket>) -> Result<()> {
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

    fn haal_uitvoer(&self) -> Result<Option<Pakket>> {
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

        let _ = buffers[0].pEvents.take();
        let Some(sample) = buffers[0].pSample.take() else {
            return Ok(None);
        };

        // SAFETY: het sample komt net uit de encoder en bevat één samenhangende buffer.
        let pakket = unsafe {
            let buffer = sample.ConvertToContiguousBuffer()?;
            let mut ptr = std::ptr::null_mut();
            let mut lengte = 0u32;
            buffer.Lock(&mut ptr, None, Some(&mut lengte))?;
            let data = std::slice::from_raw_parts(ptr, lengte as usize).to_vec();
            let _ = buffer.Unlock();

            Pakket {
                // De tijd van de encoder, niet "nu": alle pakketten van één beeld
                // moeten dezelfde tijdstempel dragen, want daaraan herkent de
                // ontvanger welke fragmenten bij elkaar horen.
                tijd_hns: sample.GetSampleTime().unwrap_or(0),
                // Zonder deze vlag weet de ontvanger niet vanaf welk beeld hij weer
                // kan aanhaken na verlies.
                keyframe: sample.GetUINT32(&MFSampleExtension_CleanPoint).unwrap_or(0) != 0,
                data,
            }
        };

        Ok((!pakket.data.is_empty()).then_some(pakket))
    }
}

// ---------------------------------------------------------------------------
// Decoder
// ---------------------------------------------------------------------------

/// Zoveel keer proberen we uitvoer op te halen na één beeld erin. Ruim genoeg voor een
/// formaatwissel plus het beeld dat erop volgt; de grens bestaat zodat een MFT die
/// blijft zeggen "nog eens proberen" de render-thread niet laat vastdraaien.
const MAX_UITVOERPOGINGEN: usize = 32;

enum Uitvoer {
    Beeld(ID3D11Texture2D),
    /// Formaatwissel of een sample zonder inhoud: nog eens proberen.
    Opnieuw,
    /// De decoder wil eerst meer invoer.
    Klaar,
}

/// Spiegelbeeld van [`Encoder`]. Neemt complete frames aan en levert BGRA-texturen.
///
/// # Waarom hier twee modellen in zitten
///
/// De encoder is altijd een asynchrone hardware-MFT. Bij decoders ligt dat anders: de
/// H.264-decoder die altijd in Windows zit is een *synchrone* software-MFT, en die
/// meldt niets via gebeurtenissen — je duwt er een frame in en haalt er uit wat eruit
/// komt. Welke van de twee we hebben vragen we op bij de transform zelf, want dat
/// hangt af van de machine en niet van de codec.
pub struct Decoder {
    transform: IMFTransform,
    /// `Some` bij een asynchrone MFT.
    events: Option<IMFMediaEventGenerator>,
    d3d: D3dContext,
    /// Pas te maken als het uitvoertype onderhandeld is: de decoder bepaalt zelf de
    /// afmeting, en die hoeft niet te zijn wat wij vroegen.
    kleur: Option<Kleuromzetter>,
    /// Voor een decoder die zijn beeld in het werkgeheugen aflevert; blijft `None`
    /// zolang we texturen binnenkrijgen.
    upload: Option<ID3D11Texture2D>,
    /// Levert de MFT zijn eigen samples, of moeten wij ze aanleveren?
    eigen_samples: bool,
    uitvoer_grootte: u32,
    uitvoer_uitlijning: u32,
    wacht_op_invoer: bool,
    frame_duur: i64,
    /// Kwam het laatste beeld als GPU-textuur binnen? Zo niet, dan gaat elk beeld via
    /// het werkgeheugen en kost het een kopie — bij 1080p60 zichtbaar in de metingen.
    op_gpu: bool,
}

// SAFETY: zelfde reden als bij `Encoder`: alleen gebruikt vanaf de render-thread van
// één stream.
unsafe impl Send for Decoder {}

impl Decoder {
    pub fn new(d3d: &D3dContext, codec: Codec, breedte: u32, hoogte: u32) -> Result<Self> {
        mf::zorg_dat_mf_draait();

        // Breed zoeken, niet alleen op hardware: de H.264-decoder die standaard in
        // Windows zit is geen hardware-MFT, en juist die willen we kunnen gebruiken.
        let vlaggen = MFT_ENUM_FLAG(
            MFT_ENUM_FLAG_HARDWARE.0
                | MFT_ENUM_FLAG_SYNCMFT.0
                | MFT_ENUM_FLAG_ASYNCMFT.0
                | MFT_ENUM_FLAG_SORTANDFILTER.0,
        );
        let activates = mf::zoek_transform_met(false, codec.subtype(), MFVideoFormat_NV12, vlaggen)
            .context("decoder zoeken")?;
        let activate = activates.first().with_context(|| {
            format!(
                "geen {}-decoder op deze machine; \
                 bij HEVC betekent dat meestal dat de HEVC Video Extensions ontbreken",
                codec.naam()
            )
        })?;

        tracing::info!(decoder = %mf::naam_van(activate), codec = codec.naam(), "decoder gekozen");

        // SAFETY: de activate komt uit MFTEnumEx en is geldig.
        let transform: IMFTransform =
            unsafe { activate.ActivateObject().context("decoder activeren")? };

        // SAFETY: `transform` is net aangemaakt en nog niet in gebruik.
        let asynchroon = unsafe {
            let attrs = transform.GetAttributes().context("decoder-attributen")?;
            let asynchroon = attrs.GetUINT32(&MF_TRANSFORM_ASYNC).unwrap_or(0) == 1;
            if asynchroon {
                attrs
                    .SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1)
                    .context("decoder ontgrendelen")?;
            }
            let _ = attrs.SetUINT32(&MF_LOW_LATENCY, 1);

            // Alleen een D3D11-bewuste decoder krijgt ons apparaat. De rest levert
            // beelden in het werkgeheugen af; dat werkt ook, maar kost een kopie.
            if attrs.GetUINT32(&MF_SA_D3D11_AWARE).unwrap_or(0) == 1 {
                let manager = mf::device_manager(&d3d.device)?;
                if let Err(e) = transform.ProcessMessage(
                    MFT_MESSAGE_SET_D3D_MANAGER,
                    std::mem::transmute_copy::<IMFDXGIDeviceManager, usize>(&manager),
                ) {
                    tracing::warn!(error = %e, "decoder wil ons D3D-apparaat niet; beeld gaat via het werkgeheugen");
                }
            } else {
                tracing::info!("decoder werkt in het werkgeheugen, niet op de GPU");
            }
            asynchroon
        };

        // Invoertype eerst: een decoder moet weten wat er binnenkomt voordat hij kan
        // zeggen wat eruit komt. Bij de encoder is het precies andersom.
        // SAFETY: het type is volledig ingevuld voordat het gezet wordt.
        unsafe {
            let in_type = MFCreateMediaType().context("invoertype")?;
            mf::zet_video_type(&in_type, codec.subtype())?;
            in_type.SetUINT64(&MF_MT_FRAME_SIZE, mf::pak(breedte, hoogte))?;
            in_type.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            transform
                .SetInputType(0, &in_type, 0)
                .context("invoertype instellen")?;
        }

        let events: Option<IMFMediaEventGenerator> = if asynchroon {
            Some(
                transform
                    .cast()
                    .context("asynchrone decoder levert geen gebeurtenissen")?,
            )
        } else {
            None
        };

        let mut decoder = Self {
            transform,
            events,
            d3d: d3d.clone(),
            kleur: None,
            upload: None,
            eigen_samples: false,
            uitvoer_grootte: 0,
            uitvoer_uitlijning: 0,
            wacht_op_invoer: false,
            frame_duur: HNS_PER_SEC / 60,
            op_gpu: false,
        };
        decoder.onderhandel_uitvoertype()?;

        // SAFETY: beide types zijn gezet, dus de decoder mag beginnen.
        unsafe {
            decoder
                .transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            decoder
                .transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
        }

        Ok(decoder)
    }

    /// Decodeert één compleet frame. Levert het nieuwste beeld op, of `None` zolang de
    /// decoder nog niet genoeg gezien heeft — na een keyframe duurt dat één frame.
    ///
    /// Komen er bij uitzondering twee beelden uit één aanroep, dan houden we het
    /// laatste: bij realtime beeld is het oudere op dat moment al niet meer actueel.
    pub fn decode(&mut self, data: &[u8], tijd_hns: i64) -> Result<Option<ID3D11Texture2D>> {
        let sample = self.maak_sample(data, tijd_hns)?;
        let mut laatste = None;

        if self.events.is_some() {
            // Asynchroon: wachten tot hij om invoer vraagt. Dat moet blokkerend —
            // met alleen polsen mis je de gebeurtenis en staat de decoder stil.
            while !self.wacht_op_invoer {
                let Some(event) = self.volgend_event(true)? else {
                    bail!("decoder leverde geen gebeurtenis meer");
                };
                self.verwerk_event(&event, &mut laatste)?;
            }
            // SAFETY: de decoder heeft net om invoer gevraagd.
            unsafe { self.transform.ProcessInput(0, &sample, 0)? };
            self.wacht_op_invoer = false;

            while let Some(event) = self.volgend_event(false)? {
                self.verwerk_event(&event, &mut laatste)?;
            }
        } else {
            // Synchroon: erin duwen en eruit halen tot hij om meer vraagt.
            // SAFETY: de types zijn onderhandeld en de transform staat te streamen.
            unsafe { self.transform.ProcessInput(0, &sample, 0)? };
            for _ in 0..MAX_UITVOERPOGINGEN {
                match self.haal_uitvoer()? {
                    Uitvoer::Beeld(t) => laatste = Some(t),
                    Uitvoer::Opnieuw => continue,
                    Uitvoer::Klaar => break,
                }
            }
        }

        Ok(laatste)
    }

    /// Afmeting van het beeld zoals de decoder het aflevert. `None` tot het eerste
    /// beeld binnen is.
    pub fn afmeting(&self) -> Option<(u32, u32)> {
        self.kleur.as_ref().map(|k| k.afmeting())
    }

    /// Of het laatste beeld als GPU-textuur binnenkwam. `false` betekent dat elk beeld
    /// een kopie door het werkgeheugen maakt; relevant zodra we latency gaan meten.
    pub fn op_gpu(&self) -> bool {
        self.op_gpu
    }

    /// Gooit weg wat er nog in de decoder zit. Nodig na verlies: doorgaan op oude
    /// referentiebeelden levert alleen maar vlekken op tot het volgende keyframe.
    pub fn spoel(&mut self) {
        // SAFETY: flushen mag op elk moment tussen twee frames door.
        unsafe {
            let _ = self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
        }
        self.wacht_op_invoer = false;
    }

    fn maak_sample(&self, data: &[u8], tijd_hns: i64) -> Result<IMFSample> {
        // `as u32` kapte de lengte af terwijl de `copy_nonoverlapping` eronder de volle
        // `usize` kopieert: een beeld van 4 GiB + 1 byte leverde een buffer van één byte
        // op met een memcpy van 4 GiB eroverheen. Onbereikbaar zolang de framegrootte
        // begrensd is (zie `fragment::MAX_FRAME_BYTES`), maar dat is een grens ergens
        // anders — hier hoort hij hard te zijn (B-45).
        let lengte = u32::try_from(data.len()).context("beeld te groot voor een MF-buffer")?;

        // SAFETY: de buffer is met precies `lengte` bytes aangemaakt, `data` is even lang,
        // en de buffer wordt hieronder netjes ontgrendeld.
        unsafe {
            let buffer = MFCreateMemoryBuffer(lengte).context("invoerbuffer aanmaken")?;
            let mut ptr = std::ptr::null_mut();
            buffer.Lock(&mut ptr, None, None)?;
            std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
            let _ = buffer.Unlock();
            buffer.SetCurrentLength(lengte)?;

            let sample = MFCreateSample().context("sample aanmaken")?;
            sample.AddBuffer(&buffer)?;
            sample.SetSampleTime(tijd_hns)?;
            sample.SetSampleDuration(self.frame_duur)?;
            Ok(sample)
        }
    }

    fn verwerk_event(
        &mut self,
        event: &IMFMediaEvent,
        laatste: &mut Option<ID3D11Texture2D>,
    ) -> Result<()> {
        // SAFETY: het event komt net uit de wachtrij.
        let soort = unsafe { event.GetType()? };
        if soort == METransformNeedInput.0 as u32 {
            self.wacht_op_invoer = true;
        } else if soort == METransformHaveOutput.0 as u32 {
            for _ in 0..MAX_UITVOERPOGINGEN {
                match self.haal_uitvoer()? {
                    Uitvoer::Beeld(t) => {
                        *laatste = Some(t);
                        break;
                    }
                    Uitvoer::Opnieuw => continue,
                    Uitvoer::Klaar => break,
                }
            }
        }
        Ok(())
    }

    fn volgend_event(&self, blokkeren: bool) -> Result<Option<IMFMediaEvent>> {
        let Some(events) = &self.events else {
            return Ok(None);
        };
        let vlag = if blokkeren {
            MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS(0)
        } else {
            MF_EVENT_FLAG_NO_WAIT
        };
        // SAFETY: `events` is geldig; "geen event" komt terug als bekende foutcode.
        match unsafe { events.GetEvent(vlag) } {
            Ok(e) => Ok(Some(e)),
            Err(e) if e.code() == MF_E_NO_EVENTS_AVAILABLE => Ok(None),
            Err(e) if e.code() == MF_E_MULTIPLE_SUBSCRIBERS => Ok(None),
            Err(e) => Err(e).context("decoder-gebeurtenis ophalen"),
        }
    }

    fn haal_uitvoer(&mut self) -> Result<Uitvoer> {
        let mut buffers = [MFT_OUTPUT_DATA_BUFFER::default()];
        if !self.eigen_samples {
            // Deze MFT levert geen eigen samples; dan moeten wij het geheugen aanleveren.
            buffers[0].pSample = std::mem::ManuallyDrop::new(Some(self.maak_uitvoersample()?));
        }
        let mut status = 0u32;

        // SAFETY: `buffers` is correct opgezet voor het model van deze MFT.
        unsafe {
            match self.transform.ProcessOutput(0, &mut buffers, &mut status) {
                Ok(()) => {}
                Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => {
                    let _ = buffers[0].pSample.take();
                    return Ok(Uitvoer::Klaar);
                }
                // De decoder heeft in de bitstroom gezien wat het echte formaat is.
                // Dat gebeurt standaard vlak na het eerste keyframe.
                Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                    let _ = buffers[0].pSample.take();
                    self.onderhandel_uitvoertype()?;
                    return Ok(Uitvoer::Opnieuw);
                }
                Err(e) => {
                    let _ = buffers[0].pSample.take();
                    return Err(e).context("gedecodeerd beeld ophalen");
                }
            }
        }

        // De MFT mag hier gebeurtenissen bijleveren; wij doen er niets mee, maar ze
        // moeten wel losgelaten worden.
        let _ = buffers[0].pEvents.take();
        let Some(sample) = buffers[0].pSample.take() else {
            return Ok(Uitvoer::Opnieuw);
        };

        let nv12 = self.textuur_uit_sample(&sample)?;
        let Some((tex, slice)) = nv12 else {
            return Ok(Uitvoer::Opnieuw);
        };

        let kleur = self
            .kleur
            .as_mut()
            .context("uitvoertype nog niet onderhandeld")?;
        Ok(Uitvoer::Beeld(kleur.naar_bgra(&tex, slice)?))
    }

    /// Haalt de NV12-textuur uit een uitvoersample. Zit het beeld in het werkgeheugen,
    /// dan gaat het eerst naar de GPU.
    fn textuur_uit_sample(&mut self, sample: &IMFSample) -> Result<Option<(ID3D11Texture2D, u32)>> {
        // SAFETY: het sample komt net uit de decoder.
        let buffer = unsafe { sample.GetBufferByIndex(0) }.context("uitvoerbuffer")?;

        if let Ok(dxgi) = buffer.cast::<IMFDXGIBuffer>() {
            self.op_gpu = true;
            let mut tex: Option<ID3D11Texture2D> = None;
            // SAFETY: `GetResource` vult de gevraagde interface of geeft een fout.
            unsafe {
                dxgi.GetResource(
                    &ID3D11Texture2D::IID,
                    &mut tex as *mut _ as *mut *mut std::ffi::c_void,
                )
                .context("textuur uit uitvoerbuffer")?;
                let slice = dxgi.GetSubresourceIndex().unwrap_or(0);
                return Ok(tex.map(|t| (t, slice)));
            }
        }

        // Werkgeheugenpad: één kopie naar de GPU. Trager, maar het werkt ook als de
        // decoder ons D3D-apparaat niet wil.
        self.op_gpu = false;
        let (breedte, hoogte) = self
            .kleur
            .as_ref()
            .context("uitvoertype nog niet onderhandeld")?
            .afmeting();
        let doel = match &self.upload {
            Some(t) => t.clone(),
            None => {
                // Precies de maat uit het uitvoertype: daarmee ligt het chroma-vlak in
                // de textuur op dezelfde plek als in de buffer van de decoder.
                let t = self.d3d.maak_nv12_textuur(even(breedte), even(hoogte))?;
                self.upload = Some(t.clone());
                t
            }
        };

        // SAFETY: de buffer wordt vergrendeld en op élk pad hieronder weer losgelaten.
        // `pitch` komt van de buffer zelf, dus de rijen liggen waar D3D11 ze verwacht, en
        // de invariant waar de kopie op leunt — er staan minstens `pitch × even(hoogte) ×
        // 3/2` leesbare bytes vanaf `ptr` — wordt hier afgedwongen en niet aangenomen.
        unsafe {
            // Vóór het vergrendelen opvragen: `Lock` en `Lock2D` horen niet door elkaar
            // gebruikt te worden, en dit is de enige plek waar de maat van de allocatie
            // los van het vergrendelen te krijgen is. `0` betekent "meldt zijn maat niet".
            let toegewezen = buffer.GetMaxLength().unwrap_or(0);

            let mut ptr = std::ptr::null_mut();
            let tweed = buffer.cast::<IMF2DBuffer>().ok();
            let (pitch, beschikbaar) = match &tweed {
                Some(t) => {
                    let mut stap = 0i32;
                    t.Lock2D(&mut ptr, &mut stap)
                        .context("beeld vergrendelen")?;
                    // Een bottom-up buffer geeft een **negatieve** stride: `ptr` wijst dan
                    // naar de bovenste rij, `S·(h−1)` bytes verderop in de allocatie, en
                    // `UpdateSubresource` leest vanaf `ptr` alleen maar vooruit — dus
                    // `S·(h−1)` bytes voorbij het einde, op 1080p ~2,07 MB. Het teken
                    // wegstrippen met `unsigned_abs` maakte daar stil een out-of-bounds
                    // read van (B-19). Weigeren is hier het enige eerlijke: een bottom-up
                    // NV12-buffer levert bovendien een beeld op zijn kop.
                    if stap <= 0 {
                        let _ = t.Unlock2D();
                        bail!("decoder gaf een buffer met stride {stap}; die is niet te lezen");
                    }
                    (stap as u32, toegewezen)
                }
                None => {
                    let mut lengte = 0u32;
                    buffer.Lock(&mut ptr, None, Some(&mut lengte))?;
                    (breedte, lengte)
                }
            };

            // Met `pDstBox = NULL` kopieert D3D de héle subresource: `pitch × even(hoogte)
            // × 3/2` bytes vooruit vanaf `ptr`. Breedte en hoogte komen uit het uitvoertype
            // dat de decoder uit de **SPS van de peer** onderhandelde, en dat wordt bij elke
            // `MF_E_TRANSFORM_STREAM_CHANGE` opnieuw gedaan — de bitstroom van de andere
            // kant bepaalt dus de leeslengte. Die hoort tegen de buffer aan gehouden te
            // worden vóór de kopie, niet erna (B-19).
            let nodig = u64::from(pitch) * u64::from(even(hoogte)) * 3 / 2;
            if beschikbaar != 0 && u64::from(beschikbaar) < nodig {
                match &tweed {
                    Some(t) => {
                        let _ = t.Unlock2D();
                    }
                    None => {
                        let _ = buffer.Unlock();
                    }
                }
                bail!("decoderbuffer meldt {beschikbaar} bytes voor een beeld van {nodig}");
            }

            self.d3d
                .context
                .UpdateSubresource(&doel, 0, None, ptr as *const _, pitch, 0);
            match &tweed {
                Some(t) => {
                    let _ = t.Unlock2D();
                }
                None => {
                    let _ = buffer.Unlock();
                }
            }
        }

        Ok(Some((doel, 0)))
    }

    fn maak_uitvoersample(&self) -> Result<IMFSample> {
        // SAFETY: de gevraagde grootte en uitlijning komen van de MFT zelf.
        unsafe {
            let buffer = if self.uitvoer_uitlijning > 0 {
                MFCreateAlignedMemoryBuffer(self.uitvoer_grootte, self.uitvoer_uitlijning - 1)
            } else {
                MFCreateMemoryBuffer(self.uitvoer_grootte)
            }
            .context("uitvoerbuffer aanmaken")?;
            let sample = MFCreateSample().context("uitvoersample aanmaken")?;
            sample.AddBuffer(&buffer)?;
            Ok(sample)
        }
    }

    /// Kiest het NV12-uitvoertype en zet de kleuromzetter op de afmeting die de
    /// decoder werkelijk levert. Wordt opnieuw gedaan bij elke formaatwissel.
    fn onderhandel_uitvoertype(&mut self) -> Result<()> {
        // SAFETY: we lopen de aangeboden types af tot NV12 erbij zit en zetten er één.
        let (breedte, hoogte, matrix, bereik) = unsafe {
            let mut gekozen = None;
            for i in 0..32 {
                let Ok(mt) = self.transform.GetOutputAvailableType(0, i) else {
                    break;
                };
                if mt.GetGUID(&MF_MT_SUBTYPE).ok() == Some(MFVideoFormat_NV12) {
                    gekozen = Some(mt);
                    break;
                }
            }
            let mt = gekozen.context("decoder biedt geen NV12 aan")?;
            self.transform
                .SetOutputType(0, &mt, 0)
                .context("uitvoertype instellen")?;

            let maat = mt.GetUINT64(&MF_MT_FRAME_SIZE).unwrap_or(0);
            (
                (maat >> 32) as u32,
                maat as u32,
                // Ontbreken deze, dan is het wat H.264 op HD standaard doet.
                mt.GetUINT32(&MF_MT_YUV_MATRIX)
                    .unwrap_or(MFVideoTransferMatrix_BT709.0 as u32),
                mt.GetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE)
                    .unwrap_or(MFNominalRange_16_235.0 as u32),
            )
        };

        if breedte == 0 || hoogte == 0 {
            bail!("decoder gaf een uitvoertype zonder afmeting");
        }

        let ruimte = Kleurruimte {
            bt709: matrix != MFVideoTransferMatrix_BT601.0 as u32,
            studio: bereik != MFNominalRange_0_255.0 as u32,
        };

        // SAFETY: de transform heeft nu een geldig uitvoertype, dus deze informatie
        // klopt bij wat er straks uitkomt.
        unsafe {
            let info = self.transform.GetOutputStreamInfo(0)?;
            let zelf_leveren = (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0
                | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0) as u32;
            self.eigen_samples = info.dwFlags & zelf_leveren != 0;
            self.uitvoer_grootte = info.cbSize;
            self.uitvoer_uitlijning = info.cbAlignment;
        }

        // Een formaatwissel maakt zowel de omzetter als de uploadtextuur ongeldig.
        if self.kleur.as_ref().map(|k| k.afmeting()) != Some((breedte, hoogte)) {
            tracing::info!(breedte, hoogte, ?ruimte, "decoder levert beeld");
            self.kleur = Some(Kleuromzetter::new(&self.d3d, breedte, hoogte, ruimte)?);
            self.upload = None;
        }
        Ok(())
    }
}

/// NV12 heeft halve chroma-resolutie, dus een oneven maat bestaat er niet.
fn even(waarde: u32) -> u32 {
    waarde + (waarde & 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_type_en_codec_zijn_elkaars_omkering() {
        for codec in [Codec::H264, Codec::Hevc] {
            assert_eq!(Codec::van_payload(codec.payload_type()), Some(codec));
        }
        assert_eq!(Codec::van_payload(fitcom_proto::PayloadType::OPUS), None);
    }

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
        let mut keyframes = 0usize;
        let mut tijden = Vec::new();
        for i in 0..30 {
            let tijd = i as i64 * (HNS_PER_SEC / 60);
            for p in enc.encode(&tex, tijd).expect("encoderen") {
                totaal += p.data.len();
                pakketten += 1;
                keyframes += usize::from(p.keyframe);
                tijden.push(p.tijd_hns);
            }
        }
        println!("{pakketten} pakketten, {totaal} bytes, {keyframes} keyframes");
        assert!(pakketten > 0, "encoder leverde helemaal niets op");
        assert!(keyframes > 0, "zonder keyframe kan niemand aanhaken");
        // Zonder oplopende tijdstempels ziet de ontvanger alle fragmenten als één beeld.
        assert!(
            tijden.windows(2).all(|w| w[1] > w[0]),
            "tijdstempels lopen niet op: {tijden:?}"
        );
    }

    /// Een effen vlak van één kleur. Effen omdat de codec dat vrijwel verliesloos
    /// bewaart: wat er aan de andere kant uitkomt mag dus dicht bij het origineel
    /// liggen, en daarmee is dit een echte controle in plaats van "er kwam iets".
    fn effen(breedte: u32, hoogte: u32, b: u8, g: u8, r: u8) -> Vec<u8> {
        (0..breedte * hoogte).flat_map(|_| [b, g, r, 255]).collect()
    }

    #[test]
    #[ignore = "vereist een GPU met hardware-encoder"]
    fn beeld_overleeft_de_hele_keten() {
        // Dit is de verificatie waar de decoder om vraagt: coderen, decoderen, en
        // controleren dat er beeld van de juiste maat én de juiste kleur uitkomt.
        // Een omgewisseld kleurkanaal of een verkeerd ingesteld kleurbereik zou hier
        // opvallen, en dat is precies wat je met kijken naar een testbeeld niet ziet.
        const B: u32 = 1280;
        const H: u32 = 720;
        let (blauw, groen, rood) = (40u8, 140u8, 220u8);

        let d3d = D3dContext::new().expect("D3D11");
        let bron = d3d
            .maak_textuur_met(B, H, &effen(B, H, blauw, groen, rood))
            .expect("brontextuur");

        let cfg = EncoderConfig {
            codec: Codec::H264,
            breedte: B,
            hoogte: H,
            fps: 60,
            bitrate: 20_000_000,
        };
        let mut enc = Encoder::new(&d3d, &cfg).expect("encoder");
        let mut dec = Decoder::new(&d3d, Codec::H264, B, H).expect("decoder");

        let mut beeld = None;
        for i in 0..30 {
            let tijd = i as i64 * (HNS_PER_SEC / 60);
            for pakket in enc.encode(&bron, tijd).expect("encoderen") {
                if let Some(t) = dec
                    .decode(&pakket.data, pakket.tijd_hns)
                    .expect("decoderen")
                {
                    beeld = Some(t);
                }
            }
            if beeld.is_some() {
                break;
            }
        }

        let beeld = beeld.expect("er kwam geen enkel beeld uit de decoder");
        println!(
            "beeld kwam {} binnen",
            if dec.op_gpu() {
                "als GPU-textuur"
            } else {
                "via het werkgeheugen"
            }
        );
        let (b, h, pixels) = d3d.lees_bgra(&beeld).expect("uitlezen");
        assert_eq!((b, h), (B, H), "afmeting veranderd onderweg");

        // Midden in het beeld kijken; aan de randen kan de codec meer afwijken.
        let midden = ((h / 2 * b + b / 2) * 4) as usize;
        let (gb, gg, gr) = (pixels[midden], pixels[midden + 1], pixels[midden + 2]);
        println!("origineel BGR {blauw},{groen},{rood} → terug {gb},{gg},{gr}");

        for (naam, verwacht, gekregen) in [
            ("blauw", blauw, gb),
            ("groen", groen, gg),
            ("rood", rood, gr),
        ] {
            let afwijking = i32::from(verwacht) - i32::from(gekregen);
            assert!(
                afwijking.abs() <= 12,
                "{naam} wijkt {afwijking} af; kanalen omgewisseld of kleurbereik verkeerd"
            );
        }
    }

    #[test]
    #[ignore = "vereist een GPU met hardware-encoder"]
    fn decoder_verwerkt_een_hele_reeks_zonder_vast_te_lopen() {
        // De encoder liep eerder vast na één beeld doordat de gebeurtenislus niet
        // klopte. Dezelfde fout is in een decoder net zo makkelijk te maken.
        const B: u32 = 640;
        const H: u32 = 360;
        let d3d = D3dContext::new().expect("D3D11");
        let bron = d3d
            .maak_textuur_met(B, H, &effen(B, H, 10, 200, 60))
            .expect("brontextuur");

        let cfg = EncoderConfig {
            codec: Codec::H264,
            breedte: B,
            hoogte: H,
            fps: 60,
            bitrate: 8_000_000,
        };
        let mut enc = Encoder::new(&d3d, &cfg).expect("encoder");
        let mut dec = Decoder::new(&d3d, Codec::H264, B, H).expect("decoder");

        let mut beelden = 0;
        for i in 0..120 {
            let tijd = i as i64 * (HNS_PER_SEC / 60);
            for pakket in enc.encode(&bron, tijd).expect("encoderen") {
                if dec
                    .decode(&pakket.data, pakket.tijd_hns)
                    .expect("decoderen")
                    .is_some()
                {
                    beelden += 1;
                }
            }
        }
        println!("{beelden} beelden uit 120 aangeboden");
        assert!(
            beelden > 60,
            "de decoder blijft steken: maar {beelden} beelden"
        );
    }
}
