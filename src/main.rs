// snapocr — 100% self-contained Wayland & X11 screen OCR
// Zero external CLI dependencies (no slurp, no grim, no maim).
// Pure in-memory freeze frame capture + instant interactive drag crop.

use std::io::Write;
use std::process::{Command, Stdio};

use arboard::Clipboard;
use eframe::egui::{
    self, Align2, Color32, ColorImage, CornerRadius, FontId, Image, Pos2, Rect, ScrollArea, Sense,
    Stroke, StrokeKind, Vec2,
};
use image::{imageops::FilterType, GrayImage, ImageFormat, Luma, RgbaImage};

const MIN_SEL: f32 = 4.0;

struct SnapApp {
    raw: RgbaImage,
    tex: egui::TextureHandle,
    start: Option<Pos2>,
    cur: Option<Pos2>,
    selected_rect: Option<Rect>,
    done: bool,
}

impl eframe::App for SnapApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        if self.done {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

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
                                self.selected_rect = Some(r);
                                self.done = true;
                            }
                        }
                        self.start = None;
                        self.cur = None;
                    }

                    // Render selection box with dimmed outside
                    if let (Some(s), Some(c)) = (self.start, self.cur) {
                        let a = Pos2::new(origin.x + s.x, origin.y + s.y);
                        let b = Pos2::new(origin.x + c.x, origin.y + c.y);
                        let r = Rect::from_two_pos(a, b);
                        let painter = ui.painter();
                        let dim = Color32::from_black_alpha(115);
                        let img_w = self.raw.width() as f32;
                        let img_h = self.raw.height() as f32;

                        // 4 outer dimming rectangles
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

                        // Selection border
                        painter.rect_stroke(
                            r,
                            0.0,
                            Stroke::new(2.0, Color32::from_rgb(255, 75, 75)),
                            StrokeKind::Outside,
                        );

                        // Dimensions label
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

        // Bottom hint bar
        egui::Area::new(egui::Id::new("hint_bar"))
            .anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -28.0))
            .show(&ctx, |ui| {
                egui::Frame::NONE
                    .fill(Color32::from_rgba_premultiplied(18, 18, 24, 235))
                    .corner_radius(CornerRadius::same(10))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(50, 50, 65)))
                    .inner_margin(egui::Margin::symmetric(16, 10))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("✂")
                                    .size(15.0)
                                    .color(Color32::from_rgb(180, 180, 200)),
                            );
                            ui.label(
                                egui::RichText::new("Drag to crop area  •  Esc to cancel")
                                    .size(13.5)
                                    .color(Color32::from_rgb(220, 220, 230)),
                            );
                        });
                    });
            });
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let lang = args
        .windows(2)
        .find(|w| w[0] == "--lang")
        .map(|w| w[1].clone())
        .unwrap_or_else(|| "auto".to_string());
    let debug = args.iter().any(|a| a == "--debug");
    let no_notify = args.iter().any(|a| a == "--no-notify");

    // 1. Fullscreen screenshot in pure Rust (libwayshot for Wayland)
    let raw = match capture_fullscreen() {
        Ok(img) => img,
        Err(e) => {
            eprintln!("snapocr capture failed: {e}");
            notify_err(&format!("Capture error: {e}"), no_notify);
            std::process::exit(1);
        }
    };

    let w = raw.width() as f32;
    let h = raw.height() as f32;

    let selected_box: std::sync::Arc<std::sync::Mutex<Option<Rect>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let selected_box_clone = selected_box.clone();

    let raw_for_app = raw.clone();

    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([w, h])
        .with_fullscreen(true)
        .with_decorations(false)
        .with_always_on_top()
        .with_active(true)
        .with_title("snapocr");

    let _ = eframe::run_native(
        "snapocr",
        eframe::NativeOptions {
            viewport,
            ..Default::default()
        },
        Box::new(move |cc| {
            let color = ColorImage::from_rgba_unmultiplied(
                [raw_for_app.width() as usize, raw_for_app.height() as usize],
                raw_for_app.as_raw(),
            );
            let tex = cc
                .egui_ctx
                .load_texture("screen", color, egui::TextureOptions::NEAREST);

            struct AppWrapper {
                inner: SnapApp,
                dest: std::sync::Arc<std::sync::Mutex<Option<Rect>>>,
            }

            impl eframe::App for AppWrapper {
                fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
                    self.inner.ui(ui, frame);
                    if let Some(r) = self.inner.selected_rect {
                        *self.dest.lock().unwrap() = Some(r);
                    }
                }
            }

            Ok(Box::new(AppWrapper {
                inner: SnapApp {
                    raw: raw_for_app,
                    tex,
                    start: None,
                    cur: None,
                    selected_rect: None,
                    done: false,
                },
                dest: selected_box_clone,
            }))
        }),
    );

    // After window closes, check if a region was selected
    let maybe_rect = *selected_box.lock().unwrap();
    let sel = match maybe_rect {
        Some(r) => r,
        None => {
            // Cancelled via Escape or closed with no selection
            std::process::exit(0);
        }
    };

    // 2. Crop directly from in-memory raw image
    let x = (sel.min.x.max(0.0) as u32).min(raw.width().saturating_sub(1));
    let y = (sel.min.y.max(0.0) as u32).min(raw.height().saturating_sub(1));
    let cw = (sel.width() as u32).clamp(1, raw.width().saturating_sub(x));
    let ch = (sel.height() as u32).clamp(1, raw.height().saturating_sub(y));

    let crop = image::imageops::crop_imm(&raw, x, y, cw, ch).to_image();

    // 3. Preprocess for OCR in memory
    let ppm_bytes = match preprocess_image(&crop, debug) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("snapocr preprocess failed: {e}");
            notify_err(&format!("Preprocess failed: {e}"), no_notify);
            std::process::exit(1);
        }
    };

    // 4. Resolve languages & run Tesseract
    let resolved_lang = if lang == "auto" || lang.is_empty() {
        detect_available_langs().unwrap_or_else(|| "eng+ara".to_string())
    } else {
        lang
    };

    let text = match ocr_pipeline(&ppm_bytes, &resolved_lang) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("snapocr: {e}");
            notify_err(&e, no_notify);
            std::process::exit(1);
        }
    };

    // 5. Copy to clipboard
    if let Err(e) = copy_to_clipboard(&text) {
        eprintln!("snapocr: clipboard error: {e}");
    }

    println!("{text}");

    // 6. Desktop notification
    if !no_notify {
        let preview = if text.len() > 140 {
            format!(
                "{}…",
                &text[..text
                    .char_indices()
                    .map(|(i, _)| i)
                    .nth(140)
                    .unwrap_or(text.len())]
            )
        } else {
            text.clone()
        };
        let _ = Command::new("notify-send")
            .arg("-a")
            .arg("snapocr")
            .arg("-i")
            .arg("edit-copy")
            .arg("Copied to clipboard")
            .arg(&preview)
            .status();
    }
}

