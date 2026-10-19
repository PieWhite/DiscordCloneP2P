//! Uitnodigingscodes (fase B1 van `docs/PLAN-ZONDER-TAILSCALE.md`).
//!
//! Een code vertelt een andere installatie wie je bent en waar hij je kan proberen:
//! `PeerId`, publieke sleutel, weergavenaam en de adressen die deze pc van zichzelf kent.
//! Hij is **niet geheim** en geeft geen toegang: binnenkomen gaat alleen met de sleutel die
//! in de code staat, en wie jouw code heeft maar jij de zijne niet, komt er bij jou niet in.
//! De adressen zijn hints; een vals adres kost hooguit een mislukte belpoging.
//!
//! ```text
//! fitcom1:<base32 zonder opvulling, kleine letters>
//!
//! [versie 1][PeerId 16][sleutel 32][naamlengte 1][naam ≤ 64][aantal 1]
//! per adres: [familie 4|6][ip 4|16][poort u16 BE]
//! [blake3 van al het voorgaande, eerste 4 bytes]
//! ```
//!
//! Binair en niet MessagePack: hij wordt geplakt in een chatapp, dus elke byte is een teken
//! dat iemand moet kopiëren. De controlesom vangt een afgekapte of half overgetypte code
//! voordat er een vreemd adres of een verkeerde sleutel uit komt.

