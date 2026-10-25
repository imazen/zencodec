//! Resolve once per track/configuration epoch; packet processing reuses the
//! decision. No codec or container is reconfigured by the resolver itself.
#![allow(dead_code)]
use crate::policy::*;
use zencodec::{At, enough::Stop};
use zencodec_media::{
    packet::{Packet, RandomAccess},
    track::{TrackInfo, TrackKind},
};

#[derive(Debug)]
pub(crate) struct TrackSelection {
    source: u64,
    epoch: u32,
    track: u32,
    pub action: MediaAction,
}
impl TrackSelection {
    pub fn applies(&self, source: u64, packet: &Packet) -> bool {
        source == self.source && packet.track() == self.track && packet.config_epoch() == self.epoch
    }
}
pub(crate) fn select_track<R: PolicyResolver + ?Sized>(
    engine: &mut Engine,
    track: &TrackInfo,
    first: &Packet,
    source: u64,
    may_drop: bool,
    resolver: &mut R,
    stop: &dyn Stop,
) -> Result<TrackSelection, At<Failure<R::Error>>> {
    assert_eq!(track.index(), first.track());
    let facts = MediaFacts {
        track: first.track(),
        epoch: first.config_epoch(),
        audio: track.kind() == TrackKind::Audio,
        first_packet_bytes: first.len() as u64,
        first_pts_known: first.pts().is_some(),
        random_access: match first.random_access() {
            RandomAccess::Yes => Some(true),
            RandomAccess::No => Some(false),
            RandomAccess::Unknown => None,
        },
    };
    let choices = [MediaAction::KeepTrack, MediaAction::DropTrack].map(|action| Alternative {
        action: Action::Media(action),
        ram: Evidence::Unknown,
        disk: Evidence::Upper(0),
        accounting: Accounting::Operation,
        effects: if action == MediaAction::KeepTrack {
            Effects::KEEP_TRACK
        } else {
            Effects(0)
        },
        needs_replay: false,
        preserves_published: true,
    });
    let requirements = Requirements {
        ram: None,
        disk: None,
        accounting: Accounting::Operation,
        effects: if may_drop {
            Effects(0)
        } else {
            Effects::KEEP_TRACK
        },
    };
    let selection = engine.resolve(
        Facts::Media(facts),
        &choices,
        requirements,
        Context {
            source_revision: source,
            epoch: u64::from(facts.epoch),
            ..Context::default()
        },
        resolver,
        stop,
    )?;
    let Facts::Media(bound) = selection.facts else {
        unreachable!()
    };
    let Action::Media(action) = selection.alternative.action else {
        unreachable!()
    };
    Ok(TrackSelection {
        source: selection.context.source_revision,
        epoch: bound.epoch,
        track: bound.track,
        action,
    })
}
