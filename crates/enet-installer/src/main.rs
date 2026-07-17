//! BMW ENET Setup Wizard — single .exe, Host (desktop) or Client (laptop).
//! Downloads the matching Windows package from GitHub Releases (or uses offline files).

mod download;
mod install;

use clap::Parser;
use download::{Role, DEFAULT_REPO};
use eframe::egui;
use install::InstallRequest;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Parser, Debug)]
#[command(name = "BMW-ENET-Setup")]
struct Args {
    /// GitHub repo that publishes release assets (owner/name)
    #[arg(long, default_value = DEFAULT_REPO)]
    repo: String,
    /// Skip GUI and install host or client from CLI
    #[arg(long)]
    role: Option<String>,
    #[arg(long, default_value = "")]
    pair_code: String,
    /// Desktop Host LAN IP (Client installs)
    #[arg(long, default_value = "")]
    peer: String,
    #[arg(long, default_value = "")]
    password: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Welcome,
    ChooseRole,
    Options,
    Working,
    Done,
    Error,
}

struct SharedProgress {
    message: String,
    done: u64,
    total: u64,
    finished: bool,
    success: bool,
    result_summary: String,
    error: String,
}

struct SetupApp {
    step: Step,
    role: Role,
    pair_code: String,
    peer: String,
    password: String,
    start_service: bool,
    start_now: bool,
    open_dashboard: bool,
    repo: String,
    setup_dir: PathBuf,
    npcap_present: Option<bool>,
    progress: Arc<Mutex<SharedProgress>>,
    worker_started: bool,
}

impl SetupApp {
    fn new(repo: String) -> Self {
        let setup_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            step: Step::Welcome,
            role: Role::Host,
            pair_code: String::new(),
            peer: String::new(),
            password: String::new(),
            start_service: true,
            start_now: true,
            open_dashboard: true,
            repo,
            setup_dir,
            npcap_present: install::npcap_present(),
            progress: Arc::new(Mutex::new(SharedProgress {
                message: String::new(),
                done: 0,
                total: 0,
                finished: false,
                success: false,
                result_summary: String::new(),
                error: String::new(),
            })),
            worker_started: false,
        }
    }

    fn start_worker(&mut self) {
        if self.worker_started {
            return;
        }
        self.worker_started = true;
        self.step = Step::Working;

        let role = self.role;
        let repo = self.repo.clone();
        let setup_dir = self.setup_dir.clone();
        let progress = Arc::clone(&self.progress);
        let req = InstallRequest {
            role,
            pair_code: self.pair_code.clone(),
            peer: self.peer.clone(),
            password: self.password.clone(),
            start_service: self.start_service,
            start_now: self.start_now,
            open_dashboard: self.open_dashboard,
        };

        thread::spawn(move || {
            let progress_cb = Arc::clone(&progress);
            let cb: Arc<download::ProgressFn> = Arc::new(move |done: u64, total: u64, msg: &str| {
                if let Ok(mut p) = progress_cb.lock() {
                    p.message = msg.to_string();
                    p.done = done;
                    p.total = total;
                }
            });

            let work = std::env::temp_dir().join("bmw-enet-setup");
            let outcome = (|| {
                let package =
                    download::prepare_package(role, &repo, &setup_dir, &work, cb.as_ref())?;
                let result = install::run_install(&req, &package, cb.as_ref())?;
                Ok::<_, anyhow::Error>(result)
            })();

            if let Ok(mut p) = progress.lock() {
                match outcome {
                    Ok(r) => {
                        p.success = true;
                        p.finished = true;
                        p.message = "Installation complete".into();
                        let mut summary = format!(
                            "Installed {} {}\nto {}\n",
                            role.label(),
                            r.version,
                            r.install_dir.display()
                        );
                        if !r.pair_code_hint.is_empty() {
                            summary.push_str(&format!("\nPair code: {}\n", r.pair_code_hint));
                        }
                        if let Some(url) = r.dashboard_url {
                            summary.push_str(&format!("\nDashboard: {url}\n"));
                        }
                        if role == Role::Host {
                            summary.push_str(
                                "\nNext: install Npcap (https://npcap.com, WinPcap mode) if prompted.\n\
                                 In ISTA / E-Sys choose interface BMW-ENET (169.254.1.1).\n\
                                 On the laptop, run Setup → Client, install Npcap, plug ENET, ignition ON.\n",
                            );
                        } else {
                            summary.push_str(
                                "\nNext: install Npcap on this laptop if prompted.\n\
                                 Open http://127.0.0.1:47903/ — Desktop should go green (auto-find).\n\
                                 Plug ENET into car + laptop, ignition ON, then open ISTA on the desktop.\n",
                            );
                        }
                        p.result_summary = summary;
                    }
                    Err(e) => {
                        p.success = false;
                        p.finished = true;
                        p.error = format!("{e:#}");
                        p.message = "Installation failed".into();
                    }
                }
            }
        });
    }
}

