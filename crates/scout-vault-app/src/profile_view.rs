//! "Profile" screen: who is signed in, the account as the server describes it,
//! the vault in numbers and what this computer remembers.
//!
//! Nothing here is secret: name, email, flags and counts. Changing the account
//! (password, 2FA, email) happens in the web vault, which the screen opens.

use gpui_kit::assets::IconName;
use gpui_kit::component::{button::*, *};
use gpui_kit::prelude::*;
use gpui_kit::*;
use scout_vault_core::vault::{ItemKind, Vault};

use crate::flow::Session;
use crate::settings::Settings;
use crate::ui;

pub enum ProfileEvent {
    Close,
    OpenSettings,
    SignOut,
}

/// What the screen shows (computed once when it opens).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileInfo {
    pub name: Option<String>,
    pub email: String,
    pub premium: bool,
    pub premium_from_organization: bool,
    pub email_verified: bool,
    pub two_factor: bool,
    /// Account creation, seconds since 1970.
    pub created: Option<u64>,
    pub avatar_color: Option<String>,
    /// `bitwarden.com`, `bitwarden.eu` or the self-hosted server's host.
    pub server: String,
    pub web_vault: String,
    pub sso: bool,
    pub device: String,
    pub organizations: Vec<String>,
    /// Logins, cards, identities, notes, SSH keys, folders.
    pub counts: [usize; 6],
    pub remember_session: bool,
    pub unlock_memory_days: u32,
    pub windows_hello: bool,
    pub auto_lock_minutes: u32,
}

impl ProfileInfo {
    pub fn new(session: &Session, vault: &Vault, settings: &Settings) -> Self {
        let profile = &session.sync.profile;
        let urls = session.client.urls();
        let host = crate::browser::host_of(&urls.identity).unwrap_or_default();
        let server = host.strip_prefix("identity.").unwrap_or(&host).to_owned();
        let web_vault = if urls.vault.is_empty() {
            format!("https://{}", server)
        } else {
            urls.vault.trim_end_matches('/').to_owned()
        };
        let mut counts = [0usize; 6];
        for item in vault.items.iter().filter(|i| !i.deleted) {
            let ix = match item.kind {
                ItemKind::Login(_) => 0,
                ItemKind::Card(_) => 1,
                ItemKind::Identity(_) => 2,
                ItemKind::SecureNote => 3,
                ItemKind::SshKey(_) => 4,
                ItemKind::Unknown(_) => continue,
            };
            counts[ix] += 1;
        }
        counts[5] = vault.folders.len();
        let mut organizations: Vec<String> = profile
            .organizations
            .iter()
            .filter_map(|o| o.name.clone())
            .filter(|n| !n.trim().is_empty())
            .collect();
        organizations.sort_by_key(|n| n.to_lowercase());
        Self {
            name: profile.name.clone().filter(|n| !n.trim().is_empty()),
            email: session.email.clone(),
            premium: profile.premium,
            premium_from_organization: profile.premium_from_organization,
            email_verified: profile.email_verified,
            two_factor: profile.two_factor_enabled,
            created: profile
                .creation_date
                .as_deref()
                .and_then(scout_vault_core::health::parse_rfc3339),
            avatar_color: profile.avatar_color.clone(),
            server,
            web_vault,
            sso: session.client.is_sso(),
            device: settings.device.name.clone(),
            organizations,
            counts,
            remember_session: settings.remember_session,
            unlock_memory_days: settings.unlock_memory_days,
            windows_hello: settings.windows_hello,
            auto_lock_minutes: settings.auto_lock_minutes,
        }
    }

    /// Up to two initials: from the name, or else from the email.
    pub fn initials(&self) -> String {
        initials_of(self.name.as_deref(), &self.email)
    }
}

