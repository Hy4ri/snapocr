// snapocr — select a screen region, OCR it, copy to clipboard.
// Wayland: grim for screenshot (works on Hyprland, Sway, etc.)
// X11/Xwayland: xcap native XCB grab.
use std::io::Write;
use std::process::{Command, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use arboard::Clipboard;
use eframe::egui::{
    self, Align2, Color32, ColorImage, CornerRadius, FontId, Image, Pos2, Rect, ScrollArea, Sense,
    Stroke, StrokeKind, Vec2,
};
use image::{ImageFormat, RgbaImage};

const MIN_SEL: f32 = 6.0;
const AUTO_CLOSE: Duration = Duration::from_millis(2800);

enum Stage {
    Ready,
    Ocr(Option<JoinHandle<Result<String, String>>>),
    Done { at: Instant, text: String },
    Fail { at: Instant, msg: String },
}

struct SnapApp {
    raw: RgbaImage,
    tex: egui::TextureHandle,
    start: Option<Pos2>,
    cur: Option<Pos2>,
    stage: Stage,
    lang: String,
    debug: bool,
}

impl SnapApp {
    fn begin_ocr(&mut self, sel: Rect) {
        let x = (sel.min.x.max(0.0) as u32).min(self.raw.width().saturating_sub(1));
        let y = (sel.min.y.max(0.0) as u32).min(self.raw.height().saturating_sub(1));
        let w = (sel.width() as u32).clamp(1, self.raw.width().saturating_sub(x));
        let h = (sel.height() as u32).clamp(1, self.raw.height().saturating_sub(y));

        let crop = image::imageops::crop_imm(&self.raw, x, y, w, h).to_image();
        let lang = self.lang.clone();
        let debug = self.debug;

        self.stage = Stage::Ocr(Some(std::thread::spawn(move || {
            // Speed optimization: In-memory grayscale + uncompressed/fast PPM encode over stdin
            // avoids disk I/O latency completely.
            let gray = image::DynamicImage::ImageRgba8(crop).to_luma8();
            let mut ppm_bytes = Vec::with_capacity((w * h + 64) as usize);
            gray.write_to(
                &mut std::io::Cursor::new(&mut ppm_bytes),
                ImageFormat::Pnm,
            )
            .map_err(|e| format!("ppm encode error: {e}"))?;

            if debug {
                let png_path =
                    std::env::temp_dir().join(format!("snapocr-{}.png", std::process::id()));
                let _ = gray.save(&png_path);
            }

            let mut child = Command::new("tesseract")
                .arg("stdin")
                .arg("stdout")
                .arg("-l")
                .arg(&lang)
                .arg("--psm")
                .arg("6")
                .arg("-c")
                .arg("tessedit_do_invert=0")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| format!("failed to spawn tesseract: {e}"))?;

            if let Some(mut stdin) = child.stdin.take() {
                stdin
                    .write_all(&ppm_bytes)
                    .map_err(|e| format!("failed to write image to tesseract stdin: {e}"))?;
            }

            let out = child
                .wait_with_output()
                .map_err(|e| format!("tesseract failed while waiting: {e}"))?;

            if !out.status.success() {
                let msg = String::from_utf8_lossy(&out.stderr);
                return Err(format!("tesseract failed: {}", msg.trim()));
            }

            let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if text.is_empty() {
                return Err("No text detected in region".to_string());
            }
            Ok(text)
        })));
    }

    fn poll_ocr(&mut self) {
        if let Stage::Ocr(ref mut opt) = self.stage {
            if opt.as_ref().map_or(false, |h| h.is_finished()) {
                if let Some(handle) = opt.take() {
                    let res = handle
                        .join()
                        .unwrap_or_else(|_| Err("ocr thread panicked".to_string()));
                    self.stage = match res {
                        Ok(text) => {
                            match Clipboard::new().and_then(|mut c| c.set_text(text.clone())) {
                                Ok(_) => Stage::Done {
                                    at: Instant::now(),
                                    text,
                                },
                                Err(e) => Stage::Fail {
                                    at: Instant::now(),
                                    msg: format!("clipboard: {e}"),
                                },
                            }
                        }
                        Err(msg) => Stage::Fail {
                            at: Instant::now(),
                            msg,
                        },
                    };
                }
            }
        }
    }

    fn maybe_auto_close(&self, ctx: &egui::Context) {
        let close = match &self.stage {
            Stage::Done { at, .. } | Stage::Fail { at, .. } => at.elapsed() > AUTO_CLOSE,
            _ => false,
        };
        if close {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

impl eframe::App for SnapApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_ocr();
        self.maybe_auto_close(&ctx);

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        let mut ocr_sel: Option<Rect> = None;

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                ScrollArea::both().show(ui, |ui| {
                    let resp = ui.add(Image::new(&self.tex).sense(Sense::drag()));
                    let origin = resp.rect.min;

                    if resp.drag_started() {
                        self.start = resp
                            .interact_pointer_pos()
                            .map(|p| Pos2::new(p.x - origin.x, p.y - origin.y));
                        self.cur = self.start;
                    }
                    if resp.dragged() {
                        self.cur = resp
                            .interact_pointer_pos()
                            .map(|p| Pos2::new(p.x - origin.x, p.y - origin.y));
                    }
                    if resp.drag_stopped() {
                        if let (Some(s), Some(c)) = (self.start, self.cur) {
                            let r = Rect::from_two_pos(s, c);
                            if r.width() >= MIN_SEL && r.height() >= MIN_SEL {
                                ocr_sel = Some(r);
                            }
                        }
                        self.start = None;
                        self.cur = None;
                    }

                    // draw selection overlay
                    if let (Some(s), Some(c)) = (self.start, self.cur) {
                        let a = Pos2::new(origin.x + s.x, origin.y + s.y);
                        let b = Pos2::new(origin.x + c.x, origin.y + c.y);
                        let r = Rect::from_two_pos(a, b);
                        let painter = ui.painter();
                        let dim = Color32::from_black_alpha(115);
                        let img_w = self.raw.width() as f32;
                        let img_h = self.raw.height() as f32;

                        // dim everything outside the selection (4 rects)
                        painter.rect_filled(
                            Rect::from_min_max(
                                Pos2::new(origin.x, origin.y),
                                Pos2::new(origin.x + img_w, r.min.y.max(origin.y)),
                            ),
                            0.0,
                            dim,
                        );
                        painter.rect_filled(
                            Rect::from_min_max(
                                Pos2::new(origin.x, r.max.y.min(origin.y + img_h)),
                                Pos2::new(origin.x + img_w, origin.y + img_h),
                            ),
                            0.0,
                            dim,
                        );
                        painter.rect_filled(
                            Rect::from_min_max(
                                Pos2::new(origin.x, r.min.y.max(origin.y)),
                                Pos2::new(r.min.x.max(origin.x), r.max.y.min(origin.y + img_h)),
                            ),
                            0.0,
                            dim,
                        );
                        painter.rect_filled(
                            Rect::from_min_max(
                                Pos2::new(r.max.x.min(origin.x + img_w), r.min.y.max(origin.y)),
                                Pos2::new(origin.x + img_w, r.max.y.min(origin.y + img_h)),
                            ),
                            0.0,
                            dim,
                        );

                        // selection border
                        painter.rect_stroke(
                            r,
                            0.0,
                            Stroke::new(2.0, Color32::from_rgb(255, 75, 75)),
                            StrokeKind::Outside,
                        );

                        // size label tooltip
                        painter.text(
                            r.min + Vec2::new(4.0, -18.0),
                            Align2::LEFT_BOTTOM,
                            format!("{}×{}", r.width() as u32, r.height() as u32),
                            FontId::monospace(13.0),
                            Color32::WHITE,
                        );
                    }
                });
            });

        if let Some(r) = ocr_sel {
            self.begin_ocr(r);
        }

        // Modern adaptive bottom notification bar
        let screen_rect = ctx.content_rect();
        let max_card_w = (screen_rect.width() * 0.7).clamp(320.0, 720.0);

        egui::Area::new(egui::Id::new("bottom_status_bar"))
            .anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -28.0))
            .show(&ctx, |ui| {
                let bg_frame = egui::Frame::NONE
                    .fill(Color32::from_rgba_premultiplied(18, 18, 24, 235))
                    .corner_radius(CornerRadius::same(10))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(50, 50, 65)))
                    .inner_margin(egui::Margin::symmetric(16, 12));

                bg_frame.show(ui, |ui| {
                    ui.set_max_width(max_card_w);
                    match &self.stage {
                        Stage::Ready => {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new("✂")
                                        .size(15.0)
                                        .color(Color32::from_rgb(180, 180, 200)),
                                );
                                ui.label(
                                    egui::RichText::new("Drag to select area  •  Esc to cancel")
                                        .size(13.5)
                                        .color(Color32::from_rgb(220, 220, 230)),
                                );
                            });
                        }
                        Stage::Ocr(_) => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(
                                    egui::RichText::new("Recognizing text...")
                                        .size(14.0)
                                        .color(Color32::from_rgb(200, 210, 255)),
                                );
                            });
                        }
                        Stage::Done { text, .. } => {
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new("✓ Copied to clipboard")
                                            .size(13.5)
                                            .strong()
                                            .color(Color32::from_rgb(90, 225, 140)),
                                    );
                                });
                                ui.add_space(4.0);
                                egui::Frame::NONE
                                    .fill(Color32::from_rgba_premultiplied(10, 10, 15, 200))
                                    .corner_radius(CornerRadius::same(6))
                                    .inner_margin(egui::Margin::symmetric(10, 8))
                                    .show(ui, |ui| {
                                        ScrollArea::vertical()
                                            .max_height(140.0)
                                            .auto_shrink([false, true])
                                            .show(ui, |ui| {
                                                ui.add(
                                                    egui::Label::new(
                                                        egui::RichText::new(text)
                                                            .monospace()
                                                            .size(12.5)
                                                            .color(Color32::from_rgb(
                                                                240, 240, 245,
                                                            )),
                                                    )
                                                    .wrap(),
                                                );
                                            });
                                    });
                            });
                        }
                        Stage::Fail { msg, .. } => {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new("✗")
                                        .size(15.0)
                                        .color(Color32::from_rgb(255, 95, 95)),
                                );
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(msg)
                                            .size(13.5)
                                            .color(Color32::from_rgb(255, 150, 150)),
                                    )
                                    .wrap(),
                                );
                            });
                        }
                    }
                });
            });
    }
}

