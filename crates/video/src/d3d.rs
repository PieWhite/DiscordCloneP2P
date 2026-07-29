//! Het D3D11-apparaat dat capture, encoder, decoder en weergave delen.
//!
//! Eén apparaat voor alles, zodat een gecapte textuur zonder omweg de encoder in kan.
//! Zou elke laag zijn eigen apparaat hebben, dan moest elk frame via gedeelde handles
//! of — nog erger — via het werkgeheugen, en dat is precies de kopie die we bij
//! 1080p60 niet kunnen betalen.

use anyhow::{Context, Result};
use windows::core::Interface;
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread, ID3D11Texture2D,
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_MAPPED_SUBRESOURCE,
    D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_NV12, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;

#[derive(Clone)]
pub struct D3dContext {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    /// Dezelfde `device`, maar in de vorm die Windows.Graphics.Capture verlangt.
    pub winrt_device: IDirect3DDevice,
}

// SAFETY: het apparaat wordt vanaf meerdere threads gebruikt (capture, encoder,
// weergave). D3D11 staat dat toe zodra de multithread-beveiliging aan staat, en dat
// gebeurt in `new` hieronder. Zonder die vlag zou dit onjuist zijn.
unsafe impl Send for D3dContext {}
unsafe impl Sync for D3dContext {}

impl D3dContext {
    pub fn new() -> Result<Self> {
        let mut device = None;
        let mut context = None;

        // SAFETY: alle uitvoerparameters wijzen naar geldige lokale variabelen.
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                windows::Win32::Foundation::HMODULE::default(),
                // BGRA voor de capture-API, VIDEO_SUPPORT voor de hardware-encoder.
                D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .context("D3D11-apparaat aanmaken")?;
        }

        let device = device.context("D3D11 gaf geen apparaat terug")?;
        let context = context.context("D3D11 gaf geen context terug")?;

        // Capture, encoder en weergave draaien op eigen threads en delen dit apparaat.
        // Zonder deze vlag is gelijktijdig gebruik ongedefinieerd gedrag.
        let mt: ID3D11Multithread = device.cast().context("multithread-interface")?;
        unsafe {
            let _ = mt.SetMultithreadProtected(true);
        }

        let dxgi: IDXGIDevice = device.cast().context("DXGI-apparaat")?;
        // SAFETY: `dxgi` is een geldig DXGI-apparaat van het net gemaakte D3D11-apparaat.
        let inspectable = unsafe {
            CreateDirect3D11DeviceFromDXGIDevice(&dxgi).context("WinRT-apparaat afleiden")?
        };
        let winrt_device: IDirect3DDevice = inspectable.cast().context("IDirect3DDevice")?;

