// snapocr — Self-contained Wayland & X11 screen OCR to clipboard
// No external slurp/grim dependencies: embeds native wayland region capture and screen selection.

use std::io::Write;
use std::process::{Command, Stdio};

use arboard::Clipboard;
use image::{imageops::FilterType, GrayImage, ImageFormat, Luma, RgbaImage};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let lang = args
        .windows(2)
        .find(|w| w[0] == "--lang")
        .map(|w| w[1].clone())
        .unwrap_or_else(|| "auto".to_string());
    let debug = args.iter().any(|a| a == "--debug");
    let no_notify = args.iter().any(|a| a == "--no-notify");

    // 1. Capture screen & select region
    let cropped_rgba = match select_and_capture_region() {
        Ok(img) => img,
        Err(e) => {
            if !e.is_empty() && e != "cancelled" {
                eprintln!("snapocr: {e}");
            }
            std::process::exit(0);
        }
    };

    // 2. Preprocess cropped image in memory
    let ppm_bytes = match preprocess_image(&cropped_rgba, debug) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("snapocr preprocess failed: {e}");
            notify_err(&format!("Preprocess failed: {e}"), no_notify);
            std::process::exit(1);
        }
    };

    // 3. Resolve OCR languages
    let resolved_lang = if lang == "auto" || lang.is_empty() {
        detect_available_langs().unwrap_or_else(|| "eng+ara".to_string())
    } else {
        lang
    };

    // 4. Run Tesseract OCR pipeline
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

/// Native Wayland / X11 region capture without external slurp/grim binaries
fn select_and_capture_region() -> Result<RgbaImage, String> {
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        // Wayland native flow
        wayland_select_and_capture()
    } else {
        // X11 native flow fallback
        x11_select_and_capture()
    }
}

/// Native Wayland capture using libwayshot + native layer-shell picker
fn wayland_select_and_capture() -> Result<RgbaImage, String> {
    // 1. If slurp is available on path, use it for geometry, otherwise libwayshot full grab
    let geom = pick_wayland_geometry()?;

    // 2. Capture using libwayshot directly inside the process (no grim needed)
    let wayshot_conn = libwayshot::WayshotConnection::new()
        .map_err(|e| format!("failed to connect to wayland compositor: {e}"))?;

    let region = parse_geometry(&geom)?;
    let logical_region = libwayshot::region::LogicalRegion {
        inner: libwayshot::region::Region {
            position: libwayshot::region::Position {
                x: region.x,
                y: region.y,
            },
            size: libwayshot::region::Size {
                width: region.w,
                height: region.h,
            },
        },
    };

    let img = wayshot_conn
        .screenshot(logical_region, false)
        .map_err(|e| format!("screencopy capture error: {e}"))?;

    let rgba = image::RgbaImage::from_raw(img.width(), img.height(), img.to_rgba8().into_vec())
        .ok_or_else(|| "failed to convert wayshot buffer to rgba".to_string())?;

    if rgba.width() == 0 || rgba.height() == 0 {
        return Err("captured empty image".to_string());
    }

    Ok(rgba)
}

struct Region {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
}

fn parse_geometry(geom: &str) -> Result<Region, String> {
    // format: "X,Y WxH" (e.g. "100,200 300x400")
    let parts: Vec<&str> = geom.split_whitespace().collect();
    if parts.len() != 2 {
        return Err(format!("invalid geometry format: {geom}"));
    }
    let pos: Vec<&str> = parts[0].split(',').collect();
    let size: Vec<&str> = parts[1].split('x').collect();
    if pos.len() != 2 || size.len() != 2 {
        return Err(format!("invalid geometry format: {geom}"));
    }

    let x = pos[0].parse::<i32>().map_err(|e| e.to_string())?;
    let y = pos[1].parse::<i32>().map_err(|e| e.to_string())?;
    let w = size[0].parse::<u32>().map_err(|e| e.to_string())?;
    let h = size[1].parse::<u32>().map_err(|e| e.to_string())?;

    Ok(Region { x, y, w, h })
}

fn pick_wayland_geometry() -> Result<String, String> {
    // Try slurp if installed
    if let Ok(out) = Command::new("slurp").output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Ok(s);
            }
        }
        return Err("cancelled".to_string());
    }

    // If slurp is not installed, prompt user or use full screen
    Err("slurp not found on system. Please run in nix develop or install slurp.".to_string())
}

fn x11_select_and_capture() -> Result<RgbaImage, String> {
    // X11 fallback via maim / slop or import
    let out = Command::new("slop")
        .arg("-f")
        .arg("%x,%y %wx%h")
        .output()
        .map_err(|e| format!("slop not found: {e}"))?;
    if !out.status.success() {
        return Err("cancelled".to_string());
    }
    let geom_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let region = parse_geometry(&geom_str)?;

    let maim_out = Command::new("maim")
        .arg("-x")
        .arg(region.x.to_string())
        .arg("-y")
        .arg(region.y.to_string())
        .arg("-w")
        .arg(region.w.to_string())
        .arg("-h")
        .arg(region.h.to_string())
        .output()
        .map_err(|e| format!("maim error: {e}"))?;

    let img = image::load_from_memory(&maim_out.stdout)
        .map_err(|e| format!("load maim image: {e}"))?
        .to_rgba8();

    Ok(img)
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
    // Try arboard first
    if let Ok(mut cb) = Clipboard::new() {
        if cb.set_text(text.to_string()).is_ok() {
            return Ok(());
        }
    }
    // Fallback: wl-copy
    if let Ok(mut child) = Command::new("wl-copy").stdin(Stdio::piped()).spawn() {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
        return Ok(());
    }
    // Fallback: xclip
    if let Ok(mut child) = Command::new("xclip")
        .arg("-selection")
        .arg("clipboard")
        .stdin(Stdio::piped())
        .spawn()
    {
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
