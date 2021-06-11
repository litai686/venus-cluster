use anyhow::{Context, Error, Result};
use crossbeam_channel::{select, Receiver, TryRecvError};
use log::{error, info, warn};

use crate::metadb::{MetaDB, MetaDocumentDB, MetaError, PrefixedMetaDB};

use event::Event;
use sector::{Sector, State};
use store::Store;

mod event;
mod sector;
mod store;

const sector_info_key: &str = "info";
const sector_meta_prefix: &str = "meta";
const sector_trace_prefix: &str = "trace";

const log_target_sealing: &str = "sealing";

macro_rules! impl_failure_error {
    ($name:ident, $ename:ident) => {
        #[derive(Debug)]
        struct $name(Error);

        impl From<Error> for $name {
            fn from(val: Error) -> Self {
                $name(val)
            }
        }

        impl From<$name> for Failure {
            fn from(val: $name) -> Self {
                Failure::$ename(val)
            }
        }
    };
}

impl_failure_error! {TemporaryError, Temporary}
impl_failure_error! {UnrecoverableError, Unrecoverable}
impl_failure_error! {PermanentError, Permanent}
impl_failure_error! {CriticalError, Critical}

enum Failure {
    Temporary(TemporaryError),
    Unrecoverable(UnrecoverableError),
    Permanent(PermanentError),
    Critical(CriticalError),
}

impl std::fmt::Debug for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Temporary(e) => f.write_str(&format!("Temporary: {:?}", e.0)),
            Failure::Unrecoverable(e) => f.write_str(&format!("Unrecoverable: {:?}", e.0)),
            Failure::Permanent(e) => f.write_str(&format!("Permanent: {:?}", e.0)),
            Failure::Critical(e) => f.write_str(&format!("Critical: {:?}", e.0)),
        }
    }
}

type HandleResult = Result<Event, Failure>;

struct Ctx<'c, DB: MetaDB> {
    sector: sector::Sector,
    trace: Vec<sector::Trace>,

    store: &'c Store<DB>,
    sector_meta: MetaDocumentDB<PrefixedMetaDB<'c, DB>>,
    trace_meta: MetaDocumentDB<PrefixedMetaDB<'c, DB>>,
}

impl<'c, DB> Ctx<'c, DB>
where
    DB: MetaDB,
{
    fn build(s: &'c Store<DB>) -> Result<Self, CriticalError> {
        let sector_meta = MetaDocumentDB::wrap(PrefixedMetaDB::wrap(sector_meta_prefix, &s.meta));

        let sector: Sector = sector_meta.get(sector_info_key).or_else(|e| match e {
            MetaError::NotFound => {
                let empty = Default::default();
                sector_meta.set(sector_info_key, &empty)?;
                Ok(empty)
            }

            MetaError::Failure(ie) => Err(ie),
        })?;

        let trace_meta = MetaDocumentDB::wrap(PrefixedMetaDB::wrap(sector_trace_prefix, &s.meta));

        Ok(Ctx {
            sector,
            trace: Vec::with_capacity(16),

            store: s,
            sector_meta,
            trace_meta,
        })
    }

    fn sync<F: FnOnce(&mut Sector) -> Result<()>>(
        &mut self,
        mut modify_fn: F,
    ) -> Result<(), CriticalError> {
        modify_fn(&mut self.sector)?;
        self.sector_meta
            .set(sector_info_key, &self.sector)
            .map_err(From::from)
    }

    fn finalize(self) -> Result<(), CriticalError> {
        self.store.cleanup()?;
        self.sector_meta.remove(sector_info_key)?;
        Ok(())
    }

    fn handle(&mut self, event: Option<Event>) -> Result<Option<Event>, Failure> {
        if let Some(evt) = event {
            self.sync(move |s| evt.apply(s))?;
        };

        match self.sector.state {
            State::Empty => self.handle_empty(),

            State::Allocated => self.handle_allocated(),

            State::DealsAcquired => self.handle_deal_acquired(),

            State::PieceAdded => self.handle_piece_added(),

            State::TicketAssigned => self.handle_ticket_assigned(),

            State::PC1Done => self.handle_pc1_done(),

            State::PC2Done => self.handle_pc2_done(),

            State::PCSubmitted => self.handle_pc_submitted(),

            State::SeedAssigned => self.handle_seed_assigned(),

            State::C1Done => self.handle_c1_done(),

            State::C2Done => self.handle_c2_done(),

            State::Persisted => self.handle_persisted(),

            State::ProofSubmitted => self.handle_proof_submitted(),

            State::Finished => return Ok(None),
        }
        .map(From::from)
    }

    fn handle_empty(&mut self) -> HandleResult {
        unimplemented!();
    }

    fn handle_allocated(&mut self) -> HandleResult {
        unimplemented!();
    }

    fn handle_deal_acquired(&mut self) -> HandleResult {
        unimplemented!();
    }

    fn handle_piece_added(&mut self) -> HandleResult {
        unimplemented!();
    }

    fn handle_ticket_assigned(&mut self) -> HandleResult {
        unimplemented!();
    }

    fn handle_pc1_done(&mut self) -> HandleResult {
        unimplemented!();
    }

    fn handle_pc2_done(&mut self) -> HandleResult {
        unimplemented!();
    }

    fn handle_pc_submitted(&mut self) -> HandleResult {
        unimplemented!();
    }

    fn handle_seed_assigned(&mut self) -> HandleResult {
        unimplemented!();
    }

    fn handle_c1_done(&mut self) -> HandleResult {
        unimplemented!();
    }

    fn handle_c2_done(&mut self) -> HandleResult {
        unimplemented!();
    }

    fn handle_persisted(&mut self) -> HandleResult {
        unimplemented!();
    }

    fn handle_proof_submitted(&mut self) -> HandleResult {
        unimplemented!();
    }
}

