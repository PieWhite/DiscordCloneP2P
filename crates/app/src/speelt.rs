//! "Speelt X" naast je naam: welk spel er bij ons vooraan staat, lokaal bepaald.
//!
//! Een spel is hier: het voorgrondvenster bedekt zijn hele monitor (schermvullend of
//! randloos schermvullend), heeft geen titelbalk, en zijn programma staat niet op de
//! lijst met bekende niet-spellen. Meer dan dat kan zonder een spellendatabase niet; de
//! lijst hieronder is de plek om een vals alarm weg te halen.
//!
//! Alles hier is goedkoop (een handvol Win32-aanroepen per meting, de versie-resource van
//! een exe één keer) en draait op een trage motortik, nooit op een UI-tik — invariant 4.
//! De naam van een *peer* komt als onbetrouwbare tekst binnen; `schoon` is de ene plek
//! die lengte en stuurtekens afdwingt, voor eigen en vreemde namen allebei.
//!
//! macOS: bewust niets. Een voorgrondvenster-met-bestandsbeschrijving vraagt daar
//! Accessibility- of Screen Recording-rechten, en dat is te veel voor een regel tekst.

use std::time::{Duration, Instant};

/// Hoe vaak de motor het voorgrondvenster bekijkt.
pub const METING_INTERVAL: Duration = Duration::from_secs(3);
/// Zo vaak achter elkaar dezelfde nieuwe waarneming voordat een ander spel geldt (~6 s).
const METINGEN_VOOR_WISSEL: u32 = 2;
/// Zo vaak achter elkaar "geen spel" voordat het spel weer weg is (~30 s). Veel langer dan
/// een wissel: even alt-tabben naar je browser of Discord is niet stoppen met spelen.
const METINGEN_VOOR_STOP: u32 = 10;
/// Langste naam in tekens, voor wat we sturen én wat we ontvangen.
pub const MAX_NAAM: usize = 48;

/// Programma's (exe-naam zonder `.exe`, kleine letters) die vaak schermvullend vooraan
/// staan maar geen spel zijn.
const GEEN_SPEL: &[&str] = &[
    // schil van het systeem
    "explorer",
    "applicationframehost",
    "searchhost",
    "shellexperiencehost",
    "startmenuexperiencehost",
    "lockapp",
    "logonui",
    "taskmgr",
    "mstsc",
    "systemsettings",
    // browsers
    "chrome",
    "msedge",
    "firefox",
    "brave",
    "opera",
    "opera_gx",
    "vivaldi",
    "arc",
    "iexplore",
    "zen",
    // wij, en wat erbij hoort
    "fitcom",
    "spotify",
    "discord",
    "discordptb",
    "discordcanary",
    "teams",
    "ms-teams",
    "slack",
    "zoom",
    "skype",
    // opnemen en streamen
    "obs64",
    "obs32",
    "obs",
    "streamlabs obs",
    "medal",
    "shadowplay",
    "nvidia share",
    // spelers
    "vlc",
    "mpv",
    "mpc-hc",
    "mpc-hc64",
    "mpc-be",
    "mpc-be64",
    "potplayermini",
    "potplayermini64",
    "wmplayer",
    "video.ui",
    "plex",
    "netflix",
    // launchers: staan vooraan terwijl je een spel kiest, niet terwijl je het speelt
    "steam",
    "steamwebhelper",
    "epicgameslauncher",
    "galaxyclient",
    "battle.net",
    "origin",
    "eadesktop",
    "upc",
    "ubisoftconnect",
    // kantoor en code
    "code",
    "devenv",
    "winword",
    "excel",
    "powerpnt",
    "outlook",
    "onenote",
    "notepad",
    "windowsterminal",
    "powershell",
    "cmd",
    // op afstand en virtueel
    "parsecd",
    "vmware",
    "vmware-vmx",
    "virtualbox",
    "virtualboxvm",
    "anydesk",
    "teamviewer",
];

