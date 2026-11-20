//! "Import" screen: choose an export from another password manager (or a browser),
//! see what is in it and add it to the vault in one go.
//!
//! The file is read here, in memory; the items are encrypted on this machine and
//! sent together ([`crate::flow::WriteOp::Import`]). The export itself has the
//! passwords in plain text, so the screen reminds the person to delete it.

use std::path::{Path, PathBuf};

use gpui_kit::assets::IconName;
use gpui_kit::component::button::*;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use scout_vault_core::import::{self, ImportError, Imported, Parsed, Skip};
use scout_vault_core::vault::{Folder, ItemKind, Vault};

use crate::ui;

/// Biggest export accepted.
pub const MAX_FILE: u64 = 32 * 1024 * 1024;
/// How many items the preview lists.
const PREVIEW: usize = 8;

pub enum ImportEvent {
    Close,
    /// Import these items (folders by name; `existing` lets them reuse the vault's).
    Import {
        items: Vec<Imported>,
        existing: Vec<Folder>,
    },
}

/// A file that was read.
struct Loaded {
    file_name: String,
    parsed: Parsed,
    /// Same position as `parsed.items`: already in the vault?
    duplicate: Vec<bool>,
}

pub struct ImportView {
    /// The vault's folders (decrypted names), to reuse them.
    folders: Vec<Folder>,
    /// What decides "already in the vault" (a copy of the open vault's items).
    vault: Vault,
    loaded: Option<Loaded>,
    error: Option<SharedString>,
    reading: bool,
    skip_duplicates: bool,
}

impl EventEmitter<ImportEvent> for ImportView {}

fn describe(e: &ImportError) -> SharedString {
    match e {
        ImportError::Encrypted => t!("import.encrypted").into(),
        ImportError::Unknown => t!("import.unknown").into(),
        ImportError::Invalid(why) => t!("import.invalid", e = why).into(),
    }
}

