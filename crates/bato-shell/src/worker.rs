//! Blocking work off the UI thread.

use std::future::Future;

/// Run `f` on a new thread and resolve with its result.
///
/// For blocking calls (HTTP, file I/O, process spawns) from an async context
/// such as iced's `Task::perform`. A thread per call is deliberate: calls are
/// rare and short, and a pool would be one more thing alive at idle.
///
/// # Panics
/// The future panics if `f` panics (the panic is re-raised, not swallowed).
pub fn spawn_blocking<T, F>(f: F) -> impl Future<Output = T> + Send + 'static
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (tx, rx) = futures::channel::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name("bato-worker".into())
        .spawn(move || {
            let _ = tx.send(std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)));
        });
    async move {
        if let Err(e) = spawned {
            panic!("bato-shell: could not spawn a worker thread: {e}");
        }
        match rx.await.expect("bato-shell worker dropped its result") {
            Ok(v) => v,
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn resolves_off_thread() {
        let main = std::thread::current().id();
        let other =
            futures::executor::block_on(super::spawn_blocking(move || std::thread::current().id()));
        assert_ne!(main, other);
    }
}
