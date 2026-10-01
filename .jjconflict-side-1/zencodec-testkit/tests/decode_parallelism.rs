//! Tested examples for the zencodec "controlling decode parallelism" docs.
//!
//! These back the README section of the same name. They demonstrate, against
//! the testkit reference codec, that:
//!
//! 1. the one-shot decode result ([`DecodeOutput`]) is `Send`;
//! 2. you can **cap a decode's internal thread count** with a sized rayon pool
//!    even though one-shot `DynDecoder` is *not* `Send` — by constructing and
//!    consuming the decoder *inside* `pool.install` and returning only the owned
//!    (`Send`) `DecodeOutput`;
//! 3. many decodes run concurrently under one capped pool;
//! 4. the **streaming** decoder *is* `Send` (it crosses a thread boundary),
//!    which is the path to reach for when you need a live decoder on another
//!    thread.

use std::borrow::Cow;

use rayon::ThreadPoolBuilder;
use rayon::prelude::*;

use zencodec::decode::{
    Decode, DecodeJob, DecodeOutput, DecoderConfig, DynDecoderConfig, DynStreamingDecoder,
};
use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
use zencodec_testkit::{ReferenceDecoderConfig, ReferenceEncoderConfig, TestImage};

const W: u32 = 32;
const H: u32 = 24;

fn assert_send<T: Send>() {}

/// Encode a small RGBA8 gradient with the reference codec → bytes.
fn encode_sample() -> Vec<u8> {
    let img = TestImage::rgba8_gradient(W, H);
    ReferenceEncoderConfig::new()
        .job()
        .encoder()
        .expect("build encoder")
        .encode(img.as_slice())
        .expect("encode")
        .into_vec()
}

/// (1) The linchpin: the owned one-shot result is `Send`, so it can be returned
/// out of `rayon::ThreadPool::install`.
#[test]
fn decode_output_is_send() {
    assert_send::<DecodeOutput>();
}

/// (2) `DynDecoder` is not `Send`, yet its internal rayon parallelism is capped
/// to the pool: build + consume it *inside* `install`, return owned pixels.
#[test]
fn dyn_decoder_thread_capped_via_pool_install() {
    let bytes = encode_sample();
    let pool = ThreadPoolBuilder::new().num_threads(2).build().unwrap();

    // The closure captures only `&bytes` (Send) and returns `DecodeOutput`
    // (Send); the non-Send `Box<dyn DynDecoder>` lives and dies on the worker.
    let out: DecodeOutput = pool.install(|| {
        ReferenceDecoderConfig
            .dyn_job()
            .into_decoder(Cow::Borrowed(&bytes), &[])
            .expect("build dyn decoder")
            .decode()
            .expect("decode")
    });

    assert_eq!((out.width(), out.height()), (W, H));
}

/// (3) Many decodes at once under a single capped pool — each decoder is built
/// and consumed inside its own `par_iter` task, so non-Send never escapes.
#[test]
fn many_concurrent_decodes_under_one_capped_pool() {
    let bytes = encode_sample();
    let pool = ThreadPoolBuilder::new().num_threads(2).build().unwrap();
    const N: usize = 8;

    let dims: Vec<(u32, u32)> = pool.install(|| {
        (0..N)
            .into_par_iter()
            .map(|_| {
                let out = ReferenceDecoderConfig
                    .job()
                    .decoder(Cow::Borrowed(&bytes), &[])
                    .expect("build decoder")
                    .decode()
                    .expect("decode");
                (out.width(), out.height())
            })
            .collect()
    });

    assert_eq!(dims.len(), N);
    assert!(dims.iter().all(|&d| d == (W, H)));
}

/// (4) The streaming decoder *is* `Send`: move a `Box<dyn DynStreamingDecoder>`
/// into a scoped thread and drive it there. This compiles only because the
/// streaming path is `Send` by contract (the one-shot path is not).
#[test]
fn streaming_decoder_is_send_and_crosses_threads() {
    let bytes = encode_sample();

    let batches: usize = std::thread::scope(|scope| {
        let sd: Box<dyn DynStreamingDecoder + '_> = ReferenceDecoderConfig
            .dyn_job()
            .into_streaming_decoder(Cow::Borrowed(&bytes), &[])
            .expect("build streaming decoder");

        scope
            .spawn(move || {
                let mut sd = sd; // moved across the thread boundary — needs Send
                let mut n = 0usize;
                while sd.next_batch().expect("next batch").is_some() {
                    n += 1;
                }
                n
            })
            .join()
            .unwrap()
    });

    assert!(batches >= 1, "streaming decode ran on another thread");
}