        Ok(Self {
            device,
            context,
            winrt_device,
        })
    }

    /// Een eigen textuur waar we een gecapt frame in kopiëren.
    ///
    /// De capture-API hergebruikt zijn eigen texturen zodra je het frame sluit, dus
    /// wie het beeld langer nodig heeft dan dat ene moment moet het overzetten.
    pub fn maak_textuur(&self, breedte: u32, hoogte: u32) -> Result<ID3D11Texture2D> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: breedte,
            Height: hoogte,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut tex = None;
        // SAFETY: `desc` is volledig ingevuld en `tex` is een geldige uitvoerparameter.
        unsafe {
            self.device
                .CreateTexture2D(&desc, None, Some(&mut tex))
                .context("textuur aanmaken")?;
        }
        tex.context("D3D11 gaf geen textuur terug")
    }

    /// Een BGRA-textuur met inhoud. `pixels` is `breedte * hoogte * 4` bytes.
    ///
    /// Alleen nodig om iets bekends de keten in te sturen; het echte beeld komt van de
    /// schermopname en die vult zijn eigen textuur.
    pub fn maak_textuur_met(
        &self,
        breedte: u32,
        hoogte: u32,
        pixels: &[u8],
    ) -> Result<ID3D11Texture2D> {
        let nodig = (breedte as usize) * (hoogte as usize) * 4;
        if pixels.len() != nodig {
            anyhow::bail!("{} bytes aangeleverd, {nodig} verwacht", pixels.len());
        }
        let desc = D3D11_TEXTURE2D_DESC {
            Width: breedte,
            Height: hoogte,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let data = D3D11_SUBRESOURCE_DATA {
            pSysMem: pixels.as_ptr() as *const _,
            SysMemPitch: breedte * 4,
            SysMemSlicePitch: 0,
        };
        let mut tex = None;
        // SAFETY: `data` wijst naar `pixels`, dat lang genoeg blijft leven, en de
        // afmetingen kloppen met de controle hierboven.
        unsafe {
            self.device
                .CreateTexture2D(&desc, Some(&data), Some(&mut tex))
                .context("textuur met inhoud aanmaken")?;
        }
        tex.context("D3D11 gaf geen textuur terug")
    }

    /// Een NV12-textuur waar een decoder die in het werkgeheugen werkt zijn beeld in
    /// kwijt kan. Hardware-decoders leveren hun eigen textuur en hebben dit niet nodig.
    pub fn maak_nv12_textuur(&self, breedte: u32, hoogte: u32) -> Result<ID3D11Texture2D> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: breedte,
            // NV12 heeft halve chroma-resolutie, dus beide maten moeten even zijn.
            Height: hoogte,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_NV12,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut tex = None;
        // SAFETY: zie `maak_textuur`.
        unsafe {
            self.device
                .CreateTexture2D(&desc, None, Some(&mut tex))
                .context("NV12-textuur aanmaken")?;
        }
        tex.context("D3D11 gaf geen textuur terug")
    }

    /// Haalt een BGRA-textuur terug naar het werkgeheugen, met de rijen aaneengesloten.
    ///
    /// Dit is een trage weg — een GPU-naar-CPU-kopie met een wachtmoment — en hoort
    /// daarom nergens in het hot path thuis. Het bestaat om te kunnen controleren dat
    /// er beeld uit de keten komt en dat de kleuren kloppen.
    pub fn lees_bgra(&self, tex: &ID3D11Texture2D) -> Result<(u32, u32, Vec<u8>)> {
        let (breedte, hoogte) = afmetingen(tex);
        let desc = D3D11_TEXTURE2D_DESC {
            Width: breedte,
            Height: hoogte,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        let mut staging = None;
        // SAFETY: `desc` is volledig ingevuld; de kopie gaat tussen twee texturen van
        // gelijke afmeting en formaat.
        let pixels = unsafe {
            self.device
                .CreateTexture2D(&desc, None, Some(&mut staging))
                .context("uitleestextuur aanmaken")?;
            let staging = staging.context("D3D11 gaf geen textuur terug")?;
            self.context.CopyResource(&staging, tex);

            let mut kaart = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut kaart))
                .context("textuur uitlezen")?;

            let mut uit = Vec::with_capacity((breedte * hoogte * 4) as usize);
            for rij in 0..hoogte {
                let begin = (kaart.pData as *const u8).add((rij * kaart.RowPitch) as usize);
                uit.extend_from_slice(std::slice::from_raw_parts(begin, (breedte * 4) as usize));
            }
            self.context.Unmap(&staging, 0);
            uit
        };
        Ok((breedte, hoogte, pixels))
    }
}

/// Afmetingen van een bestaande textuur.
pub fn afmetingen(tex: &ID3D11Texture2D) -> (u32, u32) {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    // SAFETY: `GetDesc` vult alleen de meegegeven struct.
    unsafe { tex.GetDesc(&mut desc) };
    (desc.Width, desc.Height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "vereist een GPU"]
    fn apparaat_en_textuur_zijn_te_maken() {
        let d3d = D3dContext::new().expect("D3D11-apparaat");
        let tex = d3d.maak_textuur(1920, 1080).expect("textuur");
        assert_eq!(afmetingen(&tex), (1920, 1080));
    }
}
