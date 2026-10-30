use anyhow::{anyhow, Result};
use muxide::api::{MuxerBuilder, VideoCodec as MuxideCodec};
use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::capture::FormatSubtype;

pub struct RecorderHandle {
    joined: Option<JoinHandle<Result<()>>>,
    #[allow(dead_code)]
    pub path: PathBuf,
    pub dropped: Arc<AtomicU64>,
    pub written: Arc<AtomicU64>,
}

impl RecorderHandle {
    pub fn start(
        path: PathBuf,
        width: u32,
        height: u32,
        fps: u32,
        queue: usize,
        subtype: FormatSubtype,
    ) -> Result<(Self, SyncSender<Arc<Vec<u8>>>)> {
        let (tx, rx) = sync_channel::<Arc<Vec<u8>>>(queue);
        let path_c = path.clone();
        let dropped = Arc::new(AtomicU64::new(0));
        let dropped_c = dropped.clone();
        let written = Arc::new(AtomicU64::new(0));
        let written_c = written.clone();

        let joined = thread::Builder::new()
            .name("recorder".into())
            .spawn(move || -> Result<()> {
                match subtype {
                    FormatSubtype::Mjpeg | FormatSubtype::Yuy2 => {
                        let mut writer =
                            AviWriter::create(&path_c, width, height, fps as f64, subtype)?;
                        log::info!(
                            "{} 直通录制已就绪: {}x{} @ {}fps, 输出 {}",
                            subtype.name(),
                            width,
                            height,
                            fps,
                            path_c.display()
                        );

                        loop {
                            match rx.recv_timeout(Duration::from_millis(100)) {
                                Ok(frame) => {
                                    if writer.write_frame(&frame).is_err() {
                                        break;
                                    }
                                    written_c.fetch_add(1, Ordering::Relaxed);
                                }
                                Err(RecvTimeoutError::Timeout) => continue,
                                Err(RecvTimeoutError::Disconnected) => break,
                            }
                        }

                        // finish 返回 (帧数, 实测帧率)。实测帧率由首末帧的
                        // Instant 差算出来，已回填到 AVI 头。
                        let (n, actual_fps) = writer.finish()?;
                        log::info!(
                            "录制已保存: {} ({} 帧, {:.2}fps)",
                            path_c.display(),
                            n,
                            actual_fps
                        );
                    }
                    FormatSubtype::H264 => {
                        write_h264_mp4(rx, &path_c, width, height, fps as f64, written_c)?;
                    }
                }
                let _ = dropped_c;
                Ok(())
            })?;

        Ok((
            Self {
                joined: Some(joined),
                path,
                dropped,
                written,
            },
            tx,
        ))
    }

    pub fn stop(&mut self) {
        if let Some(j) = self.joined.take() {
            match j.join() {
                Ok(Ok(())) => {}
                Ok(Err(e)) => log::error!("录制器错误: {}", e),
                Err(_) => log::error!("录制线程 panic"),
            }
        }
    }
}

impl Drop for RecorderHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---------------------------------------------------------------------------
// AVI（MJPEG / YUY2）
// ---------------------------------------------------------------------------

struct AviWriter {
    inner: BufWriter<File>,
    /// 名义帧率，作为兜底：首末帧间隔过短或帧数不足时使用
    fps: u32,
    frame_count: u32,
    max_frame_size: u32,
    movi_data_size: u32,
    offsets: Vec<(u32, u32)>,
    subtype: FormatSubtype,
    /// 录制线程收到第一帧 / 最后一帧的时刻。
    /// finish() 时用 (last - first) 算实际帧率，回填 AVI 头。
    /// 驱动可能不严格按名义帧率吐帧（如 MJPEG 名义 15fps 实际 30fps，
    /// YUY2 名义 10fps 实际 20fps），用名义值会写错播放时长。
    first_ts: Option<Instant>,
    last_ts: Option<Instant>,
}

