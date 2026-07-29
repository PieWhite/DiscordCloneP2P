//! Koppelt de oplog aan de mesh.
//!
//! Deze module neemt geen enkele beslissing over ordening of conflicten — dat zit
//! allemaal in `fitcom-store`. Hier staat alleen wanneer we wat naar wie sturen.
//!
//! # Drie manieren waarop een bericht zijn weg vindt
//!
//! 1. **Broadcast** bij het plaatsen. Dekt het normale geval waarin iedereen online is.
//! 2. **Inhaalslag bij (her)verbinding.** Dekt de peer die weg was.
//! 3. **Doorsturen en periodiek hersynchroniseren.** Dekt het geval waarin A en C
//!    elkaar niet kunnen bereiken maar B beiden wel. Zonder dit zou C een bericht van A
//!    pas zien bij zijn volgende herverbinding met B, wat uren kan duren.

use anyhow::Result;
use fitcom_net::MeshCommand;
use fitcom_proto::control::{OpBroadcast, SyncRequest, SyncResponse};
use fitcom_proto::{Channel, ControlMsg, Op, OpId, OpKind, PeerId, VersionVector};
use fitcom_store::{Store, Timeline, SYNC_BATCH};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Hoe vaak we ongevraagd onze version vector langs de verbonden peers sturen.
/// Een version vector voor drie peers is enkele tientallen bytes; dit kost niets en
/// herstelt elke toestand die de broadcast en het doorsturen gemist zouden hebben.
const HERSYNC_INTERVAL: Duration = Duration::from_secs(30);

pub struct Chat {
    store: Store,
    /// Als `Arc` zodat de motor hem elke honderd milliseconde kan publiceren zonder
    /// de hele geschiedenis te kopiëren.
    timeline: Arc<Timeline>,
    laatste_hersync: Instant,
    /// Wordt gezet zodra de timeline niet meer klopt met de oplog.
    vuil: bool,
    /// Aantal nieuwe berichten van anderen sinds de laatste keer dat de gebruiker keek,
    /// op het algemene kanaal.
    pub ongelezen: usize,
    /// Zelfde, maar per DM-gesprek — sleutel is de gesprekspartner. Los van `ongelezen`
    /// omdat je een DM kunt missen terwijl je wel naar het algemene kanaal kijkt, en
    /// andersom.
    ongelezen_dm: HashMap<PeerId, usize>,
}

impl Chat {
    pub fn new(store: Store) -> Result<Self> {
        let timeline = Arc::new(store.timeline()?);
        Ok(Self {
            store,
            timeline,
            laatste_hersync: Instant::now(),
            vuil: false,
            ongelezen: 0,
            ongelezen_dm: HashMap::new(),
        })
    }

    pub fn ongelezen_dm(&self) -> &HashMap<PeerId, usize> {
        &self.ongelezen_dm
    }

    pub fn markeer_dm_gelezen(&mut self, met: PeerId) {
        self.ongelezen_dm.remove(&met);
    }

    pub fn me(&self) -> PeerId {
        self.store.me()
    }

    pub fn timeline(&self) -> &Timeline {
        &self.timeline
    }

    pub fn timeline_arc(&self) -> Arc<Timeline> {
        self.timeline.clone()
    }

    /// Bouwt de timeline opnieuw als er iets veranderd is. Levert `true` bij een
    /// daadwerkelijke wijziging, zodat de UI weet dat hij naar beneden moet scrollen.
    pub fn refresh(&mut self) -> bool {
        if !self.vuil {
            return false;
        }
        self.vuil = false;
        match self.store.timeline() {
            Ok(t) => {
                self.timeline = Arc::new(t);
                true
            }
            Err(e) => {
                tracing::error!(error = %format!("{e:#}"), "timeline opbouwen mislukt");
                false
            }
        }
    }

    pub fn markeer_gelezen(&mut self) {
        self.ongelezen = 0;
    }

    // -- uitgaand ----------------------------------------------------------

    pub fn plaats_bericht(&mut self, tekst: &str, channel: Channel) -> Result<Vec<MeshCommand>> {
        self.eigen_op(channel, fitcom_store::post(tekst.trim()))
    }