/// `dd/mm/aaaa` (pt-BR) or `yyyy-mm-dd` (other languages) from seconds since 1970.
pub fn format_date(secs: u64) -> String {
    // Days → civil date (Howard Hinnant's algorithm).
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    if crate::i18n::current().code().starts_with("pt") {
        format!("{day:02}/{month:02}/{year}")
    } else {
        format!("{year}-{month:02}-{day:02}")
    }
}

/// `#rrggbb` → color.
fn parse_hex(hex: &str) -> Option<Hsla> {
    let hex = hex.trim().strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(hex, 16).ok()?;
    Some(rgb(n).into())
}

/// Up to two initials: from the name, or else from the email.
pub fn initials_of(name: Option<&str>, email: &str) -> String {
    let source = name
        .filter(|n| !n.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| email.split('@').next().unwrap_or_default().to_owned());
    let words: Vec<&str> = source
        .split(|c: char| c.is_whitespace() || c == '.' || c == '_' || c == '-')
        .filter(|w| !w.is_empty())
        .collect();
    let mut out: String = words
        .iter()
        .take(2)
        .filter_map(|w| w.chars().next())
        .collect();
    if out.chars().count() < 2 {
        out = source.chars().take(2).collect();
    }
    out.to_uppercase()
}

/// The avatar's color: the one chosen in the web vault, or the accent.
pub fn avatar_color(chosen: Option<&str>, cx: &App) -> Hsla {
    chosen
        .and_then(parse_hex)
        .unwrap_or_else(|| crate::appearance::accent(cx))
}

/// Round avatar with the initials.
pub fn avatar(initials: String, color: Hsla, size: Pixels) -> Div {
    div()
        .size(size)
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(color)
        .text_color(gpui_kit::white())
        .text_size(size * 0.38)
        .font_semibold()
        .child(initials)
}

pub struct ProfileView {
    info: ProfileInfo,
    /// "Sign out" asks for a second click.
    confirm_sign_out: bool,
}

impl EventEmitter<ProfileEvent> for ProfileView {}

/// A color from the item icon palette (`blue`, `amber`…), adjusted to the theme.
fn hue(id: &str, cx: &App) -> Hsla {
    crate::item_icons::color_by_id(id, cx).unwrap_or_else(|| crate::appearance::accent(cx))
}

impl ProfileView {
    pub fn new(info: ProfileInfo) -> Self {
        Self {
            info,
            confirm_sign_out: false,
        }
    }

    pub fn info(&self) -> &ProfileInfo {
        &self.info
    }

    pub fn sign_out(&mut self, cx: &mut Context<Self>) {
        if self.confirm_sign_out {
            cx.emit(ProfileEvent::SignOut);
        } else {
            self.confirm_sign_out = true;
            cx.notify();
        }
    }

    fn badge(
        id: &'static str,
        icon: IconName,
        text: SharedString,
        color: Hsla,
    ) -> impl IntoElement {
        h_flex()
            .id(id)
            .test_support()
            .aria_label(text.clone())
            .gap_1()
            .px_2()
            .py_0p5()
            .rounded_full()
            .border_1()
            .border_color(color.opacity(0.45))
            .bg(color.opacity(0.14))
            .text_xs()
            .font_semibold()
            .text_color(color)
            .child(Icon::new(icon).xsmall())
            .child(text)
    }

