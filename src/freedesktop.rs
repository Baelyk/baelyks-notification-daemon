use std::path::PathBuf;

use freedesktop_desktop_entry::{default_paths, get_languages_from_env, Iter};
use image::{RgbImage, RgbaImage};
use log::{debug, trace, warn};
use rand::distributions::Alphanumeric;
use rand::Rng;

use crate::dbus::ImageData;

pub fn find_app_name(desktop_entry_name: &str) -> Option<String> {
    let locales = get_languages_from_env();
    let mut entries = Iter::new(default_paths()).entries(Some(&locales));

    let desktop_entry_name = desktop_entry_name.to_lowercase();
    if let Some(desktop_entry) =
        entries.find(|desktop_entry| desktop_entry.appid.to_lowercase() == desktop_entry_name)
    {
        if let Some(name) = desktop_entry.name(&locales) {
            return Some(name.into_owned());
        } else {
            debug!("No name found for {}", desktop_entry_name);
        }
    } else {
        debug!("No desktop entry found for {}", desktop_entry_name);
    }

    None
}

fn tmp_path() -> Option<PathBuf> {
    let mut tries = 0;
    while tries < 3 {
        tries += 1;

        let filename: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(8)
            .map(char::from)
            .collect();
        let path = PathBuf::from(format!("/tmp/{}.png", filename));

        if path.try_exists().is_ok_and(|exists| !exists) {
            return Some(path);
        }
    }

    warn!("Unable to generate a temporary path");
    None
}

pub fn tmp_image_from_data(image_data: &ImageData) -> Option<PathBuf> {
    // Generate a path in the /tmp directory
    let path = tmp_path()?;

    // Create and save the image
    let save_result = if image_data.has_alpha {
        let Some(image) = RgbaImage::from_raw(
            image_data.width as u32,
            image_data.height as u32,
            image_data.data.clone(),
        ) else {
            warn!("Failed to create RGBA image");
            return None;
        };
        image.save(&path)
    } else {
        let Some(image) = RgbImage::from_raw(
            image_data.width as u32,
            image_data.height as u32,
            image_data.data.clone(),
        ) else {
            warn!("Failed to create RGB image");
            return None;
        };
        image.save(&path)
    };

    if let Err(err) = save_result {
        warn!(
            "Failed to save image to {} with error {}",
            path.display(),
            err
        );
        return None;
    };

    Some(path)
}

/// Gets a path for an icon by first checking if the passed icon is a path that
/// exists, and if not, searches for a matching freedesktop icon.
pub fn find_icon_path(icon_name_or_path: &str) -> Option<PathBuf> {
    /// Freedesktop Icon Theme name
    const THEME: &str = "Gruvbox-Plus-Dark";

    trace!("Checking path {icon_name_or_path}");
    // Paths are supposed to be prepended with "file://" but in practice many are not
    let path: PathBuf = icon_name_or_path.replace("file://", "").into();
    if path.exists() {
        return Some(path);
    }

    trace!("Looking for icon {icon_name_or_path}");
    freedesktop_icons::lookup(icon_name_or_path)
        .with_cache()
        .force_svg()
        .with_theme(THEME)
        .find()
        .or(freedesktop_icons::lookup(icon_name_or_path)
            .with_cache()
            .with_size(100)
            .with_theme(THEME)
            .find())
}
