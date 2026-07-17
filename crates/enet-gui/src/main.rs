//! BMW ENET Gateway GUI — friendly status + first-run guidance.

use chrono::Local;
use clap::Parser;
use eframe::egui;
use enet_core::safety::FlashSafetyReport;
use enet_core::state::GatewayState;
use enet_core::stats::StatsSnapshot;
use serde::Deserialize;
use std::time::{Duration, Instant};

#[derive(Parser, Debug)]
#[command(name = "enet-gui")]
struct Args {
    #[arg(long, default_value = "http://127.0.0.1:47901")]
    api: String,
}

#[derive(Debug, Deserialize, Clone, Default)]
struct StatusResponse {
    state: GatewayState,
    stats: StatsSnapshot,
    cpu_pct: f64,
    memory_used: u64,
    memory_total: u64,
    flash_safety: FlashSafetyReport,
    #[serde(default)]
    pair_code: String,
    #[serde(default)]
    setup_hints: Vec<String>,
    #[serde(default)]
    setup_complete: bool,
    #[serde(default)]
    friendly_status: String,
    #[serde(default)]
    update_available: Option<String>,
    #[serde(default = "default_auto_update")]
    auto_update: bool,
}

fn default_auto_update() -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct CheckUpdateResponse {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    message: String,
    #[serde(default)]
    update_available: Option<String>,
    #[serde(default)]
    current: Option<String>,
}

struct GatewayApp {
    api: String,
    status: StatusResponse,
    last_fetch: Instant,
    log_lines: Vec<String>,
    settings_open: bool,
    help_open: bool,
    password: String,
    tunnel_port: String,
    auto_update: bool,
    update_check_msg: String,
    checked_updates_on_open: bool,
    error: Option<String>,
    client: reqwest::blocking::Client,
}

impl GatewayApp {
    fn new(api: String) -> Self {
        Self {
            api,
            status: StatusResponse::default(),
            last_fetch: Instant::now() - Duration::from_secs(10),
            log_lines: vec![format!(
                "{}  Welcome — open Help if this is your first time",
                Local::now().format("%H:%M:%S")
            )],
            settings_open: false,
            help_open: true,
            password: String::new(),
            tunnel_port: "47900".into(),
            auto_update: true,
            update_check_msg: String::new(),
            checked_updates_on_open: false,
            error: None,
            client: reqwest::blocking::Client::builder()
                .timeout(Duration::from_millis(800))
                .build()
                .expect("http client"),
        }
    }

    fn push_log(&mut self, msg: impl Into<String>) {
        self.log_lines
            .push(format!("{}  {}", Local::now().format("%H:%M:%S"), msg.into()));
        if self.log_lines.len() > 500 {
            self.log_lines.drain(0..self.log_lines.len() - 500);
        }
    }

    fn refresh(&mut self) {
        match self.client.get(format!("{}/api/status", self.api)).send() {
            Ok(resp) => match resp.json::<StatusResponse>() {
                Ok(s) => {
                    if !s.setup_complete {
                        self.help_open = true;
                    }
                    self.auto_update = s.auto_update;
                    self.status = s;
                    self.error = None;
                }
                Err(e) => self.error = Some(format!("Could not read status: {e}")),
            },
            Err(_) => {
                self.error = Some(
                    "Gateway not reachable. Start it with the desktop installer, or run enet-gateway."
                        .into(),
                );
                self.help_open = true;
            }
        }
        self.last_fetch = Instant::now();
    }

    fn check_for_updates(&mut self) {
        self.push_log("Checking for updates…");
        let slow = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(25))
            .build()
            .unwrap_or_else(|_| self.client.clone());
        match slow
            .post(format!("{}/api/check-update", self.api))
            .send()
        {
            Ok(resp) => match resp.json::<CheckUpdateResponse>() {
                Ok(r) => {
                    self.update_check_msg = r.message.clone();
                    self.push_log(&r.message);
                    if let Some(v) = r.update_available {
                        self.status.update_available = Some(v);
                    } else if r.ok {
                        self.status.update_available = None;
                    }
                }
                Err(e) => {
                    self.update_check_msg = format!("Could not parse update response: {e}");
                    self.push_log(self.update_check_msg.clone());
                }
            },
            Err(e) => {
                self.update_check_msg = format!("Update check failed: {e}");
                self.push_log(self.update_check_msg.clone());
            }
        }
        self.refresh();
    }

    fn post(&mut self, path: &str) {
        match self.client.post(format!("{}{path}", self.api)).send() {
            Ok(_) => self.push_log(format!("OK {path}")),
            Err(e) => {
                self.push_log(format!("FAIL {path}: {e}"));
                self.error = Some(e.to_string());
            }
        }
    }
}

