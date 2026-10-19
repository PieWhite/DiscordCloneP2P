//! Header voor audio- en videopakketten. Gaan over plain UDP, niet over QUIC:
//! een hertransmissie die te laat aankomt is voor realtime media erger dan verlies.
//!
//! Met de hand geserialiseerd en niet via serde, omdat deze header op elk pakket zit —
//! bij 1080p60 zijn dat duizenden per seconde. Big-endian, conform netwerkconventie.

pub const MEDIA_HEADER_LEN: usize = 16;

/// Protocol 6: wat er vóór de versleutelde header staat. `[sessie-index u32][teller u64]`,
/// big-endian, onversleuteld maar wel geauthenticeerd (de AAD). De index kiest de
/// ontvanger; de nonce is vier nul-bytes gevolgd door de teller. Index 0 en teller 0 komen
/// nooit voor. Zie `crates/net/src/mediasleutel.rs`.
pub const MEDIA_KOP_LEN: usize = 12;

/// Protocol 6: de AES-256-GCM-tag achter elk pakket.
pub const MEDIA_TAG_LEN: usize = 16;

/// Protocol 6: het exporter-label voor de mediasleutels. De context is de `PeerId` van de
/// afzender, zodat elke richting een eigen sleutel heeft.
pub const MEDIA_SLEUTEL_LABEL: &[u8] = b"fitcom/media/v6";

/// Payload past onder de 1280 bytes die elk IPv6-pad moet halen, zonder IP-fragmentatie:
/// 1100 + header 16 + kop en tag 28 + IPv6 40 + UDP 8 = 1192 (1194 voor een
/// pariteitsfragment). Tot protocol 6 ging de ruimte voor kop en tag naar WireGuard.
pub const MAX_MEDIA_PAYLOAD: usize = 1100;

/// Lengte van de payload van een pariteitsfragment: twee bytes voor de lengte van het
/// stuk dat het vervangt, daarna de bytes zelf.
///
/// De lengte gaat mee door de XOR heen in plaats van er apart bij: dan levert het
/// herstel niet alleen de inhoud van het ontbrekende fragment op maar ook hoe lang het
/// was, en is het laatste (kortere) fragment geen apart geval.
pub const PARITEIT_PAYLOAD_LEN: usize = MAX_MEDIA_PAYLOAD + 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct PayloadType(pub u8);

impl PayloadType {
    pub const OPUS: Self = Self(1);
    pub const HEVC: Self = Self(2);
    pub const H264: Self = Self(3);
    /// Fase B5: een leeg pakket elke 15 s naar elke verbonden peer, zodat het gat in de NAT
    /// openstaat voordat iemand praat of deelt. Wie er een ontvangt weet dat de afzender één
    /// mediasocket heeft en mag voortaan antwoorden naar het bronadres van zijn pakketten
    /// (roaming, `crates/net/src/mediasleutel.rs`). Additief: een peer zonder B5 opent hem
    /// wel, maar audio laat alles behalve Opus liggen en een kijker alles van een andere
    /// stream. Bovenaan de reeks, zodat een volgende codec gewoon 4 kan nemen.
    pub const KEEPALIVE: Self = Self(0xff);
}

/// De stream-id van een keepalive. Geen echte stream: de deler deelt id's uit vanaf 1.
pub const KEEPALIVE_STREAM_ID: u32 = u32::MAX;

/// Stream 0 is altijd voice. Screenshare- en desktop-audio-streams krijgen id's vanaf 1,
/// toegekend door de deler.
pub const VOICE_STREAM_ID: u32 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaHeader {
    pub stream_id: u32,
    /// Monotoon per stream. Gaten betekenen verlies.
    pub seq: u32,
    /// 90 kHz voor video, 48 kHz voor audio.
    pub timestamp: u32,
    pub payload_type: PayloadType,
    pub flags: u8,
    /// Index binnen het frame. Videoframes passen zelden in één pakket.
    pub frag_index: u16,
}

impl MediaHeader {
    pub const FLAG_KEYFRAME: u8 = 0b0000_0001;
    /// Laatste fragment van dit frame. Zonder deze vlag weet de ontvanger niet
    /// wanneer een frame compleet is, want fragmenten kunnen door elkaar aankomen.
    pub const FLAG_LAST_FRAGMENT: u8 = 0b0000_0010;
    /// Geen beelddata maar de XOR van alle fragmenten van dit beeld, zodat één
    /// verloren fragment ter plekke te herstellen is in plaats van met een nieuw
    /// keyframe. Zijn `frag_index` is het *aantal* fragmenten, niet een plek erin —
    /// zo weet de ontvanger hoeveel stukken hij hoort te hebben, ook als juist het
    /// laatste (dat [`Self::FLAG_LAST_FRAGMENT`] draagt) zoek is.
    pub const FLAG_PARITEIT: u8 = 0b0000_0100;

