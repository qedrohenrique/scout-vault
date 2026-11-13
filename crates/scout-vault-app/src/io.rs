//! Ponte entre o executor do GPUI e o Tokio.
//!
//! O `reqwest` precisa de um runtime Tokio; a UI roda no executor do GPUI.
//! `run` agenda o futuro no Tokio e devolve o `JoinHandle`, que pode ser
//! aguardado de dentro de um `cx.spawn` como qualquer outro futuro.

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

/// Executa `fut` no runtime de rede e espera o resultado.
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

/// Tarefa de fundo que para quando o valor é solto.
pub struct Background(tokio::task::JoinHandle<()>);

impl Drop for Background {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Roda `fut` no runtime de rede até terminar ou o [`Background`] ser solto.
pub fn spawn<F>(fut: F) -> Background
where
    F: Future<Output = ()> + Send + 'static,
{
    Background(runtime().spawn(fut))
}

/// Executa trabalho de CPU pesado (KDF, decifrar o cofre) fora da thread de UI.
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
