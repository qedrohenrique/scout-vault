//! Bridge between the GPUI executor and Tokio.
//!
//! `reqwest` needs a Tokio runtime; the UI runs on the GPUI executor.
//! `run` schedules the future on Tokio and returns the `JoinHandle`, which can be
//! awaited from inside a `cx.spawn` like any other future.

use std::future::Future;
use std::sync::OnceLock;

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("scout-vault-io")
            .enable_all()
            .build()
            .expect("não foi possível iniciar o runtime de rede")
    })
}

/// Inside the network runtime (to create Tokio resources outside a task).
pub fn enter() -> tokio::runtime::EnterGuard<'static> {
    runtime().enter()
}

/// Runs `fut` on the network runtime and waits for the result.
pub async fn run<F>(fut: F) -> F::Output
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    match runtime().spawn(fut).await {
        Ok(output) => output,
        Err(e) => std::panic::resume_unwind(e.into_panic()),
    }
}

/// Background task that stops when the value is dropped.
pub struct Background(tokio::task::JoinHandle<()>);

impl Drop for Background {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Runs `fut` on the network runtime until it finishes or the [`Background`] is dropped.
pub fn spawn<F>(fut: F) -> Background
where
    F: Future<Output = ()> + Send + 'static,
{
    Background(runtime().spawn(fut))
}

/// Runs heavy CPU work (KDF, decrypting the vault) off the UI thread.
pub async fn blocking<F, T>(f: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    run(async move {
        match tokio::task::spawn_blocking(f).await {
            Ok(output) => output,
            Err(e) => std::panic::resume_unwind(e.into_panic()),
        }
    })
    .await
}