use crate::PeerId;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// Waar een code mee begint. Het cijfer is de versie van de vorm, niet van de app.
pub const VOORVOEGSEL: &str = "fitcom1:";
const VERSIE: u8 = 1;
const CONTROLESOM_LEN: usize = 4;
/// Langer is geen naam meer; dezelfde grens als de weergavenaam in de handshake.
pub const MAX_NAAM: usize = 64;
/// Meer adressen dan dit zijn geen hints meer maar een lijst om af te lopen.
pub const MAX_ADRESSEN: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uitnodiging {
    pub peer_id: PeerId,
    pub public_key: [u8; 32],
    pub naam: String,
    /// Control-adressen (QUIC) om te proberen. Media volgt na het verbinden vanzelf.
    pub adressen: Vec<SocketAddr>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CodeFout {
    #[error("geen geldige tekens na fitcom1:")]
    Tekens,
    #[error("de code is afgekapt of verkeerd overgenomen")]
    Controlesom,
    #[error("code van een nieuwere versie van de app ({0})")]
    Versie(u8),
    #[error("de code is beschadigd")]
    Vorm,
}

/// Of een adres als hint iets kan betekenen. Poort 0, "overal" en multicast zijn nooit een
/// peer; zulke adressen vallen er bij het lezen al uit, uit een code en van de draad.
pub fn is_bruikbaar(a: &SocketAddr) -> bool {
    let ip = a.ip();
    let broadcast = matches!(ip, IpAddr::V4(v4) if v4.is_broadcast());
    a.port() != 0 && !ip.is_unspecified() && !ip.is_multicast() && !broadcast
}

impl Uitnodiging {
    /// De code als tekst: `fitcom1:` en dan kleine letters en cijfers.
    pub fn code(&self) -> String {
        let mut b = vec![VERSIE];
        b.extend_from_slice(self.peer_id.as_bytes());
        b.extend_from_slice(&self.public_key);
        let naam = afgekapt(&self.naam, MAX_NAAM);
        b.push(naam.len() as u8);
        b.extend_from_slice(naam.as_bytes());
        let adressen: Vec<&SocketAddr> = self
            .adressen
            .iter()
            .filter(|a| is_bruikbaar(a))
            .take(MAX_ADRESSEN)
            .collect();
        b.push(adressen.len() as u8);
        for a in adressen {
            match a.ip() {
                IpAddr::V4(v4) => {
                    b.push(4);
                    b.extend_from_slice(&v4.octets());
                }
                IpAddr::V6(v6) => {
                    b.push(6);
                    b.extend_from_slice(&v6.octets());
                }
            }
            b.extend_from_slice(&a.port().to_be_bytes());
        }
        let som = blake3::hash(&b);
        b.extend_from_slice(&som.as_bytes()[..CONTROLESOM_LEN]);
        format!(
            "{VOORVOEGSEL}{}",
            data_encoding::BASE32_NOPAD.encode(&b).to_ascii_lowercase()
        )
    }

    /// Leest één code, met of zonder voorvoegsel, in hoofd- of kleine letters.
    pub fn lees(tekst: &str) -> Result<Self, CodeFout> {
        let t = tekst.trim();
        let t = match t.get(..VOORVOEGSEL.len()) {
            Some(v) if v.eq_ignore_ascii_case(VOORVOEGSEL) => &t[VOORVOEGSEL.len()..],
            _ => t,
        };
        let b = data_encoding::BASE32_NOPAD
            .decode(t.to_ascii_uppercase().as_bytes())
            .map_err(|_| CodeFout::Tekens)?;
        if b.len() < 1 + CONTROLESOM_LEN {
            return Err(CodeFout::Vorm);
        }
        let (inhoud, som) = b.split_at(b.len() - CONTROLESOM_LEN);
        if blake3::hash(inhoud).as_bytes()[..CONTROLESOM_LEN] != *som {
            return Err(CodeFout::Controlesom);
        }
        let mut r = Lezer(inhoud);
        match r.byte()? {
            VERSIE => {}
            v => return Err(CodeFout::Versie(v)),
        }
        let peer_id = PeerId::from_bytes(r.vast()?);
        let public_key = r.vast()?;
        let n = r.byte()? as usize;
        if n > MAX_NAAM {
            return Err(CodeFout::Vorm);
        }
        let naam = std::str::from_utf8(r.stuk(n)?).map_err(|_| CodeFout::Vorm)?;
        // De naam komt van een andere pc: stuurtekens eruit, zoals bij de handshake.
        let naam: String = naam.chars().filter(|c| !c.is_control()).collect();
        let m = r.byte()? as usize;
        if m > MAX_ADRESSEN {
            return Err(CodeFout::Vorm);
        }
        let mut adressen = Vec::with_capacity(m);
        for _ in 0..m {
            let ip = match r.byte()? {
                4 => IpAddr::V4(Ipv4Addr::from(r.vast::<4>()?)),
                6 => IpAddr::V6(Ipv6Addr::from(r.vast::<16>()?)),
                _ => return Err(CodeFout::Vorm),
            };
            let poort = u16::from_be_bytes(r.vast()?);
            let a = SocketAddr::new(ip, poort);
            if is_bruikbaar(&a) && !adressen.contains(&a) {
                adressen.push(a);
            }
        }
        if !r.0.is_empty() {
            return Err(CodeFout::Vorm);
        }
        Ok(Self {
            peer_id,
            public_key,
            naam: naam.trim().to_string(),
            adressen,
        })
    }

    /// Elke code in een geplakte tekst, zodat een heel WhatsApp-bericht plakken ook werkt.
    /// Een code begint bij `fitcom1:` en loopt tot het eerste teken dat geen base32 is.
    pub fn vind_in(tekst: &str) -> Vec<Result<Self, CodeFout>> {
        let klein = tekst.to_ascii_lowercase();
        let mut uit = Vec::new();
        let mut rest = klein.as_str();
        while let Some(i) = rest.find(VOORVOEGSEL) {
            let na = &rest[i + VOORVOEGSEL.len()..];
            let eind = na
                .find(|c: char| !matches!(c, 'a'..='z' | '2'..='7'))
                .unwrap_or(na.len());
            if eind > 0 {
                uit.push(Self::lees(&na[..eind]));
            }
            rest = &na[eind..];
        }
        uit
    }
}

fn afgekapt(s: &str, max: usize) -> &str {
    let mut eind = s.len().min(max);
    while !s.is_char_boundary(eind) {
        eind -= 1;
    }
    &s[..eind]
}

struct Lezer<'a>(&'a [u8]);

