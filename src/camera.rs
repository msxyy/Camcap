use anyhow::Result;

#[derive(Clone)]
pub struct CameraInfo {
    pub index: u32,
    pub name: String,
    /// USB 端口级唯一的符号链接。同名设备靠它区分。
    /// 形如 `\\?\usb#vid_a108&pid_2240&mi_00#7&1af4834d&0&0000#{...}`。
    pub symbolic_link: String,
}

#[cfg(windows)]
pub fn list_cameras() -> Result<Vec<CameraInfo>> {
    // 保证 MF 已初始化（进程级，首次真正 Startup，后续 no-op）。
    crate::mf::ensure_startup()?;

    let handle = std::thread::spawn(|| -> Result<Vec<CameraInfo>> {
        use windows::{
            core::PWSTR,
            Win32::{
                Media::MediaFoundation::*,
                System::Com::{CoInitializeEx, COINIT_MULTITHREADED},
            },
        };

        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(|e| anyhow::anyhow!("CoInitializeEx 失败: {}", e));

            let mut attrs: Option<IMFAttributes> = None;
            MFCreateAttributes(&mut attrs, 1)
                .ok()
                .ok_or_else(|| anyhow::anyhow!("MFCreateAttributes 失败"))?;
            let attrs =
                attrs.ok_or_else(|| anyhow::anyhow!("MFCreateAttributes 返回 None"))?;

            attrs
                .SetGUID(
                    &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                    &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
                )
                .ok()
                .ok_or_else(|| anyhow::anyhow!("SetGUID 失败"))?;

            let mut devices_ptr: *mut Option<IMFActivate> = std::ptr::null_mut();
            let mut count: u32 = 0;
            MFEnumDeviceSources(&attrs, &mut devices_ptr, &mut count)
                .ok()
                .ok_or_else(|| anyhow::anyhow!("MFEnumDeviceSources 失败"))?;

            let mut list = Vec::new();
            if !devices_ptr.is_null() && count > 0 {
                let devices = std::slice::from_raw_parts(devices_ptr, count as usize);
                for (i, dev) in devices.iter().enumerate() {
                    let Some(dev) = dev else { continue };

                    // FRIENDLY_NAME
                    let mut name_str = format!("Camera {}", i);
                    let mut name: PWSTR = PWSTR::null();
                    let mut name_len: u32 = 0;
                    if dev
                        .GetAllocatedString(
                            &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME,
                            &mut name,
                            &mut name_len,
                        )
                        .is_ok()
                        && !name.0.is_null()
                    {
                        name_str = name.to_string().unwrap_or(name_str);
                    }

                    // SYMBOLIC_LINK
                    let mut link_str = String::new();
                    let mut link: PWSTR = PWSTR::null();
                    let mut link_len: u32 = 0;
                    if dev
                        .GetAllocatedString(
                            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
                            &mut link,
                            &mut link_len,
                        )
                        .is_ok()
                        && !link.0.is_null()
                    {
                        link_str = link.to_string().unwrap_or_default();
                    }

                    list.push(CameraInfo {
                        index: i as u32,
                        name: name_str,
                        symbolic_link: link_str,
                    });
                }
            }
            Ok(list)
        }
    });

    handle
        .join()
        .map_err(|_| anyhow::anyhow!("设备枚举线程异常退出"))?
}

#[cfg(not(windows))]
pub fn list_cameras() -> Result<Vec<CameraInfo>> {
    Ok(Vec::new())
}