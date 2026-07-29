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
use fitcom_proto::{ControlMsg, Op, OpId, OpKind, PeerId, VersionVector};
use fitcom_store::{Store, Timeline, SYNC_BATCH};
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
    /// Aantal nieuwe berichten van anderen sinds de laatste keer dat de gebruiker keek.
    pub ongelezen: usize,
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
        })
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

    pub fn plaats_bericht(&mut self, tekst: &str) -> Result<Vec<MeshCommand>> {
        self.eigen_op(fitcom_store::post(tekst.trim()))
    }

    pub fn bewerk_bericht(&mut self, doel: OpId, tekst: &str) -> Result<Vec<MeshCommand>> {
        self.eigen_op(OpKind::Edit {
            target: doel,
            body: tekst.trim().to_string(),
        })
    }

    pub fn verwijder_bericht(&mut self, doel: OpId) -> Result<Vec<MeshCommand>> {
        self.eigen_op(OpKind::Delete { target: doel })
    }

    /// Legt de eigen weergavenaam vast in de oplog, zodat de anderen hem ook zien.
    /// Doet niets als de naam al klopt — anders groeit de log bij elke start.
    pub fn zet_naam(&mut self, naam: &str) -> Result<Vec<MeshCommand>> {
        if self.timeline.nicknames.get(&self.me()).map(String::as_str) == Some(naam) {
            return Ok(Vec::new());
        }
        self.eigen_op(OpKind::SetNick {
            name: naam.to_string(),
        })
    }

    fn eigen_op(&mut self, kind: OpKind) -> Result<Vec<MeshCommand>> {
        let op = self.store.append_local(&kind, fitcom_store::now_millis())?;
        self.vuil = true;
        Ok(vec![MeshCommand::Broadcast(ControlMsg::OpBroadcast(
            OpBroadcast { op },
        ))])
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

    fn sync_verzoek(&self, peer: PeerId) -> Result<MeshCommand> {
        Ok(MeshCommand::Send {
            to: peer,
            msg: ControlMsg::SyncRequest(SyncRequest {
                have: self.store.version_vector()?,
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

    /// Stuurt in stukken alles wat de ander mist.
    fn beantwoord_sync(&self, naar: PeerId, mut hun: VersionVector) -> Result<Vec<MeshCommand>> {
        let mut uit = Vec::new();

        loop {
            let batch = self.store.ops_missing_in(&hun, SYNC_BATCH)?;
            if batch.is_empty() {
                break;
            }
            // Bijhouden wat we al gestuurd hebben, zodat de volgende ronde verder gaat.
            for op in &batch {
                hun.observe(op.author, op.seq);
            }
            let is_last = !self.store.has_more_for(&hun)?;
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

    /// Slaat ontvangen ops op en stuurt door wat nieuw was.
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
        self.ongelezen += nieuw
            .iter()
            .filter(|o| matches!(o.kind(), Ok(Some(OpKind::Post { .. }))))
            .count();

        tracing::debug!(peer = ?van, aantal = nieuw.len(), "nieuwe ops opgeslagen");

        // Doorsturen naar de rest. Broadcast gaat ook terug naar `van`, wat overbodig
        // maar onschadelijk is: die herkent zijn eigen ops als bekend en stopt daar.
        Ok(nieuw
            .into_iter()
            .map(|op| MeshCommand::Broadcast(ControlMsg::OpBroadcast(OpBroadcast { op })))
            .collect())
    }
}
