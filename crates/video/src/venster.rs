//! Het venster waarin een inkomende stream te zien is.
//!
//! Eigen Win32-venster met eigen DXGI-swapchain, bediend door de thread die hem ook
//! aanmaakt. Zie `docs/ARCHITECTURE.md` → "Waarom video in een apart venster": eframe
//! rendert via wgpu op DX12, en een gedecodeerde D3D11-textuur daarin krijgen vereist
//! interop op `wgpu-hal`-niveau. Zo blijft het videopad volledig los van de UI en kan
//! de UI stilvallen zonder dat het beeld hapert.
//!
//! # Waarom er wél een vensterrand omheen zit
//!
//! De architectuur zegt "borderless". Dat is in de praktijk de verkeerde keuze: een
//! venster zonder rand kun je niet verplaatsen, niet vergroten en niet sluiten zonder
//! dat je dat allemaal zelf nabouwt met hit-testing. Wat er bedoeld werd — geen chroom
//! *over* het beeld, geen egui eromheen — geldt onverkort. Voor beeldvullend kijken
//! zit er een schakelaar op F11 en dubbelklik, en dán is hij randloos.
//!
//! # Waarom de swapchain niet meegroeit met het venster
//!
//! De swapchain houdt de afmeting van de stream en DXGI schaalt bij het presenteren.
//! Dat scheelt het opnieuw opbouwen van buffers bij elke muisbeweging tijdens het
//! slepen, en het schalen gebeurt toch al op de GPU. Om vervorming te voorkomen houdt
//! het venster zelf de beeldverhouding vast terwijl je sleept.

use crate::d3d::D3dContext;
use anyhow::{Context, Result};
use windows::core::{w, Interface, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGIDevice, IDXGIFactory2, IDXGIFactory5, IDXGISwapChain1, DXGI_FEATURE_PRESENT_ALLOW_TEARING,
    DXGI_MWA_NO_ALT_ENTER, DXGI_PRESENT, DXGI_PRESENT_ALLOW_TEARING, DXGI_SCALING_STRETCH,
    DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING,
    DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_F11};
use windows::Win32::UI::WindowsAndMessaging::*;

const KLASSE: PCWSTR = w!("FitcomVideoVenster");

/// Wat de vensterprocedure bijhoudt. Leeft in een `Box` achter `GWLP_USERDATA` en
/// wordt bij `WM_NCDESTROY` weer opgeruimd.
struct Staat {
    gesloten: bool,
    volledig_scherm: bool,
    /// Waar het venster stond voordat het beeldvullend werd.
    vorige_plaatsing: WINDOWPLACEMENT,
    /// Beeldverhouding van de stream, om vervorming bij het slepen te voorkomen.
    verhouding: f32,
}

pub struct Venster {
    hwnd: HWND,
    swapchain: IDXGISwapChain1,
    d3d: D3dContext,
    tearing: bool,
    afmeting: (u32, u32),
}

impl Venster {
    /// Maakt het venster aan **op de aanroepende thread**. Een Win32-venster hoort bij
    /// de thread die hem maakt: alleen die thread krijgt zijn berichten, dus alleen
    /// die thread mag [`Venster::pomp`] aanroepen.
    pub fn open(d3d: &D3dContext, titel: &str, breedte: u32, hoogte: u32) -> Result<Self> {
        registreer_klasse()?;

        let staat = Box::new(Staat {
            gesloten: false,
            volledig_scherm: false,
            vorige_plaatsing: WINDOWPLACEMENT {
                length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
                ..Default::default()
            },
            verhouding: breedte as f32 / hoogte.max(1) as f32,
        });

        let naam: Vec<u16> = titel.encode_utf16().chain(std::iter::once(0)).collect();
        // De opgegeven maat is die van het beeld; het venster moet zijn rand daar nog
        // omheen krijgen, anders is het beeld precies die rand te klein.
        let mut kader = RECT {
            left: 0,
            top: 0,
            right: breedte as i32,
            bottom: hoogte as i32,
        };

        // SAFETY: de klasse is geregistreerd en `staat` wordt in WM_NCCREATE overgenomen.
        let hwnd = unsafe {
            let _ = AdjustWindowRect(&mut kader, WS_OVERLAPPEDWINDOW, false);
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                KLASSE,
                PCWSTR(naam.as_ptr()),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                kader.right - kader.left,
                kader.bottom - kader.top,
                None,
                None,
                Some(GetModuleHandleW(None)?.into()),
                Some(Box::into_raw(staat) as *const _),
            )
            .context("videovenster aanmaken")?
        };

