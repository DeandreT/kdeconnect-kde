// SPDX-FileCopyrightText: 2026 Deandre Thomas
// SPDX-License-Identifier: GPL-3.0-only

mod backend;

use backend::{Backend, Device};
use eframe::egui;
use kdeconnect_sms::{Draft, ImageAttachment, MAX_MESSAGE_BYTES};
use std::{sync::mpsc, time::Duration};

enum Command {
    Refresh,
    Send(Device, Draft),
}
enum Reply {
    Devices(Result<Vec<Device>, String>),
    Sent(Result<(), String>),
}

struct Desktop {
    demo: bool,
    draft: Draft,
    devices: Vec<Device>,
    selected: Option<usize>,
    images: Vec<egui::TextureHandle>,
    status: String,
    pending: bool,
    confirm: bool,
    commands: mpsc::Sender<Command>,
    replies: mpsc::Receiver<Reply>,
    clipboard: Option<arboard::Clipboard>,
}

impl Desktop {
    fn new(ctx: egui::Context, demo: bool) -> Self {
        let (commands, requests) = mpsc::channel();
        let (responses, replies) = mpsc::channel();
        std::thread::spawn(move || {
            let mut backend = None;
            for command in requests {
                let result = if demo {
                    Ok(())
                } else if backend.is_none() {
                    Backend::connect().map(|connected| backend = Some(connected))
                } else {
                    Ok(())
                };
                let reply = match command {
                    Command::Refresh => Reply::Devices(if demo {
                        Ok(vec![Device {
                            id: "demo".into(),
                            name: "Demo phone (no messages sent)".into(),
                        }])
                    } else {
                        result
                            .and_then(|_| {
                                backend
                                    .as_ref()
                                    .ok_or_else(|| anyhow::anyhow!("not connected"))?
                                    .devices()
                            })
                            .map_err(|e| e.to_string())
                    }),
                    Command::Send(device, draft) => Reply::Sent(if demo {
                        Ok(())
                    } else {
                        result
                            .and_then(|_| {
                                backend
                                    .as_ref()
                                    .ok_or_else(|| anyhow::anyhow!("not connected"))?
                                    .send(&device, &draft)
                            })
                            .map_err(|e| e.to_string())
                    }),
                };
                if responses.send(reply).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
        let _ = commands.send(Command::Refresh);
        Self {
            demo,
            draft: Draft::default(),
            devices: vec![],
            selected: None,
            images: vec![],
            status: "Connecting to the installed KDE Connect service...".into(),
            pending: false,
            confirm: false,
            commands,
            replies,
            clipboard: arboard::Clipboard::new().ok(),
        }
    }

    fn paste_image(&mut self, ctx: &egui::Context, require_image: bool) -> bool {
        let result = self
            .clipboard
            .as_mut()
            .ok_or(arboard::Error::ClipboardNotSupported)
            .and_then(|clipboard| clipboard.get_image());
        match result {
            Ok(image) => {
                let size = [image.width, image.height];
                let result =
                    ImageAttachment::from_rgba(image.width, image.height, image.bytes.as_ref())
                        .and_then(|attachment| self.draft.attach(attachment));
                match result {
                    Ok(()) => {
                        self.images.push(ctx.load_texture(
                            format!("attachment-{}", self.images.len()),
                            egui::ColorImage::from_rgba_unmultiplied(size, image.bytes.as_ref()),
                            Default::default(),
                        ));
                        self.status = "Image attached. Review it before sending.".into();
                    }
                    Err(error) => self.status = error.to_string(),
                }
                true
            }
            Err(error) => {
                if require_image {
                    self.status = format!("No readable clipboard image: {error}");
                }
                false
            }
        }
    }

    fn clipboard_shortcuts(&mut self, ctx: &egui::Context) {
        if self.pending || self.confirm {
            return;
        }
        let paste = ctx.input_mut(|input| {
            input.consume_key(egui::Modifiers::COMMAND, egui::Key::V)
                || input.consume_key(egui::Modifiers::SHIFT, egui::Key::Insert)
        });
        if paste {
            let composer_focused =
                ctx.memory(|memory| memory.has_focus(egui::Id::unique("message-editor")));
            if composer_focused && self.paste_image(ctx, false) {
                return;
            }
            if let Ok(text) =
                arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text())
            {
                ctx.input_mut(|input| input.events.push(egui::Event::Paste(text)));
            }
        }
        for (key, event) in [
            (egui::Key::C, egui::Event::Copy),
            (egui::Key::X, egui::Event::Cut),
        ] {
            if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, key)) {
                ctx.input_mut(|input| input.events.push(event));
            }
        }
    }

    fn copy_image(&mut self, index: usize) {
        let image = &self.draft.attachments[index];
        let result = image.rgba().and_then(|bytes| {
            let clipboard = self
                .clipboard
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("clipboard unavailable"))?;
            clipboard.set_image(arboard::ImageData {
                width: image.width as usize,
                height: image.height as usize,
                bytes: std::borrow::Cow::Owned(bytes),
            })?;
            Ok(())
        });
        self.status = match result {
            Ok(()) => "Image copied to clipboard.".into(),
            Err(error) => error.to_string(),
        };
    }
}

