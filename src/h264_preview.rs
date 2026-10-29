use anyhow::{anyhow, Result};
use crossbeam_channel::Receiver;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::core::{GUID, Interface};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

use crate::capture::Frame;

const CLSID_CMSH264DECODERMFT: GUID = GUID::from_u128(0x62CE7E72_4C71_4D20_B15D_452831A87D9D);
const MFT_OUTPUT_STREAM_PROVIDES_SAMPLES: u32 = 0x100;

pub fn run(
    rx: Receiver<Arc<Vec<u8>>>,
    latest: Arc<Mutex<Option<Arc<Frame>>>>,
    stop: Arc<AtomicBool>,
    frames: Arc<AtomicU64>,
    // 与 YUY2 共用同一份「当前可见尺寸」，热切换后控制线程写入新值
    current_dims: Arc<Mutex<(u32, u32)>>,
    initial_w: u32,
    initial_h: u32,
) -> Result<()> {
    let mut decoder: Option<H264Decoder> = None;
    let mut sps: Option<Vec<u8>> = None;
    let mut pps: Option<Vec<u8>> = None;

    while !stop.load(Ordering::Relaxed) {
        let raw = match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let mut has_idr = false;
        for nal in annex_b_nals(&raw) {
            if nal.is_empty() {
                continue;
            }
            match nal[0] & 0x1F {
                7 => sps = Some(nal.to_vec()),
                8 => pps = Some(nal.to_vec()),
                5 => has_idr = true,
                _ => {}
            }
        }

        if decoder.is_none() {
            if !has_idr {
                continue;
            }
            let (Some(sps_v), Some(pps_v)) = (sps.as_ref(), pps.as_ref()) else {
                continue;
            };
            // 初始可见尺寸：优先用 current_dims（热切换后已更新），
            // 拿不到时回退到 start_with_format 传入的初始值。
            let (cw, ch) = *current_dims.lock().unwrap();
            let (w, h) = if cw > 0 && ch > 0 {
                (cw, ch)
            } else {
                (initial_w, initial_h)
            };
            match H264Decoder::new(w, h, 30.0, sps_v, pps_v) {
                Ok(d) => {
                    log::info!("H264 硬件解码器已就绪: {}x{}", w, h);
                    decoder = Some(d);
                }
                Err(e) => {
                    log::error!("H264 解码器创建失败: {}", e);
                    return Err(e);
                }
            }
        }

        let d = decoder.as_mut().unwrap();

        // 每帧前同步可见尺寸。热切换分辨率后控制线程已写入新值，
        // 这里下一帧就能感知到。padding（buffer 比 visible 高）不改变它。
        let (vw, vh) = *current_dims.lock().unwrap();
        if vw > 0 && vh > 0 {
            d.set_visible_dims(vw, vh);
        }

        match d.decode(&raw) {
            Ok(Some(rgb)) => {
                let (fw, fh) = d.current_dimensions();
                let frame = Arc::new(Frame {
                    width: fw,
                    height: fh,
                    rgb,
                    at: Instant::now(),
                });
                frames.fetch_add(1, Ordering::Relaxed);
                *latest.lock().unwrap() = Some(frame);
            }
            Ok(None) => {}
            Err(e) => log::warn!("H264 解码失败: {}", e),
        }
    }
    Ok(())
}

struct H264Decoder {
    transform: IMFTransform,
    // 可见尺寸：输出 RGB 的目标尺寸，不含 padding
    visible_w: u32,
    visible_h: u32,
    // buffer 尺寸：解码器实际给 NV12 buffer 的尺寸，可能含 16 对齐 padding
    buffer_w: u32,
    buffer_h: u32,
    stride: u32,
    client_provides_buffer: bool,
    output_buffer_size: u32,
    output_buffer_alignment: u32,
    frame_index: u64,
    fps: f64,
}