/// Capture the screen. Wayland: grim → file → load. X11: xcap native.
fn capture_screen() -> Result<RgbaImage, String> {
    // try grim first (wayland - works on hyprland, sway, etc.)
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        let tmp = std::env::temp_dir().join(format!("snapocr-grab-{}.png", std::process::id()));
        let out = Command::new("grim").arg(&tmp).output();
        match out {
            Ok(o) if o.status.success() => {
                let img = image::open(&tmp)
                    .map_err(|e| format!("load grim output: {e}"))?
                    .to_rgba8();
                let _ = std::fs::remove_file(&tmp);
                if img.width() > 0 && img.height() > 0 {
                    return Ok(img);
                }
            }
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                eprintln!("snapocr: grim failed: {}", stderr.trim());
            }
            Err(e) => {
                eprintln!("snapocr: grim not found ({e}), trying xcap...");
            }
        }
    }

    // fallback: xcap (works on X11/Xwayland)
    let monitors = xcap::Monitor::all().map_err(|e| format!("xcap monitors: {e}"))?;
    let monitor = monitors
        .into_iter()
        .next()
        .ok_or_else(|| "no monitors found".to_string())?;
    let raw = monitor
        .capture_image()
        .map_err(|e| format!("xcap capture: {e}"))?;
    if raw.width() == 0 || raw.height() == 0 {
        return Err("captured empty image".to_string());
    }
    Ok(raw)
}

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let lang = args
        .windows(2)
        .find(|w| w[0] == "--lang")
        .map(|w| w[1].clone())
        .unwrap_or_else(|| "eng".to_string());
    let debug = args.iter().any(|a| a == "--debug");

    let raw = match capture_screen() {
        Ok(img) => img,
        Err(e) => {
            eprintln!("snapocr: {e}");
            eprintln!("  wayland: install grim (nix-shell -p grim)");
            eprintln!("  x11: make sure DISPLAY is set");
            std::process::exit(1);
        }
    };

    let w = raw.width() as f32;
    let h = raw.height() as f32;

    // Viewport configured for true borderless fullscreen snapshot experience
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([w, h])
        .with_fullscreen(true)
        .with_decorations(false)
        .with_always_on_top()
        .with_active(true)
        .with_title("snapocr");

    eframe::run_native(
        "snapocr",
        eframe::NativeOptions {
            viewport,
            ..Default::default()
        },
        Box::new(move |cc| {
            let color = ColorImage::from_rgba_unmultiplied(
                [raw.width() as usize, raw.height() as usize],
                raw.as_raw(),
            );
            let tex = cc
                .egui_ctx
                .load_texture("screen", color, egui::TextureOptions::NEAREST);
            Ok(Box::new(SnapApp {
                raw,
                tex,
                start: None,
                cur: None,
                stage: Stage::Ready,
                lang,
                debug,
            }))
        }),
    )
}
