//! "Export" screen: saves the personal vault to a file, in Bitwarden's formats.
//!
//! The master password is asked first (as Bitwarden does): an unlocked vault on an
//! unattended computer shouldn't be one click away from a plain-text copy. The
//! password-protected JSON is the default; the plain formats come with a warning.

use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::*;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use scout_vault_core::export;
use scout_vault_core::vault::Vault;
use zeroize::Zeroizing;

use crate::flow::Session;
use crate::ui::{self, Segment};

/// Shortest password accepted for a protected export.
pub const MIN_FILE_PASSWORD: usize = 8;

pub enum ExportEvent {
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// Bitwarden JSON encrypted with a password chosen for the file.
    Protected,
    Json,
    Csv,
}

impl ExportFormat {
    const ALL: [ExportFormat; 3] = [Self::Protected, Self::Json, Self::Csv];

    fn id(self) -> &'static str {
        match self {
            Self::Protected => "protected",
            Self::Json => "json",
            Self::Csv => "csv",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Protected => t!("export.format_protected"),
            Self::Json => t!("export.format_json"),
            Self::Csv => t!("export.format_csv"),
        }
    }

    fn file_name(self) -> &'static str {
        match self {
            Self::Csv => "scout-vault-export.csv",
            _ => "scout-vault-export.json",
        }
    }
}

pub struct ExportView {
    vault: Vault,
    session: Session,
    format: ExportFormat,
    master: Entity<InputState>,
    file_password: Entity<InputState>,
    file_password_again: Entity<InputState>,
    busy: bool,
    error: Option<SharedString>,
    /// Where the last export went.
    saved: Option<PathBuf>,
}

impl EventEmitter<ExportEvent> for ExportView {}

fn password_input(
    placeholder: &'static str,
    window: &mut Window,
    cx: &mut App,
) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .masked(true)
            .placeholder(placeholder)
    })
}

/// Where the save dialog starts: Downloads, or the user's folder.
fn start_folder() -> PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let downloads = home.join("Downloads");
    if downloads.is_dir() {
        downloads
    } else {
        home
    }
}

impl ExportView {
    pub fn new(
        vault: Vault,
        session: Session,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let master = password_input(t!("export.master_placeholder"), window, cx);
        master.update(cx, |m, cx| m.focus(window, cx));
        Self {
            vault,
            session,
            format: ExportFormat::Protected,
            master,
            file_password: password_input(t!("export.file_password"), window, cx),
            file_password_again: password_input(t!("export.file_password_again"), window, cx),
            busy: false,
            error: None,
            saved: None,
        }
    }

    pub fn format(&self) -> ExportFormat {
        self.format
    }

    pub fn set_format(&mut self, format: ExportFormat, cx: &mut Context<Self>) {
        self.format = format;
        self.error = None;
        cx.notify();
    }

    pub fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    pub fn saved(&self) -> Option<&PathBuf> {
        self.saved.as_ref()
    }

    pub fn is_busy(&self) -> bool {
        self.busy
    }

    /// How many items go in (personal, not in the trash).
    pub fn item_count(&self) -> usize {
        export::personal_items(&self.vault).len()
    }

    fn fail(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.busy = false;
        self.error = Some(message.into());
        cx.notify();
    }

    /// "Export…": checks the passwords, then asks where to save.
    pub fn export(&mut self, cx: &mut Context<Self>) {
        self.run(None, cx);
    }

    /// Like [`Self::export`], straight into `path` (tests).
    pub fn export_to(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.run(Some(path), cx);
    }

