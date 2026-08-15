// snapocr — ultra-lean screen OCR to clipboard
// Wayland: slurp + grim
// X11: slop + maim (or xdotool/import fallback)
use std::io::Write;
use std::process::{Command, Stdio};

use arboard::Clipboard;
use image::{imageops::FilterType, GrayImage, ImageFormat, Luma};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let lang = args
        .windows(2)
        .find(|w| w[0] == "--lang")
        .map(|w| w[1].clone())
        .unwrap_or_else(|| "auto".to_string());
    let debug = args.iter().any(|a| a == "--debug");
    let no_notify = args.iter().any(|a| a == "--no-notify");

    // 1. Pick region interactively
    let geom = match pick_region() {
        Ok(g) => g,
        Err(e) => {
            // User cancelled or slurp exited with non-zero (e.g. pressed Escape)
            if !e.is_empty() && e != "cancelled" {
                eprintln!("snapocr: {e}");
            }
            std::process::exit(0);
        }
    };

    // 2. Capture the selected region to PNG in memory
    let png_bytes = match capture_region(&geom) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("snapocr capture failed: {e}");
            notify_err(&format!("Capture failed: {e}"), no_notify);
            std::process::exit(1);
        }
    };

    // 3. Preprocess for OCR in memory
    let ppm_bytes = match preprocess_image(&png_bytes, debug) {
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

    // 6. Native desktop notification
    if !no_notify {
        let preview = if text.len() > 140 {
            format!("{}…", &text[..text.char_indices().map(|(i, _)| i).nth(140).unwrap_or(text.len())])
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

/// Run slurp (Wayland) or slop (X11) to let the user select a region.
fn pick_region() -> Result<String, String> {
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        let out = Command::new("slurp")
            .output()
            .map_err(|e| format!("slurp not found: {e} (install slurp)"))?;
        if !out.status.success() {
            return Err("cancelled".to_string());
        }
        let geom = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if geom.is_empty() {
            return Err("cancelled".to_string());
        }
        Ok(geom)
    } else {
        // X11 fallback: try slop
        let out = Command::new("slop")
            .arg("-f")
            .arg("%x,%y %wx%h")
            .output()
            .map_err(|e| format!("slop/slurp not found: {e}"))?;
        if !out.status.success() {
            return Err("cancelled".to_string());
        }
        let geom = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if geom.is_empty() {
            return Err("cancelled".to_string());
        }
        Ok(geom)
    }
}

/// Capture selected geometry directly into memory bytes.
fn capture_region(geom: &str) -> Result<Vec<u8>, String> {
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        let out = Command::new("grim")
            .arg("-g")
            .arg(geom)
            .arg("-") // output to stdout
            .output()
            .map_err(|e| format!("grim failed: {e}"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        Ok(out.stdout)
    } else {
        // X11 fallback: maim -g <geom>
        let out = Command::new("maim")
            .arg("-g")
            .arg(geom)
            .output()
            .map_err(|e| format!("maim failed: {e}"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        Ok(out.stdout)
    }
}

/// Grayscale + 2.5x upscale + dark mode auto-inversion + 24px white padding.
fn preprocess_image(png_bytes: &[u8], debug: bool) -> Result<Vec<u8>, String> {
    let img = image::load_from_memory_with_format(png_bytes, ImageFormat::Png)
        .map_err(|e| format!("load image: {e}"))?;

    let w = img.width();
    let h = img.height();
    let target_w = (w as f32 * 2.5).round() as u32;
    let target_h = (h as f32 * 2.5).round() as u32;

    let scaled = img.resize_exact(target_w, target_h, FilterType::CatmullRom);
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
