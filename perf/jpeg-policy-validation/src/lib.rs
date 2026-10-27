//! Private native JPEG qualification. No published API.
#![forbid(unsafe_code)]

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "../../io-validation/src/policy/mod.rs"]
mod policy;
#[cfg(test)]
#[path = "../../io-validation/src/policy_disclosure.rs"]
mod policy_disclosure;

#[cfg(test)]
#[path = "../../../docs/examples/jpeg_policy.rs"]
mod guide;

#[cfg(test)]
mod control;
#[cfg(test)]
mod conversion;
#[cfg(test)]
#[path = "../../../docs/examples/pixel_conversion.rs"]
mod pixel_guide;

#[cfg(test)]
mod checked_examples;
#[cfg(test)]
#[path = "../../../docs/examples/checked_operations.rs"]
mod checked_guide;

#[cfg(test)]
mod output_scheduling;

#[cfg(test)]
mod row_driver;

#[cfg(test)]
mod requirement_scope;
