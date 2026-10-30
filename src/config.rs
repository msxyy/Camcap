use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct PreferredFormatSave {
    pub subtype: u8, // 0=Mjpeg, 1=Yuy2, 2=H264
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct SlotConfig {
    /// 摄像头索引（枚举顺序），三层回退的最后一层。兼容 v1 配置。
    #[serde(default)]
    pub camera_index: u32,
    /// 摄像头名（FRIENDLY_NAME），link 匹配失败时的第二回退。
    #[serde(default)]
    pub camera_name: String,
    /// 摄像头符号链接（USB 端口级唯一），首选匹配键。v1 无此字段，默认空。
    #[serde(default)]
    pub symbolic_link: String,
    #[serde(default)]
    pub preferred_format: Option<PreferredFormatSave>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AppConfig {
    pub version: u32,
    pub slot_a: SlotConfig,
    pub slot_b: SlotConfig,
    pub display_mode: u8,
    pub compare_layout: u8,
    pub window_size: Option<[f32; 2]>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: 2,
            slot_a: SlotConfig::default(),
            slot_b: SlotConfig::default(),
            display_mode: 0,
            compare_layout: 0,
            window_size: None,
        }
    }
}

pub fn config_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .map(|p| PathBuf::from(p).join("camcap").join("config.json"))
}

pub fn load() -> AppConfig {
    let Some(path) = config_path() else {
        return AppConfig::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(s) => match serde_json::from_str::<AppConfig>(&s) {
            Ok(mut c) => {
                // v1 → v2：v1 的 SlotConfig 没有 camera_name / symbolic_link，
                // serde(default) 会让它们为空字符串。这里的「迁移」只是把 version
                // 写成 2，让下次保存是 v2 格式。不需要重写整个文件。
                if c.version < 2 {
                    log::info!(
                        "检测到 v{} 配置，已迁移到 v2（v1 无 symbolic_link，回退按名字/index 匹配）",
                        c.version
                    );
                    c.version = 2;
                }
                log::info!("已加载配置: {}", path.display());
                c
            }
            Err(e) => {
                log::warn!("配置解析失败，使用默认值: {}", e);
                AppConfig::default()
            }
        },
        Err(_) => AppConfig::default(),
    }
}

pub fn save(cfg: &AppConfig) {
    let Some(path) = config_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(cfg) {
        Ok(s) => {
            if let Err(e) = std::fs::write(&path, s) {
                log::warn!("保存配置失败: {}", e);
            }
        }
        Err(e) => log::warn!("序列化配置失败: {}", e),
    }
}