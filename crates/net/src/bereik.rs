//! Bereikbaarheid meten, alleen lezen (A0 van `docs/PLAN-ZONDER-TAILSCALE.md`).
//!
//! Eén keer bij het starten: welke port-mappingprotocollen de router spreekt, welk IPv4-adres
//! hij naar buiten toont, en of deze pc een globaal IPv6-adres heeft. De uitkomst gaat naar
//! de log en naar één regel in Settings → Network, zodat de vrienden een screenshot kunnen
//! sturen zonder iets te hoeven doen. Daarmee is bekend of fase C nodig is vóórdat fase B
//! gebouwd wordt.
//!
//! Het externe adres vertelt de router pas bij een mapping. Daarom één tijdelijke mapping op
//! de control-poort, die meteen weer weggehaald wordt. Er praat hier niemand anders dan de
//! eigen router en, voor UPnP, het LAN (een SSDP-multicast en dan HTTP naar wie antwoordt):
//! geen STUN, geen server, niets op internet (invariant 1).
//!
//! Weggehaald is niet gegarandeerd weg: PCP en NAT-PMP geven een mapping vrij met één
//! UDP-pakket zonder antwoord, en wie de app sluit tijdens de meting (10 tot hooguit ~30 s na
//! het starten) laat hem staan. Dan verloopt hij vanzelf, na hooguit twee uur.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::num::NonZeroU16;
use std::time::Duration;

/// Probe: UPnP zoekt 1 s, PCP en NAT-PMP wachten elk 500 ms. Ruim daarboven.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// Een mapping aanvragen en terugkrijgen. UPnP gaat over HTTP naar de router en is de traagste.
const MAPPING_TIMEOUT: Duration = Duration::from_secs(10);
/// Hoe lang we het opruimen van de mapping de tijd geven voordat de client weggaat.
const OPRUIM_TIMEOUT: Duration = Duration::from_secs(5);

/// Wat de router en de interfaces over onze bereikbaarheid zeggen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bereik {
    /// Of er een router te vinden was om te vragen. Nee bij geen netwerk (nog niet, vlak na
    /// het inloggen) of een VPN die de standaardroute neemt; dan zeggen de drie hieronder
    /// niets over de router, en mag dat er in de UI ook niet zo uitzien.
    pub gateway: bool,
    pub upnp: bool,
    pub pcp: bool,
    pub nat_pmp: bool,
    /// Het IPv4-adres dat de router naar buiten toont, als een van de protocollen het
    /// vertelde.
    pub extern_v4: Option<Ipv4Addr>,
    /// Of deze pc een globaal IPv6-adres heeft.
    pub ipv6: bool,
}

/// Wat het externe adres over de verbinding naar buiten zegt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Soort {
    /// Een publiek IPv4-adres: de makkelijke kant, de router kan poorten openzetten.
    Publiek,
    /// De router zit zelf achter een NAT van de provider (`100.64.0.0/10`) of toont een
    /// privéadres: van buiten komt niemand er zomaar in.
    Cgnat,
    /// Geen protocol vertelde het.
    Onbekend,
}

impl Bereik {
    pub fn soort(&self) -> Soort {
        match self.extern_v4 {
            None => Soort::Onbekend,
            Some(ip) => {
                let o = ip.octets();
                if ip.is_private()
                    || ip.is_loopback()
                    || ip.is_link_local()
                    || ip.is_unspecified()
                    || (o[0] == 100 && o[1] & 0xc0 == 64)
                {
                    Soort::Cgnat
                } else {
                    Soort::Publiek
                }
            }
        }
    }
}

/// Meet één keer, op een eigen kleine runtime op een blokkeerthread. Mislukt er iets, dan
/// staat dat onderdeel op "nee" of "onbekend"; er komt nooit een fout uit, want dit mag het
/// starten niet tegenhouden (invariant 7).
///
/// Een eigen runtime omdat portmapper de gateway synchroon opzoekt (op Windows `SendARP` per
/// netwerkkaart), en dat hoort niet op een van de twee threads waarop de mesh en de motor
/// draaien. En als die runtime weg is, is aantoonbaar ook elke taak van portmapper weg.
pub async fn meet(poort: u16) -> Bereik {
    let gemeten = tokio::task::spawn_blocking(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map(|rt| rt.block_on(meet_hier(poort)))
    })
    .await;
    match gemeten {
        Ok(Ok(b)) => b,
        _ => Bereik::default(),
    }
}

