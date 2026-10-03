//! The Account tab: display name, status, identity, the two config-file switches, version
//! and the update check. Applying an update is the status bar's chip, not this tab's.

use super::controls::{
    code_inline, control_row, help, help_rich, label, select, switch, Body, Piece,
};
use super::{text_field, SettingsPane};
use crate::ui::commands;
use crate::ui::icons::Icon;
use crate::ui::state::UpdateState;
use crate::ui::view::widgets::{Btn, BtnKind};
use crate::ui::view::AppSnapshot;
use gpui_kit::{
    div, px, relative, ClipboardItem, Context, IntoElement, ParentElement, Styled, Window,
};

const STATUSES: [(&str, &str); 3] = [("online", "Online"), ("away", "Away"), ("busy", "Busy")];

/// What the Account tab says next to "Check for updates". The full error is spelled out
/// rather than summarised: every way this feed can break is a sentence the user needs.
pub(super) fn update_word(update: Option<&UpdateState>) -> String {
    match update {
        Some(UpdateState::Checking) => "Checking\u{2026}".into(),
        Some(UpdateState::UpToDate) => "You are on the newest version.".into(),
        Some(UpdateState::Downloading { version, .. }) => format!("Fetching {version}\u{2026}"),
        Some(UpdateState::Ready { version }) => {
            format!("Version {version} is downloaded and verified.")
        }
        Some(UpdateState::Failed { error }) if !error.is_empty() => error.clone(),
        Some(UpdateState::Failed { .. }) => "The check failed.".into(),
        None => String::new(),
    }
}

impl SettingsPane {
    pub(super) fn account(
        &mut self,
        snap: &AppSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Body {
        let s = &snap.s;
        self.sync_name(&s.me.name, window, cx);
        let wanted = STATUSES
            .iter()
            .find(|(id, _)| *id == s.me.status)
            .map_or("Online", |(_, label)| label);
        self.status.sync(
            STATUSES.iter().map(|(_, l)| l.to_string()).collect(),
            wanted,
            window,
            cx,
        );

        let checking = matches!(
            s.update,
            Some(UpdateState::Checking | UpdateState::Downloading { .. })
        );
        let ui = snap.ui.clone();
        let id = s.me.id.clone();

        Body::new()
            .h2("Account")
            .p(concat!(
                "Your identity is a UUID generated on first start and stored locally. The display ",
                "name is cosmetic and can change at any time; the others see the new name without ",
                "restarting."
            ))
            .field(
                div()
                    .child(label("Display name"))
                    .child(help("How the other two see you."))
                    .child(
                        control_row().child(text_field(&self.name, 240.)).child(
                            Btn::new("save-name", "Save")
                                .kind(BtnKind::Accent)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let name = this.name.read(cx).value();
                                    commands::set_display_name(&this.ui, &name);
                                })),
                        ),
                    ),
            )
            .field(
                div()
                    .child(label("Status"))
                    .child(help(concat!(
                        "What the others see next to your name. Volatile: after a restart you are ",
                        "plain online again."
                    )))
                    .child(control_row().child(select("set-status", &self.status, 240., false))),
            )
            .field(
                div()
                    .child(label("Identity"))
                    .child(help(concat!(
                        "Never copy this between machines. Two peers sharing one identity break ",
                        "chat synchronisation."
                    )))
                    .child(
                        control_row().child(code_inline(&id)).child(
                            Btn::new("copy-identity", "Copy")
                                .kind(BtnKind::Ghost)
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(id.clone()))
                                }),
                        ),
                    ),
            )
            .field(switch_row(
                "Start with Windows",
                &[
                    Piece::Text("Set in "),
                    Piece::Mono("config.toml"),
                    Piece::Text(" and applied at every start."),
                ],
                "set-autostart",
                s.autostart,
            ))
            .field(switch_row(
                "Close button hides to tray",
                &[
                    Piece::Text(concat!(
                        "On by default, so closing the window does not stop the app while you are ",
                        "gaming. Set in "
                    )),
                    Piece::Mono("config.toml"),
                    Piece::Text("."),
                ],
                "set-tray",
                s.minimize_to_tray,
            ))
            .field(
                div()
                    .child(label("Version"))
                    .child(help(concat!(
                        "Updates come from the signed release feed on GitHub, checked every six ",
                        "hours. Only a build signed with the release key is ever accepted."
                    )))
                    .child(
                        control_row()
                            .child(code_inline(&format!("v{}", s.app_version)))
                            .child(
                                Btn::new("btn-check-update", "Check for updates")
                                    .kind(BtnKind::Ghost)
                                    .icon(Icon::Retry)
                                    .disabled(checking)
                                    .on_click(move |_, _, _| commands::check_update(&ui)),
                            )
                            .child(
                                div()
                                    .max_w(super::controls::ch(12., 58.))
                                    .text_size(px(12.))
                                    .line_height(relative(1.5))
                                    .text_color(crate::ui::theme::c::SILVER)
                                    .child(update_word(s.update.as_ref())),
                            ),
                    ),
            )
    }
}

/// A label and its help on the left, a switch on the right. Both of these are settings of
/// `config.toml`, so the switch has no handler.
fn switch_row(
    title: &'static str,
    help_text: &[Piece],
    id: &'static str,
    on: bool,
) -> impl IntoElement {
    div()
        .flex()
        .items_start()
        .gap(px(12.))
        .child(
            div()
                .flex_1()
                .child(label(title).mb(px(2.)))
                .child(help_rich(help_text)),
        )
        .child(switch(id, on, None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_words_are_the_ones_of_the_old_tab() {
        assert_eq!(update_word(None), "");
        assert_eq!(
            update_word(Some(&UpdateState::Checking)),
            "Checking\u{2026}"
        );
        assert_eq!(
            update_word(Some(&UpdateState::UpToDate)),
            "You are on the newest version."
        );
        let downloading = UpdateState::Downloading {
            version: "1.8.0".into(),
            received: 1,
            total: 2,
        };
        assert_eq!(update_word(Some(&downloading)), "Fetching 1.8.0\u{2026}");
        let ready = UpdateState::Ready {
            version: "1.8.0".into(),
        };
        assert_eq!(
            update_word(Some(&ready)),
            "Version 1.8.0 is downloaded and verified."
        );
        let failed = UpdateState::Failed {
            error: "feed unreachable".into(),
        };
        assert_eq!(update_word(Some(&failed)), "feed unreachable");
        let silent = UpdateState::Failed {
            error: String::new(),
        };
        assert_eq!(update_word(Some(&silent)), "The check failed.");
    }
}
