//! NV12 → BGRA op de GPU.
//!
//! De decoder levert NV12, een venster wil BGRA. Die omzetting op de CPU doen is bij
//! 1080p60 ondenkbaar — drie megabyte per beeld, zestig keer per seconde — dus hij
//! gebeurt met `ID3D11VideoProcessor`, de vaste-functie-hardware die elke GPU hier al
//! voor heeft. Het beeld komt daarmee nergens in het werkgeheugen langs.
//!
//! # Waarom kleurbereik hier expliciet is
//!
//! H.264 draagt zijn beeld standaard in *studiobereik* (16-235), een monitor wil
//! 0-255. Zet je dat verkeerd, dan is het beeld niet kapot maar wél zichtbaar
//! verwassen of juist te contrastrijk — precies het soort fout dat je pas opvalt als
//! iemand anders ernaar kijkt. De waarden komen daarom uit het onderhandelde
//! uitvoertype van de decoder en niet uit een aanname.

use crate::d3d::D3dContext;
use anyhow::{Context, Result};
use windows::core::Interface;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Texture2D, ID3D11VideoContext, ID3D11VideoDevice, ID3D11VideoProcessor,
    ID3D11VideoProcessorEnumerator, ID3D11VideoProcessorInputView, ID3D11VideoProcessorOutputView,
    D3D11_TEX2D_VPIV, D3D11_TEX2D_VPOV, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
    D3D11_VIDEO_PROCESSOR_COLOR_SPACE, D3D11_VIDEO_PROCESSOR_CONTENT_DESC,
    D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0,
    D3D11_VIDEO_PROCESSOR_OUTPUT_RATE_NORMAL, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC,
    D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_STREAM,
    D3D11_VIDEO_USAGE_PLAYBACK_NORMAL, D3D11_VPIV_DIMENSION_TEXTURE2D,
    D3D11_VPOV_DIMENSION_TEXTURE2D,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_RATIONAL;

/// Zoveel invoerviews onthouden we. Een decoder werkt met een vaste ring texturen, dus
/// na een paar frames zit alles in de cache en maken we er geen enkele meer bij.
const MAX_VIEWS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kleurruimte {
    /// BT.709 (HD) als `true`, anders BT.601 (SD).
    pub bt709: bool,
    /// Studiobereik 16-235 als `true`, anders volle 0-255.
    pub studio: bool,
}

impl Default for Kleurruimte {
    /// Wat H.264 op HD-resolutie standaard doet, en dus wat onze eigen encoder levert.
    fn default() -> Self {
        Self {
            bt709: true,
            studio: true,
        }
    }
}

impl Kleurruimte {
    /// De bitjes zoals `D3D11_VIDEO_PROCESSOR_COLOR_SPACE` ze verwacht:
    /// bit0 gebruik, bit1 RGB-bereik, bit2 YCbCr-matrix, bit3 xvYCC, bit4-5 bereik.
    fn als_invoer(self) -> D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
        let matrix = u32::from(self.bt709) << 2;
        let bereik = if self.studio { 1 } else { 2 } << 4;
        D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
            _bitfield: matrix | bereik,
        }
    }

    /// De uitvoer is altijd volledig bereik: dat is wat een monitor toont.
    fn als_uitvoer() -> D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
        D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: 2 << 4 }
    }
}

pub struct Kleuromzetter {
    d3d: D3dContext,
    video: ID3D11VideoDevice,
    context: ID3D11VideoContext,
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    doel: ID3D11Texture2D,
    doel_view: ID3D11VideoProcessorOutputView,
    /// De textuur zit in de sleutel zodat hij blijft leven. Zou hier alleen het adres
    /// staan, dan kon een vrijgegeven textuur hetzelfde adres terugkrijgen en wees de
    /// cache stilzwijgend naar een ander beeld.
    views: Vec<(ID3D11Texture2D, u32, ID3D11VideoProcessorInputView)>,
    afmeting: (u32, u32),
}

// SAFETY: het D3D11-apparaat staat op multithread-protected (zie `d3d.rs`) en de
// omzetter wordt door één thread tegelijk gebruikt: de render-thread van één stream.
unsafe impl Send for Kleuromzetter {}

impl Kleuromzetter {
    pub fn new(d3d: &D3dContext, breedte: u32, hoogte: u32, ruimte: Kleurruimte) -> Result<Self> {
        let video: ID3D11VideoDevice = d3d.device.cast().context("videoapparaat")?;
        let context: ID3D11VideoContext = d3d.context.cast().context("videocontext")?;

        // De framerate hoort bij deinterlacing en frameratedoublering; wij doen geen
        // van beide, dus deze waarden zijn cosmetisch.
        let tempo = DXGI_RATIONAL {
            Numerator: 60,
            Denominator: 1,
        };
        let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            InputFrameRate: tempo,
            InputWidth: breedte,
            InputHeight: hoogte,
            OutputFrameRate: tempo,
            OutputWidth: breedte,
            OutputHeight: hoogte,
            Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
        };

        // SAFETY: `desc` is volledig ingevuld en `video` is net afgeleid van ons apparaat.
        let (enumerator, processor) = unsafe {
            let e = video
                .CreateVideoProcessorEnumerator(&desc)
                .context("videoprocessor opzoeken")?;
            let p = video
                .CreateVideoProcessor(&e, 0)
                .context("videoprocessor aanmaken")?;
            (e, p)
        };