fn read_file(path: &Path) -> Result<String, SharedString> {
    let size = std::fs::metadata(path)
        .map_err(|_| SharedString::from(t!("import.unreadable")))?
        .len();
    if size > MAX_FILE {
        return Err(t!("import.too_big").into());
    }
    let bytes = std::fs::read(path).map_err(|_| SharedString::from(t!("import.unreadable")))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

impl ImportView {
    pub fn new(vault: Vault) -> Self {
        Self {
            folders: vault.folders.clone(),
            vault,
            loaded: None,
            error: None,
            reading: false,
            skip_duplicates: true,
        }
    }

    /// Reads and checks an export (in the background: big files take a moment).
    pub fn load_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.reading = true;
        self.error = None;
        cx.notify();
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let task = cx.background_spawn(async move {
            let text = read_file(&path)?;
            import::parse(&text).map_err(|e| describe(&e))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.reading = false;
                match result {
                    Ok(parsed) => {
                        let duplicate = parsed
                            .items
                            .iter()
                            .map(|i| import::already_in(&this.vault, &i.item))
                            .collect();
                        this.loaded = Some(Loaded {
                            file_name,
                            parsed,
                            duplicate,
                        });
                    }
                    Err(e) => {
                        this.loaded = None;
                        this.error = Some(e);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn pick_file(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(t!("import.prompt").into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                if let Some(path) = paths.into_iter().next() {
                    this.update(cx, |this, cx| this.load_path(path, cx)).ok();
                }
            }
        })
        .detach();
    }

    pub fn set_skip_duplicates(&mut self, on: bool, cx: &mut Context<Self>) {
        self.skip_duplicates = on;
        cx.notify();
    }

    /// What "Import" would add now.
    pub fn to_import(&self) -> Vec<Imported> {
        let Some(l) = &self.loaded else {
            return Vec::new();
        };
        l.parsed
            .items
            .iter()
            .zip(&l.duplicate)
            .filter(|(_, dup)| !(self.skip_duplicates && **dup))
            .map(|(i, _)| i.clone())
            .collect()
    }

    /// How many items of the file are already in the vault.
    pub fn duplicates(&self) -> usize {
        self.loaded
            .as_ref()
            .map(|l| l.duplicate.iter().filter(|d| **d).count())
            .unwrap_or(0)
    }

    pub fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    pub fn run(&mut self, cx: &mut Context<Self>) {
        let items = self.to_import();
        if items.is_empty() {
            return;
        }
        cx.emit(ImportEvent::Import {
            items,
            existing: self.folders.clone(),
        });
    }

    fn summary(&self, l: &Loaded, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border, radius, warning) = (
            theme.muted_foreground,
            theme.border,
            theme.radius_lg,
            theme.warning,
        );
        let mut counts = [0usize; 5];
        for i in &l.parsed.items {
            let ix = match i.item.kind {
                ItemKind::Login(_) => 0,
                ItemKind::SecureNote => 1,
                ItemKind::Card(_) => 2,
                ItemKind::Identity(_) => 3,
                _ => 4,
            };
            counts[ix] += 1;
        }
        let labels = [
            t!("list.logins"),
            t!("list.notes"),
            t!("list.cards"),
            t!("list.identities"),
            t!("list.ssh_keys"),
        ];
        let mut folders: Vec<&str> = l
            .parsed
            .items
            .iter()
            .filter_map(|i| i.folder.as_deref())
            .collect();
        folders.sort_unstable();
        folders.dedup();
        let stat = |label: SharedString, n: usize| {
            v_flex()
                .px_3()
                .py_2()
                .rounded(cx.theme().radius)
                .bg(cx.theme().muted.opacity(0.5))
                .child(div().text_lg().font_semibold().child(n.to_string()))
                .child(div().text_xs().text_color(muted).child(label))
        };
        let mut stats = h_flex().gap_2().flex_wrap();
        for (label, n) in labels.iter().zip(counts) {
            if n > 0 {
                stats = stats.child(stat((*label).into(), n));
            }
        }
        if !folders.is_empty() {
            stats = stats.child(stat(t!("import.folders").into(), folders.len()));
        }

        let preview =
            v_flex()
                .gap_1()
                .children(
                    l.parsed
                        .items
                        .iter()
                        .zip(&l.duplicate)
                        .take(PREVIEW)
                        .map(|(i, dup)| {
                            let detail = match &i.item.kind {
                                ItemKind::Login(login) => {
                                    login.username.clone().unwrap_or_default()
                                }
                                _ => String::new(),
                            };
                            h_flex()
                                .gap_2()
                                .text_sm()
                                .when(*dup && self.skip_duplicates, |el| el.opacity(0.45))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .child(i.item.name.clone()),
                                )
                                .child(div().text_xs().text_color(muted).child(detail))
                                .when_some(i.folder.clone(), |el, f| {
                                    el.child(
                                        h_flex()
                                            .gap_1()
                                            .text_xs()
                                            .text_color(muted)
                                            .child(Icon::new(IconName::Folder).xsmall())
                                            .child(f),
                                    )
                                })
                        }),
                );
        let more = l.parsed.items.len().saturating_sub(PREVIEW);

        let skipped = (!l.parsed.skipped.is_empty()).then(|| {
            let lines: Vec<String> = l
                .parsed
                .skipped
                .iter()
                .take(4)
                .map(|(line, why)| match why {
                    Skip::Empty => t!("import.skipped_empty", n = line),
                    Skip::UnknownType(k) => t!("import.skipped_type", n = line, t = k),
                })
                .collect();
            v_flex()
                .gap_0p5()
                .text_xs()
                .text_color(warning)
                .child(t!("import.skipped", n = l.parsed.skipped.len()))
                .children(lines.into_iter().map(|l| div().child(format!("• {l}"))))
        });

        let total = self.to_import().len();
        let dups = self.duplicates();
        v_flex()
            .gap_4()
            .p_5()
            .rounded(radius)
            .border_1()
            .border_color(border)
            .child(
                h_flex()
                    .gap_3()
                    .child(ui::icon_chip(IconName::FileText, px(36.), true, cx))
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(
                                div()
                                    .id("import-file")
                                    .test_support()
                                    .aria_label(SharedString::from(l.file_name.clone()))
                                    .font_semibold()
                                    .truncate()
                                    .child(l.file_name.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(l.parsed.format.name()),
                            ),
                    ),
            )
            .child(stats)
            .child(preview)
            .when(more > 0, |el| {
                el.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(t!("import.and_more", n = more)),
                )
            })
            .children(skipped)
            .when(dups > 0, |el| {
                el.child(
                    h_flex()
                        .gap_4()
                        .justify_between()
                        .child(
                            v_flex()
                                .child(div().text_sm().child(t!("import.skip_duplicates")))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(muted)
                                        .child(t!("import.duplicates", n = dups)),
                                ),
                        )
                        .child(
                            Switch::new("import-skip-duplicates")
                                .checked(self.skip_duplicates)
                                .on_click(cx.listener(|this, v: &bool, _, cx| {
                                    this.set_skip_duplicates(*v, cx)
                                })),
                        ),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("import-other")
                            .ghost()
                            .small()
                            .label(t!("import.other_file"))
                            .on_click(cx.listener(|this, _, _, cx| this.pick_file(cx))),
                    )
                    .child(
                        Button::new("import-run")
                            .primary()
                            .small()
                            .icon(IconName::Download)
                            .label(t!("import.run", n = total))
                            .disabled(total == 0)
                            .on_click(cx.listener(|this, _, _, cx| this.run(cx))),
                    ),
            )
    }
}

