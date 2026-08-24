//! Wire-protocol voor FitCommunication.
//!
//! Deze crate is puur: geen I/O, geen Windows-API's, geen hardware. Dat is opzet —
//! hier zit de subtiele logica en die moet volledig unit-testbaar blijven.
//! Zie `docs/ARCHITECTURE.md` voor het ontwerp en de compatibiliteitsregels.

pub mod appversion;
pub mod control;
pub mod ids;
pub mod media;
pub mod op;

pub use appversion::is_newer;
pub use control::{ControlMsg, StreamKind, Typing, UserStatus, UserStatusValue};
pub use ids::{Channel, OpId, PeerId, TopicId};
pub use media::{
    MediaHeader, PayloadType, MAX_MEDIA_PAYLOAD, MEDIA_HEADER_LEN, PARITEIT_PAYLOAD_LEN,
    VOICE_STREAM_ID,
};
pub use op::{Op, OpKind, VersionVector};

/// Verhoog dit alleen bij een breuk die niet met `#[serde(default)]` of het negeren
/// van een onbekende tag op te vangen is. Peers draaien handmatig gekopieerde binaries,
/// dus een bump dwingt iedereen tot updaten voordat er weer iets werkt.
///
/// 1 → 2 bij de kanalen-uitbreiding (DM's): `VersionVector` en de bestandsoverdracht-
/// header (`crates/net/src/filestream.rs`) kregen er een `Channel` bij op een manier die
/// een oudere peer niet stilzwijgend kan negeren — een peer zonder kanaalbegrip kan een
/// per-(auteur, kanaal) version vector fundamenteel niet correct interpreteren, dus
/// stilletjes doorsyncen zou erger zijn dan een nette `VersionMismatch`. Zie
/// `docs/ARCHITECTURE.md`, sectie "Kanalen".
///
/// 2 → 3 bij subkanalen onder het algemene kanaal (fase 9): `Channel` kreeg een derde
/// `tag` (2, naast 0 = algemeen en 1 = DM). Het wire-decoderen van die extra tag is op
/// zichzelf onschadelijk (`Channel` is een map, geen tuple), maar `channel_to_blob` in
/// `crates/store/src/lib.rs` en `encode_channel` in `crates/net/src/filestream.rs` kenden
/// tot deze bump alleen tag 0 en 1: een peer zonder subkanaalbegrip zou een onbekende tag
/// stilzwijgend op dezelfde opslagsleutel als het algemene kanaal aliasen — een botsing op
/// dezelfde primary key (`author`, kanaal-blob, `seq`) met een échte algemene op van
/// diezelfde auteur, met permanent dataverlies of een verkeerd geadresseerde
/// bestandsoverdracht tot gevolg. Gevonden door de `protocol-reviewer`-agent vóór het
/// committen, niet door een test. Zie `docs/ARCHITECTURE.md`, sectie "Kanalen".
///
/// 3 → 4 bij automatische updates (fase 11): elke uni-stream voor een bulkoverdracht
/// (`crates/net/src/filestream.rs`) kreeg een 1-byte kind-prefix vóór de bestaande body,
/// om een bestandsoverdracht van een update-overdracht te kunnen onderscheiden — een
/// oudere peer zou dat eerste byte anders verkeerd als onderdeel van de oude
/// `OpId`-header lezen. `Hello`/`HelloAck.app_version` en `UpdateRequest`/`UpdateResponse`
/// zijn op zichzelf onschadelijk (map-encoded, nieuw bericht wordt genegeerd), maar horen
/// bij dezelfde bump omdat een oudere peer een `UpdateRequest` toch nooit kan versturen.
/// Zie `docs/ARCHITECTURE.md`, sectie "Automatische updates".
///
/// 4 → 5 bij het pariteitsfragment voor video (`MediaHeader::FLAG_PARITEIT`): achter elk
/// videobeeld gaat nu een extra fragment aan met de XOR van alle stukken, zodat één
/// verloren fragment ter plekke te herstellen is. Een oudere ontvanger kent de vlag niet,
/// stopt dat fragment als gewoon stuk in zijn samensteller en komt dan op één stuk *te
/// veel* uit — waarna `Reassembler::compleet()` nooit meer waar wordt en er van die deler
/// helemaal geen beeld meer verschijnt. Dat is precies de klasse fout die de eerdere bumps
/// ook moesten voorkomen: stilzwijgend kapot in plaats van een nette `VersionMismatch`.
/// Zie `docs/ARCHITECTURE.md`, sectie "Media (UDP, onbetrouwbaar)".
pub const PROTOCOL_VERSION: u32 = 5;

/// Bovengrens voor een control-frame. Grote sync-antwoorden worden opgeknipt door de
/// netwerklaag, zodat we hier bij normaal gebruik nooit tegenaan lopen.
pub const MAX_FRAME_LEN: usize = 16 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ProtoError {
    #[error("frame van {0} bytes overschrijdt de limiet van {MAX_FRAME_LEN}")]
    FrameTooLarge(usize),
    #[error("frame is te kort")]
    FrameTruncated,
    #[error("msgpack encode mislukt: {0}")]
    Encode(#[from] rmp_serde::encode::Error),
    #[error("msgpack decode mislukt: {0}")]
    Decode(#[from] rmp_serde::decode::Error),
    /// B-43: geen enkele string op de draad had een lengtegrens, en dat is het grondstofje
    /// voor B-15 (één te grote op sloopt de verbinding) en B-16 (onbegrensde oplog-groei).
    #[error("{veld} is {len} bytes; limiet is {limiet}")]
    VeldTeLang {
        veld: &'static str,
        len: usize,
        limiet: usize,
    },
    /// B-14 en B-34: SQLite kent geen `u64`. Een waarde boven `i64::MAX` wordt als negatief
    /// getal opgeslagen, en dan zijn de opslag en de vergelijking in Rust het niet meer
    /// eens over wat er staat. Weigeren bij het decoderen is de enige plek waar dat nog
    /// goedkoop is.
    #[error("{veld} = {waarde} past niet in een i64 en is dus niet op te slaan")]
    GetalTeGroot { veld: &'static str, waarde: u64 },
}

pub type Result<T> = std::result::Result<T, ProtoError>;