        let doel = d3d.maak_textuur(breedte, hoogte)?;
        let uit_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
            ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
            },
        };
        let mut doel_view = None;
        // SAFETY: `doel` is een BGRA-textuur met render-target-binding, precies wat een
        // uitvoerview verlangt.
        unsafe {
            video
                .CreateVideoProcessorOutputView(&doel, &enumerator, &uit_desc, Some(&mut doel_view))
                .context("uitvoerview aanmaken")?;

            // Geen framerateconversie: wij leveren precies de beelden die we tonen.
            context.VideoProcessorSetStreamOutputRate(
                &processor,
                0,
                D3D11_VIDEO_PROCESSOR_OUTPUT_RATE_NORMAL,
                false,
                None,
            );
            context.VideoProcessorSetStreamFrameFormat(
                &processor,
                0,
                D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            );
            context.VideoProcessorSetStreamColorSpace(&processor, 0, &ruimte.als_invoer());
            context.VideoProcessorSetOutputColorSpace(&processor, &Kleurruimte::als_uitvoer());

            // Expliciet knippen in plaats van schalen. H.264 codeert 1080 beeldlijnen
            // als 1088 — een veelvoud van zestien — dus de textuur van de decoder is
            // hoger dan het beeld. Zonder deze rechthoek zou de hele textuur worden
            // uitgerekt naar het doel en is het beeld verticaal net iets ingedrukt.
            let kader = RECT {
                left: 0,
                top: 0,
                right: breedte as i32,
                bottom: hoogte as i32,
            };
            context.VideoProcessorSetStreamSourceRect(&processor, 0, true, Some(&kader));
            context.VideoProcessorSetStreamDestRect(&processor, 0, true, Some(&kader));
        }

        Ok(Self {
            d3d: d3d.clone(),
            video,
            context,
            enumerator,
            processor,
            doel,
            doel_view: doel_view.context("D3D11 gaf geen uitvoerview")?,
            views: Vec::new(),
            afmeting: (breedte, hoogte),
        })
    }

    pub fn afmeting(&self) -> (u32, u32) {
        self.afmeting
    }

    /// Zet één NV12-beeld om naar BGRA. `slice` is de index binnen de texturenreeks van
    /// de decoder; hardware-decoders leveren hun beelden namelijk niet als losse
    /// texturen maar als plakken van één array.
    pub fn naar_bgra(&mut self, bron: &ID3D11Texture2D, slice: u32) -> Result<ID3D11Texture2D> {
        let view = self.invoerview(bron, slice)?;

        let stream = D3D11_VIDEO_PROCESSOR_STREAM {
            Enable: true.into(),
            pInputSurface: std::mem::ManuallyDrop::new(Some(view)),
            ..Default::default()
        };

        // SAFETY: de view hoort bij deze processor en blijft geldig tot na de blt.
        let uitkomst = unsafe {
            self.context.VideoProcessorBlt(
                &self.processor,
                &self.doel_view,
                0,
                std::slice::from_ref(&stream),
            )
        };
        // `pInputSurface` is `ManuallyDrop`, dus de telling die we er hierboven in
        // stopten moeten we zelf weer loslaten.
        drop(std::mem::ManuallyDrop::into_inner(stream.pInputSurface));
        uitkomst.context("kleuromzetting")?;

        Ok(self.doel.clone())
    }

    fn invoerview(
        &mut self,
        bron: &ID3D11Texture2D,
        slice: u32,
    ) -> Result<ID3D11VideoProcessorInputView> {
        if let Some((_, _, v)) = self
            .views
            .iter()
            .find(|(t, s, _)| *s == slice && t.as_raw() == bron.as_raw())
        {
            return Ok(v.clone());
        }

        let desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
            // 0 betekent: neem het formaat van de textuur zelf.
            FourCC: 0,
            ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_VPIV {
                    MipSlice: 0,
                    ArraySlice: slice,
                },
            },
        };
        let mut view = None;
        // SAFETY: `bron` is een NV12-textuur van de decoder en `slice` komt uit
        // `IMFDXGIBuffer::GetSubresourceIndex`.
        unsafe {
            self.video
                .CreateVideoProcessorInputView(bron, &self.enumerator, &desc, Some(&mut view))
                .context("invoerview aanmaken")?;
        }
        let view = view.context("D3D11 gaf geen invoerview")?;

        if self.views.len() >= MAX_VIEWS {
            self.views.remove(0);
        }
        self.views.push((bron.clone(), slice, view.clone()));
        Ok(view)
    }

    /// Het apparaat waarop deze omzetter werkt, zodat de aanroeper er texturen bij kan
    /// maken die op hetzelfde apparaat leven.
    pub fn d3d(&self) -> &D3dContext {
        &self.d3d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kleurbits_kloppen_met_de_documentatie() {
        // Deze bitjes zijn niet te zien in een debugger en fout zetten geeft geen fout,
        // alleen een verwassen beeld. Vandaar dat ze hier vastliggen.
        let hd = Kleurruimte {
            bt709: true,
            studio: true,
        };
        assert_eq!(hd.als_invoer()._bitfield, 0b01_0100);

        let vol = Kleurruimte {
            bt709: true,
            studio: false,
        };
        assert_eq!(vol.als_invoer()._bitfield, 0b10_0100);

        let sd = Kleurruimte {
            bt709: false,
            studio: true,
        };
        assert_eq!(sd.als_invoer()._bitfield, 0b01_0000);

        assert_eq!(Kleurruimte::als_uitvoer()._bitfield, 0b10_0000);
    }

    #[test]
    #[ignore = "vereist een GPU"]
    fn omzetter_is_op_te_zetten() {
        let d3d = D3dContext::new().expect("D3D11");
        let k = Kleuromzetter::new(&d3d, 1920, 1080, Kleurruimte::default()).expect("omzetter");
        assert_eq!(k.afmeting(), (1920, 1080));
    }
}