impl Render for ImportView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, warning, danger) = (theme.muted_foreground, theme.warning, theme.danger);
        let back = div().absolute().top_6().left_6().child(
            ui::back_button("import-close")
                .on_click(cx.listener(|_, _, _, cx| cx.emit(ImportEvent::Close))),
        );
        let header = h_flex()
            .gap_3()
            .child(ui::icon_chip(IconName::Download, px(44.), true, cx))
            .child(
                v_flex()
                    .gap_0p5()
                    .child(div().text_2xl().font_semibold().child(t!("import.title")))
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child(t!("import.subtitle")),
                    ),
            );
        let busy = self.reading;
        let body: AnyElement = match &self.loaded {
            Some(l) if !busy => self.summary(l, cx).into_any_element(),
            _ => ui::drop_zone(
                "import-zone",
                busy,
                cx.listener(|this, _, _, cx| {
                    if !this.reading {
                        this.pick_file(cx)
                    }
                }),
                cx.listener(|this, paths: &ExternalPaths, _, cx| {
                    if let Some(p) = paths.paths().first() {
                        this.load_path(p.clone(), cx)
                    }
                }),
                cx,
            )
            .into_any_element(),
        };
        let formats = v_flex()
            .gap_1()
            .text_xs()
            .text_color(muted)
            .child(div().font_semibold().child(t!("import.formats_title")))
            .child(t!("import.formats"));
        let content = v_flex()
            .w_full()
            .max_w(px(680.))
            .mx_auto()
            .px_8()
            .py_10()
            .gap_5()
            .child(header)
            .child(body)
            .when_some(self.error.clone(), |el, e| {
                el.child(
                    div()
                        .id("import-error")
                        .test_support()
                        .aria_label(e.clone())
                        .text_sm()
                        .text_color(danger)
                        .child(e),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .items_start()
                    .text_xs()
                    .text_color(warning)
                    .child(Icon::new(IconName::TriangleAlert).xsmall())
                    .child(div().flex_1().child(t!("import.delete_export"))),
            )
            .child(formats);
        div()
            .relative()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .id("import")
                    .size_full()
                    .overflow_y_scroll()
                    .child(content),
            )
            .child(back)
    }
}
