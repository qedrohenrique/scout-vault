//! Password health screen: weak, reused, old and leaked.
//!
//! The analysis runs on the spot, on the machine. The leak check only happens
//! when the person asks, and sends only hash prefixes (k-anonymity) — see
//! [`scout_vault_core::health`]. Clicking an item opens it in the vault.

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::*;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use scout_vault_core::health::{self, Report};
use scout_vault_core::vault::{Item, ItemKind};
use zeroize::Zeroizing;

use crate::{appearance, motion, ui};

pub enum HealthEvent {
    Close,
    OpenItem(String),
}

/// Chosen tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Weak,
    Reused,
    Old,
    Exposed,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Weak, Tab::Reused, Tab::Old, Tab::Exposed];

    pub fn id(self) -> &'static str {
        match self {
            Tab::Weak => "health-tab-weak",
            Tab::Reused => "health-tab-reused",
            Tab::Old => "health-tab-old",
            Tab::Exposed => "health-tab-exposed",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Tab::Weak => t!("health.tab_weak"),
            Tab::Reused => t!("health.tab_reused"),
            Tab::Old => t!("health.tab_old"),
            Tab::Exposed => t!("health.tab_exposed"),
        }
    }

    fn icon(self) -> IconName {
        match self {
            Tab::Weak => IconName::TriangleAlert,
            Tab::Reused => IconName::Copy,
            Tab::Old => IconName::Timer,
            Tab::Exposed => IconName::ShieldAlert,
        }
    }
}

/// State of the leak check.
#[derive(Debug, Clone, PartialEq)]
pub enum Pwned {
    NotChecked,
    Checking,
    Done(Vec<(String, u64)>),
    Failed(String),
}

pub struct HealthView {
    items: Arc<Vec<Item>>,
    report: Report,
    tab: Tab,
    pwned: Pwned,
    pwned_url: String,
}

impl EventEmitter<HealthEvent> for HealthView {}

impl HealthView {
    pub fn new(items: Vec<Item>, changed_at: &HashMap<String, u64>) -> Self {
        let report = health::analyze(&items, changed_at, crate::flow::now_secs());
        // Opens on the first tab with something to show.
        let tab = if !report.weak.is_empty() {
            Tab::Weak
        } else if !report.reused.is_empty() {
            Tab::Reused
        } else if !report.old.is_empty() {
            Tab::Old
        } else {
            Tab::Exposed
        };
        Self {
            items: Arc::new(items),
            report,
            tab,
            pwned: Pwned::NotChecked,
            pwned_url: health::PWNED_API.to_owned(),
        }
    }

    pub fn report(&self) -> &Report {
        &self.report
    }

    pub fn tab(&self) -> Tab {
        self.tab
    }

    pub fn pwned(&self) -> &Pwned {
        &self.pwned
    }

    /// Another address for the leak API (tests use a fake server).
    pub fn set_pwned_url(&mut self, url: impl Into<String>) {
        self.pwned_url = url.into();
    }

