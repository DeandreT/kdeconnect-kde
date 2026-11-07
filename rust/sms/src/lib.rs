// SPDX-FileCopyrightText: 2026 Deandre Thomas
// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Context, Result, ensure};
use image::{DynamicImage, ImageFormat, RgbaImage};
use std::{
    io::{Cursor, Write},
    path::Path,
    sync::Arc,
};
use tempfile::{Builder, NamedTempFile};

pub const MAX_MESSAGE_BYTES: usize = 600_000;
const MAX_IMAGE_PIXELS: usize = 16_000_000;

#[derive(Debug)]
pub struct ImageAttachment {
    file: NamedTempFile,
    pub width: u32,
    pub height: u32,
    pub bytes: usize,
}

impl ImageAttachment {
    pub fn from_rgba(width: usize, height: usize, rgba: &[u8]) -> Result<Self> {
        let pixels = width
            .checked_mul(height)
            .context("image dimensions overflow")?;
        ensure!(
            pixels > 0 && pixels <= MAX_IMAGE_PIXELS,
            "image dimensions exceed limit"
        );
        ensure!(
            pixels.checked_mul(4) == Some(rgba.len()),
            "invalid RGBA image length"
        );
        let width = u32::try_from(width)?;
        let height = u32::try_from(height)?;
        let image = RgbaImage::from_raw(width, height, rgba.to_vec()).context("invalid image")?;
        let mut encoded = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image).write_to(&mut encoded, ImageFormat::Png)?;
        let encoded = encoded.into_inner();
        ensure!(
            encoded.len() <= MAX_MESSAGE_BYTES,
            "image exceeds 600 KB message limit"
        );
        let mut file = Builder::new()
            .prefix("kdeconnect-rs-")
            .suffix(".png")
            .tempfile()?;
        file.write_all(&encoded)?;
        file.flush()?;
        Ok(Self {
            file,
            width,
            height,
            bytes: encoded.len(),
        })
    }

    pub fn path(&self) -> &Path {
        self.file.path()
    }

    pub fn rgba(&self) -> Result<Vec<u8>> {
        Ok(image::load_from_memory(&std::fs::read(self.path())?)?
            .to_rgba8()
            .into_raw())
    }
}

#[derive(Debug, Default, Clone)]
pub struct Draft {
    pub recipient: String,
    pub text: String,
    pub attachments: Vec<Arc<ImageAttachment>>,
}

impl Draft {
    pub fn size(&self) -> usize {
        self.attachments
            .iter()
            .fold(self.text.len(), |size, a| size.saturating_add(a.bytes))
    }

    pub fn validate(&self) -> Result<()> {
        let recipient = self.recipient.trim();
        ensure!(
            !recipient.is_empty() && recipient.len() <= 254,
            "enter a recipient"
        );
        ensure!(
            !recipient.chars().any(char::is_control),
            "invalid recipient"
        );
        ensure!(
            !self.text.is_empty() || !self.attachments.is_empty(),
            "message is empty"
        );
        ensure!(
            self.size() <= MAX_MESSAGE_BYTES,
            "message exceeds 600 KB limit"
        );
        Ok(())
    }

    pub fn attach(&mut self, image: ImageAttachment) -> Result<()> {
        ensure!(self.attachments.len() < 8, "at most 8 images per message");
        ensure!(
            self.size().saturating_add(image.bytes) <= MAX_MESSAGE_BYTES,
            "message exceeds 600 KB limit"
        );
        self.attachments.push(Arc::new(image));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_and_shared_file_lifetime() -> Result<()> {
        let image = Arc::new(ImageAttachment::from_rgba(1, 1, &[255, 0, 0, 255])?);
        let path = image.path().to_path_buf();
        let pending_job = Arc::clone(&image);
        assert_eq!(
            std::fs::read(&path)?[..8],
            [137, 80, 78, 71, 13, 10, 26, 10]
        );
        assert_eq!(image.rgba()?, vec![255, 0, 0, 255]);
        drop(image);
        assert!(path.exists());
        drop(pending_job);
        assert!(!path.exists());
        Ok(())
    }

    #[test]
    fn rejects_invalid_pixels_without_creating_attachment() {
        for (w, h, data) in [
            (0, 1, vec![]),
            (1, 1, vec![0; 3]),
            (usize::MAX, 2, vec![]),
            (8000, 8000, vec![]),
        ] {
            assert!(ImageAttachment::from_rgba(w, h, &data).is_err());
        }
    }

    #[test]
    fn rejects_oversize_encoded_image() {
        let mut state = 1_u64;
        let rgba: Vec<u8> = (0..512 * 512 * 4)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect();
        assert!(ImageAttachment::from_rgba(512, 512, &rgba).is_err());
    }

    #[test]
    fn bounds_attachment_count() -> Result<()> {
        let mut draft = Draft::default();
        for _ in 0..8 {
            draft.attach(ImageAttachment::from_rgba(1, 1, &[0; 4])?)?;
        }
        assert!(
            draft
                .attach(ImageAttachment::from_rgba(1, 1, &[0; 4])?)
                .is_err()
        );
        assert_eq!(draft.attachments.len(), 8);
        Ok(())
    }

    #[test]
    fn validates_utf8_size_and_empty_messages() -> Result<()> {
        let mut draft = Draft {
            recipient: "+15550100001".into(),
            ..Default::default()
        };
        assert!(draft.validate().is_err());
        draft.text = "a".repeat(MAX_MESSAGE_BYTES);
        draft.validate()?;
        assert!(
            draft
                .attach(ImageAttachment::from_rgba(1, 1, &[0; 4])?)
                .is_err()
        );
        assert!(draft.attachments.is_empty());
        draft.text = "é".repeat(MAX_MESSAGE_BYTES / 2 + 1);
        assert!(draft.validate().is_err());
        Ok(())
    }

    #[test]
    fn image_only_and_failed_order_isolation() -> Result<()> {
        let mut good = Draft {
            recipient: "+15550100001".into(),
            ..Default::default()
        };
        good.attach(ImageAttachment::from_rgba(1, 1, &[0; 4])?)?;
        let bad = Draft::default();
        assert!(good.validate().is_ok());
        assert!(bad.validate().is_err());
        assert!(good.validate().is_ok());
        let retry = good.clone();
        drop(good);
        assert!(retry.attachments[0].path().exists());
        Ok(())
    }
}
