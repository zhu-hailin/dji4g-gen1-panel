//! Carrier balance queries prepare an ordinary draft; only the compose confirmation sends it.
use eframe::egui::{self, Ui};

use crate::localization::{Language, LocalizedText, TextKey};

use super::sms::{ReplyStartResult, SmsComposeState};

struct BalanceTemplate {
    carrier: TextKey,
    recipient: &'static str,
    body: &'static str,
    instructions: &'static str,
}

const TEMPLATES: [BalanceTemplate; 3] = [
    BalanceTemplate {
        carrier: TextKey::CarrierChinaMobile,
        recipient: "10086",
        body: "YE",
        instructions: "https://wap.gd.10086.cn/wap2012/FAQ/Qdetails_huafei_1.shtml",
    },
    BalanceTemplate {
        carrier: TextKey::CarrierChinaUnicom,
        recipient: "10010",
        body: "CXYE",
        instructions: "https://iservice.10010.com/elecService/message.html",
    },
    BalanceTemplate {
        carrier: TextKey::CarrierChinaTelecom,
        recipient: "10001",
        body: "102",
        instructions: "https://m.gd.189.cn/gd/sms/",
    },
];

fn t(language: Language, key: TextKey) -> String {
    LocalizedText::new(language, key).text
}

/// Returns true only when a template is ready to review, including draft replacement confirmation.
pub(crate) fn render_entry(ui: &mut Ui, language: Language, state: &mut SmsComposeState) -> bool {
    let mut prepared = false;
    let entry = ui.menu_button(t(language, TextKey::SmsBalanceQuery), |ui| {
        ui.set_max_width(340.0);
        ui.label(t(language, TextKey::SmsBalanceIntro));
        ui.add_space(4.0);
        for template in &TEMPLATES {
            ui.horizontal_wrapped(|ui| {
                let label = format!(
                    "{} · {} → {}",
                    t(language, template.carrier),
                    template.body,
                    template.recipient
                );
                let choice = ui.add_enabled(
                    !state.module_read_only && !state.serial_busy,
                    egui::Button::new(label),
                );
                #[cfg(test)]
                ui.data_mut(|data| {
                    data.insert_temp(
                        egui::Id::new(("balance-template-rect", template.recipient)),
                        choice.rect,
                    )
                });
                if choice.clicked()
                    && matches!(
                        state.begin_balance_query(template.recipient, template.body),
                        ReplyStartResult::Opened | ReplyStartResult::ReplacementConfirmation
                    )
                {
                    prepared = true;
                    ui.close_menu();
                }
                ui.hyperlink_to(
                    t(language, TextKey::SmsBalanceInstructions),
                    template.instructions,
                );
            });
        }
        ui.separator();
        ui.label(t(language, TextKey::CarrierChinaBroadnet));
        let portal = ui.hyperlink_to(
            t(language, TextKey::SmsBalancePortal),
            "https://www.10099.com.cn/",
        );
        #[cfg(test)]
        ui.data_mut(|data| data.insert_temp(egui::Id::new("balance-portal-rect"), portal.rect));
        if portal.clicked() {
            ui.close_menu();
        }
        ui.label(t(language, TextKey::SmsBalanceBroadnetHelp));
        ui.separator();
        ui.label(t(language, TextKey::SmsBalanceReplyHint));
    });
    #[cfg(test)]
    ui.data_mut(|data| data.insert_temp(egui::Id::new("balance-entry-rect"), entry.response.rect));
    let _ = entry;
    prepared
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(
        ctx: &egui::Context,
        state: &mut SmsComposeState,
        language: Language,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, bool) {
        let mut prepared = false;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(500.0, 800.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    prepared = render_entry(ui, language, state);
                });
            },
        );
        (output, prepared)
    }

    fn click(
        ctx: &egui::Context,
        state: &mut SmsComposeState,
        language: Language,
        point: egui::Pos2,
    ) -> (egui::FullOutput, bool) {
        let _ = frame(
            ctx,
            state,
            language,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        frame(
            ctx,
            state,
            language,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        )
    }

    fn rect(ctx: &egui::Context, id: egui::Id) -> egui::Rect {
        ctx.data(|data| data.get_temp::<egui::Rect>(id))
            .expect("rendered balance control")
    }

    fn open_menu(ctx: &egui::Context, state: &mut SmsComposeState, language: Language) {
        for _ in 0..3 {
            assert!(!frame(ctx, state, language, vec![]).1);
        }
        assert!(
            !click(
                ctx,
                state,
                language,
                rect(ctx, egui::Id::new("balance-entry-rect")).center()
            )
            .1
        );
        for _ in 0..3 {
            assert!(!frame(ctx, state, language, vec![]).1);
        }
    }

    #[test]
    fn every_balance_template_is_a_valid_single_message() {
        for template in &TEMPLATES {
            let pdu = dji4g_at_protocol::build_ucs2_submit(template.recipient, template.body);
            assert!(pdu.is_ok(), "invalid template for {}", template.recipient);
        }
    }

    #[test]
    fn template_click_only_reports_a_reviewable_draft_in_every_language() {
        for language in [Language::ZhCn, Language::ZhTw, Language::EnUs] {
            for template in &TEMPLATES {
                for keep_original in [false, true] {
                    let ctx = egui::Context::default();
                    crate::ui::apply_style(&ctx);
                    let mut state = SmsComposeState::default();
                    if keep_original {
                        state.review_editor();
                    }
                    open_menu(&ctx, &mut state, language);
                    let point = rect(
                        &ctx,
                        egui::Id::new(("balance-template-rect", template.recipient)),
                    )
                    .center();
                    assert!(click(&ctx, &mut state, language, point).1);
                    assert!(state.open);
                    assert!(!state.search_shortcuts_allowed());
                    assert!(
                        !frame(&ctx, &mut state, language, vec![]).1,
                        "navigation is a one-shot result"
                    );
                }
            }
        }
    }

    #[test]
    fn blocked_template_and_broadnet_link_never_report_a_prepared_draft() {
        for read_only in [false, true] {
            let ctx = egui::Context::default();
            crate::ui::apply_style(&ctx);
            let mut state = SmsComposeState::default();
            state.module_read_only = read_only;
            state.serial_busy = !read_only;
            open_menu(&ctx, &mut state, Language::ZhCn);
            let point = rect(&ctx, egui::Id::new(("balance-template-rect", "10010"))).center();
            assert!(!click(&ctx, &mut state, Language::ZhCn, point).1);
            assert!(!state.open);
        }
        let ctx = egui::Context::default();
        crate::ui::apply_style(&ctx);
        let mut state = SmsComposeState::default();
        open_menu(&ctx, &mut state, Language::ZhCn);
        let point = rect(&ctx, egui::Id::new("balance-portal-rect")).center();
        let (output, prepared) = click(&ctx, &mut state, Language::ZhCn, point);
        assert!(!prepared);
        assert!(!state.open);
        assert_eq!(
            output
                .platform_output
                .open_url
                .expect("portal opens in browser")
                .url,
            "https://www.10099.com.cn/"
        );
    }
}