const ACCENT: egui::Color32 = egui::Color32::from_rgb(74, 168, 199);
const OK_GREEN: egui::Color32 = egui::Color32::from_rgb(51, 194, 116);
const WARN_AMBER: egui::Color32 = egui::Color32::from_rgb(230, 164, 59);
const ERR_RED: egui::Color32 = egui::Color32::from_rgb(220, 80, 66);

fn step_header(ui: &mut egui::Ui, current: Step) {
    let steps = [
        (Step::Welcome, "Welcome"),
        (Step::ChooseRole, "Role"),
        (Step::Options, "Options"),
        (Step::Working, "Install"),
    ];
    let active_idx = match current {
        Step::Welcome => 0,
        Step::ChooseRole => 1,
        Step::Options => 2,
        Step::Working | Step::Done | Step::Error => 3,
    };
    ui.horizontal(|ui| {
        for (i, (_, label)) in steps.iter().enumerate() {
            let done = i < active_idx;
            let active = i == active_idx;
            let (fg, text) = if active {
                (ACCENT, format!("● {label}"))
            } else if done {
                (OK_GREEN, format!("✔ {label}"))
            } else {
                (egui::Color32::from_rgb(120, 132, 148), format!("○ {label}"))
            };
            ui.colored_label(fg, egui::RichText::new(text).size(13.0));
            if i + 1 < steps.len() {
                ui.colored_label(egui::Color32::from_rgb(70, 80, 95), "—");
            }
        }
    });
}

fn role_card(ui: &mut egui::Ui, selected: bool, title: &str, lines: &[&str]) -> bool {
    let stroke = if selected {
        egui::Stroke::new(1.5_f32, ACCENT)
    } else {
        egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(70, 80, 95))
    };
    let fill = if selected {
        egui::Color32::from_rgb(26, 44, 54)
    } else {
        egui::Color32::from_rgb(28, 33, 41)
    };
    let resp = egui::Frame::group(ui.style())
        .fill(fill)
        .stroke(stroke)
        .rounding(10.0)
        .inner_margin(egui::Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            ui.set_min_width(220.0);
            ui.label(egui::RichText::new(title).strong().size(15.0));
            for l in lines {
                ui.label(egui::RichText::new(*l).size(12.0).color(egui::Color32::from_rgb(150, 162, 178)));
            }
        })
        .response
        .interact(egui::Sense::click());
    resp.clicked()
}

