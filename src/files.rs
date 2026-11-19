use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct DecodedPng {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| matches!(ext.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg"))
}

/// Images directly inside `folder`. Subfolders are not walked.
pub fn images_in_folder(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut images = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && is_image(&path) {
            images.push(path);
        }
    }
    images
}

/// Replace `path` with `bytes`, keeping the previous bytes at `path` + `.original`.
///
/// The original is only renamed away after the new bytes are on disk. If the
/// final rename fails, the original name is restored.
pub fn replace_with(path: &Path, bytes: &[u8]) -> Result<PathBuf, String> {
    if bytes.is_empty() {
        return Err("Compressed file was empty".into());
    }
    let tmp = with_suffix(path, ".bamboo-tmp");
    let backup = with_suffix(path, ".original");
    write_synced(&tmp, bytes)?;

    if backup.exists() {
        if backup.is_dir() {
            let _ = fs::remove_file(&tmp);
            return Err(format!("{} is a directory", backup.display()));
        }
        if let Err(err) = fs::remove_file(&backup) {
            let _ = fs::remove_file(&tmp);
            return Err(err.to_string());
        }
    }

    if let Err(err) = fs::rename(path, &backup) {
        let _ = fs::remove_file(&tmp);
        return Err(err.to_string());
    }

    if let Err(err) = fs::rename(&tmp, path) {
        let restored = fs::rename(&backup, path);
        let _ = fs::remove_file(&tmp);
        if restored.is_err() {
            return Err(format!(
                "failed to save compressed file ({err}); original kept at {}",
                backup.display()
            ));
        }
        return Err(format!("failed to save compressed file ({err})"));
    }

    Ok(backup)
}

pub fn reveal(path: &Path) -> Result<(), String> {
    reveal_command(path)
        .status()
        .map(|_| ())
        .map_err(|err| err.to_string())
}

pub fn app_icon_png() -> &'static [u8] {
    include_bytes!("../assets/images/bamboo-small.png")
}

pub fn decode_png(bytes: &[u8]) -> Result<DecodedPng, String> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().map_err(|err| err.to_string())?;
    let mut buf = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or("PNG is too large to decode")?
    ];
    let info = reader.next_frame(&mut buf).map_err(|err| err.to_string())?;
    if info.bit_depth != png::BitDepth::Eight {
        return Err("PNG bit depth is not 8".into());
    }
    let pixels = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => pixels.to_vec(),
        png::ColorType::Rgb => {
            let (chunks, _) = pixels.as_chunks::<3>();
            let mut rgba = Vec::with_capacity(chunks.len() * 4);
            for chunk in chunks {
                rgba.extend_from_slice(chunk);
                rgba.push(255);
            }
            rgba
        }
        other => return Err(format!("unsupported PNG color type {other:?}")),
    };
    if rgba.len() != info.width as usize * info.height as usize * 4 {
        return Err("PNG decoded to an unexpected size".into());
    }
    Ok(DecodedPng {
        width: info.width,
        height: info.height,
        rgba,
    })
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let result = (|| {
        let mut file = File::create(path).map_err(|err| err.to_string())?;
        file.write_all(bytes).map_err(|err| err.to_string())?;
        file.sync_all().map_err(|err| err.to_string())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

fn reveal_command(path: &Path) -> Command {
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("open");
        command.arg("-R").arg(path);
        command
    }
    #[cfg(target_os = "windows")]
    {
        let mut command = Command::new("explorer");
        command.arg(format!("/select,{}", path.display()));
        command
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut command = Command::new("xdg-open");
        command.arg(path.parent().unwrap_or(path));
        command
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn scratch() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "bamboo-files-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn image_extensions_are_case_insensitive() {
        assert!(is_image(Path::new("a.PNG")));
        assert!(is_image(Path::new("a.JpEg")));
        assert!(is_image(Path::new("a.jpg")));
        assert!(!is_image(Path::new("a.gif")));
        assert!(!is_image(Path::new("a")));
        assert!(!is_image(Path::new("png")));
    }

    #[test]
    fn folder_scan_is_one_level_and_skips_non_images() {
        let dir = scratch();
        fs::write(dir.join("a.PNG"), b"a").unwrap();
        fs::write(dir.join("b.jpg"), b"b").unwrap();
        fs::write(dir.join("c.jpeg"), b"c").unwrap();
        fs::write(dir.join("d.gif"), b"d").unwrap();
        fs::write(dir.join("e.txt"), b"e").unwrap();
        fs::create_dir(dir.join("nested")).unwrap();
        fs::write(dir.join("nested").join("f.png"), b"f").unwrap();
        fs::create_dir(dir.join("g.png")).unwrap();

        let mut names: Vec<_> = images_in_folder(&dir)
            .iter()
            .map(|path| path.file_name().unwrap().to_os_string())
            .collect();
        names.sort();
        assert_eq!(
            names,
            ["a.PNG", "b.jpg", "c.jpeg"].map(std::ffi::OsString::from)
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn replace_keeps_the_previous_bytes_beside_the_new_file() {
        let dir = scratch();
        let path = dir.join("photo.png");
        fs::write(&path, b"old").unwrap();
        let backup = replace_with(&path, b"new").unwrap();
        assert_eq!(backup.file_name().unwrap(), "photo.png.original");
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert_eq!(fs::read(&backup).unwrap(), b"old");
        assert!(!with_suffix(&path, ".bamboo-tmp").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn replace_overwrites_a_leftover_backup() {
        let dir = scratch();
        let path = dir.join("photo.png");
        fs::write(&path, b"current").unwrap();
        fs::write(dir.join("photo.png.original"), b"stale").unwrap();
        let backup = replace_with(&path, b"compressed").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"compressed");
        assert_eq!(fs::read(&backup).unwrap(), b"current");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn replace_of_a_missing_file_leaves_no_temp_file() {
        let dir = scratch();
        let path = dir.join("missing.png");
        let err = replace_with(&path, b"data").unwrap_err();
        assert!(!err.is_empty());
        assert!(!with_suffix(&path, ".bamboo-tmp").exists());
        assert!(!path.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn empty_replacement_is_rejected() {
        let dir = scratch();
        let path = dir.join("photo.png");
        fs::write(&path, b"old").unwrap();
        assert!(replace_with(&path, b"").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"old");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn app_icon_decodes() {
        let icon = decode_png(app_icon_png()).unwrap();
        assert_eq!((icon.width, icon.height), (200, 200));
        assert_eq!(icon.rgba.len(), 200 * 200 * 4);
    }
}