impl H264Decoder {
    fn new(visible_w: u32, visible_h: u32, fps: f64, sps: &[u8], pps: &[u8]) -> Result<Self> {
        let t0 = Instant::now();
        unsafe {
            let transform: IMFTransform =
                CoCreateInstance(&CLSID_CMSH264DECODERMFT, None, CLSCTX_INPROC_SERVER)?;

            if let Ok(attrs) = transform.GetAttributes() {
                let _ = attrs.SetUINT32(&MF_LOW_LATENCY, 1);
            }

            let fps_int = fps.round().max(1.0) as u64;
            let input_type: IMFMediaType = MFCreateMediaType()?;
            input_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            input_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
            input_type.SetUINT64(
                &MF_MT_FRAME_SIZE,
                ((visible_w as u64) << 32) | visible_h as u64,
            )?;
            input_type.SetUINT64(&MF_MT_FRAME_RATE, (fps_int << 32) | 1)?;
            input_type.SetUINT32(
                &MF_MT_INTERLACE_MODE,
                MFVideoInterlace_MixedInterlaceOrProgressive.0 as u32,
            )?;

            let mut seq = Vec::with_capacity(8 + sps.len() + pps.len());
            seq.extend_from_slice(&[0, 0, 0, 1]);
            seq.extend_from_slice(sps);
            seq.extend_from_slice(&[0, 0, 0, 1]);
            seq.extend_from_slice(pps);
            input_type.SetBlob(&MF_MT_MPEG_SEQUENCE_HEADER, &seq)?;

            transform.SetInputType(0, &input_type, 0)?;

            let mut chosen: Option<IMFMediaType> = None;
            for i in 0..10u32 {
                match transform.GetOutputAvailableType(0, i) {
                    Ok(t) => {
                        if let Ok(sub) = t.GetGUID(&MF_MT_SUBTYPE) {
                            if sub == MFVideoFormat_NV12 {
                                chosen = Some(t);
                                break;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
            let output_type =
                chosen.ok_or_else(|| anyhow!("H264 解码器未提供 NV12 输出类型"))?;
            transform.SetOutputType(0, &output_type, 0)?;

            transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;

            let info: MFT_OUTPUT_STREAM_INFO = transform.GetOutputStreamInfo(0)?;
            let client_provides = (info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES) == 0;
            let min_size = (visible_w * visible_h * 3) / 2;
            let output_buffer_size = info.cbSize.max(min_size).max(1);

            let (buffer_w, buffer_h, stride) =
                read_output_layout(&transform, visible_w, visible_h);
            log::info!(
                "H264 输出布局: visible={}x{}, buffer={}x{}, stride={}",
                visible_w,
                visible_h,
                buffer_w,
                buffer_h,
                stride
            );

            log::info!(
                "[TIMING] H264Decoder::new（创建 MFT + 协商类型）: {:?}",
                t0.elapsed()
            );

            Ok(Self {
                transform,
                visible_w,
                visible_h,
                buffer_w,
                buffer_h,
                stride,
                client_provides_buffer: client_provides,
                output_buffer_size,
                output_buffer_alignment: info.cbAlignment,
                frame_index: 0,
                fps,
            })
        }
    }

    fn current_dimensions(&self) -> (u32, u32) {
        (self.visible_w, self.visible_h)
    }

    /// 更新可见尺寸。热切换分辨率时变化；重协商（padding）不改变它。
    fn set_visible_dims(&mut self, w: u32, h: u32) {
        if (w, h) != (self.visible_w, self.visible_h) {
            log::info!(
                "H264 可见尺寸切换: {}x{} -> {}x{}",
                self.visible_w,
                self.visible_h,
                w,
                h
            );
            self.visible_w = w;
            self.visible_h = h;
        }
    }

    fn decode(&mut self, h264_data: &[u8]) -> Result<Option<Vec<u8>>> {
        unsafe {
            let input_buffer: IMFMediaBuffer =
                MFCreateMemoryBuffer(h264_data.len() as u32)?;
            let mut ptr: *mut u8 = std::ptr::null_mut();
            input_buffer.Lock(&mut ptr, None, None)?;
            std::ptr::copy_nonoverlapping(h264_data.as_ptr(), ptr, h264_data.len());
            input_buffer.Unlock()?;
            input_buffer.SetCurrentLength(h264_data.len() as u32)?;

            let input_sample: IMFSample = MFCreateSample()?;
            input_sample.AddBuffer(&input_buffer)?;
            let duration = if self.fps > 0.0 {
                (10_000_000.0 / self.fps) as i64
            } else {
                333_333
            };
            input_sample.SetSampleTime(self.frame_index as i64 * duration)?;
            input_sample.SetSampleDuration(duration)?;
            self.frame_index += 1;

            if self.transform.ProcessInput(0, &input_sample, 0).is_err() {
                let _ = self.pump_output();
                self.transform.ProcessInput(0, &input_sample, 0)?;
            }

            self.pump_output()
        }
    }

    fn pump_output(&mut self) -> Result<Option<Vec<u8>>> {
        unsafe {
            let mut result: Option<Vec<u8>> = None;

            loop {
                let mut output_buffer = MFT_OUTPUT_DATA_BUFFER {
                    dwStreamID: 0,
                    pSample: std::mem::ManuallyDrop::new(None),
                    dwStatus: 0,
                    pEvents: std::mem::ManuallyDrop::new(None::<IMFCollection>),
                };

                if self.client_provides_buffer {
                    let buf: IMFMediaBuffer = if self.output_buffer_alignment > 0 {
                        MFCreateAlignedMemoryBuffer(
                            self.output_buffer_size,
                            self.output_buffer_alignment,
                        )?
                    } else {
                        MFCreateMemoryBuffer(self.output_buffer_size)?
                    };
                    let sample: IMFSample = MFCreateSample()?;
                    sample.AddBuffer(&buf)?;
                    output_buffer.pSample = std::mem::ManuallyDrop::new(Some(sample));
                }

                let mut status: u32 = 0;
                let mut buffers = [output_buffer];
                let hr = self.transform.ProcessOutput(0, &mut buffers, &mut status);

                let events = std::mem::ManuallyDrop::take(&mut buffers[0].pEvents);
                let sample_opt = std::mem::ManuallyDrop::take(&mut buffers[0].pSample);
                drop(events);

                match hr {
                    Ok(_) => {
                        if let Some(sample) = sample_opt {
                            if let Some(rgb) = self.extract_rgb(&sample)? {
                                result = Some(rgb);
                            }
                        }
                    }
                    Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => {
                        drop(sample_opt);
                        break;
                    }
                    Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                        drop(sample_opt);
                        if let Ok(t) = self.transform.GetOutputAvailableType(0, 0) {
                            let _ = self.transform.SetOutputType(0, &t, 0);
                        }
                        self.renegotiate_output_buffer_size();
                        let (bw, bh, st) =
                            read_output_layout(&self.transform, self.visible_w, self.visible_h);
                        self.buffer_w = bw;
                        self.buffer_h = bh;
                        self.stride = st;

                        // 关键：只更新 buffer 尺寸，不更新 visible 尺寸。
                        // visible 由 current_dims 驱动（set_visible_dims），
                        // buffer 可能比 visible 高 16 对齐的若干行（padding）。
                        // 之前把 visible 也设成 bh 会把 padding 行也当图像输出，
                        // 表现为画面底部一条绿边（padding 是未初始化的 UV 数据）。
                        log::info!(
                            "H264 重协商布局: visible={}x{}, buffer={}x{}, stride={}",
                            self.visible_w,
                            self.visible_h,
                            bw,
                            bh,
                            st
                        );
                        continue;
                    }
                    Err(e) => {
                        drop(sample_opt);
                        return Err(e.into());
                    }
                }
            }

            Ok(result)
        }
    }

    fn renegotiate_output_buffer_size(&mut self) {
        unsafe {
            if let Ok(info) = self.transform.GetOutputStreamInfo(0) {
                let min_size = (self.buffer_w * self.buffer_h * 3) / 2;
                self.output_buffer_size = info.cbSize.max(min_size).max(1);
                self.output_buffer_alignment = info.cbAlignment;
            }
        }
    }

    fn extract_rgb(&self, sample: &IMFSample) -> Result<Option<Vec<u8>>> {
        unsafe {
            let buffer: IMFMediaBuffer = sample.GetBufferByIndex(0)?;

            let (data, pitch) = if let Ok(buf2d) = buffer.cast::<IMF2DBuffer>() {
                let mut scanline0: *mut u8 = std::ptr::null_mut();
                let mut pitch_i: i32 = 0;
                buf2d.Lock2D(&mut scanline0, &mut pitch_i)?;

                let p = pitch_i.unsigned_abs();
                // 读取完整 buffer（含 padding），但后面只输出前 visible_h 行
                let total = p as usize * self.buffer_h as usize * 3 / 2;
                let mut v = Vec::new();
                if v.try_reserve_exact(total).is_err() {
                    buf2d.Unlock2D()?;
                    return Ok(None);
                }
                v.extend_from_slice(std::slice::from_raw_parts(scanline0, total));
                buf2d.Unlock2D()?;
                (v, p)
            } else {
                let mut ptr: *mut u8 = std::ptr::null_mut();
                let mut len: u32 = 0;
                buffer.Lock(&mut ptr, None, Some(&mut len))?;
                if ptr.is_null() || len == 0 {
                    let _ = buffer.Unlock();
                    return Ok(None);
                }
                let data = std::slice::from_raw_parts(ptr, len as usize).to_vec();
                buffer.Unlock()?;
                (data, self.stride)
            };

            let y_size = pitch as usize * self.buffer_h as usize;
            let uv_size = y_size / 2;
            if data.len() < y_size + uv_size {
                return Ok(None);
            }

            let y_plane = &data[..y_size];
            let uv_plane = &data[y_size..y_size + uv_size];

            // 输出尺寸 = visible（不含 padding），读取时按 buffer + stride 走
            let w = self.visible_w as usize;
            let h = self.visible_h as usize;
            let mut rgb = vec![0u8; w * h * 3];

            nv12_to_rgb_multi(y_plane, uv_plane, &mut rgb, w, h, pitch as usize);
            Ok(Some(rgb))
        }
    }
}

fn read_output_layout(
    transform: &IMFTransform,
    fallback_w: u32,
    fallback_h: u32,
) -> (u32, u32, u32) {
    unsafe {
        let Ok(out_type) = transform.GetOutputCurrentType(0) else {
            return (fallback_w, fallback_h, fallback_w);
        };
        let size = out_type.GetUINT64(&MF_MT_FRAME_SIZE).unwrap_or(0);
        let bw = if size == 0 {
            fallback_w
        } else {
            (size >> 32) as u32
        };
        let bh = if size == 0 {
            fallback_h
        } else {
            (size & 0xFFFF_FFFF) as u32
        };
        let stride = out_type
            .GetUINT32(&MF_MT_DEFAULT_STRIDE)
            .map(|s| s as u32)
            .unwrap_or(bw);
        (bw, bh, stride)
    }
}

fn nv12_to_rgb_multi(
    y_plane: &[u8],
    uv_plane: &[u8],
    dst: &mut [u8],
    w: usize,
    h: usize,
    stride: usize,
) {
    let num_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(4)
        .max(1);

    if num_threads <= 1 || h < num_threads * 4 {
        nv12_to_rgb_single(y_plane, uv_plane, dst, w, h, stride);
        return;
    }

    let rows_per_chunk = (h + num_threads - 1) / num_threads;
    let bytes_per_chunk = rows_per_chunk * w * 3;

    std::thread::scope(|scope| {
        for (chunk_idx, dst_chunk) in dst.chunks_mut(bytes_per_chunk).enumerate() {
            let row_start = chunk_idx * rows_per_chunk;
            let actual_rows = dst_chunk.len() / (w * 3);

            scope.spawn(move || {
                for local_row in 0..actual_rows {
                    let row = row_start + local_row;
                    if row >= h {
                        break;
                    }
                    let y_row = row * stride;
                    let uv_row = (row / 2) * stride;
                    let out_base = local_row * w * 3;

                    for col in 0..w {
                        let y_idx = y_row + col;
                        if y_idx >= y_plane.len() {
                            continue;
                        }
                        let y = y_plane[y_idx] as i32;

                        let uv_col = (col / 2) * 2;
                        let uv_idx = uv_row + uv_col;
                        if uv_idx + 1 >= uv_plane.len() {
                            continue;
                        }
                        let u = uv_plane[uv_idx] as i32 - 128;
                        let v = uv_plane[uv_idx + 1] as i32 - 128;

                        let c = (y - 16).max(0);
                        let r = clamp_u8((298 * c + 409 * v + 128) >> 8);
                        let g = clamp_u8((298 * c - 100 * u - 208 * v + 128) >> 8);
                        let b = clamp_u8((298 * c + 516 * u + 128) >> 8);
                        let out = out_base + col * 3;
                        dst_chunk[out] = r;
                        dst_chunk[out + 1] = g;
                        dst_chunk[out + 2] = b;
                    }
                }
            });
        }
    });
}

fn nv12_to_rgb_single(
    y_plane: &[u8],
    uv_plane: &[u8],
    dst: &mut [u8],
    w: usize,
    h: usize,
    stride: usize,
) {
    for row in 0..h {
        let y_row = row * stride;
        let uv_row = (row / 2) * stride;
        for col in 0..w {
            let y_idx = y_row + col;
            if y_idx >= y_plane.len() {
                continue;
            }
            let y = y_plane[y_idx] as i32;

            let uv_col = (col / 2) * 2;
            let uv_idx = uv_row + uv_col;
            if uv_idx + 1 >= uv_plane.len() {
                continue;
            }
            let u = uv_plane[uv_idx] as i32 - 128;
            let v = uv_plane[uv_idx + 1] as i32 - 128;

            let c = (y - 16).max(0);
            let r = clamp_u8((298 * c + 409 * v + 128) >> 8);
            let g = clamp_u8((298 * c - 100 * u - 208 * v + 128) >> 8);
            let b = clamp_u8((298 * c + 516 * u + 128) >> 8);
            let out = (row * w + col) * 3;
            dst[out] = r;
            dst[out + 1] = g;
            dst[out + 2] = b;
        }
    }
}

fn clamp_u8(v: i32) -> u8 {
    if v < 0 {
        0
    } else if v > 255 {
        255
    } else {
        v as u8
    }
}

fn annex_b_nals(data: &[u8]) -> Vec<&[u8]> {
    let mut nals = Vec::new();
    let mut i = 0;
    let mut nal_start: Option<usize> = None;
    while i < data.len() {
        let start_len = if i + 4 <= data.len()
            && data[i] == 0
            && data[i + 1] == 0
            && data[i + 2] == 0
            && data[i + 3] == 1
        {
            4
        } else if i + 3 <= data.len()
            && data[i] == 0
            && data[i + 1] == 0
            && data[i + 2] == 1
        {
            3
        } else {
            0
        };
        if start_len > 0 {
            if let Some(start) = nal_start {
                if start < i {
                    nals.push(&data[start..i]);
                }
            }
            nal_start = Some(i + start_len);
            i += start_len;
        } else {
            i += 1;
        }
    }
    if let Some(start) = nal_start {
        if start < data.len() {
            nals.push(&data[start..]);
        }
    }
    nals
}