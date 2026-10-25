//! Compile the exact private resolver without std or the native I/O harness.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

// This mode isolates the resolver from the root's unrelated pixel/gain-map
// graph. It compiles the ACTUAL fidelity source, not an imitation of its types.
#[cfg(not(feature = "full-zencodec"))]
extern crate self as zencodec;
#[cfg(not(feature = "full-zencodec"))]
pub(crate) use enough::{self, Unstoppable};
#[cfg(not(feature = "full-zencodec"))]
pub(crate) use whereat::At;
#[cfg(not(feature = "full-zencodec"))]
pub(crate) use zenpixels::Cicp;
#[cfg(not(feature = "full-zencodec"))]
#[allow(dead_code)]
#[path = "../../../src/fidelity.rs"]
mod fidelity;
#[cfg(not(feature = "full-zencodec"))]
pub(crate) mod encode {
    pub(crate) use crate::fidelity::{Fidelity, LossyTarget};
}

#[path = "../../io-validation/src/policy/mod.rs"]
mod policy;

#[allow(dead_code)]
fn resolve_without_std() -> Result<(), zencodec::At<policy::Failure<core::convert::Infallible>>> {
    use policy::*;
    let mut preset = PolicySet::default();
    let mut engine = Engine::new(preset, 4);
    let alternatives = [Alternative {
        action: Action::Execution(Execution::Sequential),
        ram: Evidence::Upper(8),
        disk: Evidence::Upper(0),
        accounting: Accounting::Operation,
        effects: Effects(0),
        needs_replay: false,
        preserves_published: true,
    }];
    engine.resolve(
        Facts::Execution(ExecutionFacts {
            resident_input: 0,
            source_length: None,
            network: true,
        }),
        &alternatives,
        Requirements {
            ram: Some(8),
            disk: Some(0),
            accounting: Accounting::Operation,
            effects: Effects(0),
        },
        Context::default(),
        &mut preset,
        &zencodec::Unstoppable,
    )?;
    Ok(())
}

#[test]
fn the_no_std_consumer_path_runs_on_the_host() {
    resolve_without_std().unwrap();
}