        // SAFETY: `hwnd` is net aangemaakt.
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        }

        let (swapchain, tearing) = maak_swapchain(d3d, hwnd, breedte, hoogte)?;

        tracing::info!(titel, breedte, hoogte, tearing, "videovenster geopend");

        Ok(Self {
            hwnd,
            swapchain,
            d3d: d3d.clone(),
            tearing,
            afmeting: (breedte, hoogte),
        })
    }

    pub fn afmeting(&self) -> (u32, u32) {
        self.afmeting
    }

    /// Handelt de wachtende vensterberichten af. `false` betekent dat de gebruiker het
    /// venster gesloten heeft; dan hoort de aanroeper te stoppen met kijken.
    ///
    /// Dit blokkeert niet. Het venster hoeft niet vloeiend te blijven als er geen beeld
    /// binnenkomt — maar het moet wel te sluiten zijn, en dat is wat dit garandeert.
    pub fn pomp(&mut self) -> bool {
        let mut msg = MSG::default();
        // SAFETY: we pompen de berichten van onze eigen thread.
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        !self.is_gesloten()
    }

    fn is_gesloten(&self) -> bool {
        // SAFETY: de staat blijft leven tot WM_NCDESTROY, en dan is `gesloten` allang
        // gezet en heeft de aanroeper ons opgeruimd.
        unsafe {
            let ptr = GetWindowLongPtrW(self.hwnd, GWLP_USERDATA) as *const Staat;
            ptr.is_null() || (*ptr).gesloten
        }
    }

    /// Zet één beeld op het scherm.
    ///
    /// Wijkt de afmeting af van waarmee het venster geopend is — de deler kondigde iets
    /// anders aan dan zijn encoder uiteindelijk levert, of hij wisselde van resolutie —
    /// dan past de swapchain zich aan. Het venster zelf blijft staan waar het staat.
    pub fn toon(&mut self, beeld: &ID3D11Texture2D) -> Result<()> {
        let maat = crate::d3d::afmetingen(beeld);
        if maat != self.afmeting {
            self.pas_maat_aan(maat)?;
        }

        // SAFETY: de backbuffer hoort bij deze swapchain en heeft hetzelfde formaat en
        // dezelfde maat als het beeld.
        unsafe {
            let achter: ID3D11Texture2D = self.swapchain.GetBuffer(0).context("backbuffer")?;
            self.d3d.context.CopyResource(&achter, beeld);

            // Zonder vsync: elke gewachte vblank is zestien milliseconde die je bij het
            // wijzen naar iets op een gedeeld scherm merkt. Scheuren is hier het
            // kleinere kwaad, en het treedt alleen op als de zender uit de pas loopt
            // met de monitor van de kijker.
            let vlaggen = if self.tearing {
                DXGI_PRESENT_ALLOW_TEARING
            } else {
                DXGI_PRESENT(0)
            };
            self.swapchain
                .Present(0, vlaggen)
                .ok()
                .context("beeld tonen")?;
        }
        Ok(())
    }

    fn pas_maat_aan(&mut self, (breedte, hoogte): (u32, u32)) -> Result<()> {
        tracing::info!(van = ?self.afmeting, naar = ?(breedte, hoogte), "stream van maat veranderd");

        // SAFETY: er staat geen enkele verwijzing naar een backbuffer meer open —
        // `toon` haalt hem op en laat hem in dezelfde aanroep weer los.
        unsafe {
            self.swapchain
                .ResizeBuffers(
                    2,
                    breedte,
                    hoogte,
                    DXGI_FORMAT_B8G8R8A8_UNORM,
                    if self.tearing {
                        DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING
                    } else {
                        DXGI_SWAP_CHAIN_FLAG(0)
                    },
                )
                .context("swapchain aanpassen")?;

            // De beeldverhouding die het venster bewaakt klopt nu niet meer.
            let ptr = GetWindowLongPtrW(self.hwnd, GWLP_USERDATA) as *mut Staat;
            if !ptr.is_null() {
                (*ptr).verhouding = breedte as f32 / hoogte.max(1) as f32;
            }
        }
        self.afmeting = (breedte, hoogte);
        Ok(())
    }
}