    /// Kanaal komt van `doel.channel`: een bewerking hoort per definitie in hetzelfde
    /// kanaal als het bericht dat hij bewerkt, anders zou de store hem straks negeren
    /// (zie `fitcom_store::timeline`) en in het DM-geval zou het origineel ook nog eens
    /// verkeerd geadresseerd worden.
    pub fn bewerk_bericht(&mut self, doel: OpId, tekst: &str) -> Result<Vec<MeshCommand>> {
        self.eigen_op(
            doel.channel,
            OpKind::Edit {
                target: doel,
                body: tekst.trim().to_string(),
            },
        )
    }

    pub fn verwijder_bericht(&mut self, doel: OpId) -> Result<Vec<MeshCommand>> {
        self.eigen_op(doel.channel, OpKind::Delete { target: doel })
    }

    /// Legt de eigen weergavenaam vast in de oplog, zodat de anderen hem ook zien.
    /// Doet niets als de naam al klopt — anders groeit de log bij elke start. Altijd op
    /// het algemene kanaal: een weergavenaam is identiteit, geen gespreksinhoud.
    pub fn zet_naam(&mut self, naam: &str) -> Result<Vec<MeshCommand>> {
        if self.timeline.nicknames.get(&self.me()).map(String::as_str) == Some(naam) {
            return Ok(Vec::new());
        }
        self.eigen_op(
            Channel::GENERAL,
            OpKind::SetNick {
                name: naam.to_string(),
            },
        )
    }

    /// Legt een bestandsaanbod vast in de oplog, precies zoals een bericht — dat is het
    /// hele punt van de generieke oplog. Levert de `OpId` op die de overdracht zelf
    /// identificeert, zodat de motor kan onthouden waar het originele bestand staat.
    pub fn deel_bestand(
        &mut self,
        naam: &str,
        grootte: u64,
        hash: [u8; 32],
        channel: Channel,
    ) -> Result<(OpId, Vec<MeshCommand>)> {
        let kind = fitcom_store::offer_file(naam, grootte, hash);
        let op = self
            .store
            .append_local(channel, &kind, fitcom_store::now_millis())?;
        self.vuil = true;
        let id = op.id();
        Ok((id, vec![stuur_op(op)]))
    }

    fn eigen_op(&mut self, channel: Channel, kind: OpKind) -> Result<Vec<MeshCommand>> {
        let op = self
            .store
            .append_local(channel, &kind, fitcom_store::now_millis())?;
        self.vuil = true;
        Ok(vec![stuur_op(op)])
    }

    // -- inkomend ----------------------------------------------------------

    /// Een peer is net verbonden: vraag om wat we missen.
    pub fn bij_verbinding(&self, peer: PeerId) -> Result<Vec<MeshCommand>> {
        Ok(vec![self.sync_verzoek(peer)?])
    }

    /// Periodieke herstelactie. Levert niets op als het nog geen tijd is.
    pub fn tick(&mut self, verbonden: &[PeerId]) -> Result<Vec<MeshCommand>> {
        if self.laatste_hersync.elapsed() < HERSYNC_INTERVAL {
            return Ok(Vec::new());
        }
        self.laatste_hersync = Instant::now();
        verbonden.iter().map(|&p| self.sync_verzoek(p)).collect()
    }

    /// `have` is beperkt tot wat `peer` ooit mag zien (algemeen + zijn eigen DM's met
    /// ons): zonder die beperking zou de vector zelf al metadata lekken over met wie we
    /// nog meer DM's hebben.
    fn sync_verzoek(&self, peer: PeerId) -> Result<MeshCommand> {
        Ok(MeshCommand::Send {
            to: peer,
            msg: ControlMsg::SyncRequest(SyncRequest {
                have: self.store.version_vector_for(peer)?,
            }),
        })
    }