    pub fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        cx.notify();
    }

    /// How many items the tab shows (`None`: leaks not checked yet).
    pub fn count(&self, tab: Tab) -> Option<usize> {
        match tab {
            Tab::Weak => Some(self.report.weak.len()),
            Tab::Reused => Some(self.report.reused.iter().map(Vec::len).sum()),
            Tab::Old => Some(self.report.old.len()),
            Tab::Exposed => match &self.pwned {
                Pwned::Done(found) => Some(found.len()),
                _ => None,
            },
        }
    }

    /// Items with some problem, counting the leaked ones (if already checked).
    fn affected(&self) -> usize {
        let mut ids: Vec<&String> = self.report.weak.iter().map(|(id, _)| id).collect();
        ids.extend(self.report.reused.iter().flatten());
        ids.extend(self.report.old.iter().map(|(id, _)| id));
        if let Pwned::Done(found) = &self.pwned {
            ids.extend(found.iter().map(|(id, _)| id));
        }
        ids.sort();
        ids.dedup();
        ids.len()
    }

    /// Checks for leaks (only hash prefixes leave the machine).
    pub fn check_pwned(&mut self, cx: &mut Context<Self>) {
        if self.pwned == Pwned::Checking {
            return;
        }
        let passwords: Vec<(String, Zeroizing<String>)> = self
            .items
            .iter()
            .filter_map(|i| {
                health::login_password(i).map(|p| (i.id.clone(), Zeroizing::new(p.to_owned())))
            })
            .collect();
        let url = self.pwned_url.clone();
        self.pwned = Pwned::Checking;
        self.tab = Tab::Exposed;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = crate::io::run(async move {
                let refs: Vec<(String, &str)> = passwords
                    .iter()
                    .map(|(id, p)| (id.clone(), p.as_str()))
                    .collect();
                health::check_pwned(&url, &refs).await
            })
            .await;
            this.update(cx, |this, cx| {
                this.pwned = match result {
                    Ok(found) => Pwned::Done(found),
                    Err(_) => Pwned::Failed(t!("health.pwned_failed").to_owned()),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn item(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    fn score(&self) -> Option<u32> {
        let checked = self.report.checked;
        (checked > 0).then(|| {
            let healthy = checked.saturating_sub(self.affected());
            ((healthy as f64 / checked as f64) * 100.).round() as u32
        })
    }

    fn summary(&self, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let score = self.score();
        let color = match score {
            Some(s) if s >= 90 => theme.success,
            Some(s) if s >= 60 => theme.warning,
            Some(_) => theme.danger,
            None => theme.muted_foreground,
        };
        let fraction = score.map_or(0., |s| s as f32 / 100.);
        let affected = self.affected();
        let headline = match (self.report.checked, affected) {
            (0, _) => t!("health.nothing_to_check").to_owned(),
            (_, 0) => t!("health.all_good").to_owned(),
            (_, 1) => t!("health.one_needs_attention").to_owned(),
            (_, n) => t!("health.many_need_attention", n = n),
        };
        h_flex()
            .gap_5()
            .p_5()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .child(
                v_flex()
                    .items_center()
                    .w(px(96.))
                    .child(
                        div()
                            .id("health-score")
                            .test_support()
                            .aria_label(SharedString::from(
                                score.map_or("—".to_owned(), |s| format!("{s}%")),
                            ))
                            .text_3xl()
                            .font_semibold()
                            .text_color(color)
                            .child(motion::pop(
                                div().child(score.map_or("—".to_owned(), |s| format!("{s}%"))),
                                motion::key("saude-nota", score.unwrap_or(0)),
                            )),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(t!("health.healthy")),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .gap_2()
                    .child(div().text_base().font_semibold().child(headline))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(t!("health.checked", n = self.report.checked)),
                    )
                    .child(
                        // Score bar: grows to the percentage on open.
                        div()
                            .h(px(6.))
                            .w_full()
                            .rounded_full()
                            .bg(theme.muted)
                            .child(
                                div().h_full().rounded_full().bg(color).with_animation(
                                    motion::key("saude-barra", score.unwrap_or(0)),
                                    Animation::new(std::time::Duration::from_millis(700))
                                        .with_easing(ease_out_quint()),
                                    move |el, t| el.w(relative(fraction * t)),
                                ),
                            ),
                    ),
            )
    }

    fn tab_card(&self, tab: Tab, order: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let accent = appearance::accent(cx);
        let selected = self.tab == tab;
        let count = self.count(tab);
        let number_color = match count {
            Some(0) => theme.success,
            Some(_) => theme.danger,
            None => theme.muted_foreground,
        };
        let checking = tab == Tab::Exposed && self.pwned == Pwned::Checking;
        let card = v_flex()
            .id(tab.id())
            .test_support()
            .role(Role::Tab)
            .aria_label(SharedString::from(format!(
                "{}: {}",
                tab.label(),
                count.map_or("?".to_owned(), |c| c.to_string())
            )))
            .flex_1()
            .gap_2()
            .p_3()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(if selected { accent } else { theme.border })
            .bg(if selected {
                accent.opacity(0.08)
            } else {
                theme.popover
            })
            .cursor_pointer()
            .hover(|s| s.bg(accent.opacity(0.06)))
            .active(|s| s.opacity(0.8))
            .child(
                h_flex()
                    .justify_between()
                    .child(ui::icon_chip(tab.icon(), px(28.), selected, cx))
                    .child(
                        div()
                            .text_2xl()
                            .font_semibold()
                            .text_color(number_color)
                            .child(if checking {
                                "…".to_owned()
                            } else {
                                count.map_or("?".to_owned(), |c| c.to_string())
                            }),
                    ),
            )
            .child(div().text_sm().font_semibold().child(tab.label()))
            .on_click(cx.listener(move |this, _, _, cx| this.set_tab(tab, cx)));
        motion::cascade(
            div().flex_1().flex().child(card),
            motion::key("saude-aba", tab.id()),
            order,
        )
    }

    fn item_row(
        &self,
        item: &Item,
        badge: String,
        danger: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let accent = appearance::accent(cx);
        let username = match &item.kind {
            ItemKind::Login(l) => l.username.clone().unwrap_or_default(),
            _ => String::new(),
        };
        let id = item.id.clone();
        h_flex()
            .id(SharedString::from(format!("health-item-{}", item.id)))
            .test_support()
            .role(Role::Button)
            .aria_label(SharedString::from(item.name.clone()))
            .gap_3()
            .px_3()
            .py_2()
            .rounded(theme.radius)
            .cursor_pointer()
            .hover(move |s| s.bg(accent.opacity(0.08)))
            .active(|s| s.opacity(0.8))
            .child(crate::item_icons::chip(item, px(32.), false, cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .font_semibold()
                            .child(item.name.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(if username.is_empty() {
                                " ".to_owned()
                            } else {
                                username
                            }),
                    ),
            )
            .child(
                div()
                    .px_2()
                    .py_0p5()
                    .rounded_full()
                    .text_xs()
                    .font_semibold()
                    .bg(if danger {
                        theme.danger.opacity(0.12)
                    } else {
                        theme.warning.opacity(0.14)
                    })
                    .text_color(if danger { theme.danger } else { theme.warning })
                    .child(badge),
            )
            .child(
                Icon::new(IconName::ChevronRight)
                    .small()
                    .text_color(theme.muted_foreground),
            )
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(HealthEvent::OpenItem(id.clone()))))
    }

    fn empty(text: &'static str, cx: &App) -> AnyElement {
        let theme = cx.theme();
        v_flex()
            .items_center()
            .gap_2()
            .py_10()
            .text_color(theme.muted_foreground)
            .child(
                Icon::new(IconName::CircleCheck)
                    .size(px(28.))
                    .text_color(theme.success),
            )
            .child(div().text_sm().child(text))
            .into_any_element()
    }

    fn list(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut list = v_flex().gap_0p5();
        match self.tab {
            Tab::Weak => {
                if self.report.weak.is_empty() {
                    return Self::empty(t!("health.no_weak"), cx);
                }
                for (id, bits) in self.report.weak.clone() {
                    if let Some(item) = self.item(&id).cloned() {
                        let label = if bits < 28. {
                            t!("health.very_weak")
                        } else {
                            t!("health.weak")
                        };
                        list = list.child(self.item_row(&item, label.into(), bits < 28., cx));
                    }
                }
            }
            Tab::Reused => {
                if self.report.reused.is_empty() {
                    return Self::empty(t!("health.no_reused"), cx);
                }
                for (n, group) in self.report.reused.clone().into_iter().enumerate() {
                    list = list.child(
                        div()
                            .px_3()
                            .pt(if n == 0 { px(0.) } else { px(12.) })
                            .pb_1()
                            .text_xs()
                            .font_semibold()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("health.same_password", n = group.len())),
                    );
                    for id in &group {
                        if let Some(item) = self.item(id).cloned() {
                            let badge = t!("health.in_items", n = group.len());
                            list = list.child(self.item_row(&item, badge, true, cx));
                        }
                    }
                }
            }
            Tab::Old => {
                if self.report.old.is_empty() {
                    return Self::empty(t!("health.no_old"), cx);
                }
                for (id, days) in self.report.old.clone() {
                    if let Some(item) = self.item(&id).cloned() {
                        let years = days / 365;
                        let badge = if years >= 2 {
                            t!("health.years_ago", n = years)
                        } else {
                            t!("health.days_ago", n = days)
                        };
                        list = list.child(self.item_row(&item, badge, false, cx));
                    }
                }
            }
            Tab::Exposed => return self.exposed(cx),
        }
        list.into_any_element()
    }

    fn exposed(&self, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let explain = div()
            .text_xs()
            .text_color(muted)
            .child(t!("health.pwned_explain"));
        match &self.pwned {
            Pwned::NotChecked | Pwned::Failed(_) => {
                let failed = match &self.pwned {
                    Pwned::Failed(e) => Some(e.clone()),
                    _ => None,
                };
                v_flex()
                    .gap_3()
                    .p_4()
                    .child(explain)
                    .when_some(failed, |el, e| {
                        el.child(div().text_sm().text_color(cx.theme().danger).child(e))
                    })
                    .child(
                        h_flex().child(
                            Button::new("health-check-pwned")
                                .primary()
                                .icon(IconName::ShieldCheck)
                                .label(t!("health.check_pwned"))
                                .on_click(cx.listener(|this, _, _, cx| this.check_pwned(cx))),
                        ),
                    )
                    .into_any_element()
            }
            Pwned::Checking => v_flex()
                .gap_3()
                .p_4()
                .child(explain)
                .child(
                    h_flex()
                        .gap_2()
                        .text_sm()
                        .child(
                            Icon::new(IconName::RefreshCw)
                                .small()
                                .text_color(appearance::accent(cx))
                                .with_animation(
                                    "saude-girando",
                                    Animation::new(std::time::Duration::from_millis(900)).repeat(),
                                    |icon, t| icon.rotate(radians(t * std::f32::consts::TAU)),
                                ),
                        )
                        .child(t!("health.checking")),
                )
                .into_any_element(),
            Pwned::Done(found) => {
                if found.is_empty() {
                    return Self::empty(t!("health.no_exposed"), cx);
                }
                let mut list = v_flex().gap_0p5();
                for (id, times) in found.clone() {
                    if let Some(item) = self.item(&id).cloned() {
                        let badge = t!("health.leaked_times", n = thousands(times));
                        list = list.child(self.item_row(&item, badge, true, cx));
                    }
                }
                list.into_any_element()
            }
        }
    }
}

/// 9659365 → "9.659.365".
fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push_str(t!("health.thousands_sep"));
        }
        out.push(c);
    }
    out
}

