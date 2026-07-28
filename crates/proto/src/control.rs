//! Control-berichten. Gaan betrouwbaar over QUIC.
//!
//! # Compatibiliteitsregels
//!
//! - Elke variant heeft een **expliciete numerieke tag**. Hergebruik nooit een tag,
//!   ook niet van een verwijderd bericht.
//! - Een onbekende tag levert `Ok(None)` op, geen fout. Een oudere peer moet een
//!   nieuwer bericht kunnen negeren zonder de verbinding te verbreken.
//! - Payloads zijn als map (veldnamen) gecodeerd, niet als array. Daardoor mag je
//!   velden toevoegen en herordenen; nieuwe velden krijgen `#[serde(default)]`.
//!   Control-verkeer is klein en zeldzaam, dus de extra bytes zijn irrelevant.
//! - Elk bericht heeft een eigen struct, ook als die leeg is, zodat er later velden
//!   bij kunnen zonder de vorm van het bericht te veranderen.

use crate::op::{Op, VersionVector};
use crate::PeerId;
use serde::{Deserialize, Serialize};

/// Soort gedeelde bron. Als `u8` op de draad zodat een onbekende soort van een nieuwere
/// peer geen decodeerfout geeft maar gewoon als "onbekend" doorkomt.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StreamKind(pub u8);

impl StreamKind {
    pub const MONITOR: Self = Self(1);
    pub const WINDOW: Self = Self(2);
    pub const DESKTOP_AUDIO: Self = Self(3);

    pub fn is_known(self) -> bool {
        matches!(self, Self::MONITOR | Self::WINDOW | Self::DESKTOP_AUDIO)
    }
}

impl std::fmt::Debug for StreamKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::MONITOR => write!(f, "Monitor"),
            Self::WINDOW => write!(f, "Window"),
            Self::DESKTOP_AUDIO => write!(f, "DesktopAudio"),
            Self(n) => write!(f, "Unknown({n})"),
        }
    }
}

macro_rules! control_messages {
    ($( $tag:literal => $variant:ident($payload:ty) ),* $(,)?) => {
        /// Alle control-berichten. Voeg varianten **alleen aan het eind** toe.
        #[derive(Debug, Clone, PartialEq)]
        pub enum ControlMsg {
            $( $variant($payload), )*
        }

        impl ControlMsg {
            pub fn tag(&self) -> u16 {
                match self { $( Self::$variant(_) => $tag, )* }
            }

            /// `[u16 tag big-endian][msgpack payload]`. De lengteprefix zit in de
            /// netwerklaag, niet hier — deze crate doet geen I/O.
            pub fn encode(&self) -> crate::Result<Vec<u8>> {
                let mut out = Vec::with_capacity(64);
                out.extend_from_slice(&self.tag().to_be_bytes());
                match self {
                    $( Self::$variant(p) => {
                        let mut ser = rmp_serde::Serializer::new(&mut out).with_struct_map();
                        serde::Serialize::serialize(p, &mut ser)?;
                    } )*
                }
                Ok(out)
            }

            /// `Ok(None)` betekent: onbekende tag, van een nieuwere peer. Loggen en
            /// negeren, niet als fout behandelen.
            pub fn decode(frame: &[u8]) -> crate::Result<Option<Self>> {
                if frame.len() < 2 {
                    return Err(crate::ProtoError::FrameTruncated);
                }
                let tag = u16::from_be_bytes([frame[0], frame[1]]);
                let body = &frame[2..];
                Ok(match tag {
                    $( $tag => Some(Self::$variant(rmp_serde::from_slice(body)?)), )*
                    _ => None,
                })
            }
        }
    };
}

