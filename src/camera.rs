use anyhow::Result;

#[derive(Clone)]
pub struct CameraInfo {
    pub index: u32,
    pub name: String,
}

#[cfg(windows)]
pub fn list_cameras() -> Result<Vec<CameraInfo>> {
    // 在主线程之外的新线程中执行 COM 初始化与设备枚举。
    // 主线程可能已被 eframe 初始化为 STA，新线程可自由使用 MTA。
    let handle = std::thread::spawn(|| -> Result<Vec<CameraInfo>> {
        use windows::{
            core::PWSTR,
            Win32::{
                Media::MediaFoundation::*,
                System::Com::{CoInitializeEx, COINIT_MULTITHREADED},
            },
        };

        unsafe {
            // 新线程首次初始化 COM，正常返回 S_OK 或 S_FALSE。
            // 如果仍返回 RPC_E_CHANGED_MODE，说明该线程已被外部初始化，
            // 此时忽略错误继续执行即可（COM 已可用）。
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(|e| anyhow::anyhow!("CoInitializeEx 失败: {}", e));

            MFStartup(MF_VERSION, MFSTARTUP_FULL)
                .map_err(|e| anyhow::anyhow!("MFStartup 失败: {}", e))?;

            let result = (|| -> Result<Vec<CameraInfo>> {
                let mut attrs: Option<IMFAttributes> = None;
                MFCreateAttributes(&mut attrs, 1)
                    .ok()
                    .ok_or_else(|| anyhow::anyhow!("MFCreateAttributes 失败"))?;
                let attrs = attrs
                    .ok_or_else(|| anyhow::anyhow!("MFCreateAttributes 返回 None"))?;

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
                        if let Some(dev) = dev {
                            let mut name: PWSTR = PWSTR::null();
                            let mut len: u32 = 0;
                            if dev
                                .GetAllocatedString(
                                    &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME,
                                    &mut name,
                                    &mut len,
                                )
                                .is_ok()
                                && !name.0.is_null()
                            {
                                let s = name
                                    .to_string()
                                    .unwrap_or_else(|_| format!("Camera {}", i));
                                list.push(CameraInfo {
                                    index: i as u32,
                                    name: s,
                                });
                            }
                        }
                    }
                }
                Ok(list)
            })();

            let _ = MFShutdown();
            result
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