/// Capture fullscreen in pure Rust via libwayshot (Wayland)
fn capture_fullscreen() -> Result<RgbaImage, String> {
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        let wayshot_conn = libwayshot::WayshotConnection::new()
            .map_err(|e| format!("Failed to connect to wayland: {e}"))?;

        let img = wayshot_conn
            .screenshot_all(false)
            .map_err(|e| format!("screencopy error: {e}"))?;

        let rgba = image::RgbaImage::from_raw(img.width(), img.height(), img.to_rgba8().into_vec())
            .ok_or_else(|| "failed to convert wayshot buffer".to_string())?;

        Ok(rgba)
    } else {
        Err("WAYLAND_DISPLAY not set. Only Wayland is currently supported.".to_string())
    }
}

/// Grayscale + 2.5x upscale + dark mode auto-inversion + 24px white padding.
fn preprocess_image(crop: &RgbaImage, debug: bool) -> Result<Vec<u8>, String> {
    let w = crop.width();
    let h = crop.height();
    let target_w = (w as f32 * 2.5).round() as u32;
    let target_h = (h as f32 * 2.5).round() as u32;

    let dyn_img = image::DynamicImage::ImageRgba8(crop.clone());
    let scaled = dyn_img.resize_exact(target_w, target_h, FilterType::CatmullRom);
    let mut gray = scaled.to_luma8();

    // Auto-invert dark backgrounds
    let mut border_sum: u64 = 0;
    let mut border_count: u64 = 0;
    let gw = gray.width();
    let gh = gray.height();
    for gx in 0..gw {
        border_sum += gray.get_pixel(gx, 0)[0] as u64;
        border_sum += gray.get_pixel(gx, gh - 1)[0] as u64;
        border_count += 2;
    }
    for gy in 1..gh - 1 {
        border_sum += gray.get_pixel(0, gy)[0] as u64;
        border_sum += gray.get_pixel(gw - 1, gy)[0] as u64;
        border_count += 2;
    }
    let avg_border_luma = (border_sum / border_count.max(1)) as u8;
    if avg_border_luma < 128 {
        for p in gray.pixels_mut() {
            p[0] = 255 - p[0];
        }
    }

    // 24px border padding
    let pad = 24u32;
    let padded_w = gw + pad * 2;
    let padded_h = gh + pad * 2;
    let mut padded_img = GrayImage::from_pixel(padded_w, padded_h, Luma([255]));
    image::imageops::overlay(&mut padded_img, &gray, pad as i64, pad as i64);

    let mut ppm_bytes = Vec::with_capacity((padded_w * padded_h + 64) as usize);
    padded_img
        .write_to(&mut std::io::Cursor::new(&mut ppm_bytes), ImageFormat::Pnm)
        .map_err(|e| format!("ppm encode error: {e}"))?;

    if debug {
        let debug_path = std::env::temp_dir().join(format!("snapocr-{}.png", std::process::id()));
        let _ = padded_img.save(&debug_path);
        eprintln!("snapocr: saved debug preprocessed image to {:?}", debug_path);
    }

    Ok(ppm_bytes)
}