pub struct Worker<DB>
where
    DB: MetaDB,
{
    store: Store<DB>,
    resume_rx: Receiver<()>,
    done_rx: Receiver<()>,
}

impl<DB> Worker<DB>
where
    DB: MetaDB,
{
    pub fn new(s: Store<DB>, resume_rx: Receiver<()>, done_rx: Receiver<()>) -> Self {
        Worker {
            store: s,
            resume_rx,
            done_rx,
        }
    }

    pub fn start_seal(&mut self) -> Result<()> {
        let mut wait_for_resume = false;
        'SEAL_LOOP: loop {
            if wait_for_resume {
                warn!(target: log_target_sealing, "waiting for resume signal");

                select! {
                    recv(self.resume_rx) -> resume_res => {
                        resume_res.context("resume signal channel closed unexpectedly")?;
                    },

                    recv(self.done_rx) -> _done_res => {
                        return Ok(())
                    },
                }
            }

            if self.done_rx.try_recv() != Err(TryRecvError::Empty) {
                return Ok(());
            }

            if let Err(failure) = self.seal_one() {
                error!(target: log_target_sealing, "sealing failed: {:?}", failure);
                match failure {
                    Failure::Temporary(_) | Failure::Unrecoverable(_) | Failure::Critical(_) => {
                        if let Failure::Temporary(_) = failure {
                            error!(
                                target: log_target_sealing,
                                "temporary error should not be popagated to the top level"
                            );
                        };

                        wait_for_resume = true;
                        continue 'SEAL_LOOP;
                    }

                    Failure::Permanent(_) => {}
                };
            }

            self.store.config.seal_interval.as_ref().map(|d| {
                info!("wait {:?} before sealing", d);
                std::thread::sleep(*d);
            });
        }
    }

    fn seal_one(&mut self) -> Result<(), Failure> {
        let mut ctx = Ctx::build(&self.store)?;

        let mut event = None;
        loop {
            match ctx.handle(event.take()) {
                Ok(Some(evt)) => {
                    event.replace(evt);
                }

                Ok(None) => return Ok(()),

                Err(Failure::Temporary(terr)) => {
                    if ctx.sector.retry >= ctx.store.config.max_retries {
                        return Err(Failure::Unrecoverable(terr.0.into()));
                    }

                    ctx.sync(|s| {
                        warn!(
                            target: log_target_sealing,
                            "temp error occurred: {:?}, retry={}", terr.0, s.retry,
                        );

                        s.retry += 1;

                        Ok(())
                    })?;

                    ctx.store.config.recover_interval.as_ref().map(|d| {
                        info!(target: log_target_sealing, "wait {:?} before recovering", d);
                        std::thread::sleep(*d);
                    });
                }

                Err(pf @ Failure::Permanent(_)) => return Err(pf),

                Err(cf @ Failure::Critical(_)) => return Err(cf),

                Err(uf @ Failure::Unrecoverable(_)) => return Err(uf),
            }
        }
    }
}