pub fn is_geen_spel(exe_stem: &str) -> bool {
    let s = exe_stem.to_lowercase();
    GEEN_SPEL.contains(&s.as_str())
}

/// Maakt van een willekeurige tekst iets dat veilig als gewone tekst te tonen is: stuur- en
/// opmaaktekens (ook die de leesrichting omdraaien of onzichtbaar zijn) eruit, witruimte
/// ingekort, niet langer dan [`MAX_NAAM`] tekens. `None` als er niets leesbaars overblijft.
pub fn schoon(ruw: &str) -> Option<String> {
    let mut uit = String::new();
    let mut spatie = false;
    for ch in ruw.chars() {
        if ch.is_whitespace() {
            spatie = !uit.is_empty();
            continue;
        }
        if ch.is_control() || is_opmaakteken(ch) {
            continue;
        }
        if spatie {
            uit.push(' ');
            spatie = false;
        }
        uit.push(ch);
        if uit.chars().count() >= MAX_NAAM {
            break;
        }
    }
    let uit = uit.trim().to_string();
    (!uit.is_empty()).then_some(uit)
}

/// Zero-width, bidi-override en soortgelijke tekens: geen stuurteken in `char::is_control`
/// maar wel goed voor een naam die er anders uitziet dan hij is.
fn is_opmaakteken(ch: char) -> bool {
    matches!(ch,
        '\u{00AD}' | '\u{061C}' | '\u{180E}'
        | '\u{200B}'..='\u{200F}'
        | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{206F}'
        | '\u{FEFF}'
        | '\u{E0000}'..='\u{E007F}'
        | '\u{FE00}'..='\u{FE0F}'
        | '\u{034F}' | '\u{115F}' | '\u{1160}' | '\u{3164}' | '\u{FFA0}' | '\u{2800}'
        | '\u{FFF9}'..='\u{FFFB}')
}

/// Laat een waarneming pas een wissel worden als hij een paar metingen standhoudt, zodat een
/// snelle alt-tab de regel niet laat flapperen. Zuiver en dus testbaar.
#[derive(Debug, Default)]
pub struct Rust {
    /// Wat we nu als waarheid melden.
    huidig: Option<String>,
    /// Wat de laatste metingen opleverden en hoe vaak achter elkaar.
    kandidaat: Option<String>,
    herhaald: u32,
}

impl Rust {
    pub fn huidig(&self) -> Option<&str> {
        self.huidig.as_deref()
    }

    /// Terug naar "geen spel", zonder wachttijd (delen uitgezet).
    pub fn wis(&mut self) {
        *self = Self::default();
    }

    /// Verwerkt één meting. `Some(nieuw)` als de gemelde waarde nu wisselt.
    pub fn meet(&mut self, waarneming: Option<String>) -> Option<Option<String>> {
        if waarneming == self.huidig {
            self.kandidaat = None;
            self.herhaald = 0;
            return None;
        }
        if waarneming == self.kandidaat {
            self.herhaald += 1;
        } else {
            self.kandidaat = waarneming.clone();
            self.herhaald = 1;
        }
        let nodig = if waarneming.is_some() {
            METINGEN_VOOR_WISSEL
        } else {
            METINGEN_VOOR_STOP
        };
        if self.herhaald < nodig {
            return None;
        }
        self.kandidaat = None;
        self.herhaald = 0;
        self.huidig = waarneming.clone();
        Some(waarneming)
    }
}

/// Meet het voorgrondvenster en onthoudt per exe hoe het spel heet.
#[derive(Default)]
pub struct Zoeker {
    #[cfg(windows)]
    namen: std::collections::HashMap<String, Option<String>>,
    /// Het venster (`HWND`) van het laatst gevonden spel, voor "Stream <spel>". Blijft staan
    /// als je naar ons venster klikt: dan staat het spel juist niet meer vooraan.
    pub venster: Option<isize>,
}

