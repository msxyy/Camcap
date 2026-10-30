//! 独立探测程序：列出所有视频捕获设备的 FriendlyName 和 SymbolicLink。
//!
//! 用途：确认同名设备能否通过 SymbolicLink 区分。
//! 不影响主程序，跑完即止。
//!
//! 运行：
//!   cargo run --example probe_devices
//!
//! 验证方式：
//!   1. 只接设备 A，跑一次，记下 A 的 SymbolicLink
//!   2. 只接设备 B，跑一次，记下 B 的 SymbolicLink
//!   3. 两个都接，跑一次，看是否两个 link 都在、且和单接时一致
//!   4. 如果 A 和 B 的 SymbolicLink 不同 → 方案 B 可行

use anyhow::Result;
use windows::core::PWSTR;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

fn read_alloc_string(dev: &IMFActivate, key: &windows::core::GUID) -> Option<String> {
    unsafe {
        let mut s: PWSTR = PWSTR::null();
        let mut len: u32 = 0;
        if dev.GetAllocatedString(key, &mut s, &mut len).is_ok() && !s.0.is_null() {
            s.to_string().ok()
        } else {
            None
        }
    }
}

/// 从 symbolic link 里粗解析 VID/PID，方便肉眼对比。
/// 形如 `\\?\usb#vid_1234&pid_5678&mi_00#7&1a2b3c4d&0&0000#{...}`。
fn parse_vid_pid(link: &str) -> Option<(String, String)> {
    let lower = link.to_lowercase();
    let vid_pos = lower.find("vid_")?;
    let vid = lower.get(vid_pos + 4..vid_pos + 8)?.to_string();
    let pid_pos = lower.find("pid_")?;
    let pid = lower.get(pid_pos + 4..pid_pos + 8)?.to_string();
    Some((vid, pid))
}

fn main() -> Result<()> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(|e| anyhow::anyhow!("CoInitializeEx 失败: {}", e));

        MFStartup(MF_VERSION, MFSTARTUP_FULL)
            .map_err(|e| anyhow::anyhow!("MFStartup 失败: {}", e))?;

        let result = (|| -> Result<()> {
            let mut attrs_opt: Option<IMFAttributes> = None;
            MFCreateAttributes(&mut attrs_opt, 1)?;
            let attrs = attrs_opt
                .ok_or_else(|| anyhow::anyhow!("MFCreateAttributes 返回 None"))?;
            attrs.SetGUID(
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
            )?;

            let mut devices_ptr: *mut Option<IMFActivate> = std::ptr::null_mut();
            let mut count: u32 = 0;
            MFEnumDeviceSources(&attrs, &mut devices_ptr, &mut count)?;

            println!("=== 共 {} 个视频捕获设备 ===\n", count);

            if count > 0 && !devices_ptr.is_null() {
                let devices = std::slice::from_raw_parts(devices_ptr, count as usize);
                for (i, dev) in devices.iter().enumerate() {
                    let Some(dev) = dev else { continue };

                    let name = read_alloc_string(
                        dev,
                        &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME,
                    )
                    .unwrap_or_else(|| "<无 FRIENDLY_NAME>".to_string());

                    let link = read_alloc_string(
                        dev,
                        &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
                    )
                    .unwrap_or_else(|| "<无 SYMBOLIC_LINK>".to_string());

                    let vid_pid = parse_vid_pid(&link)
                        .map(|(v, p)| format!("VID={} PID={}", v, p))
                        .unwrap_or_else(|| "VID/PID 解析失败".to_string());

                    println!("[{}] FriendlyName : {}", i, name);
                    println!("    SymbolicLink : {}", link);
                    println!("    {}", vid_pid);
                    println!();
                }
            }

            // 汇总：检查是否有重名设备
            let mut names: Vec<String> = Vec::new();
            if count > 0 && !devices_ptr.is_null() {
                let devices = std::slice::from_raw_parts(devices_ptr, count as usize);
                for dev in devices.iter().flatten() {
                    if let Some(n) = read_alloc_string(
                        dev,
                        &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME,
                    ) {
                        names.push(n);
                    }
                }
            }
            let mut dup = false;
            for i in 0..names.len() {
                for j in (i + 1)..names.len() {
                    if names[i] == names[j] {
                        println!(
                            "⚠ 检测到重名设备: \"{}\"（index {} 和 {}）",
                            names[i], i, j
                        );
                        dup = true;
                    }
                }
            }
            if !dup && !names.is_empty() {
                println!("✓ 没有重名设备");
            }

            Ok(())
        })();

        let _ = MFShutdown();
        result
    }
}