impl eframe::App for SetupApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Poll worker
        if self.step == Step::Working {
            let finished = self
                .progress
                .lock()
                .map(|p| p.finished)
                .unwrap_or(false);
            if finished {
                let ok = self.progress.lock().map(|p| p.success).unwrap_or(false);
                self.step = if ok { Step::Done } else { Step::Error };
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("BMW ENET Gateway Setup").heading().strong());
            });
            ui.label(
                egui::RichText::new("Host on the ISTA / E-Sys desktop · Client on the laptop at the car")
                    .size(12.5)
                    .color(egui::Color32::from_rgb(150, 162, 178)),
            );
            ui.add_space(6.0);
            step_header(ui, self.step);
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(10.0);

            match self.step {
                Step::Welcome => {
                    ui.label("This wizard installs everything — no Rust, Git, or scripts needed.");
                    ui.add_space(8.0);
                    if download::has_embedded_packages() {
                        ui.colored_label(
                            OK_GREEN,
                            "✔ Host and Client packages are built into this Setup.exe (works offline).",
                        );
                    } else {
                        ui.label("Packages load from files next to Setup.exe, or download if the repo is public.");
                    }
                    match self.npcap_present {
                        Some(true) => {
                            ui.colored_label(OK_GREEN, "✔ Npcap detected — ISTA car traffic will work.");
                        }
                        Some(false) => {
                            ui.colored_label(
                                WARN_AMBER,
                                "⚠ Npcap not detected. Setup will download & launch the Npcap installer (enable WinPcap mode) for ISTA.",
                            );
                        }
                        None => {}
                    }
                    ui.add_space(8.0);
                    ui.label("Requirements: Windows 10/11 x64 · Administrator approval (UAC)");
                    ui.add_space(16.0);
                    if ui
                        .add(egui::Button::new(egui::RichText::new("Continue  →").strong()).min_size([120.0, 32.0].into()))
                        .clicked()
                    {
                        self.step = Step::ChooseRole;
                    }
                }
                Step::ChooseRole => {
                    ui.label(egui::RichText::new("Which PC is this?").strong().size(15.0));
                    ui.add_space(10.0);

                    ui.horizontal(|ui| {
                        if role_card(
                            ui,
                            self.role == Role::Host,
                            "🖥  Host — Desktop",
                            &[
                                "Runs ISTA+, E-Sys, BimmerUtility",
                                "Gateway + browser dashboard :47901",
                                "Creates BMW-ENET adapter for ISTA",
                            ],
                        ) {
                            self.role = Role::Host;
                        }
                        ui.add_space(8.0);
                        if role_card(
                            ui,
                            self.role == Role::Client,
                            "💻  Client — Laptop",
                            &[
                                "Stays at the car with the ENET cable",
                                "Status page :47903, auto-finds Host",
                                "Forwards car frames over Wi‑Fi",
                            ],
                        ) {
                            self.role = Role::Client;
                        }
                    });

                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        if ui.button("←  Back").clicked() {
                            self.step = Step::Welcome;
                        }
                        if ui
                            .add(egui::Button::new(egui::RichText::new("Next  →").strong()).min_size([100.0, 30.0].into()))
                            .clicked()
                        {
                            self.step = Step::Options;
                        }
                    });
                }
                Step::Options => {
                    ui.label(
                        egui::RichText::new(format!("Installing: {}", self.role.label()))
                            .strong()
                            .color(ACCENT),
                    );
                    ui.add_space(10.0);

                    if self.role == Role::Client {
                        ui.group(|ui| {
                            ui.label(egui::RichText::new("Pairing").strong());
                            ui.add_space(4.0);
                            ui.label("Pair code from the Host dashboard (recommended):");
                            ui.add(egui::TextEdit::singleline(&mut self.pair_code).hint_text("BMW-XXXX"));
                            ui.add_space(6.0);
                            ui.label("Desktop IP hint (optional — auto-detected by pair code):");
                            ui.add(egui::TextEdit::singleline(&mut self.peer).hint_text("leave blank for auto-find"));
                            ui.small("Only needed if Auto-find fails (Guest Wi‑Fi / AP isolation).");
                        });
                        ui.add_space(8.0);
                    }

                    ui.group(|ui| {
                        ui.label(egui::RichText::new("Security").strong());
                        ui.add_space(4.0);
                        ui.label("Shared password (optional; recommended for Internet / relay):");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.password)
                                .password(true)
                                .hint_text("empty = no encryption on LAN"),
                        );
                    });
                    ui.add_space(8.0);

                    ui.group(|ui| {
                        ui.label(egui::RichText::new("Launch").strong());
                        ui.add_space(4.0);
                        ui.checkbox(&mut self.start_now, "Start now");
                        ui.checkbox(&mut self.start_service, "Auto-start at boot (recommended)");
                        let open_label = if self.role == Role::Host {
                            "Open dashboard when finished (http://127.0.0.1:47901/)"
                        } else {
                            "Open Client status when finished (http://127.0.0.1:47903/)"
                        };
                        ui.checkbox(&mut self.open_dashboard, open_label);
                    });

                    if self.npcap_present == Some(false) {
                        ui.add_space(6.0);
                        ui.colored_label(
                            WARN_AMBER,
                            "⚠ Npcap missing — Setup downloads & launches the installer. ISTA needs it on BOTH PCs.",
                        );
                    }

                    ui.add_space(6.0);
                    ui.small(format!("Release source: github.com/{}", self.repo));
                    ui.small(format!("Setup folder: {}", self.setup_dir.display()));

                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        if ui.button("←  Back").clicked() {
                            self.step = Step::ChooseRole;
                        }
                        if ui
                            .add(
                                egui::Button::new(egui::RichText::new("Install").strong().size(15.0))
                                    .fill(egui::Color32::from_rgb(20, 120, 80))
                                    .min_size([130.0, 34.0].into()),
                            )
                            .clicked()
                        {
                            self.start_worker();
                        }
                    });
                }
                Step::Working => {
                    let (msg, done, total) = self
                        .progress
                        .lock()
                        .map(|p| (p.message.clone(), p.done, p.total))
                        .unwrap_or_default();
                    ui.label(egui::RichText::new("Installing…").strong().size(15.0));
                    ui.add_space(10.0);
                    if total > 0 {
                        let frac = (done as f32 / total as f32).clamp(0.0, 1.0);
                        ui.add(egui::ProgressBar::new(frac).text(format!(
                            "{:.1} / {:.1} MB",
                            done as f32 / 1_048_576.0,
                            total as f32 / 1_048_576.0
                        )));
                    } else {
                        ui.add(egui::ProgressBar::new(0.4).animate(true));
                    }
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new(&msg).size(13.0).color(egui::Color32::from_rgb(150, 162, 178)));
                }
                Step::Done => {
                    let summary = self
                        .progress
                        .lock()
                        .map(|p| p.result_summary.clone())
                        .unwrap_or_default();
                    ui.colored_label(OK_GREEN, egui::RichText::new("✔ Installation complete").strong().size(16.0));
                    ui.add_space(10.0);
                    egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                        ui.label(summary);
                    });
                    ui.add_space(14.0);
                    if ui
                        .add(egui::Button::new(egui::RichText::new("Close").strong()).min_size([110.0, 30.0].into()))
                        .clicked()
                    {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
                Step::Error => {
                    let err = self
                        .progress
                        .lock()
                        .map(|p| p.error.clone())
                        .unwrap_or_default();
                    ui.colored_label(ERR_RED, egui::RichText::new("✕ Setup failed").strong().size(16.0));
                    ui.add_space(8.0);
                    egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                        ui.label(&err);
                    });
                    ui.add_space(8.0);
                    ui.label("Tips:");
                    ui.label("• Run as Administrator");
                    ui.label("• Prefer the Setup.exe from the latest Release (packages are built in)");
                    ui.label("• Or extract BMW-ENET-Windows-Installer.zip and keep the role zip next to Setup.exe");
                    ui.label("• Private GitHub repos cannot be downloaded anonymously");
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        if ui.button("←  Back").clicked() {
                            self.worker_started = false;
                            if let Ok(mut p) = self.progress.lock() {
                                p.finished = false;
                                p.success = false;
                                p.error.clear();
                                p.message.clear();
                            }
                            self.step = Step::Options;
                        }
                        if ui.button("Close").clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                }
            }
        });
    }
}

