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
use crate::{Channel, OpId, PeerId};
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
    /// Een webcam. Op de draad niet te onderscheiden van een scherm — dezelfde H.264 in
    /// dezelfde fragmenten — maar de app moet het verschil wél weten: bureaubladgeluid
    /// hangt aan een gedeeld scherm en niet aan een camera. Toegevoegd zonder
    /// `protocol_version`-bump: een oudere peer ziet `is_known() == false`, negeert de
    /// aankondiging en tekent er niet op in. Dat is precies het gedrag dat bij een
    /// onbekende soort hoort.
    pub const CAMERA: Self = Self(4);

    pub fn is_known(self) -> bool {
        matches!(
            self,
            Self::MONITOR | Self::WINDOW | Self::DESKTOP_AUDIO | Self::CAMERA
        )
    }

    /// Of dit iets is dat je in een venster bekijkt. Geluid heeft geen venster, en een
    /// onbekende soort van een nieuwere peer krijgt er zeker geen.
    pub fn is_beeld(self) -> bool {
        matches!(self, Self::MONITOR | Self::WINDOW | Self::CAMERA)
    }

    /// Of dit een gedeeld scherm of venster is — de twee soorten waar bureaubladgeluid
    /// bij hoort. Een camera hoort er níét bij: die deelt geen systeemgeluid.
    pub fn is_scherm(self) -> bool {
        matches!(self, Self::MONITOR | Self::WINDOW)
    }
}

impl std::fmt::Debug for StreamKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::MONITOR => write!(f, "Monitor"),
            Self::WINDOW => write!(f, "Window"),
            Self::DESKTOP_AUDIO => write!(f, "DesktopAudio"),
            Self::CAMERA => write!(f, "Camera"),
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

    // 40-49: file transfer. De aanbieding zelf is een gewone oplog-op (`OpKind::FileMeta`)
    // en verspreidt zich dus al gratis mee via de sync; deze twee gaan alleen over het
    // daadwerkelijk ophalen van de bytes bij de aanbieder. Zie docs/ARCHITECTURE.md.
    40 => FileRequest(FileRequest),
    41 => FileResponse(FileResponse),

    // fase 11: automatische updates. Geen `version`/`file`-veld nodig — de identiteit is
    // impliciet ("jouw huidige, draaiende exe"). Zie docs/ARCHITECTURE.md.
    42 => UpdateRequest(UpdateRequest),
    43 => UpdateResponse(UpdateResponse),

    // 44-45: vluchtige chat-status (2026-08-22). Allebei bewust géén oplog-op: een
    // typindicatie of een status is geen geschiedenis — hij is na een paar seconden of
    // na een herstart zijn betekenis kwijt en zou de log alleen maar laten groeien.
    // Daarmee delen ze het lot van de voice-meldingen (20-22): single-hop, geen
    // doorgifte door een derde peer, en weg zodra de verbinding wegvalt.
    //
    // Een oudere peer decodeert een onbekende tag als `Ok(None)` en slaat hem stil over;
    // niets hieronder kan een verbinding breken, dus geen protocolbump.
    44 => Typing(Typing),
    45 => UserStatus(UserStatus),
}

/// "Ik ben tekst aan het typen in dit kanaal." Vluchtig: de ontvanger toont hem hooguit
/// een paar seconden en vergeet hem daarna vanzelf; er komt geen bevestiging terug.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Typing {
    pub channel: Channel,
}

/// Een door de gebruiker gekozen aanwezigheidsstatus, als `u8` op de draad zodat een
/// onbekende waarde van een nieuwere peer gewoon als "onbekend" doorkomt in plaats van
/// het hele bericht te laten vallen — zelfde reden als bij `StreamKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UserStatusValue(pub u8);

impl UserStatusValue {
    /// Gewoon aanwezig. Dit is ook wat een peer die nog nooit een `UserStatus` stuurde
    /// impliciet is.
    pub const ONLINE: Self = Self(0);
    /// Afwezig, maar bereikbaar.
    pub const AWAY: Self = Self(1);
    /// Bezet; niet storen tenzij het belangrijk is.
    pub const BUSY: Self = Self(2);

    pub fn is_known(self) -> bool {
        matches!(self, Self::ONLINE | Self::AWAY | Self::BUSY)
    }
}

/// Online is de nulwaarde: een peer die nog niets stuurde is er impliciet.
impl Default for UserStatusValue {
    fn default() -> Self {
        Self::ONLINE
    }
}

impl std::fmt::Display for UserStatusValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::ONLINE => write!(f, "online"),
            Self::AWAY => write!(f, "away"),
            Self::BUSY => write!(f, "busy"),
            Self(n) => write!(f, "unknown({n})"),
        }
    }
}