    pub fn is_keyframe(&self) -> bool {
        self.flags & Self::FLAG_KEYFRAME != 0
    }

    pub fn is_last_fragment(&self) -> bool {
        self.flags & Self::FLAG_LAST_FRAGMENT != 0
    }

    pub fn is_pariteit(&self) -> bool {
        self.flags & Self::FLAG_PARITEIT != 0
    }

    pub fn write_to(&self, out: &mut [u8; MEDIA_HEADER_LEN]) {
        out[0..4].copy_from_slice(&self.stream_id.to_be_bytes());
        out[4..8].copy_from_slice(&self.seq.to_be_bytes());
        out[8..12].copy_from_slice(&self.timestamp.to_be_bytes());
        out[12] = self.payload_type.0;
        out[13] = self.flags;
        out[14..16].copy_from_slice(&self.frag_index.to_be_bytes());
    }

    /// `None` bij een pakket dat te kort is om een header te bevatten.
    pub fn parse(buf: &[u8]) -> Option<(Self, &[u8])> {
        if buf.len() < MEDIA_HEADER_LEN {
            return None;
        }
        let h = Self {
            stream_id: u32::from_be_bytes(buf[0..4].try_into().ok()?),
            seq: u32::from_be_bytes(buf[4..8].try_into().ok()?),
            timestamp: u32::from_be_bytes(buf[8..12].try_into().ok()?),
            payload_type: PayloadType(buf[12]),
            flags: buf[13],
            frag_index: u16::from_be_bytes(buf[14..16].try_into().ok()?),
        };
        Some((h, &buf[MEDIA_HEADER_LEN..]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_roundtrip() {
        let h = MediaHeader {
            stream_id: 7,
            seq: 123_456,
            timestamp: 90_000,
            payload_type: PayloadType::HEVC,
            flags: MediaHeader::FLAG_KEYFRAME | MediaHeader::FLAG_LAST_FRAGMENT,
            frag_index: 42,
        };
        let mut buf = [0u8; MEDIA_HEADER_LEN];
        h.write_to(&mut buf);
        let (back, rest) = MediaHeader::parse(&buf).unwrap();
        assert_eq!(back, h);
        assert!(rest.is_empty());
        assert!(back.is_keyframe() && back.is_last_fragment());
    }

    #[test]
    fn het_grootste_pakket_past_onder_de_ipv6_ondergrens() {
        // 1280 is wat elk IPv6-pad zonder fragmentatie moet halen; daarboven kan een
        // keyframe onderweg in stukken vallen of stil verdwijnen.
        let grootste = MEDIA_KOP_LEN + MEDIA_HEADER_LEN + PARITEIT_PAYLOAD_LEN + MEDIA_TAG_LEN;
        assert!(
            grootste + 40 + 8 <= 1280,
            "{grootste} bytes UDP-payload is te veel"
        );
    }

    #[test]
    fn keepalive_ligt_vast_en_botst_met_niets() {
        // Een peer die dit type kent leert er een bronadres van; het mag dus nooit een codec
        // worden, en het getal staat vast tussen versies.
        assert_eq!(PayloadType::KEEPALIVE.0, 0xff);
        for codec in [PayloadType::OPUS, PayloadType::HEVC, PayloadType::H264] {
            assert_ne!(codec, PayloadType::KEEPALIVE);
        }
        assert_ne!(KEEPALIVE_STREAM_ID, VOICE_STREAM_ID);
    }

    #[test]
    fn header_is_exact_zestien_bytes() {
        // De layout ligt vast in docs/ARCHITECTURE.md en tussen versies. Wijzigen
        // hiervan breekt elke draaiende peer zonder waarschuwing.
        assert_eq!(MEDIA_HEADER_LEN, 16);
    }

    #[test]
    fn te_kort_pakket_geeft_none_geen_panic() {
        assert!(MediaHeader::parse(&[0u8; 15]).is_none());
        assert!(MediaHeader::parse(&[]).is_none());
    }

    #[test]
    fn payload_volgt_direct_op_header() {
        let h = MediaHeader {
            stream_id: 1,
            seq: 1,
            timestamp: 0,
            payload_type: PayloadType::OPUS,
            flags: 0,
            frag_index: 0,
        };
        let mut pkt = [0u8; MEDIA_HEADER_LEN].to_vec();
        h.write_to((&mut pkt[..MEDIA_HEADER_LEN]).try_into().unwrap());
        pkt.extend_from_slice(b"audio");
        let (_, payload) = MediaHeader::parse(&pkt).unwrap();
        assert_eq!(payload, b"audio");
    }
}
