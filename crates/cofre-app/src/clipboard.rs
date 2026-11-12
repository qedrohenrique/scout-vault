//! Copiar com limpeza automática da área de transferência.

use std::time::Duration;

use gpui_kit::*;
use zeroize::Zeroizing;

pub const CLEAR_AFTER: Duration = Duration::from_secs(30);

/// Copia `text` e agenda a limpeza. Só apaga se a área de transferência ainda
/// tiver o mesmo valor — não destrói algo que o usuário copiou depois.
///
/// A tarefa é desacoplada da tela: bloquear o cofre não cancela a limpeza.
pub fn copy_and_clear_later(text: &str, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
    let expected = Zeroizing::new(text.to_owned());
    cx.spawn(async move |cx| {
        cx.background_executor().timer(CLEAR_AFTER).await;
        cx.update(|cx| clear_if_unchanged(&expected, cx));
    })
    .detach();
}

pub fn clear_if_unchanged(expected: &str, cx: &mut App) {
    let current = cx.read_from_clipboard().and_then(|item| item.text());
    if current.as_deref() == Some(expected) {
        cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
    }
}
