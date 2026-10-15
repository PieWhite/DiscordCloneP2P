//! Lijnen op een BGRA-doek: voor de laag over een kijkvenster of een gedeeld scherm, en voor
//! de tegels in het hoofdvenster (beslissing 45). Zuiver Rust, op beide platforms, zonder
//! GPU en zonder tekenbibliotheek: een lijn is een reeks stukken met ronde uiteinden, en elke
//! pixel krijgt de dekking van zijn afstand tot het stuk.
//!
//! Het doek is **voorvermenigvuldigd** BGRA, zoals `UpdateLayeredWindow` het wil: een kleur
//! die niet met zijn alfa vermenigvuldigd is geeft lichte randen om elke lijn. Op een dekkend
//! beeld (een tegel) blijft alles dekkend.
//!
//! Er wordt alleen getekend als de tekening verandert, nooit per videobeeld.

use fitcom_proto::teken::{Lijn, Tekening};
use std::time::Duration;

/// Hooguit zo vaak tekent een laag opnieuw, hoeveel berichten er ook binnenkomen. Een
/// eerlijke tekenaar stuurt elke 40 ms een stukje; dit begrenst wat een overspoelende peer
/// kost, vooral op de PC van de deler. Wat ertussen binnenkomt wacht; alleen de laatste
/// tekening telt.
pub const MIN_HERTEKEN: Duration = Duration::from_millis(30);

/// Een gebied op het doek in pixels, `x0..x1` bij `y0..y1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kader {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl Kader {
    fn unie(self, o: Kader) -> Kader {
        Kader {
            x0: self.x0.min(o.x0),
            y0: self.y0.min(o.y0),
            x1: self.x1.max(o.x1),
            y1: self.y1.max(o.y1),
        }
    }
}

fn unie(a: Option<Kader>, b: Option<Kader>) -> Option<Kader> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.unie(b)),
        (a, b) => a.or(b),
    }
}

/// Een BGRA-buffer van `breedte` × `hoogte`, rij na rij, zonder opvulling.
pub struct Doek<'a> {
    pub px: &'a mut [u8],
    pub breedte: u32,
    pub hoogte: u32,
}