impl AviWriter {
    fn create(
        path: &PathBuf,
        width: u32,
        height: u32,
        fps: f64,
        subtype: FormatSubtype,
    ) -> Result<Self> {
        let file = File::create(path)?;
        let mut w = BufWriter::with_capacity(1 << 20, file);
        let fps_int = fps.round().max(1.0) as u32;
        let usec_per_frame = if fps_int > 0 {
            1_000_000 / fps_int
        } else {
            33333
        };

        let (fourcc, bit_count, bi_height, size_image): (&[u8; 4], u16, i32, u32) = match subtype
        {
            FormatSubtype::Mjpeg => (b"MJPG", 24, height as i32, width * height * 3),
            FormatSubtype::Yuy2 => (b"YUY2", 16, -(height as i32), width * height * 2),
            FormatSubtype::H264 => return Err(anyhow!("H264 不走 AVI 路径")),
        };

        let hdrl_size: u32 = 192;
        let strl_size: u32 = 116;

        w.write_all(b"RIFF")?;
        w.write_all(&0u32.to_le_bytes())?;
        w.write_all(b"AVI ")?;

        w.write_all(b"LIST")?;
        w.write_all(&hdrl_size.to_le_bytes())?;
        w.write_all(b"hdrl")?;

        w.write_all(b"avih")?;
        w.write_all(&56u32.to_le_bytes())?;
        w.write_all(&usec_per_frame.to_le_bytes())?;
        w.write_all(&0u32.to_le_bytes())?;
        w.write_all(&0u32.to_le_bytes())?;
        w.write_all(&0x0000_0010u32.to_le_bytes())?;
        w.write_all(&0u32.to_le_bytes())?;
        w.write_all(&0u32.to_le_bytes())?;
        w.write_all(&1u32.to_le_bytes())?;
        w.write_all(&0u32.to_le_bytes())?;
        w.write_all(&width.to_le_bytes())?;
        w.write_all(&height.to_le_bytes())?;
        w.write_all(&[0u8; 16])?;

        w.write_all(b"LIST")?;
        w.write_all(&strl_size.to_le_bytes())?;
        w.write_all(b"strl")?;

        w.write_all(b"strh")?;
        w.write_all(&56u32.to_le_bytes())?;
        w.write_all(b"vids")?;
        w.write_all(fourcc)?;
        w.write_all(&0u32.to_le_bytes())?;
        w.write_all(&0u32.to_le_bytes())?;
        w.write_all(&0u32.to_le_bytes())?;
        w.write_all(&1u32.to_le_bytes())?;       // dwScale 占位，finish 回填
        w.write_all(&fps_int.to_le_bytes())?;    // dwRate 占位，finish 回填
        w.write_all(&0u32.to_le_bytes())?;
        w.write_all(&0u32.to_le_bytes())?;
        w.write_all(&0u32.to_le_bytes())?;
        w.write_all(&0xFFFF_FFFFu32.to_le_bytes())?;
        w.write_all(&0u32.to_le_bytes())?;
        w.write_all(&0i16.to_le_bytes())?;
        w.write_all(&0i16.to_le_bytes())?;
        w.write_all(&(width as i16).to_le_bytes())?;
        w.write_all(&(height as i16).to_le_bytes())?;

        w.write_all(b"strf")?;
        w.write_all(&40u32.to_le_bytes())?;
        w.write_all(&40u32.to_le_bytes())?;
        w.write_all(&(width as i32).to_le_bytes())?;
        w.write_all(&bi_height.to_le_bytes())?;
        w.write_all(&1u16.to_le_bytes())?;
        w.write_all(&bit_count.to_le_bytes())?;
        w.write_all(fourcc)?;
        w.write_all(&size_image.to_le_bytes())?;
        w.write_all(&0i32.to_le_bytes())?;
        w.write_all(&0i32.to_le_bytes())?;
        w.write_all(&0u32.to_le_bytes())?;
        w.write_all(&0u32.to_le_bytes())?;

        w.write_all(b"LIST")?;
        w.write_all(&0u32.to_le_bytes())?;
        w.write_all(b"movi")?;

        Ok(Self {
            inner: w,
            fps: fps_int,
            frame_count: 0,
            max_frame_size: 0,
            movi_data_size: 0,
            offsets: Vec::new(),
            subtype,
            first_ts: None,
            last_ts: None,
        })
    }