/// Cascade OCR: PSM 6 -> PSM 13 -> PSM 3.
fn ocr_pipeline(ppm_bytes: &[u8], lang: &str) -> Result<String, String> {
    let mut text = run_tesseract(ppm_bytes, lang, "6").unwrap_or_default();
    if text.is_empty() {
        text = run_tesseract(ppm_bytes, lang, "13").unwrap_or_default();
    }
    if text.is_empty() {
        text = run_tesseract(ppm_bytes, lang, "3").unwrap_or_default();
    }
    if text.is_empty() {
        return Err("No text detected in region".to_string());
    }
    Ok(text)
}

fn run_tesseract(ppm_bytes: &[u8], lang: &str, psm: &str) -> Result<String, String> {
    let mut child = Command::new("tesseract")
        .arg("stdin")
        .arg("stdout")
        .arg("-l")
        .arg(lang)
        .arg("--psm")
        .arg(psm)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn tesseract: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(ppm_bytes);
    }

    let out = child
        .wait_with_output()
        .map_err(|e| format!("tesseract failed while waiting: {e}"))?;

    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr);
        return Err(format!("tesseract error: {}", msg.trim()));
    }

    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() {
        return Err("empty result".to_string());
    }
    Ok(text)
}

fn copy_to_clipboard(text: &str) -> Result<(), String> {
    if let Ok(mut cb) = Clipboard::new() {
        if cb.set_text(text.to_string()).is_ok() {
            return Ok(());
        }
    }
    if let Ok(mut child) = Command::new("wl-copy").stdin(Stdio::piped()).spawn() {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
        return Ok(());
    }
    Err("Failed to access clipboard".to_string())
}

fn detect_available_langs() -> Option<String> {
    let out = Command::new("tesseract").arg("--list-langs").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut langs = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with("List of")
            || line.contains('/')
            || line.contains('\\')
            || line == "osd"
        {
            continue;
        }
        langs.push(line.to_string());
    }
    if langs.is_empty() {
        None
    } else {
        langs.sort_by(|a, b| {
            if a == "eng" {
                std::cmp::Ordering::Less
            } else if b == "eng" {
                std::cmp::Ordering::Greater
            } else {
                a.cmp(b)
            }
        });
        Some(langs.join("+"))
    }
}

fn notify_err(msg: &str, no_notify: bool) {
    if !no_notify {
        let _ = Command::new("notify-send")
            .arg("-a")
            .arg("snapocr")
            .arg("-u")
            .arg("critical")
            .arg("OCR Failed")
            .arg(msg)
            .status();
    }
}