impl eframe::App for GatewayApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.last_fetch.elapsed() > Duration::from_millis(500) {
            self.refresh();
            // Host already checked GitHub on its own start; surface that here.
            if !self.checked_updates_on_open && self.error.is_none() {
                self.checked_updates_on_open = true;
                if let Some(v) = self.status.update_available.clone() {
                    self.push_log(format!("Update available: v{v} — open Settings to install"));
                    self.update_check_msg = format!("Update available: v{v}");
                } else {
                    self.push_log(format!(
                        "Up to date (v{})",
                        self.status.state.version
                    ));
                }
            }
        }
        ctx.request_repaint_after(Duration::from_millis(250));

        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = egui::Color32::from_rgb(18, 22, 28);
        visuals.window_fill = egui::Color32::from_rgb(24, 30, 38);
        visuals.override_text_color = Some(egui::Color32::from_rgb(230, 234, 240));
        visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(36, 46, 58);
        visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(48, 62, 78);
        visuals.selection.bg_fill = egui::Color32::from_rgb(0, 140, 160);
        ctx.set_visuals(visuals);

        egui::TopBottomPanel::top("brand").show(ctx, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.heading(
                    egui::RichText::new("BMW ENET Gateway")
                        .size(28.0)
                        .color(egui::Color32::from_rgb(220, 230, 240)),
                );
                ui.add_space(16.0);
                if !self.status.pair_code.is_empty() {
                    ui.label(
                        egui::RichText::new(format!("Pair code {}", self.status.pair_code))
                            .size(22.0)
                            .color(egui::Color32::from_rgb(0, 180, 200)),
                    );
                }
            });
            let friendly = if self.status.friendly_status.is_empty() {
                self.status.state.status_message.clone()
            } else {
                self.status.friendly_status.clone()
            };
            ui.label(
                egui::RichText::new(friendly)
                    .size(15.0)
                    .color(egui::Color32::from_rgb(140, 160, 175)),
            );
            ui.add_space(6.0);
        });

        egui::TopBottomPanel::bottom("actions").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Start").clicked() {
                    self.post("/api/start");
                }
                if ui.button("Stop").clicked() {
                    self.post("/api/stop");
                }
                if ui.button("Restart").clicked() {
                    self.post("/api/restart");
                }
                if ui.button("Settings").clicked() {
                    self.settings_open = true;
                }
                if ui.button("How to use").clicked() {
                    self.help_open = true;
                }
                if ui.button("Open in browser").clicked() {
                    self.push_log(format!("Open {} in your browser", self.api));
                    #[cfg(target_os = "windows")]
                    let _ = std::process::Command::new("cmd")
                        .args(["/C", "start", &self.api])
                        .spawn();
                    #[cfg(target_os = "linux")]
                    let _ = std::process::Command::new("xdg-open").arg(&self.api).spawn();
                    #[cfg(target_os = "macos")]
                    let _ = std::process::Command::new("open").arg(&self.api).spawn();
                }
                if ui.button("Export Logs").clicked() {
                    self.post("/api/export-logs");
                }
            });
            ui.add_space(6.0);
        });

        egui::SidePanel::left("status_panel")
            .resizable(true)
            .default_width(280.0)
            .show(ctx, |ui| {
                ui.heading("Connection");
                ui.separator();
                status_row(ui, "Gateway running", self.status.state.gateway_running);
                status_row(ui, "Laptop connected", self.status.state.laptop_connected);
                status_row(ui, "Vehicle connected", self.status.state.vehicle.link_up);
                status_row(ui, "Vehicle awake", self.status.state.vehicle.awake);
                status_row(
                    ui,
                    "Tunnel ready",
                    matches!(
                        self.status.state.connection,
                        enet_core::state::ConnectionState::Connected
                    ),
                );
                ui.separator();
                if let Some(err) = &self.error {
                    ui.colored_label(egui::Color32::from_rgb(220, 90, 70), err);
                    ui.label("Tip: run the desktop installer, then click How to use.");
                }
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Live stats");
            ui.separator();
            let s = &self.status.stats;
            ui.columns(3, |cols| {
                cols[0].label(format!("TX pps: {:.1}", s.tx_pps));
                cols[0].label(format!("RX pps: {:.1}", s.rx_pps));
                cols[1].label(format!("RTT: {:.2} ms", s.rtt_ms));
                cols[1].label(format!("RTT p99: {:.2} ms", s.rtt_p99_ms));
                cols[2].label(format!("Loss: {:.4}%", s.loss_rate * 100.0));
                cols[2].label(format!("CPU: {:.1}%", self.status.cpu_pct));
            });

            ui.add_space(12.0);
            ui.heading("Flash safety");
            ui.separator();
            let safe = self.status.flash_safety.safe;
            let color = if safe {
                egui::Color32::from_rgb(60, 180, 120)
            } else {
                egui::Color32::from_rgb(220, 120, 60)
            };
            ui.colored_label(
                color,
                if safe {
                    "SAFE — thresholds met (still flash at your own risk)"
                } else {
                    "NOT SAFE — do not flash ECUs yet"
                },
            );
            ui.label(&self.status.flash_safety.warning);

            ui.add_space(12.0);
            ui.heading("Activity log");
            ui.separator();
            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .max_height(200.0)
                .show(ui, |ui| {
                    for line in &self.log_lines {
                        ui.monospace(line);
                    }
                });
        });

        if self.help_open {
            egui::Window::new("How to use — setup & daily workflow")
                .collapsible(false)
                .resizable(true)
                .default_width(520.0)
                .show(ctx, |ui| {
                    ui.heading("First-time setup");
                    ui.label("You do not need to know networking. Follow these steps:");
                    ui.add_space(6.0);
                    if self.status.setup_hints.is_empty() {
                        ui.label("1. Keep this desktop on your home network.");
                        ui.label("2. Install/start the Gateway on this PC.");
                        ui.label("3. On the laptop, run the Agent installer (auto-finds this PC).");
                        ui.label("4. Plug ENET into the car + laptop, ignition ON.");
                        ui.label("5. When Laptop + Vehicle lights are green, open ISTA/E-Sys here.");
                    } else {
                        for h in &self.status.setup_hints {
                            ui.label(h);
                        }
                    }
                    ui.add_space(8.0);
                    if !self.status.pair_code.is_empty() {
                        ui.label(
                            egui::RichText::new(format!(
                                "Tell the laptop this pair code: {}",
                                self.status.pair_code
                            ))
                            .size(18.0)
                            .color(egui::Color32::from_rgb(0, 180, 200)),
                        );
                    }
                    ui.add_space(10.0);
                    ui.heading("Every time you use the car");
                    ui.label("1. Gateway running (this PC) + Agent running (laptop).");
                    ui.label("2. Plug ENET cable into car OBD + laptop.");
                    ui.label("3. Ignition ON — wait for Vehicle awake (green).");
                    ui.label("4. Open ISTA / E-Sys on this desktop.");
                    ui.label("5. Flash only if Flash safety says SAFE.");
                    ui.add_space(8.0);
                    ui.label("Buttons: Start/Stop/Restart control the tunnel · Settings = port/password · Export Logs = troubleshooting.");
                    ui.label("Full guide: README.md and docs/HOW_TO_USE.md");
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("I finished setup").clicked() {
                            self.post("/api/complete-setup");
                            self.help_open = false;
                        }
                        if ui.button("Close").clicked() {
                            self.help_open = false;
                        }
                    });
                });
        }

        if self.settings_open {
            egui::Window::new("Settings")
                .collapsible(false)
                .resizable(true)
                .default_width(420.0)
                .show(ctx, |ui| {
                    ui.heading("Tunnel");
                    ui.label("Tunnel port");
                    ui.text_edit_singleline(&mut self.tunnel_port);
                    ui.label("Optional password (same on laptop)");
                    ui.text_edit_singleline(&mut self.password);

                    ui.add_space(12.0);
                    ui.heading("Updates");
                    ui.label(format!(
                        "Installed version: v{}",
                        if self.status.state.version.is_empty() {
                            "?".into()
                        } else {
                            self.status.state.version.clone()
                        }
                    ));
                    ui.checkbox(&mut self.auto_update, "Automatically install updates when idle");
                    if !self.update_check_msg.is_empty() {
                        ui.label(
                            egui::RichText::new(&self.update_check_msg)
                                .color(egui::Color32::from_rgb(0, 180, 200)),
                        );
                    }
                    if let Some(v) = &self.status.update_available {
                        ui.colored_label(
                            egui::Color32::from_rgb(60, 180, 120),
                            format!("Update available: v{v}"),
                        );
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Check for updates").clicked() {
                            self.check_for_updates();
                        }
                        if self.status.update_available.is_some()
                            && ui.button("Update now").clicked()
                        {
                            self.post("/api/update");
                            self.push_log("Update requested — app will restart shortly");
                        }
                    });

                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() {
                            let port: u16 = self.tunnel_port.parse().unwrap_or(47900);
                            let body = serde_json::json!({
                                "tunnel_port": port,
                                "password": self.password,
                                "auto_update": self.auto_update,
                            });
                            let _ = self
                                .client
                                .post(format!("{}/api/settings", self.api))
                                .json(&body)
                                .send();
                            self.push_log("Settings saved");
                            self.settings_open = false;
                        }
                        if ui.button("Close").clicked() {
                            self.settings_open = false;
                        }
                    });
                });
        }
    }
}

fn status_row(ui: &mut egui::Ui, label: &str, ok: bool) {
    ui.horizontal(|ui| {
        let (dot, color) = if ok {
            ("●", egui::Color32::from_rgb(60, 180, 120))
        } else {
            ("●", egui::Color32::from_rgb(120, 130, 140))
        };
        ui.colored_label(color, dot);
        ui.label(label);
    });
}

fn main() -> eframe::Result<()> {
    let args = Args::parse();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([980.0, 680.0])
            .with_title("BMW ENET Gateway"),
        ..Default::default()
    };
    eframe::run_native(
        "BMW ENET Gateway",
        options,
        Box::new(move |_cc| Ok(Box::new(GatewayApp::new(args.api)))),
    )
}