    pub fn bij_bericht(&mut self, van: PeerId, msg: ControlMsg) -> Result<Vec<MeshCommand>> {
        match msg {
            ControlMsg::SyncRequest(req) => self.beantwoord_sync(van, req.have),
            ControlMsg::SyncResponse(resp) => self.neem_over(van, &resp.ops),
            ControlMsg::OpBroadcast(b) => self.neem_over(van, std::slice::from_ref(&b.op)),
            // Alles wat hier langskomt en niet van de chat is, hoort bij een andere
            // laag (voice, screenshare) en wordt daar afgehandeld.
            _ => Ok(Vec::new()),
        }
    }

    /// Stuurt in stukken alles wat de ander mist — alleen wat hij ooit mag zien.
    fn beantwoord_sync(&self, naar: PeerId, mut hun: VersionVector) -> Result<Vec<MeshCommand>> {
        let mut uit = Vec::new();

        loop {
            let batch = self.store.ops_missing_in_for(naar, &hun, SYNC_BATCH)?;
            if batch.is_empty() {
                break;
            }
            // Bijhouden wat we al gestuurd hebben, zodat de volgende ronde verder gaat.
            for op in &batch {
                hun.observe(op.author, op.channel, op.seq);
            }
            let is_last = !self.store.has_more_for_viewer(naar, &hun)?;
            uit.push(MeshCommand::Send {
                to: naar,
                msg: ControlMsg::SyncResponse(SyncResponse {
                    ops: batch,
                    is_last,
                }),
            });
            if is_last {
                break;
            }
        }

        if uit.is_empty() {
            // Ook "je bent bij" is een antwoord. Zonder dit weet de ander niet of de
            // inhaalslag klaar is of nog loopt.
            uit.push(MeshCommand::Send {
                to: naar,
                msg: ControlMsg::SyncResponse(SyncResponse {
                    ops: Vec::new(),
                    is_last: true,
                }),
            });
        }

        if uit.len() > 1 {
            tracing::info!(peer = ?naar, batches = uit.len(), "inhaalslag verstuurd");
        }
        Ok(uit)
    }

    /// Slaat ontvangen ops op en stuurt door wat nieuw was — alleen het algemene kanaal.
    fn neem_over(&mut self, van: PeerId, ops: &[Op]) -> Result<Vec<MeshCommand>> {
        if ops.is_empty() {
            return Ok(Vec::new());
        }

        // We hebben de ops nodig die écht nieuw waren; alleen die doorsturen, anders
        // blijft een broadcast eindeloos rondzingen tussen de peers.
        let mut nieuw: Vec<Op> = Vec::new();
        for op in ops {
            if self.store.apply_remote(op)? {
                nieuw.push(op.clone());
            }
        }

        if nieuw.is_empty() {
            return Ok(Vec::new());
        }

        self.vuil = true;
        for op in &nieuw {
            if !matches!(op.kind(), Ok(Some(OpKind::Post { .. }))) {
                continue;
            }
            match op.channel.dm_peer() {
                None => self.ongelezen += 1,
                Some(_) => *self.ongelezen_dm.entry(op.author).or_insert(0) += 1,
            }
        }

        tracing::debug!(peer = ?van, aantal = nieuw.len(), "nieuwe ops opgeslagen");

        // Doorsturen naar de rest — maar uitsluitend het algemene kanaal. Een DM is al
        // bij zijn enige geoorloofde ontvanger; die verder doorgeven aan een derde peer
        // zou precies de lekpreventie ondermijnen waar kanalen voor bestaan. Dit dekt
        // zowel een `OpBroadcast` als ops die net via een `SyncResponse` binnenkwamen.
        Ok(nieuw
            .into_iter()
            .filter(|op| op.channel.is_general())
            .map(|op| MeshCommand::Broadcast(ControlMsg::OpBroadcast(OpBroadcast { op })))
            .collect())
    }
}

/// Route een net aangemaakte op naar wie hem mag zien: breed voor het algemene kanaal,
/// uitsluitend naar de geadresseerde voor een DM.
fn stuur_op(op: Op) -> MeshCommand {
    match op.channel.dm_peer() {
        Some(naar) => MeshCommand::Send {
            to: naar,
            msg: ControlMsg::OpBroadcast(OpBroadcast { op }),
        },
        None => MeshCommand::Broadcast(ControlMsg::OpBroadcast(OpBroadcast { op })),
    }
}
