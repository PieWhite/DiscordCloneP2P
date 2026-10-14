//! Spotify vinden, zodat "Share Spotify" alleen zijn geluid opneemt.
//!
//! Alleen het proces-id: wat er speelt leest de app zelf (`crates/app/src/muziek.rs`), en
//! met Spotify praten doen we niet — geen Web API, geen account, niets over het internet.

/// Het pid van de draaiende Spotify-desktopapp, of `None` als hij niet draait.
///
/// Op Windows is Spotify een CEF-app met een handvol `Spotify.exe`'s; dit is de bovenste,
/// want de proces-loopback neemt met `include_tree` zijn hele boom mee — ook het kind
/// dat het geluid echt afspeelt. Goedkoop genoeg (één procesmomentopname) om elke paar
/// seconden te vragen of hij er nog is.
#[cfg(windows)]
pub fn pid() -> Option<u32> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    let momentopname = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.ok()?;
    let mut e = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut spotify: Vec<(u32, u32)> = Vec::new();
    if unsafe { Process32FirstW(momentopname, &mut e) }.is_ok() {
        loop {
            let lengte = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(0);
            let naam = String::from_utf16_lossy(&e.szExeFile[..lengte]);
            if naam.eq_ignore_ascii_case("Spotify.exe") {
                spotify.push((e.th32ProcessID, e.th32ParentProcessID));
            }
            if unsafe { Process32NextW(momentopname, &mut e) }.is_err() {
                break;
            }
        }
    }
    unsafe {
        let _ = CloseHandle(momentopname);
    }
    wortel(&spotify)
}

/// Op macOS heet het hoofdproces `Spotify`; de helpers heten anders (`Spotify Helper`),
/// dus een exacte naam volstaat. ScreenCaptureKit filtert daarna op de app, niet op de boom.
#[cfg(target_os = "macos")]
pub fn pid() -> Option<u32> {
    let uit = std::process::Command::new("pgrep")
        .args(["-x", "Spotify"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&uit.stdout)
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .min()
}

/// Van `(pid, ouder)`-paren het proces waarvan de ouder er zelf niet tussen staat.
#[cfg_attr(not(windows), allow(dead_code))]
fn wortel(processen: &[(u32, u32)]) -> Option<u32> {
    processen
        .iter()
        .find(|(_, ouder)| !processen.iter().any(|(p, _)| p == ouder))
        .map(|(p, _)| *p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn de_bovenste_spotify_is_de_wortel() {
        // Explorer (1) startte Spotify (40), die zijn renderer (41) en geluid (42) startte.
        assert_eq!(wortel(&[(41, 40), (42, 40), (40, 1)]), Some(40));
        assert_eq!(wortel(&[]), None);
    }
}
