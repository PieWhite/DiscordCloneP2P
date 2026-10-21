//! Een HDR-scherm opnemen als SDR, op de GPU (beslissing 52).
//!
//! Staat HDR aan, dan kan het beeld licht bevatten boven SDR-wit: een spel in HDR, of
//! Auto HDR op een DX11-spel. Vraag je de opname in 8 bit BGRA, dan knipt Windows elk
//! kanaal los af op SDR-wit — gemeten, zie de beslissing. Alles wat helder is wordt een
//! vlakke witte of gele vlek met verschoven tinten, en dat ziet de kijker en staat in de
//! clip. Het bureaublad valt het niet op, want dat komt nooit boven SDR-wit.
//!
//! Dus vragen we op een HDR-scherm de opname in `R16G16B16A16_FLOAT` en zetten we hem
//! zelf om naar de BGRA die de encoder al kende: de hoge lichten met een zachte knie
//! samendrukken in plaats van afknippen, en sRGB-gamma erop. Eén pixelshader die de
//! `CopyResource` vervangt die er al stond, dus geen beeld extra en niets in het
//! werkgeheugen. Op een SDR-scherm loopt alles precies als vroeger.
//!
//! **Schaal 1,0, niet delen door het SDR-witniveau.** In de FP16-opname van
//! Windows.Graphics.Capture staat SDR-wit al op 1,0, wat de schuif "SDR-inhoud" ook zegt
//! (gemeten op 22631 met de schuif op 3,5). Bij desktopduplicatie is dat anders, en wie
//! daar leest dat je moet delen maakt hier alles 3,5× te donker.

use crate::d3d::D3dContext;
use anyhow::{Context, Result};
use windows::core::{Interface, PCSTR};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{ID3DBlob, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST};
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Multithread, ID3D11PixelShader, ID3D11RenderTargetView, ID3D11ShaderResourceView,
    ID3D11Texture2D, ID3D11VertexShader, D3D11_VIEWPORT,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020;
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, IDXGIOutput6};
use windows::Win32::Graphics::Gdi::HMONITOR;

/// Of dit scherm nu in HDR staat. Goedkoop genoeg om eens per seconde te vragen.
pub fn staat_in_hdr(monitor: HMONITOR) -> bool {
    // Via een eigen factory en niet via de adapter van ons apparaat: het scherm kan aan
    // een andere GPU hangen dan die waarop we coderen. En elke keer een nieuwe, want een
    // oude factory blijft de toestand van zijn eigen moment melden.
    // SAFETY: gewone DXGI-opsomming; alles wat we aanraken komt uit de aanroep ervoor.
    unsafe {
        let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else {
            return false;
        };
        let mut a = 0;
        while let Ok(adapter) = factory.EnumAdapters1(a) {
            let mut o = 0;
            while let Ok(output) = adapter.EnumOutputs(o) {
                if let Ok(desc) = output.cast::<IDXGIOutput6>().and_then(|o6| o6.GetDesc1()) {
                    if desc.Monitor == monitor {
                        return desc.ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020;
                    }
                }
                o += 1;
            }
            a += 1;
        }
    }
    false
}

/// Eén driehoek over het hele doel, zonder vertexbuffer, en per pixel precies één texel:
/// bron en doel zijn even groot, dus `Load` en geen sampler.
///
/// De knie: tot `KNIE` blijft alles zoals het was, daarboven loopt het zacht naar wit
/// (helling 1 bij de knie, dus geen zichtbare naad). Zo blijft een wolk in een HDR-lucht
/// een wolk in plaats van een witte vlek. Het samendrukken gaat op het sterkste kanaal en
/// de kleur schaalt mee, zodat fel oranje oranje blijft en niet geel wordt.
///
/// `KNIE` is de ene knop. Hoger: SDR-inhoud exacter, hoge lichten vlakker. Lager:
/// andersom. Elke curve die iets boven 1,0 wil bewaren moet ruimte onder 1,0 afstaan; bij
/// 0,8 komt SDR-wit op 0,9 lineair (sRGB 243 in plaats van 255) en valt 1,0-3,0 in 243-254.
const SHADER: &str = r"
Texture2D<float4> bron : register(t0);

float4 vs(uint id : SV_VertexID) : SV_Position {
    float2 uv = float2((id << 1) & 2, id & 2);
    return float4(uv * float2(2, -2) + float2(-1, 1), 0, 1);
}

static const float KNIE = 0.8;

float4 ps(float4 pos : SV_Position) : SV_Target {
    float3 c = max(bron.Load(int3(pos.xy, 0)).rgb, 0);
    float m = max(max(c.r, c.g), c.b);
    if (m > KNIE) {
        float x = (m - KNIE) / (1 - KNIE);
        c *= (KNIE + (1 - KNIE) * x / (1 + x)) / m;
    }
    float3 laag = c * 12.92;
    float3 hoog = 1.055 * pow(c, 1 / 2.4) - 0.055;
    return float4(c <= 0.0031308 ? laag : hoog, 1);
}
";

/// scRGB → BGRA. Wordt alleen gemaakt als het scherm in HDR staat.
pub struct Hdrnaarsdr {
    d3d: D3dContext,
    mt: ID3D11Multithread,
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    /// Het WGC-oppervlak in de sleutel, zodat het blijft leven (zelfde reden als de
    /// views in `kleur.rs`). De pool heeft er twee; bij `Recreate` komen er nieuwe.
    bronnen: Vec<(ID3D11Texture2D, ID3D11ShaderResourceView)>,
    doel: Option<(ID3D11Texture2D, ID3D11RenderTargetView)>,
}