    fn run(&mut self, target: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.error = None;
        self.saved = None;
        let master = Zeroizing::new(self.master.read(cx).value().to_string());
        if master.is_empty() {
            return self.fail(t!("export.master_required"), cx);
        }
        let file_password = Zeroizing::new(self.file_password.read(cx).value().to_string());
        if self.format == ExportFormat::Protected {
            let again = self.file_password_again.read(cx).value().to_string();
            if file_password.chars().count() < MIN_FILE_PASSWORD {
                return self.fail(t!("export.file_password_short", n = MIN_FILE_PASSWORD), cx);
            }
            if *file_password != again {
                return self.fail(t!("export.file_password_mismatch"), cx);
            }
        }
        self.busy = true;
        cx.notify();
        let session = self.session.clone();
        let format = self.format;
        // The text is built here, from this screen's copy of the vault.
        let (plain, skipped) = match format {
            ExportFormat::Csv => {
                let (csv, skipped) = export::to_csv(&self.vault);
                (Zeroizing::new(csv), skipped)
            }
            _ => (Zeroizing::new(export::to_json(&self.vault)), 0),
        };
        let count = self.item_count() - skipped;
        cx.spawn(async move |this, cx| {
            if !crate::flow::check_master_password(&session, master).await {
                this.update(cx, |this, cx| this.fail(t!("export.master_wrong"), cx))
                    .ok();
                return;
            }
            // Encrypting (600 000 PBKDF2 rounds) runs off the UI thread.
            let content = cx
                .background_spawn(async move {
                    match format {
                        ExportFormat::Protected => {
                            export::protect(&plain, &file_password, export::PROTECT_ITERATIONS)
                                .map(Zeroizing::new)
                                .map_err(|e| e.to_string())
                        }
                        _ => Ok(plain),
                    }
                })
                .await;
            let content = match content {
                Ok(c) => c,
                Err(e) => {
                    this.update(cx, |this, cx| this.fail(t!("export.failed", e = e), cx))
                        .ok();
                    return;
                }
            };
            let path = match target {
                Some(path) => Some(path),
                None => {
                    let answer = cx.update(|cx| {
                        cx.prompt_for_new_path(&start_folder(), Some(format.file_name()))
                    });
                    answer.await.ok().and_then(Result::ok).flatten()
                }
            };
            let Some(path) = path else {
                // Canceled in the dialog.
                this.update(cx, |this, cx| {
                    this.busy = false;
                    cx.notify();
                })
                .ok();
                return;
            };
            let written = std::fs::write(&path, content.as_bytes());
            this.update(cx, |this, cx| {
                this.busy = false;
                match written {
                    Ok(()) => {
                        crate::toast::success(t!("export.done", n = count), cx);
                        this.saved = Some(path);
                    }
                    Err(e) => this.error = Some(t!("export.failed", e = e).into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Render for ExportView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, warning, danger, border, radius) = (
            theme.muted_foreground,
            theme.warning,
            theme.danger,
            theme.border,
            theme.radius_lg,
        );
        let back = div().absolute().top_6().left_6().child(
            ui::back_button("export-close")
                .on_click(cx.listener(|_, _, _, cx| cx.emit(ExportEvent::Close))),
        );
        let header = h_flex()
            .gap_3()
            .child(ui::icon_chip(IconName::Upload, px(44.), true, cx))
            .child(
                v_flex()
                    .gap_0p5()
                    .child(div().text_2xl().font_semibold().child(t!("export.title")))
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child(t!("export.subtitle", n = self.item_count())),
                    ),
            );
        let this = cx.entity().downgrade();
        let formats = ui::segmented(
            "export-format",
            ExportFormat::ALL
                .iter()
                .map(|f| Segment {
                    id: format!("export-format-{}", f.id()).into(),
                    label: f.label().into(),
                    icon: None,
                    selected: *f == self.format,
                })
                .collect(),
            move |ix, _, cx| {
                this.update(cx, |v, cx| v.set_format(ExportFormat::ALL[ix], cx))
                    .ok();
            },
            cx,
        );
        let explain: SharedString = match self.format {
            ExportFormat::Protected => t!("export.protected_desc").into(),
            ExportFormat::Json => t!("export.json_desc").into(),
            ExportFormat::Csv => t!("export.csv_desc").into(),
        };
        let busy = self.busy;
        let mut card = v_flex()
            .gap_4()
            .p_5()
            .rounded(radius)
            .border_1()
            .border_color(border)
            .child(formats)
            .child(div().text_sm().text_color(muted).child(explain));
        if self.format == ExportFormat::Protected {
            card = card.child(
                v_flex()
                    .gap_2()
                    .child(
                        Input::new(&self.file_password)
                            .id("export-file-password")
                            .mask_toggle()
                            .disabled(busy),
                    )
                    .child(
                        Input::new(&self.file_password_again)
                            .id("export-file-password-again")
                            .mask_toggle()
                            .disabled(busy),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(t!("export.file_password_hint")),
                    ),
            );
        } else {
            card = card.child(
                h_flex()
                    .gap_2()
                    .items_start()
                    .text_xs()
                    .text_color(warning)
                    .child(Icon::new(IconName::TriangleAlert).xsmall())
                    .child(div().flex_1().child(t!("export.plain_warning"))),
            );
        }
        card = card
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .child(t!("export.master_title")),
                    )
                    .child(
                        Input::new(&self.master)
                            .id("export-master")
                            .mask_toggle()
                            .disabled(busy),
                    ),
            )
            .when_some(self.error.clone(), |el, e| {
                el.child(
                    div()
                        .id("export-error")
                        .test_support()
                        .aria_label(e.clone())
                        .text_sm()
                        .text_color(danger)
                        .child(e),
                )
            })
            .when_some(self.saved.clone(), |el, path| {
                el.child(
                    div()
                        .id("export-saved")
                        .test_support()
                        .text_sm()
                        .text_color(muted)
                        .child(t!("export.saved_to", p = path.display())),
                )
            })
            .child(
                h_flex().justify_end().child(
                    Button::new("export-run")
                        .primary()
                        .icon(IconName::Upload)
                        .label(t!("export.run"))
                        .loading(busy)
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.export(cx))),
                ),
            );
        let content = v_flex()
            .w_full()
            .max_w(px(640.))
            .mx_auto()
            .px_8()
            .py_10()
            .gap_5()
            .child(header)
            .child(card)
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(t!("export.scope_note")),
            );
        div()
            .relative()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .id("export")
                    .size_full()
                    .overflow_y_scroll()
                    .child(content),
            )
            .child(back)
    }
}
