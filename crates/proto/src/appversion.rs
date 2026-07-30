//! Vergelijken van app-versies (`env!("CARGO_PKG_VERSION")`), uitgewisseld in `Hello`/
//! `HelloAck` zodat een peer kan zien dat een ander een nieuwere build draait (fase 11).
//!
//! Geen `semver`-dependency: het formaat ligt vast op `workspace.package.version` in
//! `Cargo.toml` (altijd `MAJOR.MINOR.PATCH`), dus een simpele tuple-vergelijking volstaat.
//! Een onleesbaar onderdeel valt terug op `0` in plaats van te paniceren — deze string komt
//! van een peer en mag nooit crashen op onverwachte invoer.

fn parse(versie: &str) -> (u64, u64, u64) {
    let mut delen = versie.split('.').map(|d| d.parse::<u64>().unwrap_or(0));
    (
        delen.next().unwrap_or(0),
        delen.next().unwrap_or(0),
        delen.next().unwrap_or(0),
    )
}

/// Is `theirs` nieuwer dan `ours`? Gelijke versies of een onleesbare `theirs` geven `false`.
pub fn is_newer(theirs: &str, ours: &str) -> bool {
    parse(theirs) > parse(ours)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hogere_patch_is_nieuwer() {
        assert!(is_newer("0.1.2", "0.1.1"));
        assert!(!is_newer("0.1.1", "0.1.2"));
    }

    #[test]
    fn hogere_minor_wint_van_lagere_patch() {
        assert!(is_newer("0.2.0", "0.1.9"));
    }

    #[test]
    fn gelijke_versies_zijn_niet_nieuwer() {
        assert!(!is_newer("0.1.0", "0.1.0"));
    }

    #[test]
    fn onleesbare_versie_van_een_peer_crasht_niet_en_telt_als_ouder() {
        assert!(!is_newer("rommel", "0.1.0"));
        assert!(!is_newer("", "0.1.0"));
        assert!(is_newer("0.1.0", "rommel")); // "rommel" parseert als 0.0.0
    }
}
