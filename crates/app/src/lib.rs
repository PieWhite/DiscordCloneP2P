//! FitCommunication als bibliotheek.
//!
//! `main.rs` is een dunne schil hieromheen. Het bestaan van deze lib heeft één reden:
//! de koppeling tussen oplog en mesh — wie wanneer wat naar wie stuurt — is met een
//! integratietest te controleren in plaats van met twee vensters en een logbestand.

pub mod chat;
pub mod clips;
pub mod config;
pub mod engine;
pub mod files;
pub mod geluid;
pub mod notify;
pub mod release;
pub mod streams;
pub mod tags;
pub mod tray;
pub mod ui;
pub mod updates;
pub mod wordle;
pub mod youtube;
