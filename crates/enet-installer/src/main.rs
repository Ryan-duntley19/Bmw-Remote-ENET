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
    password: String,
    start_service: bool,
    open_dashboard: bool,
    repo: String,
    setup_dir: PathBuf,
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
            password: String::new(),
            start_service: true,
            open_dashboard: true,
            repo,
            setup_dir,
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
            password: self.password.clone(),
            start_service: self.start_service,
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
                                "\nNext: on the laptop, run this same Setup.exe and choose Client.\n",
                            );
                        } else {
                            summary.push_str(
                                "\nNext: plug ENET into the car, ignition ON, wait for green lights on the desktop dashboard.\n",
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
            ui.add_space(12.0);
            ui.heading("BMW ENET Gateway Setup");
            ui.label("Install the Host on your desktop (ISTA / E-Sys) or the Client on the laptop at the car.");
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(12.0);

            match self.step {
                Step::Welcome => {
                    ui.label("This wizard downloads the correct Windows package for you.");
                    ui.label("You do not need Rust, Git, or .bat scripts.");
                    ui.add_space(8.0);
                    ui.label("Requirements:");
                    ui.label("• Windows 10/11 x64");
                    ui.label("• Administrator approval (UAC)");
                    ui.label("• Internet access once (to download the Host or Client package)");
                    ui.add_space(16.0);
                    if ui.button("Continue").clicked() {
                        self.step = Step::ChooseRole;
                    }
                }
                Step::ChooseRole => {
                    ui.label("Which PC is this?");
                    ui.add_space(8.0);

                    ui.horizontal(|ui| {
                        let host = ui.selectable_label(
                            self.role == Role::Host,
                            "Host (Desktop)",
                        );
                        if host.clicked() {
                            self.role = Role::Host;
                        }
                        let client = ui.selectable_label(
                            self.role == Role::Client,
                            "Client (Laptop)",
                        );
                        if client.clicked() {
                            self.role = Role::Client;
                        }
                    });
                    ui.add_space(8.0);

                    match self.role {
                        Role::Host => {
                            ui.label("Host installs the desktop gateway + browser dashboard.");
                            ui.label("Run this on the PC that has ISTA+, E-Sys, BimmerUtility, etc.");
                        }
                        Role::Client => {
                            ui.label("Client installs the laptop agent for the ENET (OBD) cable.");
                            ui.label("Run this on the PC that stays near the car.");
                        }
                    }

                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        if ui.button("Back").clicked() {
                            self.step = Step::Welcome;
                        }
                        if ui.button("Next").clicked() {
                            self.step = Step::Options;
                        }
                    });
                }
                Step::Options => {
                    ui.label(format!("Role: {}", self.role.label()));
                    ui.add_space(8.0);

                    if self.role == Role::Client {
                        ui.label("Pair code from the Host dashboard (optional — leave blank to auto-find on LAN):");
                        ui.text_edit_singleline(&mut self.pair_code);
                        ui.add_space(6.0);
                    }

                    ui.label("Optional shared password (recommended for Internet / relay):");
                    ui.text_edit_singleline(&mut self.password);
                    ui.add_space(6.0);

                    ui.checkbox(&mut self.start_service, "Install and start Windows service (auto-start)");
                    if self.role == Role::Host {
                        ui.checkbox(
                            &mut self.open_dashboard,
                            "Open dashboard when finished (http://127.0.0.1:47901/)",
                        );
                    }

                    ui.add_space(8.0);
                    ui.small(format!("Release source: github.com/{}", self.repo));
                    ui.small(format!("Setup folder: {}", self.setup_dir.display()));

                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        if ui.button("Back").clicked() {
                            self.step = Step::ChooseRole;
                        }
                        if ui
                            .add(egui::Button::new("Install").fill(egui::Color32::from_rgb(20, 120, 80)))
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
                    ui.label("Working — please wait...");
                    ui.add_space(8.0);
                    ui.label(&msg);
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
                }
                Step::Done => {
                    let summary = self
                        .progress
                        .lock()
                        .map(|p| p.result_summary.clone())
                        .unwrap_or_default();
                    ui.colored_label(egui::Color32::from_rgb(40, 160, 90), "Success");
                    ui.add_space(8.0);
                    ui.label(summary);
                    ui.add_space(16.0);
                    if ui.button("Close").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
                Step::Error => {
                    let err = self
                        .progress
                        .lock()
                        .map(|p| p.error.clone())
                        .unwrap_or_default();
                    ui.colored_label(egui::Color32::from_rgb(200, 60, 60), "Setup failed");
                    ui.add_space(8.0);
                    ui.label(&err);
                    ui.add_space(8.0);
                    ui.label("Tips:");
                    ui.label("• Run as Administrator");
                    ui.label("• Check internet access, or place the Host/Client zip next to this Setup.exe");
                    ui.label("• Confirm a Windows release exists on GitHub with the package assets");
                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        if ui.button("Back").clicked() {
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
                        password: args.password,
                        start_service: true,
                        open_dashboard: role == Role::Host,
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
            .with_inner_size([560.0, 420.0])
            .with_min_inner_size([480.0, 360.0])
            .with_title("BMW ENET Gateway Setup"),
        ..Default::default()
    };

    eframe::run_native(
        "BMW ENET Gateway Setup",
        options,
        Box::new(move |_cc| Ok(Box::new(SetupApp::new(args.repo.clone())))),
    )
}