control_messages! {
    // 1-9: handshake en liveness
    1  => Hello(Hello),
    2  => HelloAck(HelloAck),
    3  => Ping(Ping),
    4  => Pong(Pong),

    // 10-19: oplog-synchronisatie (chat en alle latere gedeelde state)
    10 => SyncRequest(SyncRequest),
    11 => SyncResponse(SyncResponse),
    12 => OpBroadcast(OpBroadcast),

    // 20-29: voice
    20 => VoiceJoin(VoiceJoin),
    21 => VoiceLeave(VoiceLeave),
    22 => VoiceState(VoiceState),

    // 30-39: screenshare
    30 => StreamAnnounce(StreamAnnounce),
    31 => StreamRevoke(StreamRevoke),
    32 => StreamSubscribe(StreamSubscribe),
    33 => StreamUnsubscribe(StreamUnsubscribe),
    34 => StreamStats(StreamStats),
    35 => RequestKeyframe(RequestKeyframe),

    // 40-49: GERESERVEERD voor file transfer. Zie TODO.md. Niet gebruiken voor iets anders.
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hello {
    pub protocol_version: u32,
    pub peer_id: PeerId,
    pub display_name: String,
    /// UDP-poort waarop deze peer media verwacht.
    pub media_port: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HelloAck {
    pub protocol_version: u32,
    pub peer_id: PeerId,
    pub display_name: String,
    pub media_port: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ping {
    pub nonce: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pong {
    pub nonce: u64,
}

/// Beide kanten sturen dit bij het opzetten van een verbinding. De ontvanger stuurt
/// terug wat de afzender mist. Eén ronde is genoeg voor volledige convergentie,
/// ongeacht hoe lang een peer offline was.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncRequest {
    pub have: VersionVector,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncResponse {
    pub ops: Vec<Op>,
    /// `false` betekent: er volgt nog een `SyncResponse`. Grote inhaalslagen worden
    /// opgeknipt zodat we niet tegen `MAX_FRAME_LEN` aan lopen.
    #[serde(default = "default_true")]
    pub is_last: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpBroadcast {
    pub op: Op,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoiceJoin {
    pub media_port: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoiceLeave {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoiceState {
    pub muted: bool,
    pub deafened: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamAnnounce {
    pub stream_id: u32,
    pub kind: StreamKind,
    pub title: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamRevoke {
    pub stream_id: u32,
}

/// De deler begint pas te encoderen zodra hier iemand op intekent. Dat is de reden
/// dat de app niets doet als niemand kijkt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamSubscribe {
    pub stream_id: u32,
    pub media_port: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamUnsubscribe {
    pub stream_id: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamStats {
    pub stream_id: u32,
    pub loss_pct: f32,
    pub rtt_ms: u32,
}

/// Na verlies van een keyframe: zonder dit blijft het beeld kapot tot de volgende
/// periodieke IDR.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestKeyframe {
    pub stream_id: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_behoudt_inhoud() {
        let msg = ControlMsg::Hello(Hello {
            protocol_version: crate::PROTOCOL_VERSION,
            peer_id: PeerId::new_random(),
            display_name: "Rick".into(),
            media_port: 41700,
        });
        let bytes = msg.encode().unwrap();
        assert_eq!(ControlMsg::decode(&bytes).unwrap(), Some(msg));
    }

    #[test]
    fn onbekende_tag_wordt_genegeerd_niet_gefaald() {
        // Simuleert een bericht van een nieuwere peer. Dit mag de verbinding niet slopen.
        let mut frame = 9999u16.to_be_bytes().to_vec();
        frame.extend_from_slice(&[0x80]); // lege msgpack-map
        assert_eq!(ControlMsg::decode(&frame).unwrap(), None);
    }

    #[test]
    fn tags_zijn_uniek() {
        // Vangt de kopieer-plakfout waarbij twee varianten dezelfde tag krijgen.
        let all = [
            ControlMsg::Ping(Ping { nonce: 0 }),
            ControlMsg::Pong(Pong { nonce: 0 }),
            ControlMsg::VoiceLeave(VoiceLeave {}),
            ControlMsg::StreamRevoke(StreamRevoke { stream_id: 0 }),
            ControlMsg::StreamUnsubscribe(StreamUnsubscribe { stream_id: 0 }),
            ControlMsg::RequestKeyframe(RequestKeyframe { stream_id: 0 }),
        ];
        let mut tags: Vec<u16> = all.iter().map(|m| m.tag()).collect();
        tags.sort_unstable();
        let len = tags.len();
        tags.dedup();
        assert_eq!(tags.len(), len, "dubbele control-tag gevonden");
    }

    #[test]
    fn nieuw_veld_met_default_breekt_oude_payload_niet() {
        // Een oude peer stuurt SyncResponse zonder `is_last`; wij moeten dat aankunnen.
        #[derive(Serialize)]
        struct OudeSyncResponse {
            ops: Vec<Op>,
        }
        let mut body = 11u16.to_be_bytes().to_vec();
        let mut ser = rmp_serde::Serializer::new(&mut body).with_struct_map();
        serde::Serialize::serialize(&OudeSyncResponse { ops: vec![] }, &mut ser).unwrap();

        let decoded = ControlMsg::decode(&body).unwrap().unwrap();
        match decoded {
            ControlMsg::SyncResponse(r) => assert!(r.is_last),
            other => panic!("verkeerde variant: {other:?}"),
        }
    }

    #[test]
    fn onbekende_streamkind_decodeert_zonder_fout() {
        let msg = ControlMsg::StreamAnnounce(StreamAnnounce {
            stream_id: 1,
            kind: StreamKind(200), // soort die wij nog niet kennen
            title: "iets nieuws".into(),
            width: 1920,
            height: 1080,
        });
        let bytes = msg.encode().unwrap();
        let back = ControlMsg::decode(&bytes).unwrap().unwrap();
        match back {
            ControlMsg::StreamAnnounce(a) => assert!(!a.kind.is_known()),
            other => panic!("verkeerde variant: {other:?}"),
        }
    }
}