    /// Small colored square with an icon, at the start of a row.
    fn chip(icon: IconName, color: Hsla) -> impl IntoElement {
        div()
            .size(px(28.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(7.))
            .bg(color.opacity(0.16))
            .text_color(color)
            .child(Icon::new(icon).small())
    }

    fn card(title: SharedString, rows: Vec<AnyElement>, cx: &App) -> impl IntoElement {
        let border = cx.theme().border;
        let count = rows.len();
        v_flex()
            .gap_2()
            .child(ui::section_label(title, cx).px_0())
            .child(
                v_flex()
                    .rounded(cx.theme().radius_lg)
                    .border_1()
                    .border_color(border)
                    .bg(cx.theme().popover)
                    .children(rows.into_iter().enumerate().map(move |(ix, row)| {
                        div()
                            .px_4()
                            .py_2p5()
                            .when(ix + 1 < count, |el| el.border_b_1().border_color(border))
                            .child(row)
                    })),
            )
    }

    fn row(icon: IconName, color: Hsla, label: SharedString, value: AnyElement) -> AnyElement {
        h_flex()
            .justify_between()
            .gap_4()
            .child(
                h_flex()
                    .gap_3()
                    .child(Self::chip(icon, color))
                    .child(div().text_sm().child(label)),
            )
            .child(value)
            .into_any_element()
    }

    fn text_value(value: SharedString, cx: &App) -> AnyElement {
        div()
            .min_w_0()
            .truncate()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(value)
            .into_any_element()
    }

    /// "On" in green, "Off" in gray (or any text with its color).
    fn pill(text: SharedString, color: Hsla) -> AnyElement {
        h_flex()
            .gap_1p5()
            .px_2()
            .py_0p5()
            .rounded_full()
            .bg(color.opacity(0.14))
            .text_xs()
            .font_semibold()
            .text_color(color)
            .child(div().size(px(6.)).rounded_full().bg(color))
            .child(text)
            .into_any_element()
    }
}

impl Render for ProfileView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let info = self.info.clone();
        let theme = cx.theme();
        let (muted, border, success, warning) = (
            theme.muted_foreground,
            theme.border,
            theme.success,
            theme.warning,
        );
        let color = avatar_color(info.avatar_color.as_deref(), cx);
        let (c_blue, c_violet, c_amber, c_teal, c_green, c_orange, c_pink) = (
            hue("blue", cx),
            hue("violet", cx),
            hue("amber", cx),
            hue("teal", cx),
            hue("green", cx),
            hue("orange", cx),
            hue("pink", cx),
        );