fn main() -> eframe::Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("info")
        .try_init();

    let args = Args::parse();

    if let Some(role_s) = args.role.as_deref() {
        let role = match role_s.to_ascii_lowercase().as_str() {
            "host" | "desktop" | "gateway" => Role::Host,
            "client" | "laptop" | "agent" => Role::Client,
            other => {
                eprintln!("Unknown --role {other}. Use host or client.");
                std::process::exit(2);
            }
        };
        let setup_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));
        let work = std::env::temp_dir().join("bmw-enet-setup-cli");
        let progress = |_: u64, _: u64, msg: &str| println!("… {msg}");
        match download::prepare_package(role, &args.repo, &setup_dir, &work, &progress)
            .and_then(|pkg| {
                install::run_install(
                    &InstallRequest {
                        role,
                        pair_code: args.pair_code,
                        peer: args.peer,
                        password: args.password,
                        start_service: true,
                        start_now: true,
                        open_dashboard: true,
                    },
                    &pkg,
                    &progress,
                )
            }) {
            Ok(r) => {
                println!("Installed to {}", r.install_dir.display());
                if !r.pair_code_hint.is_empty() {
                    println!("Pair code: {}", r.pair_code_hint);
                }
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("ERROR: {e:#}");
                std::process::exit(1);
            }
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([560.0, 480.0])
            .with_min_inner_size([480.0, 400.0])
            .with_title("BMW ENET Gateway Setup"),
        ..Default::default()
    };

    eframe::run_native(
        "BMW ENET Gateway Setup",
        options,
        Box::new(move |_cc| Ok(Box::new(SetupApp::new(args.repo.clone())))),
    )
}
