// snapocr — select a screen region, OCR it, copy to clipboard.
// Wayland: grim for screenshot (works on Hyprland, Sway, etc.)
// X11/Xwayland: xcap native XCB grab.
use std::process::Command;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use arboard::Clipboard;
use eframe::egui::{
    self, Align2, Color32, ColorImage, FontId, Image, Pos2, Rect, ScrollArea, Sense, Stroke,
    StrokeKind, Vec2,
};
use image::RgbaImage;

const MIN_SEL: f32 = 6.0;
const AUTO_CLOSE: Duration = Duration::from_millis(2500);

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
        let x = (sel.min.x.max(0.0) as u32).min(self.raw.width() - 1);
        let y = (sel.min.y.max(0.0) as u32).min(self.raw.height() - 1);
        let w = (sel.width() as u32).clamp(1, self.raw.width() - x);
        let h = (sel.height() as u32).clamp(1, self.raw.height() - y);

        let crop = image::imageops::crop_imm(&self.raw, x, y, w, h).to_image();
        let png_path = std::env::temp_dir().join(format!("snapocr-{}.png", std::process::id()));
        let lang = self.lang.clone();
        let debug = self.debug;

        self.stage = Stage::Ocr(Some(std::thread::spawn(move || {
            crop.save(&png_path)
                .map_err(|e| format!("save crop: {e}"))?;
            let out = Command::new("tesseract")
                .arg(&png_path)
                .arg("stdout")
                .arg("-l")
                .arg(&lang)
                .arg("--psm")
                .arg("6")
                .output()
                .map_err(|e| format!("tesseract: {e}"))?;
            if !out.status.success() {
                let msg = String::from_utf8_lossy(&out.stderr);
                return Err(format!("tesseract failed: {}", msg.trim()));
            }
            let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !debug {
                let _ = std::fs::remove_file(&png_path);
            }
            if text.is_empty() {
                return Err("no text found in region".to_string());
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
                let dim = Color32::from_black_alpha(110);
                let img_w = self.raw.width() as f32;
                let img_h = self.raw.height() as f32;

                // dim everything outside the selection (4 rects: top, bottom, left, right)
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
                    Stroke::new(2.0, Color32::from_rgb(255, 90, 90)),
                    StrokeKind::Outside,
                );

                // size label
                painter.text(
                    r.min + Vec2::new(4.0, -20.0),
                    Align2::LEFT_BOTTOM,
                    format!("{}×{}", r.width() as u32, r.height() as u32),
                    FontId::monospace(14.0),
                    Color32::WHITE,
                );
            }
        });

        if let Some(r) = ocr_sel {
            self.begin_ocr(r);
        }

        // status banner at bottom center
        egui::Area::new(egui::Id::new("status"))
            .anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -24.0))
            .show(&ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| match &self.stage {
                    Stage::Ready => {
                        ui.label("drag to select a region · esc to quit");
                    }
                    Stage::Ocr(_) => {
                        ui.label("ocr…");
                    }
                    Stage::Done { text, .. } => {
                        ui.colored_label(
                            Color32::from_rgb(120, 220, 120),
                            "copied to clipboard ✓",
                        );
                        ui.separator();
                        ui.label(egui::RichText::new(text).monospace().size(13.0));
                    }
                    Stage::Fail { msg, .. } => {
                        ui.colored_label(
                            Color32::from_rgb(255, 120, 120),
                            format!("✗ {msg} · esc to quit"),
                        );
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
        let out = Command::new("grim")
            .arg(&tmp)
            .output();
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
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([w, h])
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