        let display_name: SharedString = info
            .name
            .clone()
            .unwrap_or_else(|| info.email.split('@').next().unwrap_or_default().to_owned())
            .into();
        // Header: a band in the avatar's color, the avatar and the badges.
        let hero = v_flex()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(border)
            .overflow_hidden()
            .bg(linear_gradient(
                160.,
                linear_color_stop(color.opacity(0.30), 0.),
                linear_color_stop(color.opacity(0.04), 1.),
            ))
            .p_5()
            .child(
                h_flex()
                    .gap_4()
                    .child(
                        div()
                            .id("profile-avatar")
                            .p_1()
                            .rounded_full()
                            .bg(color.opacity(0.35))
                            .child(avatar(info.initials(), color, px(68.))),
                    )
                    .child(
                        v_flex()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .id("profile-name")
                                    .test_support()
                                    .aria_label(display_name.clone())
                                    .text_2xl()
                                    .font_semibold()
                                    .truncate()
                                    .child(display_name),
                            )
                            .child(
                                div()
                                    .id("profile-email")
                                    .test_support()
                                    .aria_label(SharedString::from(info.email.clone()))
                                    .text_sm()
                                    .text_color(muted)
                                    .truncate()
                                    .child(info.email.clone()),
                            )
                            .child(
                                h_flex()
                                    .flex_wrap()
                                    .gap_1p5()
                                    .pt_1()
                                    .child(Self::badge(
                                        "profile-premium",
                                        IconName::Star,
                                        if info.premium && info.premium_from_organization {
                                            t!("profile.premium_org").into()
                                        } else if info.premium {
                                            t!("profile.premium").into()
                                        } else {
                                            t!("profile.free").into()
                                        },
                                        if info.premium { c_amber } else { muted },
                                    ))
                                    .child(Self::badge(
                                        "profile-email-verified",
                                        if info.email_verified {
                                            IconName::CircleCheck
                                        } else {
                                            IconName::TriangleAlert
                                        },
                                        if info.email_verified {
                                            t!("profile.email_verified").into()
                                        } else {
                                            t!("profile.email_not_verified").into()
                                        },
                                        if info.email_verified {
                                            success
                                        } else {
                                            warning
                                        },
                                    ))
                                    .child(Self::badge(
                                        "profile-2fa",
                                        if info.two_factor {
                                            IconName::ShieldCheck
                                        } else {
                                            IconName::ShieldAlert
                                        },
                                        if info.two_factor {
                                            t!("profile.two_factor_on").into()
                                        } else {
                                            t!("profile.two_factor_off").into()
                                        },
                                        if info.two_factor { success } else { warning },
                                    )),
                            ),
                    ),
            );

        let account = Self::card(
            t!("profile.group_account").into(),
            vec![
                Self::row(
                    IconName::Server,
                    c_blue,
                    t!("profile.server").into(),
                    Self::text_value(info.server.clone().into(), cx),
                ),
                Self::row(
                    IconName::KeyRound,
                    c_violet,
                    t!("profile.sign_in").into(),
                    Self::text_value(
                        if info.sso {
                            t!("profile.sign_in_sso").into()
                        } else {
                            t!("profile.sign_in_password").into()
                        },
                        cx,
                    ),
                ),
                Self::row(
                    IconName::Calendar,
                    c_orange,
                    t!("profile.created").into(),
                    Self::text_value(
                        info.created
                            .map(format_date)
                            .unwrap_or_else(|| "—".into())
                            .into(),
                        cx,
                    ),
                ),
                Self::row(
                    IconName::Laptop,
                    c_teal,
                    t!("profile.device").into(),
                    Self::text_value(info.device.clone().into(), cx),
                ),
            ],
            cx,
        );

        // The vault in numbers: one color per kind of item.
        let tiles = [
            (t!("profile.logins"), IconName::Globe, c_blue),
            (t!("profile.cards"), IconName::CreditCard, c_amber),
            (t!("profile.identities"), IconName::IdCard, c_violet),
            (t!("profile.notes"), IconName::StickyNote, c_green),
            (t!("profile.ssh_keys"), IconName::KeyRound, c_teal),
            (t!("profile.folders"), IconName::Folder, c_orange),
        ];
        let mut stats = h_flex().flex_wrap().gap_2();
        for (ix, ((label, icon, tint), n)) in tiles.into_iter().zip(info.counts).enumerate() {
            stats = stats.child(
                v_flex()
                    .id(SharedString::from(format!("profile-count-{ix}")))
                    .test_support()
                    .aria_label(SharedString::from(format!("{label}: {n}")))
                    .w(px(98.))
                    .gap_1()
                    .px_3()
                    .py_2p5()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(tint.opacity(0.25))
                    .bg(tint.opacity(0.08))
                    .child(div().text_color(tint).child(Icon::new(icon).small()))
                    .child(div().text_xl().font_semibold().child(n.to_string()))
                    .child(div().text_xs().text_color(muted).child(label)),
            );
        }
        let vault = v_flex()
            .gap_2()
            .child(ui::section_label(t!("profile.group_vault"), cx).px_0())
            .child(stats);

        let organizations = Self::card(
            t!("profile.group_organizations").into(),
            if info.organizations.is_empty() {
                vec![div()
                    .text_sm()
                    .text_color(muted)
                    .child(t!("profile.no_organizations"))
                    .into_any_element()]
            } else {
                info.organizations
                    .iter()
                    .map(|name| {
                        h_flex()
                            .gap_3()
                            .child(Self::chip(IconName::Building2, c_pink))
                            .child(div().text_sm().child(name.clone()))
                            .into_any_element()
                    })
                    .collect()
            },
            cx,
        );

        let on_off = |on: bool| -> AnyElement {
            if on {
                Self::pill(t!("profile.on").into(), success)
            } else {
                Self::pill(t!("profile.off").into(), muted)
            }
        };
        let computer = Self::card(
            t!("profile.group_computer").into(),
            vec![
                Self::row(
                    IconName::RefreshCw,
                    c_blue,
                    t!("profile.remember_session").into(),
                    on_off(info.remember_session),
                ),
                Self::row(
                    IconName::Lock,
                    c_amber,
                    t!("profile.unlock_memory").into(),
                    Self::pill(
                        if info.unlock_memory_days == 0 {
                            t!("profile.always").into()
                        } else {
                            t!("profile.every_days", n = info.unlock_memory_days).into()
                        },
                        c_amber,
                    ),
                ),
                Self::row(
                    IconName::ScanFace,
                    c_violet,
                    t!("profile.windows_hello").into(),
                    on_off(info.windows_hello),
                ),
                Self::row(
                    IconName::Timer,
                    c_teal,
                    t!("profile.auto_lock").into(),
                    if info.auto_lock_minutes == 0 {
                        Self::pill(t!("profile.never").into(), warning)
                    } else {
                        Self::pill(
                            crate::settings_view::minutes_label(info.auto_lock_minutes),
                            success,
                        )
                    },
                ),
            ],
            cx,
        );

        let web = info.web_vault.clone();
        let actions = h_flex()
            .gap_2()
            .justify_between()
            .child(
                Button::new("profile-web-vault")
                    .outline()
                    .icon(IconName::ExternalLink)
                    .label(t!("profile.open_web_vault"))
                    .tooltip(t!("profile.open_web_vault_hint"))
                    .on_click(move |_, _, cx| {
                        if let Err(e) = crate::sso::open_browser(&web) {
                            crate::toast::error(e, cx);
                        }
                    }),
            )
            .child(
                Button::new("profile-sign-out")
                    .danger()
                    .icon(IconName::LogOut)
                    .label(if self.confirm_sign_out {
                        t!("profile.sign_out_confirm")
                    } else {
                        t!("profile.sign_out")
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.sign_out(cx))),
            );

        let back = div().absolute().top_6().left_6().child(
            Button::new("profile-close")
                .ghost()
                .small()
                .icon(IconName::ArrowLeft)
                .label(t!("common.back"))
                .tooltip(t!("common.close_esc"))
                .on_click(cx.listener(|_, _, _, cx| cx.emit(ProfileEvent::Close))),
        );

        div()
            .relative()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div().id("profile").size_full().overflow_y_scroll().child(
                    v_flex()
                        .w_full()
                        .max_w(px(680.))
                        .mx_auto()
                        .px_8()
                        .py_10()
                        .gap_8()
                        .child(hero)
                        .child(account)
                        .child(vault)
                        .child(organizations)
                        .child(computer)
                        .child(actions),
                ),
            )
            .child(back)
    }
}