impl Doek<'_> {
    /// Helemaal doorzichtig.
    pub fn wis(&mut self) {
        self.px.fill(0);
    }

    /// Alle lijnen, oudste eerst. Levert het gebied dat geraakt is.
    pub fn teken_alles(&mut self, t: &Tekening) -> Option<Kader> {
        t.lijnen()
            .iter()
            .fold(None, |k, l| unie(k, self.teken_lijn(l, 0)))
    }

    /// Alleen wat [`aanvulling`] als nieuw aanwees.
    pub fn teken_aanvulling(&mut self, t: &Tekening, nieuw: &[(usize, usize)]) -> Option<Kader> {
        nieuw.iter().fold(None, |k, &(i, vanaf)| {
            unie(k, t.lijnen().get(i).and_then(|l| self.teken_lijn(l, vanaf)))
        })
    }

    /// Eén lijn, vanaf punt `vanaf` (het stuk dat daar aansluit hoort erbij).
    pub fn teken_lijn(&mut self, lijn: &Lijn, vanaf: usize) -> Option<Kader> {
        let (b, h) = (self.breedte as f32, self.hoogte as f32);
        let naar_px = |p: [u16; 2]| (f32::from(p[0]) / 65535.0 * b, f32::from(p[1]) / 65535.0 * h);
        // Op een tegel van 108 regels is een dunne lijn minder dan een pixel breed; daar
        // minstens twee pixels, anders is hij niet meer dan een waas.
        let straal = (lijn.dikte() * h / 2.0).max(1.0);
        let [r, g, bl] = lijn.rgb();
        let kleur = [f32::from(bl), f32::from(g), f32::from(r)];

        match lijn.punten.as_slice() {
            [] => None,
            [p] if vanaf == 0 => self.stuk(naar_px(*p), naar_px(*p), straal, kleur),
            punten => (vanaf.max(1)..punten.len()).fold(None, |k, i| {
                unie(
                    k,
                    self.stuk(naar_px(punten[i - 1]), naar_px(punten[i]), straal, kleur),
                )
            }),
        }
    }

    /// Eén stuk met ronde uiteinden. `kleur` is BGR.
    fn stuk(
        &mut self,
        a: (f32, f32),
        z: (f32, f32),
        straal: f32,
        kleur: [f32; 3],
    ) -> Option<Kader> {
        let rand = straal + 1.0;
        let x0 = ((a.0.min(z.0) - rand).floor() as i32).max(0);
        let y0 = ((a.1.min(z.1) - rand).floor() as i32).max(0);
        let x1 = ((a.0.max(z.0) + rand).ceil() as i32).min(self.breedte as i32);
        let y1 = ((a.1.max(z.1) + rand).ceil() as i32).min(self.hoogte as i32);
        if x0 >= x1 || y0 >= y1 {
            return None;
        }

        let (dx, dy) = (z.0 - a.0, z.1 - a.1);
        let lengte2 = dx * dx + dy * dy;
        let rij = self.breedte as usize * 4;
        for y in y0..y1 {
            let py = y as f32 + 0.5;
            for x in x0..x1 {
                let px = x as f32 + 0.5;
                let t = if lengte2 > 0.0 {
                    (((px - a.0) * dx + (py - a.1) * dy) / lengte2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let (ox, oy) = (px - (a.0 + t * dx), py - (a.1 + t * dy));
                let dekking = (straal + 0.5 - (ox * ox + oy * oy).sqrt()).clamp(0.0, 1.0);
                if dekking <= 0.0 {
                    continue;
                }
                let i = y as usize * rij + x as usize * 4;
                let pixel = &mut self.px[i..i + 4];
                let rest = 1.0 - dekking;
                for k in 0..3 {
                    pixel[k] = (kleur[k] * dekking + f32::from(pixel[k]) * rest).round() as u8;
                }
                pixel[3] = (255.0 * dekking + f32::from(pixel[3]) * rest).round() as u8;
            }
        }
        Some(Kader { x0, y0, x1, y1 })
    }
}

/// Wat er sinds `oud` bijkwam, als er alleen iets bijkwam: per lijn van `nieuw` vanaf welk
/// punt er getekend moet worden. `None` als er iets verdween of verschoof (ongedaan, wissen,
/// een oudste lijn die eraf viel); dan moet alles opnieuw. Zo kost tekenen terwijl iemand
/// tekent alleen de nieuwe stukjes, niet het hele doek.
pub fn aanvulling(oud: &Tekening, nieuw: &Tekening) -> Option<Vec<(usize, usize)>> {
    let (o, n) = (oud.lijnen(), nieuw.lijnen());
    if n.len() < o.len() {
        return None;
    }
    let mut uit = Vec::new();
    for (i, (a, b)) in o.iter().zip(n).enumerate() {
        if a.tekenaar != b.tekenaar || a.id != b.id || b.punten.len() < a.punten.len() {
            return None;
        }
        if b.punten.len() > a.punten.len() {
            uit.push((i, a.punten.len()));
        }
    }
    uit.extend((o.len()..n.len()).map(|i| (i, 0)));
    Some(uit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fitcom_proto::control::Draw;
    use fitcom_proto::PeerId;

    fn tekening(lijnen: &[(u32, &[[u16; 2]])]) -> (Tekening, PeerId) {
        let wie = PeerId::new_random();
        let mut t = Tekening::nieuw(true);
        for (id, punten) in lijnen {
            assert!(t.teken(
                wie,
                &Draw {
                    stream_id: 1,
                    author: None,
                    line: *id,
                    color: 0,
                    width: 2,
                    start: 0,
                    points: punten.to_vec(),
                }
            ));
        }
        (t, wie)
    }

    #[test]
    fn een_lijn_landt_waar_hij_hoort_en_is_voorvermenigvuldigd() {
        let (t, _) = tekening(&[(1, &[[0, 32767], [65535, 32767]])]);
        let (b, h) = (400u32, 400u32);
        let mut px = vec![0u8; (b * h * 4) as usize];
        let mut doek = Doek {
            px: &mut px,
            breedte: b,
            hoogte: h,
        };
        let kader = doek.teken_alles(&t).expect("geraakt");
        assert!(kader.y0 <= 197 && kader.y1 >= 203);

        // Midden op de lijn: de kleur van het palet (rood), dekkend, als BGRA.
        let midden = ((200 * b + 200) * 4) as usize;
        assert_eq!(&px[midden..midden + 4], &[0x3B, 0x3B, 0xFF, 255]);
        // Ver ervan af: niets.
        assert_eq!(
            &px[(10 * b * 4) as usize..(10 * b * 4) as usize + 4],
            &[0; 4]
        );
        // Voorvermenigvuldigd: geen kanaal boven zijn alfa, ook niet op de zachte rand.
        assert!(px
            .chunks_exact(4)
            .all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]));
        assert!(
            px.chunks_exact(4).any(|p| p[3] > 0 && p[3] < 255),
            "zachte rand"
        );
    }

    #[test]
    fn op_een_dekkend_beeld_blijft_alles_dekkend() {
        let (t, _) = tekening(&[(1, &[[0, 0], [65535, 65535]])]);
        let mut px = [0x40u8, 0x40, 0x40, 255].repeat(64 * 36);
        let mut doek = Doek {
            px: &mut px,
            breedte: 64,
            hoogte: 36,
        };
        doek.teken_alles(&t);
        assert!(px.chunks_exact(4).all(|p| p[3] == 255));
    }

    #[test]
    fn de_rand_van_het_doek_wordt_niet_overschreden() {
        // Een punt precies in de hoek met een dikke lijn: het stuk valt deels buiten beeld.
        let (t, _) = tekening(&[(1, &[[65535, 65535], [65535, 65535]])]);
        let mut px = vec![0u8; 10 * 10 * 4];
        let mut doek = Doek {
            px: &mut px,
            breedte: 10,
            hoogte: 10,
        };
        let k = doek.teken_alles(&t).expect("de hoek zelf is geraakt");
        assert!(k.x1 <= 10 && k.y1 <= 10);
    }

    #[test]
    fn aanvullen_kent_alleen_bijkomen() {
        let wie = PeerId::new_random();
        let draw = |id: u32, start: u32, points: &[[u16; 2]]| Draw {
            stream_id: 1,
            author: None,
            line: id,
            color: 0,
            width: 0,
            start,
            points: points.to_vec(),
        };
        let mut oud = Tekening::nieuw(true);
        oud.teken(wie, &draw(1, 0, &[[0, 0], [1, 1]]));
        let mut nieuw = oud.clone();
        nieuw.teken(wie, &draw(1, 2, &[[2, 2]]));
        nieuw.teken(wie, &draw(2, 0, &[[5, 5], [6, 6]]));
        assert_eq!(aanvulling(&oud, &nieuw), Some(vec![(0, 2), (1, 0)]));
        assert_eq!(aanvulling(&nieuw, &nieuw), Some(vec![]));

        let mut minder = nieuw.clone();
        minder.ongedaan(wie);
        assert_eq!(aanvulling(&nieuw, &minder), None);
        minder.ongedaan(wie);
        minder.teken(wie, &draw(3, 0, &[[5, 5], [6, 6]]));
        assert_eq!(
            aanvulling(&oud, &minder),
            None,
            "andere lijn op dezelfde plek"
        );
    }
}
