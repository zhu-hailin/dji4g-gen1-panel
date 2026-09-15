//! Debug-only native screenshot review with simulated data and no-op commands.
#[cfg(debug_assertions)]
mod capture {
    use dji4g_panel::{
        app::{Page, PanelApp, PanelInputs, UiCommandSink},
        demo::{DemoScenario, demo_snapshot},
    };
    use eframe::egui;
    use std::{
        path::PathBuf,
        sync::Arc,
        time::{Duration, SystemTime},
    };
    struct Noop;
    impl UiCommandSink for Noop {
        fn try_send(
            &self,
            _: dji4g_application::UiCommand,
        ) -> Result<(), dji4g_application::UiSendError> {
            Ok(())
        }
    }
    struct Capture {
        app: PanelApp,
        output: PathBuf,
        page: usize,
        ticks: usize,
        scale: f32,
        size: egui::Vec2,
        resized: bool,
        sms_mode: bool,
        page_started: std::time::Instant,
        requested: bool,
        pages: Vec<(Page, &'static str)>,
    }
    const PAGES: [(Page, &str); 5] = [
        (Page::Overview, "overview"),
        (Page::Sms, "sms"),
        (Page::Diagnostics, "diagnostics"),
        (Page::Repairs, "repairs"),
        (Page::Settings, "settings"),
    ];
    impl eframe::App for Capture {
        fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
            if self.page >= self.pages.len() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            let images = ctx.input(|i| {
                i.events
                    .iter()
                    .filter_map(|e| {
                        if let egui::Event::Screenshot { image, .. } = e {
                            Some(image.clone())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            });
            for image in images {
                bmp(
                    &self.output.join(format!("{}.bmp", self.pages[self.page].1)),
                    &image,
                )
                .unwrap();
                self.page += 1;
                self.ticks = 0;
                self.page_started = std::time::Instant::now();
                self.requested = false;
                if self.page == self.pages.len() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
            }
            ctx.set_pixels_per_point(self.scale);
            if !self.resized && self.ticks >= 2 {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(self.size));
                self.resized = true;
            }
            self.app.set_review_page(self.pages[self.page].0);
            if self.sms_mode && self.ticks == 0 {
                self.app.set_review_sms(self.page == 1);
                if self.page == 3 {
                    self.app.set_review_sms_editor();
                }
                if self.page == 2 {
                    self.app.set_review_sms_confirmation();
                }
            }
            egui::TopBottomPanel::top("simulated-review").show(ctx, |ui| {
                ui.label("界面验收 · 模拟数据 · 不连接设备 / 不发送短信");
            });
            self.app.render(ctx, frame);
            self.ticks += 1;
            if self.ticks >= 5
                && !self.requested
                && self.page_started.elapsed() >= Duration::from_millis(500)
            {
                self.requested = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
            }
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
    fn bmp(path: &std::path::Path, image: &egui::ColorImage) -> std::io::Result<()> {
        use std::io::Write;
        let (w, h) = (image.size[0] as u32, image.size[1] as u32);
        let mut data = Vec::new();
        data.extend_from_slice(b"BM");
        data.extend_from_slice(&(54 + w * h * 4).to_le_bytes());
        data.extend_from_slice(&[0; 4]);
        data.extend_from_slice(&54u32.to_le_bytes());
        data.extend_from_slice(&40u32.to_le_bytes());
        data.extend_from_slice(&w.to_le_bytes());
        data.extend_from_slice(&h.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&32u16.to_le_bytes());
        data.extend_from_slice(&[0; 24]);
        for y in (0..h as usize).rev() {
            for x in 0..w as usize {
                let p = image.pixels[y * w as usize + x];
                data.extend_from_slice(&[p.b(), p.g(), p.r(), 255]);
            }
        }
        std::fs::File::create(path)?.write_all(&data)
    }
    pub fn main() -> eframe::Result {
        let args = std::env::args().collect::<Vec<_>>();
        let width = args
            .get(1)
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(1100.0);
        let height = args
            .get(2)
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(760.0);
        let scale = args
            .get(3)
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(1.0);
        let sms_mode = args.get(4).is_some_and(|s| s == "sms" || s == "mail");
        let wireless_mode = args.get(4).is_some_and(|s| s == "wireless");
        let error_mode = args.get(4).is_some_and(|s| s == "sms");
        let output = PathBuf::from("docs/implementation-20260915/sms-redesign-screenshots").join(
            format!("{}x{}-{}pct", width, height, (scale * 100.0) as u32),
        );
        let output = if sms_mode {
            output.join(if error_mode {
                "sms-populated"
            } else {
                "mail-workspace"
            })
        } else {
            output
        };
        std::fs::create_dir_all(&output).unwrap();
        eframe::run_native(
            "DJI 面板 · 模拟界面验收",
            eframe::NativeOptions {
                viewport: egui::ViewportBuilder::default()
                    .with_inner_size([width * scale, height * scale])
                    .with_visible(true),
                ..Default::default()
            },
            Box::new(move |cc| {
                cc.egui_ctx.set_pixels_per_point(scale);
                let mut snapshot = demo_snapshot(DemoScenario::Available, SystemTime::now());
                if sms_mode {
                    use dji4g_domain::*;
                    let mut incoming = SmsMessage::new(
                        1,
                        SmsStorageId("SM".into()),
                        1,
                        0,
                        "10086",
                        "【模拟短信】这是界面排版验收数据，不是真实运营商通知。",
                        SmsEncoding::Ucs2,
                        SmsStatus::Received,
                    );
                    incoming.read = Some(false);
                    incoming.service_centre_timestamp = Some("26/09/15,12:30:00+32".into());
                    let outgoing = SmsMessage::new_outgoing(
                        2,
                        1,
                        0,
                        "10086",
                        "【模拟发送记录】此内容未发送。",
                        SmsEncoding::Ucs2,
                        SmsStatus::Failed,
                    );
                    snapshot.sms_messages = vec![incoming, outgoing];
                    snapshot.sms_inbox = SmsInboxSummary {
                        message_count: 1,
                        unread_count: 1,
                        capacity: Some((1, 30)),
                        status: FeatureStatus::Supported,
                        has_incomplete: false,
                        evicted: 0,
                    };
                    let mut failure = SmsFailureDetail::new(
                        SmsSendPhase::WaitingForResult,
                        "sms:module_rejected",
                        true,
                    );
                    failure.cms_code = Some(500);
                    snapshot.sms_send = Some(SmsSendSnapshot {
                        request_id: 2,
                        phase: SmsSendPhase::Finished,
                        result: Some(SmsSendResult::Failed),
                        failure: Some(failure),
                    });
                }
                if sms_mode && !error_mode {
                    snapshot.sms_send = None;
                }
                let snapshot = Arc::new(snapshot);
                let (_, rx) = dji4g_application::sync::watch::channel(snapshot);
                let mut app = PanelApp::new(PanelInputs::new(rx, Arc::new(Noop), None, None), cc);
                if wireless_mode {
                    app.set_review_wireless();
                }
                Ok(Box::new(Capture {
                    app,
                    output,
                    page: 0,
                    ticks: 0,
                    scale,
                    size: egui::vec2(width, height),
                    resized: false,
                    sms_mode,
                    page_started: std::time::Instant::now(),
                    requested: false,
                    pages: if wireless_mode {
                        vec![(Page::Overview, "wireless")]
                    } else if sms_mode {
                        vec![
                            (Page::Sms, "incoming-error"),
                            (Page::Sms, "outgoing-error"),
                            (Page::Sms, "confirmation"),
                            (Page::Sms, "compose"),
                        ]
                    } else {
                        PAGES.to_vec()
                    },
                }))
            }),
        )
    }
}
#[cfg(debug_assertions)]
fn main() -> eframe::Result {
    capture::main()
}
#[cfg(not(debug_assertions))]
fn main() {
    eprintln!("UI capture is available only in debug builds.");
}