#[cfg(test)]
mod tests {
    use super::{format_date, ProfileInfo};

    fn info(name: Option<&str>, email: &str) -> ProfileInfo {
        ProfileInfo {
            name: name.map(Into::into),
            email: email.into(),
            premium: false,
            premium_from_organization: false,
            email_verified: true,
            two_factor: false,
            created: None,
            avatar_color: None,
            server: "bitwarden.com".into(),
            web_vault: "https://vault.bitwarden.com".into(),
            sso: false,
            device: "Scout Vault".into(),
            organizations: Vec::new(),
            counts: [0; 6],
            remember_session: true,
            unlock_memory_days: 30,
            windows_hello: false,
            auto_lock_minutes: 5,
        }
    }

    #[test]
    fn initials_come_from_the_name_or_the_email() {
        assert_eq!(info(Some("Pessoa Teste"), "x@y.z").initials(), "PT");
        assert_eq!(info(None, "maria.silva@example.com").initials(), "MS");
        assert_eq!(info(None, "zed@example.com").initials(), "ZE");
        assert_eq!(info(Some("ana"), "x@y.z").initials(), "AN");
    }

    #[test]
    fn dates_are_civil() {
        crate::i18n::set(crate::i18n::Language::PtBr);
        assert_eq!(format_date(0), "01/01/1970");
        assert_eq!(format_date(1_700_000_000), "14/11/2023");
        assert_eq!(format_date(951_782_400), "29/02/2000");
    }
}
