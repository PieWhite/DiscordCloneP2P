//! Tekst van en naar het Windows-klembord, voor het delen tussen een deler en zijn bediener
//! (beslissing 54). Alleen `CF_UNICODETEXT`, alleen zolang er bediend wordt; wie luistert
//! (`AddClipboardFormatListener`) staat bij de deler in [`crate::bediening`] en bij de kijker
//! in het kijkvenster.
//!
//! # Geen pingpong
//!
//! Wat wij zelf op het klembord zetten mag niet als "verandering" teruggaan, anders kaatsen
//! twee peers dezelfde tekst eindeloos heen en weer (en bij twee die elkaar tegelijk bedienen
//! ook via een tweede weg). Daarom onthoudt dit proces van zijn laatste eigen schrijfactie het
//! volgnummer van het klembord en een hash van de tekst, voor elke luisteraar in het proces
//! tegelijk. Nooit de tekst zelf, en nooit in een logregel: het kan een wachtwoord zijn.

use fitcom_proto::bediening::{schoon_klembord, MAX_KLEMBORD};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber, OpenClipboard,
    SetClipboardData,
};
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
use windows::Win32::System::Ole::CF_UNICODETEXT;

/// Hash van de laatste tekst die dit proces op het klembord zette; 0 = nog niets.
static EIGEN_HASH: AtomicU64 = AtomicU64::new(0);
/// Het volgnummer van het klembord direct na die schrijfactie.
static EIGEN_VOLGNUMMER: AtomicU32 = AtomicU32::new(0);

fn hash(tekst: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    tekst.hash(&mut h);
    h.finish().max(1)
}

/// Het klembord openen. Een ander programma kan het net vasthouden (het programma dat
/// kopieerde, of een klembordbeheerder); even opnieuw proberen, nooit lang wachten.
fn open(hwnd: HWND) -> bool {
    for poging in 0..5 {
        // SAFETY: een eigen venster als eigenaar; elke geslaagde open wordt gesloten.
        if unsafe { OpenClipboard(Some(hwnd)) }.is_ok() {
            return true;
        }
        if poging < 4 {
            std::thread::sleep(Duration::from_millis(4));
        }
    }
    false
}

/// De tekst op het klembord, of `None` als er geen tekst op staat, hij te groot is of het
/// klembord niet open wil.
fn lees(hwnd: HWND) -> Option<String> {
    if !open(hwnd) {
        return None;
    }
    // SAFETY: het klembord is open; de handle is van het klembord en wordt alleen tussen
    // lock en unlock gelezen, binnen de grootte die Windows opgeeft.
    let tekst = unsafe {
        let tekst = GetClipboardData(u32::from(CF_UNICODETEXT.0))
            .ok()
            .and_then(|h| {
                let mem = HGLOBAL(h.0);
                let eenheden = GlobalSize(mem) / 2;
                // Elke UTF-16-eenheid is minstens één byte UTF-8: meer eenheden dan de grens is
                // zeker te groot, en dan lezen we niets.
                if eenheden == 0 || eenheden > MAX_KLEMBORD {
                    return None;
                }
                let p = GlobalLock(mem) as *const u16;
                if p.is_null() {
                    return None;
                }
                let alles = std::slice::from_raw_parts(p, eenheden);
                let eind = alles.iter().position(|&c| c == 0).unwrap_or(eenheden);
                let s = String::from_utf16_lossy(&alles[..eind]);
                let _ = GlobalUnlock(mem);
                Some(s)
            });
        let _ = CloseClipboard();
        tekst
    };
    tekst.as_deref().and_then(schoon_klembord)
}

/// Een verandering van het klembord die niet van ons zelf komt, schoongemaakt en binnen de
/// grens. `None` als het onze eigen schrijfactie was of er niets bruikbaars op staat.
pub fn verandering(hwnd: HWND) -> Option<String> {
    // SAFETY: kale aanroep zonder argumenten.
    let nu = unsafe { GetClipboardSequenceNumber() };
    if nu != 0 && nu == EIGEN_VOLGNUMMER.load(Ordering::Relaxed) {
        return None;
    }
    let tekst = lees(hwnd)?;
    (hash(&tekst) != EIGEN_HASH.load(Ordering::Relaxed)).then_some(tekst)
}

/// Zet tekst op het klembord. Wordt eerst schoongemaakt; wat niet door
/// `bediening::schoon_klembord` komt gaat er niet op. `false` als het niet lukte.
pub fn schrijf(hwnd: HWND, tekst: &str) -> bool {
    let Some(tekst) = schoon_klembord(tekst) else {
        return false;
    };
    // Vóór het schrijven: de melding van de verandering kan al tijdens `SetClipboardData`
    // bij een luisteraar in dit proces aankomen.
    EIGEN_HASH.store(hash(&tekst), Ordering::Relaxed);
    let breed: Vec<u16> = tekst.encode_utf16().chain(std::iter::once(0)).collect();
    if !open(hwnd) {
        return false;
    }
    // SAFETY: het klembord is open; het geheugen is groot genoeg voor `breed` en gaat bij
    // succes over naar het klembord, anders geven we het zelf vrij.
    let gelukt = unsafe {
        let gelukt = (|| {
            EmptyClipboard().ok()?;
            let mem = GlobalAlloc(GMEM_MOVEABLE, breed.len() * 2).ok()?;
            let p = GlobalLock(mem) as *mut u16;
            if p.is_null() {
                let _ = GlobalFree(Some(mem));
                return None;
            }
            std::ptr::copy_nonoverlapping(breed.as_ptr(), p, breed.len());
            let _ = GlobalUnlock(mem);
            if SetClipboardData(u32::from(CF_UNICODETEXT.0), Some(HANDLE(mem.0))).is_err() {
                let _ = GlobalFree(Some(mem));
                return None;
            }
            Some(())
        })()
        .is_some();
        let _ = CloseClipboard();
        gelukt
    };
    if gelukt {
        // SAFETY: kale aanroep zonder argumenten.
        EIGEN_VOLGNUMMER.store(unsafe { GetClipboardSequenceNumber() }, Ordering::Relaxed);
    }
    gelukt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn de_hash_is_nooit_de_beginwaarde() {
        // 0 betekent "nog niets geschreven"; een tekst mag daar nooit toevallig op uitkomen.
        assert_ne!(hash(""), 0);
        assert_ne!(hash("a"), hash("b"));
        assert_eq!(hash("zelfde"), hash("zelfde"));
    }
}