impl Zoeker {
    /// De naam van het schermvullende spel dat nu vooraan staat, of `None`.
    pub fn voorgrond_spel(&mut self) -> Option<String> {
        #[cfg(windows)]
        {
            let (naam, hwnd) = win::voorgrond_spel(&mut self.namen)?;
            self.venster = Some(hwnd);
            Some(naam)
        }
        #[cfg(not(windows))]
        {
            None
        }
    }
}

/// Voor de motortik: wanneer de volgende meting aan de beurt is.
pub fn is_aan_de_beurt(laatste: Option<Instant>, nu: Instant) -> bool {
    laatste.is_none_or(|l| nu.duration_since(l) >= METING_INTERVAL)
}

#[cfg(windows)]
mod win {
    use super::{is_geen_spel, schoon};
    use std::collections::HashMap;
    use std::ffi::c_void;
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
    };
    use windows::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

    pub fn voorgrond_spel(namen: &mut HashMap<String, Option<String>>) -> Option<(String, isize)> {
        // Wat "schermvullend vooraan" is staat op één plek; de tekenlaag bij een gedeeld
        // scherm wijkt voor precies hetzelfde (beslissing 45).
        let (hwnd, _) = fitcom_video::tekenlaag::schermvullend_vooraan()?;
        // SAFETY: kale Win32-aanroepen met uitsluitend eigen, correct gegroeide buffers.
        unsafe {
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == 0 || pid == GetCurrentProcessId() {
                return None;
            }
            let pad = exe_pad(pid)?;
            let stem = std::path::Path::new(&pad)
                .file_stem()?
                .to_string_lossy()
                .into_owned();
            if is_geen_spel(&stem) {
                return None;
            }

            if let Some(Some(naam)) = namen.get(&pad) {
                return Some((naam.clone(), hwnd.0 as isize));
            }
            // Bewust nooit de venstertitel: die kan een documentnaam bevatten en gaat naar
            // alle peers. Beschrijving van de exe, anders de exe-naam.
            let naam = bestandsbeschrijving(&pad)
                .and_then(|b| schoon(&b))
                .or_else(|| schoon(&stem));
            // Alleen een gevonden naam onthouden.
            if naam.is_some() {
                namen.insert(pad, naam.clone());
            }
            naam.map(|n| (n, hwnd.0 as isize))
        }
    }

    unsafe fn exe_pad(pid: u32) -> Option<String> {
        let proces = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let r = QueryFullProcessImageNameW(
            proces,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(proces);
        r.ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }

    /// `FileDescription` uit de versie-resource, in de eerste taal die de exe opgeeft.
    unsafe fn bestandsbeschrijving(pad: &str) -> Option<String> {
        let mut breed: Vec<u16> = pad.encode_utf16().collect();
        breed.push(0);
        let pad = PCWSTR(breed.as_ptr());
        let grootte = GetFileVersionInfoSizeW(pad, None);
        if grootte == 0 {
            return None;
        }
        let mut data = vec![0u8; grootte as usize];
        GetFileVersionInfoW(pad, None, grootte, data.as_mut_ptr() as *mut c_void).ok()?;

        let mut ptr: *mut c_void = std::ptr::null_mut();
        let mut len = 0u32;
        let talen: Vec<u16> = "\\VarFileInfo\\Translation\0".encode_utf16().collect();
        let (taal, codepage) = if VerQueryValueW(
            data.as_ptr() as *const c_void,
            PCWSTR(talen.as_ptr()),
            &mut ptr,
            &mut len,
        )
        .as_bool()
            && len >= 4
        {
            let p = ptr as *const u16;
            (*p, *p.add(1))
        } else {
            (0x0409, 0x04B0)
        };
        let vraag: Vec<u16> =
            format!("\\StringFileInfo\\{taal:04x}{codepage:04x}\\FileDescription\0")
                .encode_utf16()
                .collect();
        if !VerQueryValueW(
            data.as_ptr() as *const c_void,
            PCWSTR(vraag.as_ptr()),
            &mut ptr,
            &mut len,
        )
        .as_bool()
            || len == 0
        {
            return None;
        }
        let s = std::slice::from_raw_parts(ptr as *const u16, len as usize);
        let eind = s.iter().position(|&c| c == 0).unwrap_or(s.len());
        Some(String::from_utf16_lossy(&s[..eind]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(t: &str) -> Option<String> {
        Some(t.to_string())
    }

    #[test]
    fn schoon_haalt_stuurtekens_en_bidi_weg_en_kapt_af() {
        assert_eq!(schoon("  Half\u{0}-Life\n\t2 "), s("Half-Life 2"));
        assert_eq!(schoon("evil\u{202E}txt.exe"), s("eviltxt.exe"));
        assert_eq!(schoon("a\u{200B}b"), s("ab"));
        assert_eq!(
            schoon("a\u{E0041}\u{FE0F}\u{034F}\u{3164}\u{2800}b"),
            s("ab")
        );
        assert_eq!(schoon("\u{2800}\u{3164}"), None);
        assert_eq!(schoon("\u{7}\u{1b}[31m"), s("[31m"));
        assert_eq!(schoon("   \u{0}\n"), None);
        assert_eq!(schoon(""), None);
        let lang = "x".repeat(500);
        assert_eq!(schoon(&lang).unwrap().chars().count(), MAX_NAAM);
        // Meerbyte-tekens mogen niet middenin doorgesneden worden.
        let emoji = "\u{1F3AE}".repeat(500);
        assert_eq!(schoon(&emoji).unwrap().chars().count(), MAX_NAAM);
    }

    #[test]
    fn lijst_kent_browsers_en_ons_eigen_programma() {
        assert!(is_geen_spel("Chrome"));
        assert!(is_geen_spel("fitcom"));
        assert!(is_geen_spel("Spotify"));
        assert!(!is_geen_spel("eldenring"));
    }

    #[test]
    fn een_wissel_wacht_op_een_tweede_meting() {
        let mut r = Rust::default();
        assert_eq!(r.meet(s("Elden Ring")), None);
        assert_eq!(r.meet(s("Elden Ring")), Some(s("Elden Ring")));
        assert_eq!(r.huidig(), Some("Elden Ring"));
        // Dezelfde waarneming daarna is geen wissel meer.
        assert_eq!(r.meet(s("Elden Ring")), None);
    }

    #[test]
    fn een_flapperende_waarneming_wordt_nooit_een_wissel() {
        let mut r = Rust::default();
        for _ in 0..5 {
            assert_eq!(r.meet(s("A")), None);
            assert_eq!(r.meet(s("B")), None);
        }
        assert_eq!(r.huidig(), None);
    }

    #[test]
    fn even_weg_van_het_spel_wist_niets_maar_lang_weg_wel() {
        let mut r = Rust::default();
        r.meet(s("A"));
        r.meet(s("A"));
        // Alt-tab van een paar metingen, dan weer terug.
        for _ in 0..(METINGEN_VOOR_STOP - 1) {
            assert_eq!(r.meet(None), None);
        }
        assert_eq!(r.meet(s("A")), None);
        assert_eq!(r.huidig(), Some("A"));
        // Echt weg.
        for _ in 0..(METINGEN_VOOR_STOP - 1) {
            assert_eq!(r.meet(None), None);
        }
        assert_eq!(r.meet(None), Some(None));
        assert_eq!(r.huidig(), None);
    }

    #[test]
    fn van_het_ene_spel_naar_het_andere() {
        let mut r = Rust::default();
        r.meet(s("A"));
        r.meet(s("A"));
        assert_eq!(r.meet(s("B")), None);
        assert_eq!(r.meet(s("B")), Some(s("B")));
    }
}
