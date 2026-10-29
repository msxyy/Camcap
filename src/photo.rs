use crate::capture::Frame;
use anyhow::{anyhow, Result};
use std::path::Path;

pub fn save_frame(frame: &Frame, path: &Path) -> Result<()> {
    image::save_buffer(
        path,
        &frame.rgb,
        frame.width,
        frame.height,
        image::ColorType::Rgb8,
    )
    .map_err(|e| anyhow!("保存图片失败: {}", e))
}