    fn write_frame(&mut self, data: &[u8]) -> Result<()> {
        let now = Instant::now();
        if self.first_ts.is_none() {
            self.first_ts = Some(now);
        }
        self.last_ts = Some(now);

        let size = data.len() as u32;
        let offset = 4 + self.movi_data_size;
        self.offsets.push((offset, size));

        self.inner.write_all(b"00dc")?;
        self.inner.write_all(&size.to_le_bytes())?;
        self.inner.write_all(data)?;

        let mut chunk_total: u32 = 8 + size;
        if size % 2 == 1 {
            self.inner.write_all(&[0u8])?;
            chunk_total += 1;
        }
        self.movi_data_size += chunk_total;

        self.frame_count += 1;
        if size > self.max_frame_size {
            self.max_frame_size = size;
        }
        Ok(())
    }

    /// 返回 (帧数, 实测帧率)。
    /// 实测帧率 = (frame_count - 1) / (last_ts - first_ts)，回填到 AVI 头，
    /// 避免驱动超发/欠发导致播放时长错。
    fn finish(mut self) -> Result<(u32, f32)> {
        self.inner.flush()?;
        let file = self.inner.get_mut();

        let frame_count = self.frame_count;
        let max_frame_size = self.max_frame_size;
        let movi_size = 4 + self.movi_data_size;
        let idx1_size: u32 = 16 * frame_count;
        let riff_size: u32 = 4 + 8 + 192 + 8 + movi_size + 8 + idx1_size;

        // 实际帧率：帧数 ≥ 2 且耗时 ≥ 100ms 时用实测值，否则回退名义帧率
        let (dw_scale, dw_rate, usec_per_frame, measured_fps) =
            match (self.first_ts, self.last_ts) {
                (Some(first), Some(last)) if frame_count >= 2 => {
                    let elapsed_us = last.duration_since(first).as_micros() as u64;
                    if elapsed_us >= 100_000 {
                        let intervals = (frame_count - 1) as u64;
                        // dwScale=1000，dwRate = 实际 fps × 1000，保留 3 位小数
                        let dw_rate = (intervals * 1_000_000_000 / elapsed_us) as u32;
                        let dw_scale = 1000u32;
                        let usec = (elapsed_us / intervals) as u32;
                        let fps = dw_rate as f32 / dw_scale as f32;
                        (dw_scale, dw_rate.max(1), usec.max(1), fps)
                    } else {
                        let usec = 1_000_000 / self.fps.max(1);
                        (1u32, self.fps, usec, self.fps as f32)
                    }
                }
                _ => {
                    let usec = 1_000_000 / self.fps.max(1);
                    (1u32, self.fps, usec, self.fps as f32)
                }
            };
        // dwMaxBytesPerSec 用实测帧率算，u64 中转避免溢出
        let max_bps = ((max_frame_size as u64) * (dw_rate as u64) / (dw_scale.max(1) as u64))
            as u32;

        file.seek(SeekFrom::Start(4))?;
        file.write_all(&riff_size.to_le_bytes())?;
        // avih.dwMicroSecPerFrame (offset 32) + avih.dwMaxBytesPerSec (offset 36)
        file.seek(SeekFrom::Start(32))?;
        file.write_all(&usec_per_frame.to_le_bytes())?;
        file.write_all(&max_bps.to_le_bytes())?;
        file.seek(SeekFrom::Start(48))?;
        file.write_all(&frame_count.to_le_bytes())?;
        file.seek(SeekFrom::Start(60))?;
        file.write_all(&max_frame_size.to_le_bytes())?;
        // strh.dwScale (offset 128) + strh.dwRate (offset 132)
        file.seek(SeekFrom::Start(128))?;
        file.write_all(&dw_scale.to_le_bytes())?;
        file.write_all(&dw_rate.to_le_bytes())?;
        file.seek(SeekFrom::Start(140))?;
        file.write_all(&frame_count.to_le_bytes())?;
        file.seek(SeekFrom::Start(144))?;
        file.write_all(&max_frame_size.to_le_bytes())?;
        file.seek(SeekFrom::Start(216))?;
        file.write_all(&movi_size.to_le_bytes())?;
        file.seek(SeekFrom::End(0))?;

        file.write_all(b"idx1")?;
        file.write_all(&idx1_size.to_le_bytes())?;
        for (offset, size) in &self.offsets {
            file.write_all(&0x6364_3030u32.to_le_bytes())?;
            file.write_all(&0x10u32.to_le_bytes())?;
            file.write_all(&offset.to_le_bytes())?;
            file.write_all(&size.to_le_bytes())?;
        }
        file.flush()?;
        let _ = self.subtype;
        log::info!(
            "AVI 完成: {} 帧, 实测 {:.2}fps (dwScale={}, dwRate={})",
            frame_count,
            measured_fps,
            dw_scale,
            dw_rate
        );
        Ok((frame_count, measured_fps))
    }
}