impl Render for HealthView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (radius, border, popover, muted) = (
            theme.radius_lg,
            theme.border,
            theme.popover,
            theme.muted_foreground,
        );
        let header = h_flex()
            .justify_between()
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_2xl().font_semibold().child(t!("health.title")))
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child(t!("health.subtitle")),
                    ),
            )
            .child(
                Button::new("health-close")
                    .ghost()
                    .small()
                    .icon(IconName::Close)
                    .tooltip(t!("common.close_esc"))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(HealthEvent::Close))),
            );
        let mut tabs = h_flex().gap_3();
        for (order, tab) in Tab::ALL.into_iter().enumerate() {
            tabs = tabs.child(self.tab_card(tab, order, cx));
        }
        let list = self.list(cx);
        let tab = self.tab;
        v_flex()
            .id("health-view")
            .size_full()
            .overflow_y_scroll()
            .p_8()
            .gap_5()
            .child(header.flex_shrink_0())
            .child(div().flex_shrink_0().child(self.summary(cx)))
            .child(tabs.flex_shrink_0())
            .child(motion::rise(
                div()
                    .flex_shrink_0()
                    .rounded(radius)
                    .border_1()
                    .border_color(border)
                    .bg(popover)
                    .p_2()
                    .child(list),
                motion::key("saude-lista", tab.id()),
                4.,
            ))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn thousands_separators() {
        assert_eq!(super::thousands(9_659_365), "9.659.365");
        assert_eq!(super::thousands(42), "42");
        assert_eq!(super::thousands(1000), "1.000");
    }
}