async fn meet_hier(poort: u16) -> Bereik {
    let client = portmapper::Client::new(portmapper::Config::default());
    let probe = tokio::time::timeout(PROBE_TIMEOUT, client.probe()).await;
    let (gateway, upnp, pcp, nat_pmp) = match probe {
        Ok(Ok(Ok(p))) => (true, p.upnp, p.pcp, p.nat_pmp),
        // De probe liep wel, maar er was geen router te vinden of hij kwam niet terug.
        _ => (false, false, false, false),
    };

    let mut extern_v4 = None;
    if let (true, Some(poort)) = (upnp || pcp || nat_pmp, NonZeroU16::new(poort)) {
        let mut adres = client.watch_external_address();
        client.update_local_port(poort);
        extern_v4 = tokio::time::timeout(MAPPING_TIMEOUT, async {
            loop {
                if let Some(a) = *adres.borrow_and_update() {
                    return Some(*a.ip());
                }
                adres.changed().await.ok()?;
            }
        })
        .await
        .ok()
        .flatten();

        // Opruimen. De dienst handelt berichten één voor één af, en het vrijgeven van de
        // mapping hoort bij het afhandelen van `deactivate`. Een probe daarna komt dus pas
        // terug als de mapping vrijgegeven is; zonder die wachtpost zou het weggooien van de
        // client het vrijgeven halverwege afbreken en bleef de mapping tot zijn lease verloopt
        // op de router staan. De tweede probe slaat wat net gezien is over en probeert alleen
        // wat niet antwoordde opnieuw (bij UPnP nog een seconde zoeken).
        client.deactivate();
        let _ = tokio::time::timeout(OPRUIM_TIMEOUT, client.probe()).await;
    }
    drop(client);

    Bereik {
        gateway,
        upnp,
        pcp,
        nat_pmp,
        extern_v4,
        ipv6: heeft_globaal_ipv6(),
    }
}

/// Of deze pc een globaal IPv6-adres heeft.
///
/// Een UDP-socket "verbinden" stuurt niets: het systeem kiest alleen een route en een
/// bronadres. Is dat bronadres globaal (`2000::/3`), dan is er IPv6 naar buiten. Het doel is
/// een documentatieadres (`2001:db8::/32`) dat nooit iemand is; er gaat geen pakket heen.
/// Een unieke lokale (`fc00::/7`, zoals het tailnet) of link-local adres telt niet.
///
/// shortcut: alleen het voorkeursbronadres en geen lijst van alle interfaces (netwatch
/// opent daarvoor op Windows WMI); bij B7 de interfaces echt opsommen.
fn heeft_globaal_ipv6() -> bool {
    let bron = std::net::UdpSocket::bind("[::]:0")
        .and_then(|s| s.connect("[2001:db8::1]:9").map(|()| s))
        .and_then(|s| s.local_addr());
    matches!(bron, Ok(std::net::SocketAddr::V6(a)) if is_globaal(a.ip()))
}

fn is_globaal(ip: &Ipv6Addr) -> bool {
    let s = ip.segments();
    // Teredo (`2001::/32`) en 6to4 (`2002::/16`) zijn tunnels, geen IPv6 waarop iemand ons
    // van buiten bereikt.
    s[0] & 0xe000 == 0x2000 && s[..2] != [0x2001, 0x0db8] && s[..2] != [0x2001, 0] && s[0] != 0x2002
}

impl std::fmt::Display for Bereik {
    /// De regel voor de log. Het venster maakt er zijn eigen Engelse regel van.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ja = |b: bool| if b { "ja" } else { "nee" };
        if !self.gateway {
            return write!(f, "geen router gevonden · IPv6 {}", ja(self.ipv6));
        }
        write!(
            f,
            "PCP {} · NAT-PMP {} · UPnP {} · extern {} ({:?}) · IPv6 {}",
            ja(self.pcp),
            ja(self.nat_pmp),
            ja(self.upnp),
            self.extern_v4
                .map_or_else(|| "onbekend".to_string(), |ip| ip.to_string()),
            self.soort(),
            ja(self.ipv6)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn met(ip: &str) -> Bereik {
        Bereik {
            extern_v4: Some(ip.parse().unwrap()),
            ..Bereik::default()
        }
    }

    /// Tegen de echte router van deze pc. Maakt kort een mapping op poort 42660 en haalt hem
    /// weer weg; kijk daarna in de router of hij er niet meer staat.
    #[tokio::test]
    #[ignore = "vraagt de echte router"]
    async fn meet_de_echte_router() {
        let b = meet(42660).await;
        println!("{b}");
        assert!(
            b.gateway,
            "geen router gevonden: geen netwerk, of een VPN-exitnode?"
        );
    }

    #[test]
    fn het_externe_adres_bepaalt_de_soort() {
        assert_eq!(met("84.104.12.9").soort(), Soort::Publiek);
        // De NAT van de provider (RFC 6598): dan is de router zelf niet van buiten bereikbaar.
        assert_eq!(met("100.64.0.1").soort(), Soort::Cgnat);
        assert_eq!(met("100.127.255.254").soort(), Soort::Cgnat);
        assert_eq!(met("100.128.0.1").soort(), Soort::Publiek);
        // Een router achter nog een router toont een privéadres.
        assert_eq!(met("192.168.2.254").soort(), Soort::Cgnat);
        assert_eq!(met("10.20.30.40").soort(), Soort::Cgnat);
        assert_eq!(Bereik::default().soort(), Soort::Onbekend);
    }

    #[test]
    fn alleen_echt_globale_ipv6_telt() {
        for (ip, globaal) in [
            ("2a02:a46b:1234::5", true),
            ("2001:1c00::1", true),
            ("fd7a:115c:a1e0::1", false), // het tailnet
            ("fe80::1", false),
            ("::1", false),
            ("2001:db8::1", false),         // het doel van de meting zelf
            ("2001:0:4136:e378::1", false), // Teredo
            ("2002:5591:857b::1", false),   // 6to4
        ] {
            assert_eq!(is_globaal(&ip.parse().unwrap()), globaal, "{ip}");
        }
    }
}