// ---------------------------------------------------------------------------
// MP4（H264）
// ---------------------------------------------------------------------------

fn write_h264_mp4(
    rx: std::sync::mpsc::Receiver<Arc<Vec<u8>>>,
    path: &PathBuf,
    width: u32,
    height: u32,
    fps: f64,
    written: Arc<AtomicU64>,
) -> Result<()> {
    let file = File::create(path)?;
    let mut muxer = MuxerBuilder::new(file)
        .video(MuxideCodec::H264, width, height, fps)
        .build()?;

    let mut sps: Option<Vec<u8>> = None;
    let mut pps: Option<Vec<u8>> = None;
    let mut first_idr_done = false;
    let mut first_ts: Option<f64> = None;
    let mut ts_origin: Option<std::time::Instant> = None;
    let mut frame_count: u32 = 0;

    log::info!("H264 MP4 录制已就绪: {}x{} @ {:.0}fps, 输出 {}",
        width, height, fps, path.display());

    loop {
        let frame = match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(f) => f,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };

        // 扫描 NAL，缓存 SPS/PPS
        let nals = annex_b_nals(&frame);
        let mut has_sps = false;
        let mut has_pps = false;
        let mut has_idr = false;
        for nal in &nals {
            if nal.is_empty() { continue; }
            match nal[0] & 0x1F {
                7 => { sps = Some(nal.to_vec()); has_sps = true; }
                8 => { pps = Some(nal.to_vec()); has_pps = true; }
                5 => { has_idr = true; }
                _ => {}
            }
        }

        if !first_idr_done {
            if !has_idr { continue; }
            let (Some(sps_v), Some(pps_v)) = (sps.as_ref(), pps.as_ref()) else {
                continue;
            };

            // 若 IDR 自带 SPS/PPS，直接用；否则前置
            let keyframe = if has_sps && has_pps {
                frame.to_vec()
            } else {
                let mut v = Vec::with_capacity(8 + sps_v.len() + pps_v.len() + frame.len());
                if !has_sps {
                    v.extend_from_slice(&[0, 0, 0, 1]);
                    v.extend_from_slice(sps_v);
                }
                if !has_pps {
                    v.extend_from_slice(&[0, 0, 0, 1]);
                    v.extend_from_slice(pps_v);
                }
                v.extend_from_slice(&frame);
                v
            };

            if ts_origin.is_none() {
                ts_origin = Some(std::time::Instant::now());
                first_ts = Some(0.0);
            }
            muxer.write_video(0.0, &keyframe, true)?;
            first_idr_done = true;
            frame_count += 1;
            written.fetch_add(1, Ordering::Relaxed);
            log::info!("H264: 首个 IDR 已写入，开始正式录制");
            continue;
        }

        let origin = ts_origin.unwrap_or_else(std::time::Instant::now);
        let pts = origin.elapsed().as_secs_f64();
        let _ = first_ts;

        muxer.write_video(pts, &frame, false)?;
        frame_count += 1;
        written.fetch_add(1, Ordering::Relaxed);
    }

    muxer.finish()?;
    log::info!("录制已保存: {} ({} 帧)", path.display(), frame_count);
    Ok(())
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