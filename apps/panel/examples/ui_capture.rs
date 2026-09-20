//! Debug-only native screenshot review with simulated data and no-op commands.
//!
//! Every screen here renders a *simulated* snapshot into a real window; nothing connects to a
//! device, opens a serial port, installs a driver or sends a message. Screenshots are therefore
//! evidence about layout only, never about device behaviour.
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

    /// One screenshot: which page to show, which debug fixture to arm, and what the file is
    /// called. The simulated data is chosen per screen so a capture never mixes fixtures.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Screen {
        Overview,
        Diagnostics,
        Repairs,
        Settings,
        Wireless,
        SmsList,
        SmsDetail,
        SmsOutgoing,
        SmsNoMatch,
        SmsCompose,
        SmsConfirmation,
        ToolsPreset,
        ToolsQuery,
        ToolsExpert,
        ToolsBusy,
        ToolsUnknown,
    }

    /// Which simulated device-tools payload a screen renders. All values are invented; no device
    /// is contacted and no command is sent.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum ToolsFixture {
        /// A finished capability sweep plus one frozen expert command.
        Finished,
        /// A running batch sweep, so the progress strip and its cancel button are visible.
        Busy,
        /// A finished expert task whose effect could not be confirmed.
        Unknown,
    }

    impl Screen {
        fn name(self) -> &'static str {
            match self {
                Self::Overview => "overview",
                Self::Diagnostics => "diagnostics",
                Self::Repairs => "repairs",
                Self::Settings => "settings",
                Self::Wireless => "wireless",
                Self::SmsList => "sms-list",
                Self::SmsDetail => "sms-detail",
                Self::SmsOutgoing => "sms-outgoing",
                Self::SmsNoMatch => "sms-no-match",
                Self::SmsCompose => "sms-compose",
                Self::SmsConfirmation => "sms-confirmation",
                Self::ToolsPreset => "tools-preset",
                Self::ToolsQuery => "tools-query",
                Self::ToolsExpert => "tools-expert",
                Self::ToolsBusy => "tools-busy",
                Self::ToolsUnknown => "tools-unknown",
            }
        }

        fn page(self) -> Page {
            match self {
                Self::Overview | Self::Wireless => Page::Overview,
                Self::Diagnostics => Page::Diagnostics,
                Self::Repairs => Page::Repairs,
                Self::Settings => Page::Settings,
                Self::SmsList
                | Self::SmsDetail
                | Self::SmsOutgoing
                | Self::SmsNoMatch
                | Self::SmsCompose
                | Self::SmsConfirmation => Page::Sms,
                Self::ToolsPreset
                | Self::ToolsQuery
                | Self::ToolsExpert
                | Self::ToolsBusy
                | Self::ToolsUnknown => Page::DeviceTools,
            }
        }

        fn is_sms_fixture(self) -> bool {
            matches!(
                self,
                Self::SmsList
                    | Self::SmsDetail
                    | Self::SmsOutgoing
                    | Self::SmsNoMatch
                    | Self::SmsCompose
                    | Self::SmsConfirmation
            )
        }

        /// The simulated `device_tools` payload this screen publishes, if any. Non-tools screens
        /// keep whichever snapshot the mode already built.
        fn tools_fixture(self) -> Option<ToolsFixture> {
            match self {
                Self::ToolsPreset | Self::ToolsQuery | Self::ToolsExpert => {
                    Some(ToolsFixture::Finished)
                }
                Self::ToolsBusy => Some(ToolsFixture::Busy),
                Self::ToolsUnknown => Some(ToolsFixture::Unknown),
                _ => None,
            }
        }

        /// Frames to wait before capturing this screen.  The overview runs one simulated
        /// observation cycle per tick, so it needs enough ticks for the temperature trend to fill
        /// its window before the picture is taken.
        fn settle_ticks(self) -> usize {
            match self {
                Self::Overview => 65,
                _ => 5,
            }
        }

        fn apply(self, app: &mut PanelApp) {
            app.set_review_page(self.page());
            match self {
                Self::Wireless => app.set_review_wireless(),
                Self::SmsList => app.set_review_sms_view(false, None),
                Self::SmsDetail => app.set_review_sms_view(false, Some(1)),
                Self::SmsOutgoing => app.set_review_sms_view(true, None),
                Self::SmsNoMatch => {
                    app.set_review_sms_view(false, None);
                    app.set_review_sms_search("不存在的号码 ZZZ");
                }
                Self::SmsCompose => {
                    app.set_review_sms_view(false, Some(1));
                    app.set_review_sms_editor();
                }
                Self::SmsConfirmation => {
                    app.set_review_sms_view(false, Some(1));
                    app.set_review_sms_confirmation();
                }
                Self::ToolsPreset | Self::ToolsBusy | Self::ToolsUnknown => {
                    app.set_review_device_tools(0);
                }
                Self::ToolsQuery => app.set_review_device_tools(1),
                Self::ToolsExpert => app.set_review_device_tools(2),
                Self::Overview | Self::Diagnostics | Self::Repairs | Self::Settings => {}
            }
        }
    }

    /// The page list for each mode. `pages` walks the whole navigation strip.
    fn screens(mode: &str) -> Vec<Screen> {
        match mode {
            "sms" | "mail" => vec![
                Screen::SmsList,
                Screen::SmsDetail,
                Screen::SmsOutgoing,
                Screen::SmsNoMatch,
                Screen::SmsCompose,
                Screen::SmsConfirmation,
            ],
            "wireless" => vec![Screen::Wireless],
            "overview" => vec![Screen::Overview],
            "tools" => vec![
                Screen::ToolsPreset,
                Screen::ToolsQuery,
                Screen::ToolsExpert,
                Screen::ToolsBusy,
                Screen::ToolsUnknown,
            ],
            _ => vec![
                Screen::Overview,
                Screen::SmsList,
                Screen::Diagnostics,
                Screen::Repairs,
                Screen::Settings,
            ],
        }
    }

    struct Capture {
        app: PanelApp,
        /// The simulated snapshot stream. A tools screen publishes its own payload here before it
        /// is captured; the panel drains it through its ordinary snapshot path.
        snapshot_tx:
            dji4g_application::sync::watch::Sender<Arc<dji4g_application::ControllerSnapshot>>,
        output: PathBuf,
        page: usize,
        /// Index of the screen whose fixture is already published.
        applied_page: Option<usize>,
        ticks: usize,
        scale: f32,
        size: egui::Vec2,
        resized: bool,
        page_started: std::time::Instant,
        requested: bool,
        screens: Vec<Screen>,
        /// Observation cycles already published for the overview review, and the wall clock the
        /// first one is dated from; together they give the temperature trend a real timeline.
        overview_cycles: usize,
        review_base: SystemTime,
        /// The simulated optional-probe record the overview correlates its rows against.
        probe: Arc<std::sync::Mutex<dji4g_panel::feature_probe::FeatureProbeState>>,
    }

    impl Capture {
        /// Publish the screen's simulated `device_tools` payload once per screen change.
        fn publish_fixture(&mut self) {
            if self.applied_page == Some(self.page) {
                return;
            }
            self.applied_page = Some(self.page);
            let Some(kind) = self.screens[self.page].tools_fixture() else {
                return;
            };
            let mut snapshot = demo_snapshot(DemoScenario::Available, SystemTime::now());
            snapshot.device_tools = tools_fixture(kind);
            // Keep the page header in the same simulated context the tools payload describes.
            snapshot.sim_epoch = 3;
            if let Some(device) = snapshot.app.device.clone() {
                snapshot.app = Arc::new(dji4g_domain::AppSnapshot {
                    device: Some(dji4g_domain::DeviceSnapshot {
                        epoch: dji4g_domain::DeviceEpoch(7),
                        identity: simulated_identity(),
                        at_port: Some("COM7".to_owned()),
                        ..device
                    }),
                    ..(*snapshot.app).clone()
                });
            }
            let _ = self.snapshot_tx.send(Arc::new(snapshot));
        }

        /// Publish one simulated observation cycle per tick for the overview review.
        ///
        /// Every tick is a cycle dated ten seconds after the previous one, so the 「模块温度」 trend
        /// is drawn from the very ring the panel builds in production — including two cycles that
        /// reported nothing, which stay an honest gap.  The reading mimics the DJI positional
        /// layout (`+QTEMP: v,v-6,v-6`) the module really answers with.  The same cycle carries a
        /// simulated throughput pair (a quiet link with a late upload burst), so the rate chart is
        /// reviewed with data in it too.  No device is contacted: the screenshots are evidence
        /// about layout only.
        fn publish_overview_fixture(&mut self) {
            if !matches!(self.screens[self.page], Screen::Overview) {
                return;
            }
            let cycle = self.overview_cycles;
            self.overview_cycles += 1;
            // A warm-up ramp that plateaus, with two cycles that could not be read.
            let reading = match cycle {
                40..=41 => None,
                _ => Some(48 + (cycle / 8).min(9) as i16),
            };
            let observed_at = self.review_base + Duration::from_secs(cycle as u64 * 10);
            let mut snapshot = demo_snapshot(DemoScenario::Available, SystemTime::now());
            let mut app = (*snapshot.app).clone();
            app.observed_at = observed_at;
            // The demo scenarios carry no cellular block, so the review supplies the module the
            // temperature belongs to.  Every value here is invented.
            app.cellular = Some(dji4g_domain::CellularSnapshot {
                sim: dji4g_domain::SimState::Ready,
                registration: dji4g_domain::RegistrationState::RegisteredHome,
                attached: dji4g_domain::AttachState::Attached,
                carrier: Some("CHN-UNICOM".to_owned()),
                radio_access_technology: Some("LTE".to_owned()),
                signal_rssi_dbm: Some(-63),
                apn: Some("cmnet".to_owned()),
                pdp_address: Some("10.1.2.3".to_owned()),
                pdp_state: Some("active".to_owned()),
                firmware: Some("QDC507GLEFM21".to_owned()),
                serving_cell: None,
                sim_identity: None,
                numbers: None,
                temperature_celsius: reading,
                temperature_status: dji4g_domain::FeatureStatus::Supported,
            });
            // A quiet downlink and a late uplink burst: the rate chart must follow these numbers
            // with its own y-axis instead of a fixed ceiling.
            // The reported case: a 86.2 KB/s burst inside the minute, ~47.5 KB/s now.
            let down = if (24..28).contains(&cycle) {
                86_200
            } else {
                47_500 - (cycle as u64 % 5) * 240
            };
            let up = if cycle >= 52 {
                8_700
            } else {
                1_100 + (cycle as u64 % 5) * 210
            };
            app.network = Some(dji4g_domain::NetworkSnapshot {
                adapter_id: "{adapter}".to_owned(),
                addresses: vec!["192.168.225.30".to_owned()],
                gateways: vec!["192.168.225.1".to_owned()],
                dns_servers: vec!["192.168.225.1".to_owned()],
                adapter_state: dji4g_domain::AdapterState::UsableAddressAndRoute,
                bound_public: dji4g_domain::BoundPublicStatus::Succeeded,
                bound_dns: dji4g_domain::BoundDnsStatus::Succeeded,
                protocol_coverage: dji4g_domain::ProtocolCoverage::AllRequiredFamilies,
                system_default_route: dji4g_domain::DefaultRouteOwner::TargetAdapter,
                down_bytes_per_sec: Some(down),
                up_bytes_per_sec: Some(up),
            });
            snapshot.app = Arc::new(app);
            let probe = {
                let cellular = snapshot.app.cellular.as_ref();
                dji4g_panel::feature_probe::FeatureProbeState {
                    epoch: snapshot
                        .app
                        .device
                        .as_ref()
                        .map_or(dji4g_domain::DeviceEpoch(0), |device| device.epoch),
                    numbers: cellular.and_then(|cellular| cellular.numbers.clone()),
                    sim_identity: cellular.and_then(|cellular| cellular.sim_identity.clone()),
                    serving_cell: cellular.and_then(|cellular| cellular.serving_cell.clone()),
                    temperature_celsius: reading,
                    temperature_sensors: reading.map_or_else(Vec::new, |celsius| {
                        vec![
                            dji4g_at_protocol::SensorTemperature {
                                name: None,
                                celsius,
                            },
                            dji4g_at_protocol::SensorTemperature {
                                name: None,
                                celsius: celsius - 6,
                            },
                            dji4g_at_protocol::SensorTemperature {
                                name: None,
                                celsius: celsius - 6,
                            },
                        ]
                    }),
                    temperature_raw: reading.map(|celsius| {
                        let low = celsius - 6;
                        format!("+QTEMP: {celsius},{low},{low}")
                    }),
                    ..dji4g_panel::feature_probe::FeatureProbeState::default()
                }
            };
            if let Ok(mut slot) = self.probe.lock() {
                *slot = probe;
            }
            let _ = self.snapshot_tx.send(Arc::new(snapshot));
        }
    }

    impl eframe::App for Capture {
        fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
            if self.page >= self.screens.len() {
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
                    &self
                        .output
                        .join(format!("{}.bmp", self.screens[self.page].name())),
                    &image,
                )
                .unwrap();
                self.page += 1;
                self.ticks = 0;
                self.page_started = std::time::Instant::now();
                self.requested = false;
                if self.page == self.screens.len() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
            }
            ctx.set_pixels_per_point(self.scale);
            if !self.resized && self.ticks >= 2 {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(self.size));
                self.resized = true;
            }
            self.publish_fixture();
            self.publish_overview_fixture();
            self.app.receive_latest_nonblocking(ctx);
            // This harness drives `render` directly instead of `PanelApp::update`, so the UI-side
            // rings are fed through the same public entry points the update loop uses, on the
            // fixture's synthetic clock (one second and one observation cycle per tick).
            self.app.sample_rates_on_cadence(
                self.review_base + Duration::from_secs(self.overview_cycles as u64),
            );
            self.app.sample_temperature_on_observation();
            self.screens[self.page].apply(&mut self.app);
            egui::TopBottomPanel::top("simulated-review").show(ctx, |ui| {
                ui.label("界面验收 · 模拟数据 · 不连接设备 / 不发送短信");
            });
            self.app.render(ctx, frame);
            self.ticks += 1;
            if self.ticks >= self.screens[self.page].settle_ticks()
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

    /// 30 stored messages for the layout review: long numbers, Chinese and ASCII bodies, unread
    /// and read rows, long multi-part bodies, rows with no timestamp, and outgoing records in
    /// every submission state. None of this content was ever transmitted.
    fn sms_fixture() -> (Vec<dji4g_domain::SmsMessage>, dji4g_domain::SmsInboxSummary) {
        use dji4g_domain::*;

        const SENDERS: [&str; 6] = [
            "+8613800138000",
            "106900000000000000000000",
            "13800138000",
            "+861066668888",
            "95533",
            "+86 138 0013 8000",
        ];
        const BODIES: [&str; 6] = [
            "【模拟数据·界面验收】这条短信只用于截图排版，从未真实收发。",
            "验证码 123456，5 分钟内有效。请勿泄露给任何人。【模拟】",
            "这是一条很长的模拟短信，用来验证长正文在阅读区里换行、滚动是否正常；它包含中文标点、数字 0123456789 与英文 mixed content，长度明显超过列表预览的两行，因此只有打开阅读区才能看全。",
            "网络流量提醒：本月已使用 12.5GB，剩余 7.5GB。【模拟数据，未发送】",
            "[Bank] Your simulated statement is ready. This text was never sent to anyone.",
            "短",
        ];
        let mut messages = Vec::new();
        for index in 1..=24u32 {
            let mut message = SmsMessage::new(
                index,
                SmsStorageId("SM".to_owned()),
                1,
                0,
                SENDERS[(index as usize) % SENDERS.len()],
                BODIES[(index as usize) % BODIES.len()],
                if index % 3 == 0 {
                    SmsEncoding::Gsm7
                } else {
                    SmsEncoding::Ucs2
                },
                SmsStatus::Received,
            );
            message.read = Some(index % 4 != 0);
            // Every third row has no service-centre timestamp, so "时间未提供" is visible.
            if index % 3 != 0 {
                message.service_centre_timestamp = Some(format!(
                    "26/09/{:02},12:{:02}:00+32",
                    (index % 28) + 1,
                    index % 60
                ));
            }
            if index % 5 == 0 {
                message.multipart = Some(SmsMultipartInfo {
                    reference: index as u8,
                    total: 3,
                    sequence: 2,
                });
                message.status = SmsStatus::Incomplete;
            }
            messages.push(message);
        }
        let outgoing_states = [
            SmsStatus::Submitted,
            SmsStatus::Submitted,
            SmsStatus::Failed,
            SmsStatus::OutcomeUnknown,
            SmsStatus::Submitted,
            SmsStatus::Failed,
        ];
        // Transaction id 2 is the one the narrow-screen detail screenshot selects.
        for (offset, status) in outgoing_states.into_iter().enumerate() {
            messages.push(SmsMessage::new_outgoing(
                offset as u32 + 1,
                1,
                0,
                "+8613800138000",
                format!("【模拟发送记录 {}】此内容从未提交给模块。", offset + 1),
                SmsEncoding::Ucs2,
                status,
            ));
        }
        let summary = SmsInboxSummary {
            message_count: 24,
            unread_count: 6,
            capacity: Some((30, 50)),
            status: FeatureStatus::Supported,
            has_incomplete: true,
            evicted: 2,
        };
        (messages, summary)
    }

    fn sms_send_fixture() -> dji4g_application::SmsSendSnapshot {
        use dji4g_application::*;
        let mut failure =
            SmsFailureDetail::new(SmsSendPhase::WaitingForResult, "sms:module_rejected", true);
        failure.cms_code = Some(500);
        SmsSendSnapshot {
            request_id: 2,
            phase: SmsSendPhase::Finished,
            result: Some(SmsSendResult::Failed),
            failure: Some(failure),
        }
    }

    /// The invented device identity the tools fixtures use, so the header, the capability rows and
    /// the history all describe one simulated module.
    fn simulated_identity() -> dji4g_domain::StableDeviceIdentity {
        dji4g_domain::StableDeviceIdentity {
            container_id: "SWD\\VID_2CA3&PID_4006\\5&2A1B3C4D&0&1".to_owned(),
            device_instance_id: "USB\\VID_2CA3&PID_4006\\5&2A1B3C4D&0&1".to_owned(),
            vid: 0x2CA3,
            pid: 0x4006,
        }
    }

    /// Simulated device-tools payload for the layout review. Everything here is invented: no
    /// device was contacted, no serial port opened, and no command was sent to any module.
    fn tools_fixture(kind: ToolsFixture) -> dji4g_application::DeviceToolsSnapshot {
        use dji4g_application::*;
        use dji4g_at_protocol::{
            AtCommand, AtFinalCode, AtResponse, SensorTemperature, ToolReadId, ValidatedToolLine,
            VerifiedUsbNetProfile,
        };

        let now = SystemTime::now();
        let context = ToolContext {
            device_epoch: dji4g_domain::DeviceEpoch(7),
            sim_epoch: 3,
            identity: simulated_identity(),
            at_port: "COM7".to_owned(),
        };
        let pdp_contexts = dji4g_at_protocol::parse_pdp_contexts(&AtResponse {
            epoch: dji4g_domain::DeviceEpoch(7),
            command: AtCommand::PdpContexts,
            lines: vec![
                "+CGDCONT: 1,\"IP\",\"internet\"".to_owned(),
                "+CGDCONT: 2,\"IPV4V6\",\"ims\"".to_owned(),
            ],
            final_code: AtFinalCode::Ok,
        })
        .unwrap_or_default();

        let mut tools = DeviceToolsSnapshot {
            profile: ModuleProfile {
                manufacturer: Some("Quectel".to_owned()),
                model: Some("EC200A-CN".to_owned()),
                revision: Some("EC200ACNAAR02A05M08".to_owned()),
                usb_net: Some(UsbNetReading::Verified(VerifiedUsbNetProfile::DjiNdis)),
                pdp_contexts,
                temperature: vec![
                    SensorTemperature {
                        name: Some("cpu".to_owned()),
                        celsius: 42,
                    },
                    SensorTemperature {
                        name: Some("pa".to_owned()),
                        celsius: 37,
                    },
                ],
                observed_at: Some(now - Duration::from_secs(12)),
                context: Some(context.clone()),
            },
            ..DeviceToolsSnapshot::default()
        };

        for (id, observed_ago) in [
            (ToolReadId::Attention, 12_u64),
            (ToolReadId::Manufacturer, 12),
            (ToolReadId::Model, 12),
            (ToolReadId::Revision, 12),
            (ToolReadId::SignalQuality, 11),
            (ToolReadId::UsbNet, 11),
        ] {
            tools.record_capability(ToolCapabilityRow::new(
                id,
                ToolOutcome::Ok,
                context.clone(),
                now - Duration::from_secs(observed_ago),
            ));
        }
        tools.record_capability(ToolCapabilityRow::empty(
            ToolReadId::SmsStorage,
            context.clone(),
            now - Duration::from_secs(11),
        ));
        tools.record_capability(ToolCapabilityRow::new(
            ToolReadId::ServingCell,
            ToolOutcome::TransportFailure,
            context.clone(),
            now - Duration::from_secs(9),
        ));
        tools.history.push(ToolHistoryEntry {
            id: 41,
            operation: ToolOperationKind::Read(ToolReadId::Manufacturer),
            outcome: ToolOutcome::Ok,
            elapsed: Duration::from_millis(420),
            finished_at: now - Duration::from_secs(8),
            transcript: Arc::new(ToolTranscript::from_lines(vec![
                "+CGMI: \"Quectel\"".to_owned(),
                "OK".to_owned(),
            ])),
        });

        match kind {
            ToolsFixture::Finished => {
                tools.task = Some(ToolTaskSnapshot {
                    id: 42,
                    context: context.clone(),
                    operation: ToolOperationKind::ProbeAll,
                    phase: ToolPhase::Finished,
                    outcome: Some(ToolOutcome::Ok),
                    completed_items: 17,
                    total_items: 17,
                });
                tools.pending_expert =
                    ValidatedToolLine::parse("AT+QCFG=\"usbnet\",1")
                        .ok()
                        .map(|line| PendingExpertTool {
                            id: 43,
                            line,
                            expires_at: now + Duration::from_secs(23),
                        });
            }
            ToolsFixture::Busy => {
                tools.task = Some(ToolTaskSnapshot {
                    id: 44,
                    context: context.clone(),
                    operation: ToolOperationKind::ProbeAll,
                    phase: ToolPhase::Running,
                    outcome: None,
                    completed_items: 4,
                    total_items: 17,
                });
            }
            ToolsFixture::Unknown => {
                tools.task = Some(ToolTaskSnapshot {
                    id: 45,
                    context: context.clone(),
                    operation: ToolOperationKind::Expert,
                    phase: ToolPhase::Finished,
                    outcome: Some(ToolOutcome::OutcomeUnknown),
                    completed_items: 1,
                    total_items: 1,
                });
                tools.history.push(ToolHistoryEntry {
                    id: 45,
                    operation: ToolOperationKind::Expert,
                    outcome: ToolOutcome::OutcomeUnknown,
                    elapsed: Duration::from_secs(10),
                    finished_at: now,
                    transcript: Arc::new(ToolTranscript::from_lines(vec![
                        "+QCFG: \"usbnet\",1".to_owned(),
                        "（模拟数据：未收到最终应答）".to_owned(),
                    ])),
                });
            }
        }
        tools
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
        let mode = args.get(4).cloned().unwrap_or_else(|| "pages".to_owned());
        let screens = screens(&mode);
        let sms_fixture_wanted = screens.iter().any(|screen| screen.is_sms_fixture());

        // New directory for this round: the previous round's screenshots must stay untouched.
        let output = PathBuf::from("docs/implementation-20260919/screenshots")
            .join(format!(
                "{}x{}-{}pct",
                width,
                height,
                (scale * 100.0) as u32
            ))
            .join(&mode);
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
                if sms_fixture_wanted {
                    let (messages, summary) = sms_fixture();
                    snapshot.sms_messages = messages;
                    snapshot.sms_inbox = summary;
                    snapshot.sms_send = Some(sms_send_fixture());
                }
                let snapshot = Arc::new(snapshot);
                let (snapshot_tx, rx) = dji4g_application::sync::watch::channel(snapshot);
                let probe = Arc::new(std::sync::Mutex::new(
                    dji4g_panel::feature_probe::FeatureProbeState::default(),
                ));
                let app = PanelApp::new(
                    PanelInputs::new(rx, Arc::new(Noop), None, None)
                        .with_feature_probe(Arc::clone(&probe)),
                    cc,
                );
                Ok(Box::new(Capture {
                    app,
                    snapshot_tx,
                    output,
                    page: 0,
                    applied_page: None,
                    ticks: 0,
                    scale,
                    size: egui::vec2(width, height),
                    resized: false,
                    page_started: std::time::Instant::now(),
                    requested: false,
                    screens,
                    // The simulated timeline ends at "now": the newest cycle is the current
                    // reading, and the window covers the ten minutes before it.
                    review_base: SystemTime::now() - Duration::from_secs(660),
                    overview_cycles: 0,
                    probe,
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