/// "Mijn gekozen status is nu deze." Wordt verstuurd bij elke wisseling én direct nadat
/// een verbinding opkomt — de ontvanger onthoudt hem per peer voor deze sessie en begint
/// na een herstart weer op "online", net zo vluchtig als het bericht zelf.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserStatus {
    pub status: UserStatusValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hello {
    pub protocol_version: u32,
    pub peer_id: PeerId,
    pub display_name: String,
    /// UDP-poort waarop deze peer media verwacht.
    pub media_port: u16,
    /// `env!("CARGO_PKG_VERSION")` van deze build. Voor fase 11 (automatische updates):
    /// hiermee ziet een peer dat een ander een nieuwere build draait. Een oudere peer die
    /// dit veld nog niet kent, decodeert gewoon zonder — `#[serde(default)]` levert dan
    /// `"0.0.0"`, wat nooit een update triggert.
    #[serde(default = "onbekende_app_versie")]
    pub app_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HelloAck {
    pub protocol_version: u32,
    pub peer_id: PeerId,
    pub display_name: String,
    pub media_port: u16,
    #[serde(default = "onbekende_app_versie")]
    pub app_version: String,
}

fn onbekende_app_versie() -> String {
    "0.0.0".to_string()
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

/// "Ik wil dit bestand." `file` is de `OpId` van de `FileMeta`-op — die is al globaal
/// uniek, dus een apart transfer-id erbovenop zou alleen maar een tweede naam voor
/// hetzelfde ding zijn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileRequest {
    pub file: OpId,
    /// Bytes die de aanvrager al op schijf heeft staan van een eerdere, onderbroken
    /// poging. `0` voor een verse download. Zo hervat een download vanaf waar hij
    /// stopte in plaats van opnieuw te beginnen.
    #[serde(default)]
    pub have_bytes: u64,
}

/// Als `u8` op de draad, net als `StreamKind`: zo geeft een onbekende uitkomst van een
/// nieuwere peer geen decodeerfout die de hele `FileResponse` (en daarmee het frame)
/// laat verdwijnen, maar komt gewoon als "onbekend" door.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FileOutcome(pub u8);

impl FileOutcome {
    /// Er volgt een eigen (uni-)stream op dezelfde QUIC-verbinding met de bytes vanaf
    /// `have_bytes`. Niet over de control-stream: dat zou chat en de rest van het
    /// verkeer laten wachten op een bulkoverdracht.
    pub const READY: Self = Self(1);
    /// De aanbieder heeft dit bestand niet meer (bijvoorbeeld verplaatst of verwijderd
    /// van schijf). Geen fout, gewoon een nette afwijzing.
    pub const NOT_AVAILABLE: Self = Self(2);

    pub fn is_known(self) -> bool {
        matches!(self, Self::READY | Self::NOT_AVAILABLE)
    }
}

impl std::fmt::Debug for FileOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::READY => write!(f, "Ready"),
            Self::NOT_AVAILABLE => write!(f, "NotAvailable"),
            Self(n) => write!(f, "Unknown({n})"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileResponse {
    pub file: OpId,
    pub outcome: FileOutcome,
}

/// "Jij draait een nieuwere versie dan ik; stuur me je exe." Verstuurd door de peer met
/// de oudere versie, zodra hij via `Hello`/`HelloAck.app_version` ziet dat een ander
/// verder is. `have_bytes` werkt net als bij `FileRequest`: wat er al op schijf staat van
/// een eerdere, onderbroken poging aan precies deze versie.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateRequest {
    #[serde(default)]
    pub have_bytes: u64,
}

/// Antwoord van de peer met de nieuwere versie. Anders dan `FileResponse` draagt dit ook
/// grootte en hash mee: bij een update is er geen voorafgaande `FileMeta`-op die dat al
/// vastlegt, dus de aanvrager moet het hier vernemen om te kunnen verifiëren.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateResponse {
    pub outcome: FileOutcome,
    pub size: u64,
    pub hash: [u8; 32],
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Channel;

    #[test]
    fn roundtrip_behoudt_inhoud() {
        let msg = ControlMsg::Hello(Hello {
            protocol_version: crate::PROTOCOL_VERSION,
            peer_id: PeerId::new_random(),
            display_name: "Rick".into(),
            media_port: 41700,
            app_version: "0.1.0".into(),
        });
        let bytes = msg.encode().unwrap();
        assert_eq!(ControlMsg::decode(&bytes).unwrap(), Some(msg));
    }

    #[test]
    fn hello_zonder_app_versie_valt_terug_op_onbekend() {
        // Een oudere peer die het veld nog niet kent — mag nooit een update triggeren.
        #[derive(Serialize)]
        struct OudeHello {
            protocol_version: u32,
            peer_id: PeerId,
            display_name: String,
            media_port: u16,
        }
        let mut body = 1u16.to_be_bytes().to_vec();
        let mut ser = rmp_serde::Serializer::new(&mut body).with_struct_map();
        serde::Serialize::serialize(
            &OudeHello {
                protocol_version: crate::PROTOCOL_VERSION,
                peer_id: PeerId::new_random(),
                display_name: "Rick".into(),
                media_port: 41700,
            },
            &mut ser,
        )
        .unwrap();

        match ControlMsg::decode(&body).unwrap().unwrap() {
            ControlMsg::Hello(h) => assert_eq!(h.app_version, "0.0.0"),
            other => panic!("verkeerde variant: {other:?}"),
        }
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
            ControlMsg::FileRequest(FileRequest {
                file: OpId::new(PeerId::new_random(), Channel::GENERAL, 1),
                have_bytes: 0,
            }),
            ControlMsg::FileResponse(FileResponse {
                file: OpId::new(PeerId::new_random(), Channel::GENERAL, 1),
                outcome: FileOutcome::READY,
            }),
            ControlMsg::UpdateRequest(UpdateRequest { have_bytes: 0 }),
            ControlMsg::UpdateResponse(UpdateResponse {
                outcome: FileOutcome::READY,
                size: 0,
                hash: [0u8; 32],
            }),
            ControlMsg::Typing(Typing {
                channel: Channel::GENERAL,
            }),
            ControlMsg::UserStatus(UserStatus {
                status: UserStatusValue::BUSY,
            }),
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
    fn file_request_zonder_have_bytes_valt_terug_op_nul() {
        // Een verse aanvraag, of een oudere peer die het veld nog niet kende.
        #[derive(Serialize)]
        struct OudeFileRequest {
            file: OpId,
        }
        let mut body = 40u16.to_be_bytes().to_vec();
        let mut ser = rmp_serde::Serializer::new(&mut body).with_struct_map();
        serde::Serialize::serialize(
            &OudeFileRequest {
                file: OpId::new(PeerId::new_random(), Channel::GENERAL, 3),
            },
            &mut ser,
        )
        .unwrap();

        match ControlMsg::decode(&body).unwrap().unwrap() {
            ControlMsg::FileRequest(r) => assert_eq!(r.have_bytes, 0),
            other => panic!("verkeerde variant: {other:?}"),
        }
    }

    #[test]
    fn file_response_roundtrip_behoudt_de_uitkomst() {
        let file = OpId::new(PeerId::new_random(), Channel::GENERAL, 7);
        for outcome in [FileOutcome::READY, FileOutcome::NOT_AVAILABLE] {
            let msg = ControlMsg::FileResponse(FileResponse { file, outcome });
            let bytes = msg.encode().unwrap();
            assert_eq!(ControlMsg::decode(&bytes).unwrap(), Some(msg));
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

    #[test]
    fn update_request_response_roundtrip() {
        let req = ControlMsg::UpdateRequest(UpdateRequest { have_bytes: 12345 });
        let bytes = req.encode().unwrap();
        assert_eq!(ControlMsg::decode(&bytes).unwrap(), Some(req));

        let resp = ControlMsg::UpdateResponse(UpdateResponse {
            outcome: FileOutcome::READY,
            size: 999,
            hash: [7u8; 32],
        });
        let bytes = resp.encode().unwrap();
        assert_eq!(ControlMsg::decode(&bytes).unwrap(), Some(resp));
    }

    #[test]
    fn update_request_zonder_have_bytes_valt_terug_op_nul() {
        #[derive(Serialize)]
        struct OudeUpdateRequest {}
        let mut body = 42u16.to_be_bytes().to_vec();
        let mut ser = rmp_serde::Serializer::new(&mut body).with_struct_map();
        serde::Serialize::serialize(&OudeUpdateRequest {}, &mut ser).unwrap();

        match ControlMsg::decode(&body).unwrap().unwrap() {
            ControlMsg::UpdateRequest(r) => assert_eq!(r.have_bytes, 0),
            other => panic!("verkeerde variant: {other:?}"),
        }
    }

    #[test]
    fn onbekende_file_outcome_decodeert_zonder_fout() {
        // Zelfde reden als bij StreamKind: een toekomstige derde uitkomst (bijvoorbeeld
        // "Denied" voor een quotafunctie) mag de hele FileResponse niet laten mislukken.
        let msg = ControlMsg::FileResponse(FileResponse {
            file: OpId::new(PeerId::new_random(), Channel::GENERAL, 1),
            outcome: FileOutcome(200),
        });
        let bytes = msg.encode().unwrap();
        let back = ControlMsg::decode(&bytes).unwrap().unwrap();
        match back {
            ControlMsg::FileResponse(r) => assert!(!r.outcome.is_known()),
            other => panic!("verkeerde variant: {other:?}"),
        }
    }

    #[test]
    fn typing_en_status_roundtrip() {
        let typing = ControlMsg::Typing(Typing {
            channel: Channel::dm(PeerId::new_random()),
        });
        let bytes = typing.encode().unwrap();
        assert_eq!(ControlMsg::decode(&bytes).unwrap(), Some(typing));

        for status in [
            UserStatusValue::ONLINE,
            UserStatusValue::AWAY,
            UserStatusValue::BUSY,
            UserStatusValue(200), // van een nieuwere peer; komt als onbekend door
        ] {
            let msg = ControlMsg::UserStatus(UserStatus { status });
            let bytes = msg.encode().unwrap();
            match ControlMsg::decode(&bytes).unwrap().unwrap() {
                ControlMsg::UserStatus(s) => {
                    assert_eq!(s.status, status);
                    assert_eq!(s.status.is_known(), status.is_known());
                }
                other => panic!("verkeerde variant: {other:?}"),
            }
        }
    }
}
