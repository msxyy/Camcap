//! Media Foundation 进程级初始化。
//!
//! MF 是进程级单例。设备枚举（`camera.rs`）和 reader 创建（`capture.rs`）
//! 都需要 MF 已 Startup。把 Startup 提到进程级，只在首次调用时真正执行，
//! 退出时由 `main.rs` 调 `shutdown()` 配对。
//!
//! 为什么需要这个：
//! - 热插拔轮询每 2 秒调一次 `list_cameras`，如果每次都 Startup/Shutdown
//!   单次 200ms+，UI 会每 2 秒卡一次。
//! - 跨 subtype 切换时重复 Startup/Shutdown 每次也浪费 200ms。
//! 提到进程级后两者都消失。

use anyhow::{anyhow, Result};
use std::sync::OnceLock;
use std::time::Instant;
use windows::Win32::Media::MediaFoundation::{
    MFShutdown, MFStartup, MFSTARTUP_FULL, MF_VERSION,
};

static MF_INIT: OnceLock<()> = OnceLock::new();

/// 确保 MF 已初始化。首次调用执行 `MFStartup`，后续调用是 no-op。
pub fn ensure_startup() -> Result<()> {
    if MF_INIT.get().is_none() {
        unsafe {
            MFStartup(MF_VERSION, MFSTARTUP_FULL)
                .map_err(|e| anyhow!("MFStartup 失败: {}", e))?;
        }
        let _ = MF_INIT.set(());
        log::info!("MFStartup 完成（进程级首次）");
    }
    Ok(())
}

/// 进程退出时调一次，配对 `MFStartup`。未初始化时是 no-op。
pub fn shutdown() {
    if MF_INIT.get().is_some() {
        unsafe {
            let t = Instant::now();
            let _ = MFShutdown();
            log::info!("[TIMING] 进程退出 MFShutdown: {:?}", t.elapsed());
        }
    }
}