impl eframe::App for Desktop {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.pending && ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.status = "Wait for the pending request before closing.".into();
        }
        self.clipboard_shortcuts(&ctx);
        for reply in self.replies.try_iter() {
            match reply {
                Reply::Devices(Ok(devices)) => {
                    self.devices = devices;
                    self.selected = (!self.devices.is_empty()).then_some(0);
                    self.status = if self.devices.is_empty() {
                        "No paired, reachable phones. Pair in KDE Connect, then refresh.".into()
                    } else {
                        "Ready. New messages use your phone's default SIM.".into()
                    };
                }
                Reply::Devices(Err(error)) => self.status = error,
                Reply::Sent(result) => {
                    self.pending = false;
                    match result {
                        Ok(()) => {
                            self.draft.text.clear();
                            self.draft.attachments.clear();
                            self.images.clear();
                            self.status = if self.demo {
                                "Demo send completed. Nothing was transmitted.".into()
                            } else {
                                "Request handed to KDE Connect. Phone delivery is not confirmed."
                                    .into()
                            };
                        }
                        Err(error) => {
                            self.status = format!(
                                "Send result uncertain: {error}. Draft retained; check your phone before retrying."
                            )
                        }
                    }
                }
            }
        }

        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("KDE Connect Rust");
            ui.label("SMS/MMS composer - Rust port with installed-daemon compatibility");
            if self.demo { ui.colored_label(egui::Color32::YELLOW, "DEMO MODE: no phone access and no messages transmitted"); }
            ui.separator();
            ui.add_enabled_ui(!self.pending && !self.confirm, |ui| {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_label("Phone")
                        .selected_text(self.selected.and_then(|i| self.devices.get(i)).map_or("Select a phone", |d| &d.name))
                        .show_ui(ui, |ui| {
                            for (i, device) in self.devices.iter().enumerate() {
                                ui.selectable_value(&mut self.selected, Some(i), &device.name);
                            }
                        });
                    if ui.button("Refresh devices").clicked() {
                        let _ = self.commands.send(Command::Refresh);
                        self.status = "Refreshing devices...".into();
                    }
                });
                ui.label("Recipient (phone number or SMS email address)");
                ui.add(egui::TextEdit::singleline(&mut self.draft.recipient).desired_width(f32::INFINITY));
                ui.label("Message");
                let editor_id = egui::Id::unique("message-editor");
                ui.add(egui::TextEdit::multiline(&mut self.draft.text).id(editor_id).desired_rows(6).desired_width(f32::INFINITY));
                ui.horizontal(|ui| {
                    if ui.button("Paste image").clicked() { self.paste_image(&ctx, true); }
                    if ui.button("Clear attachments").clicked() {
                        self.images.clear(); self.draft.attachments.clear();
                    }
                    ui.label(format!("{} / {MAX_MESSAGE_BYTES} bytes", self.draft.size()));
                });
                egui::ScrollArea::horizontal().max_height(160.0).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let mut copy = None;
                        for (index, image) in self.images.iter().enumerate() {
                            ui.vertical(|ui| {
                                ui.add(egui::Image::new(image).fit_to_exact_size(image.size_vec2() * (112.0 / image.size_vec2().max_elem())));
                                if ui.button("Copy image").clicked() { copy = Some(index); }
                            });
                        }
                        if let Some(index) = copy { self.copy_image(index); }
                    });
                });
                if ui.add_enabled(self.selected.is_some() && self.draft.validate().is_ok(), egui::Button::new("Review and send")).clicked() {
                    self.confirm = true;
                }
            });
            ui.separator();
            ui.label(&self.status);
            if self.pending { ui.spinner(); }
            ui.separator();
            ui.small("Not a full replacement yet: history, native pairing/networking, and other plugins are still being ported.");
        });
        if self.confirm {
            egui::Window::new("Confirm message")
                .collapsible(false)
                .resizable(false)
                .show(&ctx, |ui| {
                    ui.label(format!("Send to {}?", self.draft.recipient));
                    if let Some(device) = self.selected.and_then(|index| self.devices.get(index)) {
                        ui.label(format!("From phone: {}", device.name));
                    }
                    ui.label(format!(
                        "{} attachment(s). Uses the phone's default SIM.",
                        self.draft.attachments.len()
                    ));
                    ui.label("Carrier SMS/MMS charges may apply.");
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            self.confirm = false;
                        }
                        if ui
                            .button(if self.demo {
                                "Simulate send"
                            } else {
                                "Send message"
                            })
                            .clicked()
                        {
                            self.confirm = false;
                            if let Some(device) = self.selected.and_then(|i| self.devices.get(i)) {
                                if self
                                    .commands
                                    .send(Command::Send(device.clone(), self.draft.clone()))
                                    .is_ok()
                                {
                                    self.pending = true;
                                    self.status = "Sending request...".into();
                                } else {
                                    self.status =
                                        "Background worker stopped. Draft retained.".into();
                                }
                            }
                        }
                    });
                });
        }
        ctx.request_repaint_after(Duration::from_secs(1));
    }
}

fn main() -> eframe::Result {
    let mut demo = false;
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--demo" => demo = true,
            "--help" | "-h" => {
                println!(
                    "Usage: kdeconnect-rs [--demo]\n--demo  Run without accessing phones or transmitting messages"
                );
                return Ok(());
            }
            _ => {
                eprintln!("Unknown argument: {argument}. Use --help.");
                std::process::exit(2);
            }
        }
    }
    eframe::run_native(
        "KDE Connect Rust",
        eframe::NativeOptions {
            clipboard_shortcuts: false,
            viewport: egui::ViewportBuilder::default().with_inner_size([850.0, 650.0]),
            ..Default::default()
        },
        Box::new(move |cc| Ok(Box::new(Desktop::new(cc.egui_ctx.clone(), demo)))),
    )
}
