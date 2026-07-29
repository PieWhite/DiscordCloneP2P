//! Herkennen van `@naam`-tags in chatberichten. Puur tekstwerk, geen state — gedeeld
//! tussen de meldingslaag (`engine.rs`) en het highlighten in de UI (`ui.rs`), zodat
//! "geldige tag" overal precies hetzelfde betekent.

/// Of `body` een geldige `@naam`-tag naar `naam` bevat. Hoofdletterongevoelig, met een
/// woordgrens aan beide kanten — zonder die grens zou `@Rick` ook `@Rickie` raken.
pub fn bevat_tag(body: &str, naam: &str) -> bool {
    if naam.is_empty() {
        return false;
    }
    let doel = format!("@{naam}").to_lowercase();
    let lower = body.to_lowercase();

    let mut vanaf = 0;
    while let Some(rel) = lower[vanaf..].find(&doel) {
        let start = vanaf + rel;
        let eind = start + doel.len();
        let voor_ok = lower[..start]
            .chars()
            .next_back()
            .map(|c| !is_tag_teken(c))
            .unwrap_or(true);
        let na_ok = lower[eind..]
            .chars()
            .next()
            .map(|c| !is_tag_teken(c))
            .unwrap_or(true);
        if voor_ok && na_ok {
            return true;
        }
        vanaf = start + 1;
    }
    false
}

/// Vindt een tag die nog getypt wordt: een `@`-run zonder witruimte die tot aan de
/// cursor doorloopt. `cursor_byte` is de byte-positie van de cursor in `tekst`.
/// Levert de byte-offset van de `@` en het al getypte deel erna op.
pub fn actieve_tag(tekst: &str, cursor_byte: usize) -> Option<(usize, &str)> {
    let voor = tekst.get(..cursor_byte)?;
    let start = voor
        .char_indices()
        .rev()
        .take_while(|(_, c)| !c.is_whitespace())
        .last()
        .map(|(i, _)| i)?;
    if !voor[start..].starts_with('@') {
        return None;
    }
    Some((start, &voor[start + 1..]))
}

/// Filtert `namen` op een hoofdletterongevoelig prefix van `query`, gesorteerd en
/// zonder dubbelingen.
pub fn tag_suggesties<'a>(namen: &'a [String], query: &str) -> Vec<&'a str> {
    let query_lower = query.to_lowercase();
    let mut gevonden: Vec<&str> = namen
        .iter()
        .map(String::as_str)
        .filter(|n| !n.is_empty() && n.to_lowercase().starts_with(&query_lower))
        .collect();
    gevonden.sort_unstable();
    gevonden.dedup();
    gevonden
}

fn is_tag_teken(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn herkent_een_gewone_tag() {
        assert!(bevat_tag("hoi @Rick, kijk eens", "Rick"));
    }

    #[test]
    fn is_hoofdletterongevoelig() {
        assert!(bevat_tag("hoi @rick", "Rick"));
        assert!(bevat_tag("hoi @RICK", "rick"));
    }

    #[test]
    fn respecteert_de_woordgrens() {
        assert!(!bevat_tag("hoi @Rickie", "Rick"));
        assert!(!bevat_tag("een adres zoals bla@Rick.nl", "Rick"));
    }

    #[test]
    fn tag_aan_het_begin_of_einde_van_het_bericht_telt() {
        assert!(bevat_tag("@Rick", "Rick"));
        assert!(bevat_tag("kijk @Rick", "Rick"));
        assert!(bevat_tag("@Rick kijk", "Rick"));
    }

    #[test]
    fn lege_naam_matcht_nooit() {
        assert!(!bevat_tag("@ per ongeluk", ""));
    }

    #[test]
    fn actieve_tag_vindt_de_lopende_run() {
        let tekst = "hallo @ric";
        assert_eq!(actieve_tag(tekst, tekst.len()), Some((6, "ric")));
    }

    #[test]
    fn actieve_tag_stopt_bij_witruimte() {
        let tekst = "hallo daar";
        assert_eq!(actieve_tag(tekst, tekst.len()), None);
    }

    #[test]
    fn actieve_tag_alleen_bij_een_at_teken() {
        let tekst = "gewoonwoord";
        assert_eq!(actieve_tag(tekst, tekst.len()), None);
    }

    #[test]
    fn actieve_tag_direct_na_het_at_teken() {
        let tekst = "@";
        assert_eq!(actieve_tag(tekst, 1), Some((0, "")));
    }

    #[test]
    fn actieve_tag_kijkt_niet_voorbij_de_cursor() {
        // De cursor staat middenin "@rick", dus alleen "@ri" telt mee.
        let tekst = "@rick";
        assert_eq!(actieve_tag(tekst, 3), Some((0, "ri")));
    }

    #[test]
    fn suggesties_filteren_op_prefix_en_zijn_gesorteerd() {
        let namen = vec!["Rick".to_string(), "Robin".to_string(), "Sam".to_string()];
        assert_eq!(tag_suggesties(&namen, "r"), vec!["Rick", "Robin"]);
        assert_eq!(tag_suggesties(&namen, ""), vec!["Rick", "Robin", "Sam"]);
        assert_eq!(tag_suggesties(&namen, "z"), Vec::<&str>::new());
    }
}