impl Drop for Venster {
    fn drop(&mut self) {
        // SAFETY: `hwnd` is van deze thread; na DestroyWindow raakt niemand hem meer aan.
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

fn registreer_klasse() -> Result<()> {
    use std::sync::Once;
    static EEN_KEER: Once = Once::new();
    let mut uitkomst = Ok(());
    EEN_KEER.call_once(|| {
        // SAFETY: de klasse wordt precies één keer per proces geregistreerd.
        unsafe {
            let klasse = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
                lpfnWndProc: Some(venster_proc),
                hInstance: match GetModuleHandleW(None) {
                    Ok(h) => h.into(),
                    Err(e) => {
                        uitkomst = Err(e).context("module-handle");
                        return;
                    }
                },
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                lpszClassName: KLASSE,
                ..Default::default()
            };
            if RegisterClassW(&klasse) == 0 {
                uitkomst = Err(windows::core::Error::from_thread()).context("vensterklasse");
            }
        }
    });
    uitkomst
}

fn maak_swapchain(
    d3d: &D3dContext,
    hwnd: HWND,
    breedte: u32,
    hoogte: u32,
) -> Result<(IDXGISwapChain1, bool)> {
    let dxgi: IDXGIDevice = d3d.device.cast().context("DXGI-apparaat")?;
    // SAFETY: het apparaat is geldig; adapter en factory horen er per definitie bij.
    let factory: IDXGIFactory2 = unsafe {
        let adapter = dxgi.GetAdapter().context("adapter")?;
        adapter.GetParent().context("DXGI-factory")?
    };

    // Scheuren toestaan kan alleen als de hele keten het ondersteunt. Op een machine
    // waar dat niet zo is werkt alles nog, alleen met iets meer vertraging.
    let tearing = factory
        .cast::<IDXGIFactory5>()
        .ok()
        .and_then(|f5| {
            let mut aan: u32 = 0;
            // SAFETY: uitvoerparameter is een geldige lokale variabele.
            unsafe {
                f5.CheckFeatureSupport(
                    DXGI_FEATURE_PRESENT_ALLOW_TEARING,
                    &mut aan as *mut _ as *mut _,
                    std::mem::size_of::<u32>() as u32,
                )
                .ok()?;
            }
            Some(aan != 0)
        })
        .unwrap_or(false);

    let desc = DXGI_SWAP_CHAIN_DESC1 {
        Width: breedte,
        Height: hoogte,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        Stereo: false.into(),
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        // Twee buffers en het flip-model: dat is het pad zonder tussenkopie in de
        // desktopcompositor.
        BufferCount: 2,
        // Rekken naar de venstermaat. Het venster bewaakt zelf de beeldverhouding.
        Scaling: DXGI_SCALING_STRETCH,
        SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
        AlphaMode: DXGI_ALPHA_MODE_IGNORE,
        Flags: if tearing {
            DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING.0 as u32
        } else {
            0
        },
    };

    // SAFETY: `desc` is volledig ingevuld en `hwnd` is een geldig venster van deze thread.
    let swapchain = unsafe {
        let sc = factory
            .CreateSwapChainForHwnd(&d3d.device, hwnd, &desc, None, None)
            .context("swapchain aanmaken")?;
        // DXGI's eigen alt-enter zet een echte fullscreen-modus aan, met een
        // modeswitch en al. Wij doen beeldvullend zelf, zonder modeswitch.
        let _ = factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER);
        sc
    };