// SAFETY: zie `Schermcapture`; het apparaat staat op multithread-protected en de omzetter
// wordt door één thread gebruikt, die van de opname.
unsafe impl Send for Hdrnaarsdr {}

impl Hdrnaarsdr {
    pub fn new(d3d: &D3dContext) -> Result<Self> {
        let vs_code = compileer("vs", "vs_5_0")?;
        let ps_code = compileer("ps", "ps_5_0")?;
        let (mut vs, mut ps) = (None, None);
        // SAFETY: de bytecode komt net uit de compiler.
        unsafe {
            d3d.device
                .CreateVertexShader(blob(&vs_code), None, Some(&mut vs))
                .context("vertexshader")?;
            d3d.device
                .CreatePixelShader(blob(&ps_code), None, Some(&mut ps))
                .context("pixelshader")?;
        }
        Ok(Self {
            d3d: d3d.clone(),
            mt: d3d.device.cast().context("multithread-interface")?,
            vs: vs.context("geen vertexshader")?,
            ps: ps.context("geen pixelshader")?,
            bronnen: Vec::new(),
            doel: None,
        })
    }

    /// Zet `bron` (FP16 scRGB, van de opname) om in `doel` (BGRA, even groot).
    pub fn zet_om(&mut self, bron: &ID3D11Texture2D, doel: &ID3D11Texture2D) -> Result<()> {
        let srv = self.bronview(bron)?;
        let rtv = self.doelview(doel)?;
        let (breedte, hoogte) = crate::d3d::afmetingen(doel);
        let ctx = &self.d3d.context;
        // SAFETY: alle objecten zijn tegen dit apparaat gemaakt. Enter/Leave omdat clip en
        // deler tegelijk op dezelfde context kunnen tekenen: los is elke aanroep beveiligd,
        // maar binden-en-tekenen moet als één geheel, anders tekent de een met de bron van
        // de ander. Het is dezelfde sectie die elke D3D-aanroep hier al neemt.
        unsafe {
            self.mt.Enter();
            ctx.IASetInputLayout(None);
            ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            ctx.VSSetShader(&self.vs, None);
            ctx.PSSetShader(&self.ps, None);
            ctx.PSSetShaderResources(0, Some(&[Some(srv)]));
            ctx.OMSetRenderTargets(Some(&[Some(rtv)]), None);
            ctx.RSSetViewports(Some(&[D3D11_VIEWPORT {
                Width: breedte as f32,
                Height: hoogte as f32,
                MaxDepth: 1.0,
                ..Default::default()
            }]));
            ctx.Draw(3, 0);
            // Loslaten: het oppervlak gaat terug naar de pool en `doel` naar de encoder.
            ctx.PSSetShaderResources(0, Some(&[None]));
            ctx.OMSetRenderTargets(None, None);
            self.mt.Leave();
        }
        Ok(())
    }

    fn bronview(&mut self, bron: &ID3D11Texture2D) -> Result<ID3D11ShaderResourceView> {
        if let Some((_, v)) = self
            .bronnen
            .iter()
            .find(|(t, _)| t.as_raw() == bron.as_raw())
        {
            return Ok(v.clone());
        }
        let mut v = None;
        // SAFETY: een WGC-oppervlak heeft shader-binding; gemeten, zonder zou
        // `hdr_weg_geeft_hetzelfde_beeld_als_bgra` in `capture.rs` geen beeld krijgen.
        unsafe {
            self.d3d
                .device
                .CreateShaderResourceView(bron, None, Some(&mut v))
                .context("view op het opnameoppervlak")?;
        }
        let v = v.context("D3D11 gaf geen view")?;
        if self.bronnen.len() >= 4 {
            self.bronnen.remove(0);
        }
        self.bronnen.push((bron.clone(), v.clone()));
        Ok(v)
    }

    fn doelview(&mut self, doel: &ID3D11Texture2D) -> Result<ID3D11RenderTargetView> {
        if let Some((t, v)) = &self.doel {
            if t.as_raw() == doel.as_raw() {
                return Ok(v.clone());
            }
        }
        let mut v = None;
        // SAFETY: `doel` komt uit `maak_textuur` en heeft render-target-binding.
        unsafe {
            self.d3d
                .device
                .CreateRenderTargetView(doel, None, Some(&mut v))
                .context("view op het doel")?;
        }
        let v = v.context("D3D11 gaf geen view")?;
        self.doel = Some((doel.clone(), v.clone()));
        Ok(v)
    }
}

fn compileer(ingang: &str, doel: &str) -> Result<ID3DBlob> {
    let ingang = format!("{ingang}\0");
    let doel = format!("{doel}\0");
    let (mut code, mut fout) = (None, None);
    // SAFETY: bron en namen leven tot na de aanroep; de namen eindigen op een nul.
    let r = unsafe {
        D3DCompile(
            SHADER.as_ptr() as *const _,
            SHADER.len(),
            PCSTR::null(),
            None,
            None,
            PCSTR(ingang.as_ptr()),
            PCSTR(doel.as_ptr()),
            0,
            0,
            &mut code,
            Some(&mut fout),
        )
    };
    if let Err(e) = r {
        let melding = fout
            .map(|b| String::from_utf8_lossy(blob(&b)).into_owned())
            .unwrap_or_default();
        anyhow::bail!("shader compileren: {e} {melding}");
    }
    code.context("compiler gaf geen code")
}

fn blob(b: &ID3DBlob) -> &[u8] {
    // SAFETY: wijzer en lengte komen samen van de blob, die zolang `b` leeft geldig is.
    unsafe { std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize()) }
}
