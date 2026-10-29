// These controls qualify scheduling ownership, not native codec memory usage.

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn aborted_request_keeps_admission_until_blocking_worker_exits() {
    let slots = std::sync::Arc::new(tokio::sync::Semaphore::new(1));
    let permit = slots.clone().acquire_owned().await.unwrap();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let pulse = RecordingPulse::default();
    let worker_pulse = pulse.clone();
    let request_pulse = pulse.clone();
    let (exited_tx, exited_rx) = tokio::sync::oneshot::channel();
    let request = tokio::spawn(cancel_on_request_drop(
        async move {
            tokio::task::spawn_blocking(move || {
                // Move admission into the worker, never leave it in the request.
                let _permit = permit;
                started_tx.send(()).unwrap();
                // Model a non-preemptible call without burning CPU in a spin loop.
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                assert!(enough::Stop::check(&worker_pulse).is_err());
                exited_tx.send(()).unwrap();
            })
            .await
            .unwrap();
            Ok::<_, std::convert::Infallible>(())
        },
        move || request_pulse.cancel(),
    ));
    tokio::time::timeout(Duration::from_secs(5), started_rx)
        .await
        .unwrap()
        .unwrap();
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    assert!(enough::Stop::check(&pulse).is_err());
    // Aborting the HTTP-side task did not free admission or stop the worker.
    assert!(slots.clone().try_acquire_owned().is_err());
    release_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), exited_rx)
        .await
        .unwrap()
        .unwrap();
    // Await the permit too: notification is sent just before worker-local drop.
    let _recovered = tokio::time::timeout(Duration::from_secs(5), slots.acquire())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manual_worker_retains_affine_state_and_releases_admission_on_join() {
    let slots = std::sync::Arc::new(tokio::sync::Semaphore::new(1));
    let permit = slots.clone().acquire_owned().await.unwrap();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let worker = std::thread::Builder::new()
        .name("affine-codec".into())
        .spawn(move || {
            let _permit = permit;
            // Rc cannot cross the boundary; construct it on its owning thread.
            let local_state = std::rc::Rc::new(std::thread::current().id());
            started_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(*local_state, std::thread::current().id());
            42u32 // Only the Send result crosses back to the caller.
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), started_rx)
        .await
        .unwrap()
        .unwrap();
    assert!(slots.clone().try_acquire_owned().is_err());
    release_tx.send(()).unwrap();
    // A possibly blocking join must not run on the polling thread.
    let result = tokio::task::spawn_blocking(move || worker.join())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result, 42);
    assert!(slots.try_acquire().is_ok());
}