    Ok((swapchain, tearing))
}

/// Beeldvullend zonder modeswitch: rand eraf en over het scherm heen waar het venster
/// op staat. Een echte fullscreen-modus zou de resolutie omgooien, en dat is precies
/// wat je niet wilt terwijl er op datzelfde scherm een game draait.
fn schakel_volledig_scherm(hwnd: HWND, staat: &mut Staat) {
    // SAFETY: `hwnd` is geldig en we draaien op de thread van het venster.
    unsafe {
        if staat.volledig_scherm {
            let _ = SetWindowLongPtrW(hwnd, GWL_STYLE, WS_OVERLAPPEDWINDOW.0 as isize);
            let _ = SetWindowPlacement(hwnd, &staat.vorige_plaatsing);
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
            );
            staat.volledig_scherm = false;
            return;
        }

        let _ = GetWindowPlacement(hwnd, &mut staat.vorige_plaatsing);
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return;
        }
        let r = info.rcMonitor;
        let _ = SetWindowLongPtrW(hwnd, GWL_STYLE, WS_POPUP.0 as isize);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOP),
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            SWP_NOZORDER | SWP_FRAMECHANGED,
        );
        staat.volledig_scherm = true;
    }
}

/// Houdt de beeldverhouding vast tijdens het slepen aan een rand. Zonder dit rekt het
/// beeld mee met het venster, want de swapchain schaalt zonder naar de vorm te kijken.
fn bewaak_verhouding(rand: u32, kader: &mut RECT, verhouding: f32) {
    let breedte = (kader.right - kader.left) as f32;
    let hoogte = (kader.bottom - kader.top) as f32;
    if breedte <= 0.0 || hoogte <= 0.0 {
        return;
    }

    let horizontaal = matches!(
        rand,
        WMSZ_LEFT | WMSZ_RIGHT | WMSZ_TOPLEFT | WMSZ_TOPRIGHT | WMSZ_BOTTOMLEFT | WMSZ_BOTTOMRIGHT
    );
    if horizontaal {
        let nieuw = (breedte / verhouding).round() as i32;
        if matches!(rand, WMSZ_TOPLEFT | WMSZ_TOPRIGHT) {
            kader.top = kader.bottom - nieuw;
        } else {
            kader.bottom = kader.top + nieuw;
        }
    } else {
        let nieuw = (hoogte * verhouding).round() as i32;
        if rand == WMSZ_TOP || rand == WMSZ_BOTTOM {
            kader.right = kader.left + nieuw;
        }
    }
}

const WMSZ_LEFT: u32 = 1;
const WMSZ_RIGHT: u32 = 2;
const WMSZ_TOP: u32 = 3;
const WMSZ_TOPLEFT: u32 = 4;
const WMSZ_TOPRIGHT: u32 = 5;
const WMSZ_BOTTOM: u32 = 6;
const WMSZ_BOTTOMLEFT: u32 = 7;
const WMSZ_BOTTOMRIGHT: u32 = 8;