impl Lezer<'_> {
    fn stuk(&mut self, n: usize) -> Result<&[u8], CodeFout> {
        if self.0.len() < n {
            return Err(CodeFout::Vorm);
        }
        let (kop, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(kop)
    }

    fn byte(&mut self) -> Result<u8, CodeFout> {
        Ok(self.stuk(1)?[0])
    }

    fn vast<const N: usize>(&mut self) -> Result<[u8; N], CodeFout> {
        Ok(self.stuk(N)?.try_into().expect("lengte net gecontroleerd"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voorbeeld() -> Uitnodiging {
        Uitnodiging {
            peer_id: PeerId::new_random(),
            public_key: [7; 32],
            naam: "Rick".into(),
            adressen: vec![
                "84.104.12.9:41650".parse().unwrap(),
                "192.168.2.10:41650".parse().unwrap(),
                "100.101.102.103:41650".parse().unwrap(),
                "[2a02:a46b::5]:41650".parse().unwrap(),
            ],
        }
    }

    #[test]
    fn een_code_komt_heelhuids_terug() {
        let u = voorbeeld();
        let code = u.code();
        assert!(code.starts_with(VOORVOEGSEL));
        assert!(
            code.len() < 200,
            "{} tekens is te veel om te plakken",
            code.len()
        );
        assert_eq!(Uitnodiging::lees(&code), Ok(u.clone()));
        // Hoofdletters (een telefoon die de zin begint) en zonder voorvoegsel ook.
        assert_eq!(Uitnodiging::lees(&code.to_uppercase()), Ok(u.clone()));
        assert_eq!(Uitnodiging::lees(&code[VOORVOEGSEL.len()..]), Ok(u));
    }

    #[test]
    fn een_omgedraaid_teken_of_een_afgekapte_code_wordt_geweigerd() {
        let code = voorbeeld().code();
        let mut tekens: Vec<char> = code.chars().collect();
        let i = VOORVOEGSEL.len() + 20;
        tekens[i] = if tekens[i] == 'a' { 'b' } else { 'a' };
        let fout: String = tekens.into_iter().collect();
        assert_eq!(Uitnodiging::lees(&fout), Err(CodeFout::Controlesom));
        assert!(Uitnodiging::lees(&code[..code.len() - 10]).is_err());
        assert_eq!(Uitnodiging::lees("fitcom1:0189"), Err(CodeFout::Tekens));
    }

    #[test]
    fn codes_worden_uit_een_heel_bericht_gevist() {
        let (a, b) = (voorbeeld(), voorbeeld());
        let bericht = format!(
            "Hoi! Hier is de mijne: {} en die van Sam staat hieronder\n\n{}.\nGroetjes",
            a.code(),
            b.code().to_uppercase()
        );
        assert_eq!(Uitnodiging::vind_in(&bericht), vec![Ok(a), Ok(b)]);
        assert!(Uitnodiging::vind_in("geen code hier, alleen fitcom1: en tekst").is_empty());
    }

    #[test]
    fn grenzen_en_rommel() {
        let mut u = voorbeeld();
        u.naam = "é".repeat(100) + "\u{7}";
        u.adressen = (1..=20)
            .map(|i| SocketAddr::from(([10, 0, 0, i], 41650)))
            .chain([
                "0.0.0.0:41650".parse().unwrap(),
                "1.2.3.4:0".parse().unwrap(),
            ])
            .collect();
        let terug = Uitnodiging::lees(&u.code()).unwrap();
        assert_eq!(
            terug.naam,
            "é".repeat(MAX_NAAM / 2),
            "afgekapt op een tekengrens"
        );
        assert_eq!(terug.adressen.len(), MAX_ADRESSEN);
        assert!(terug.adressen.iter().all(is_bruikbaar));
    }

    #[test]
    fn een_nieuwere_versie_zegt_dat_ook() {
        let mut b = vec![2u8; 60];
        let som = blake3::hash(&b);
        b.extend_from_slice(&som.as_bytes()[..CONTROLESOM_LEN]);
        let code = data_encoding::BASE32_NOPAD.encode(&b);
        assert_eq!(Uitnodiging::lees(&code), Err(CodeFout::Versie(2)));
    }
}