unsafe extern "system" fn venster_proc(
    hwnd: HWND,
    bericht: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: alles hieronder draait op de thread van het venster, en `Staat` staat in
    // GWLP_USERDATA vanaf WM_NCCREATE tot WM_NCDESTROY.
    unsafe {
        if bericht == WM_NCCREATE {
            let cs = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
            return DefWindowProcW(hwnd, bericht, wparam, lparam);
        }

        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Staat;
        if ptr.is_null() {
            return DefWindowProcW(hwnd, bericht, wparam, lparam);
        }
        let staat = &mut *ptr;

        match bericht {
            // Sluiten melden we alleen; het opruimen doet de eigenaar via `drop`.
            // Zou dit venster zichzelf vernietigen, dan hield de kijkende thread een
            // ongeldige handle over.
            WM_CLOSE => {
                staat.gesloten = true;
                LRESULT(0)
            }
            WM_KEYDOWN => {
                // Escape haalt je alleen uit beeldvullend; als sluitknop is hij te
                // makkelijk per ongeluk te raken terwijl je meekijkt.
                let toets = wparam.0 as u16;
                if toets == VK_F11.0 || (toets == VK_ESCAPE.0 && staat.volledig_scherm) {
                    schakel_volledig_scherm(hwnd, staat);
                }
                LRESULT(0)
            }
            WM_LBUTTONDBLCLK => {
                schakel_volledig_scherm(hwnd, staat);
                LRESULT(0)
            }
            WM_SIZING => {
                bewaak_verhouding(
                    wparam.0 as u32,
                    &mut *(lparam.0 as *mut RECT),
                    staat.verhouding,
                );
                LRESULT(1)
            }
            // De swapchain vult het hele venster; de achtergrond wissen zou alleen maar
            // flikkeren opleveren.
            WM_ERASEBKGND => LRESULT(1),
            WM_NCDESTROY => {
                staat.gesloten = true;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(ptr));
                DefWindowProcW(hwnd, bericht, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, bericht, wparam, lparam),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verhouding_blijft_gelijk_bij_slepen() {
        // 16:9 vasthouden. Bij slepen aan een zijkant volgt de hoogte de breedte.
        let mut kader = RECT {
            left: 0,
            top: 0,
            right: 1600,
            bottom: 500,
        };
        bewaak_verhouding(WMSZ_RIGHT, &mut kader, 16.0 / 9.0);
        assert_eq!(kader.bottom - kader.top, 900);

        // Aan de bovenkant slepen verplaatst de bovenrand, niet de onderrand: anders
        // zou het venster onder je muis vandaan lopen.
        let mut kader = RECT {
            left: 0,
            top: 100,
            right: 1600,
            bottom: 1000,
        };
        bewaak_verhouding(WMSZ_TOPRIGHT, &mut kader, 16.0 / 9.0);
        assert_eq!(kader.bottom - kader.top, 900);
        assert_eq!(kader.bottom, 1000, "de onderrand hoort te blijven staan");
    }

    #[test]
    fn slepen_aan_de_onderrand_past_de_breedte_aan() {
        let mut kader = RECT {
            left: 0,
            top: 0,
            right: 100,
            bottom: 900,
        };
        bewaak_verhouding(WMSZ_BOTTOM, &mut kader, 16.0 / 9.0);
        assert_eq!(kader.right - kader.left, 1600);
    }

    #[test]
    #[ignore = "opent een echt venster"]
    fn venster_toont_een_beeld() {
        use std::time::{Duration, Instant};

        let d3d = D3dContext::new().expect("D3D11");
        let (b, h) = (640u32, 360u32);
        let pixels: Vec<u8> = (0..b * h)
            .flat_map(|i| {
                // Verloop, zodat meteen te zien is of het beeld op zijn kop staat of
                // dat de kleurkanalen omgewisseld zijn.
                let x = (i % b) as u8;
                let y = (i / b) as u8;
                [x, y, 128, 255]
            })
            .collect();
        let tex = d3d.maak_textuur_met(b, h, &pixels).expect("textuur");

        let mut venster = Venster::open(&d3d, "FitCom — test", b, h).expect("venster");
        let tot = Instant::now() + Duration::from_secs(5);
        while Instant::now() < tot && venster.pomp() {
            venster.toon(&tex).expect("tonen");
            std::thread::sleep(Duration::from_millis(16));
        }
    }
}
