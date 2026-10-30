use anyhow::{anyhow, Result};
use crossbeam_channel::{bounded, Receiver, Sender};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use zune_core::{colorspace::ColorSpace, options::DecoderOptions};
use zune_jpeg::JpegDecoder;

use windows::core::{implement, GUID, Interface, Result as WinResult, HRESULT, IUnknown};
use windows::Win32::Foundation::BOOL;
use windows::Win32::Media::DirectShow::{IAMCameraControl, IAMVideoProcAmp};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

const DECODER_THREADS: usize = 2;
const PREVIEW_QUEUE_CAP: usize = 16;

const FIRST_FRAME_WAIT_MS: u64 = 100;
const SWITCH_SETTLE_TIMEOUT_MS: u64 = 2000;
const SWITCH_REPLY_TIMEOUT_MS: u64 = 3000;

const FRAME_GAP_WARN_TICKS: i64 = 600_000;
const TICKS_PER_MS: i64 = 10_000;

/// 每个摄像头的格式列表只在本次运行期内打印一次，避免切换 reader 时刷屏
fn should_log_formats(camera_index: u32) -> bool {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};
    static LOGGED: OnceLock<Mutex<HashSet<u32>>> = OnceLock::new();
    let m = LOGGED.get_or_init(|| Mutex::new(HashSet::new()));
    match m.lock() {
        Ok(mut s) => s.insert(camera_index),
        Err(_) => true,
    }
}

// ---------------------------------------------------------------------------
// P6.4：格式列表缓存
// ---------------------------------------------------------------------------

/// 包装 `IMFMediaType`，使其可存入进程级 static 缓存。
///
/// windows-rs 默认不给 COM 接口类型实现 `Send`/`Sync`，因为编译器无法确认
/// 线程安全（`IUnknown` 内部是 `NonNull<c_void>`）。但 `mf_control_loop`
/// 里 `CoInitializeEx(COINIT_MULTITHREADED)` 是 **MTA**，COM 对象在 MTA 下
/// 可跨线程 AddRef/Release/SetCurrentMediaType，这是 COM 规范保证的。
///
/// **如果未来把 `COINIT_MULTITHREADED` 改成 STA，必须去掉这个包装**，
/// 否则跨线程访问 COM 对象是未定义行为。
struct SendMediaType(IMFMediaType);
unsafe impl Send for SendMediaType {}
unsafe impl Sync for SendMediaType {}

static FORMAT_CACHE: OnceLock<Mutex<HashMap<u32, (Vec<SendMediaType>, Vec<VideoFormat>)>>> =
    OnceLock::new();

fn format_cache() -> &'static Mutex<HashMap<u32, (Vec<SendMediaType>, Vec<VideoFormat>)>> {
    FORMAT_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

// ---------------------------------------------------------------------------
// 格式类型
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FormatSubtype {
    Mjpeg,
    Yuy2,
    H264,
}

impl FormatSubtype {
    pub fn name(self) -> &'static str {
        match self {
            Self::Mjpeg => "MJPEG",
            Self::Yuy2 => "YUY2",
            Self::H264 => "H264",
        }
    }

    fn from_guid(guid: &GUID) -> Option<Self> {
        if *guid == MFVideoFormat_MJPG {
            Some(Self::Mjpeg)
        } else if *guid == MFVideoFormat_YUY2 {
            Some(Self::Yuy2)
        } else if *guid == MFVideoFormat_H264 {
            Some(Self::H264)
        } else {
            None
        }
    }

    /// 配置持久化用：subtype → u8
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Mjpeg => 0,
            Self::Yuy2 => 1,
            Self::H264 => 2,
        }
    }

    /// 配置加载用：u8 → subtype
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Mjpeg),
            1 => Some(Self::Yuy2),
            2 => Some(Self::H264),
            _ => None,
        }
    }
}

pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
    #[allow(dead_code)]
    pub at: Instant,
}

#[derive(Clone, Debug)]
pub struct VideoFormat {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub subtype: FormatSubtype,
}

impl VideoFormat {
    fn key(&self) -> (u32, u32, u32, FormatSubtype) {
        (self.width, self.height, self.fps, self.subtype)
    }
    pub fn label(&self) -> String {
        format!(
            "{} {}x{} @ {}fps",
            self.subtype.name(),
            self.width,
            self.height,
            self.fps
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropertyKind {
    ProcAmp,
    CameraControl,
}

#[derive(Clone, Debug)]
pub struct PropertyInfo {
    pub kind: PropertyKind,
    pub id: i32,
    pub name: &'static str,
    pub min: i32,
    pub max: i32,
    pub step: i32,
    pub default: i32,
    pub current: i32,
    pub supports_auto: bool,
    pub supports_manual: bool,
    pub is_auto: bool,
}

// ---------------------------------------------------------------------------
// 控制命令
// ---------------------------------------------------------------------------

enum ControlCommand {
    SetProperty {
        kind: PropertyKind,
        id: i32,
        value: i32,
        manual: bool,
    },
    SwitchFormat {
        index: usize,
        reply: SyncSender<Result<(), String>>,
    },
    Quit,
}

// ---------------------------------------------------------------------------
// 共享状态
// ---------------------------------------------------------------------------

struct SharedState {
    reader: Mutex<Option<IMFSourceReader>>,
    proc_amp: Mutex<Option<IAMVideoProcAmp>>,
    camera_control: Mutex<Option<IAMCameraControl>>,
    preview_tx: Sender<Arc<Vec<u8>>>,
    /// preview 队列的 clone，切换格式时用于 drain 掉切换前积压的旧帧。
    preview_rx_for_drain: Receiver<Arc<Vec<u8>>>,
    /// CaptureHandle 的 latest clone，切换时清空，让 UI 显示“等待首帧”
    latest: Arc<Mutex<Option<Arc<Frame>>>>,
    /// 当前生效的采集尺寸。YUY2 / H264 解码线程每帧读取。
    current_dims: Arc<Mutex<(u32, u32)>>,
    recorder_tx: Arc<Mutex<Option<SyncSender<Arc<Vec<u8>>>>>>,
    capturing: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    last_ts: Mutex<i64>,
    send_drops: AtomicU64,
    /// 采集错误标志。`OnReadSample` 收到 `hrstatus.is_err()` 时置位，
    /// `CaptureHandle::stop()` 时清零。UI 侧读它显示「设备已断开」。
    error_flag: Arc<AtomicBool>,

    switching: AtomicBool,
    in_callback: AtomicUsize,
    read_pending: AtomicBool,
}

unsafe impl Send for SharedState {}
unsafe impl Sync for SharedState {}

// ---------------------------------------------------------------------------
// 异步回调
// ---------------------------------------------------------------------------

#[implement(IMFSourceReaderCallback)]
struct CaptureCallback {
    state: Arc<SharedState>,
}

impl IMFSourceReaderCallback_Impl for CaptureCallback_Impl {
    fn OnReadSample(
        &self,
        hrstatus: HRESULT,
        _dwstreamindex: u32,
        _dwstreamflags: u32,
        lltimestamp: i64,
        psample: Option<&IMFSample>,
    ) -> WinResult<()> {
        self.state.read_pending.store(false, Ordering::Release);

        if hrstatus.is_err() {
            // 驱动报错，设备可能已断开。置位标志让 UI 显示提示。
            // 不续读，避免反复报错刷屏。
            if !self.state.error_flag.load(Ordering::Relaxed) {
                log::warn!("采集错误 0x{:08X}，设备可能已断开", hrstatus.0);
                self.state.error_flag.store(true, Ordering::Relaxed);
            }
            return Ok(());
        }

        if self.state.stop.load(Ordering::Relaxed) {
            return Ok(());
        }

        if self.state.switching.load(Ordering::Acquire) {
            return Ok(());
        }

        self.state.in_callback.fetch_add(1, Ordering::AcqRel);

        if let Some(sample) = psample {
            unsafe {
                let Ok(buffer) = sample.ConvertToContiguousBuffer() else {
                    self.state.in_callback.fetch_sub(1, Ordering::AcqRel);
                    return Ok(());
                };
                let mut ptr: *mut u8 = std::ptr::null_mut();
                let mut len: u32 = 0;
                if buffer.Lock(&mut ptr, None, Some(&mut len)).is_err() {
                    self.state.in_callback.fetch_sub(1, Ordering::AcqRel);
                    return Ok(());
                }
                if ptr.is_null() || len == 0 {
                    let _ = buffer.Unlock();
                    self.state.in_callback.fetch_sub(1, Ordering::AcqRel);
                    return Ok(());
                }
                let raw = Arc::new(std::slice::from_raw_parts(ptr, len as usize).to_vec());
                let _ = buffer.Unlock();

                if lltimestamp > 0 {
                    let mut last = self.state.last_ts.lock().unwrap();
                    if *last > 0 {
                        let delta_ticks = lltimestamp - *last;
                        if delta_ticks > FRAME_GAP_WARN_TICKS {
                            log::warn!(
                                "采集端帧间隔 {}ms（真丢帧），预览端累计丢 {}",
                                delta_ticks / TICKS_PER_MS,
                                self.state.send_drops.load(Ordering::Relaxed)
                            );
                        }
                    }
                    *last = lltimestamp;
                }

                if self.state.preview_tx.try_send(raw.clone()).is_err() {
                    let n = self.state.send_drops.fetch_add(1, Ordering::Relaxed);
                    if n < 5 {
                        log::warn!("preview_tx 满，预览端丢帧 #{}", n + 1);
                    }
                }

                if self.state.capturing.load(Ordering::Relaxed) {
                    if let Some(tx) = self.state.recorder_tx.lock().unwrap().clone() {
                        let _ = tx.try_send(raw);
                    }
                }
            }
        }

        self.state.in_callback.fetch_sub(1, Ordering::AcqRel);

        let reader_guard = self.state.reader.lock().unwrap();
        if self.state.switching.load(Ordering::Acquire) {
            return Ok(());
        }
        if let Some(reader) = reader_guard.as_ref() {
            if !self.state.read_pending.swap(true, Ordering::AcqRel) {
                unsafe {
                    let _ = reader.ReadSample(
                        MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32,
                        0,
                        None,
                        None,
                        None,
                        None,
                    );
                }
            }
        }
        Ok(())
    }

    fn OnFlush(&self, _dwstreamindex: u32) -> WinResult<()> {
        Ok(())
    }

    fn OnEvent(&self, _dwstreamindex: u32, _pevent: Option<&IMFMediaEvent>) -> WinResult<()> {
        Ok(())
    }
}

pub type PreferredFormat = Option<(FormatSubtype, u32, u32, u32)>;

// ---------------------------------------------------------------------------
// 异步切换状态
// ---------------------------------------------------------------------------

struct PendingSwitchState {
    target_idx: usize,
    reply_rx: std::sync::mpsc::Receiver<Result<(), String>>,
    started_at: Instant,
}

/// `poll_switch` 的返回类型。
/// - `Success(fmt)`：切换成功，`CaptureHandle` 的 `width/height/current_format_idx` 已更新
/// - `Failed(e)`：切换失败或超时，`group_idx/fps_idx` 需要 UI 侧回滚
#[derive(Clone, Debug)]
pub enum SwitchOutcome {
    Success(VideoFormat),
    Failed(String),
}

// ---------------------------------------------------------------------------
// CaptureHandle
// ---------------------------------------------------------------------------

pub struct CaptureHandle {
    latest: Arc<Mutex<Option<Arc<Frame>>>>,
    recorder_tx: Arc<Mutex<Option<SyncSender<Arc<Vec<u8>>>>>>,
    capturing: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    control_tx: Sender<ControlCommand>,
    _control_join: Option<JoinHandle<()>>,
    _decode_joins: Vec<JoinHandle<()>>,
    pub width: u32,
    pub height: u32,
    frames: Arc<AtomicU64>,
    #[allow(dead_code)]
    pub recorder_dropped: Arc<AtomicU64>,
    pub available_formats: Vec<VideoFormat>,
    pub initial_properties: Vec<PropertyInfo>,
    pub current_format_idx: usize,
    pub current_subtype: FormatSubtype,
    current_dims: Arc<Mutex<(u32, u32)>>,
    pending_switch: Mutex<Option<PendingSwitchState>>,
    /// 采集错误标志。与 `SharedState.error_flag` 是同一个 `Arc`。
    error_flag: Arc<AtomicBool>,
}

impl CaptureHandle {
    #[allow(dead_code)]
    pub fn start(camera_index: u32) -> Result<Self> {
        Self::start_with_format(camera_index, None)
    }

    pub fn start_with_format(
        camera_index: u32,
        preferred: PreferredFormat,
    ) -> Result<Self> {
        let t_start = Instant::now();

        let latest = Arc::new(Mutex::new(None::<Arc<Frame>>));
        let recorder_tx = Arc::new(Mutex::new(None::<SyncSender<Arc<Vec<u8>>>>));
        let capturing = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let frames = Arc::new(AtomicU64::new(0));
        let current_dims = Arc::new(Mutex::new((0u32, 0u32)));
        let error_flag = Arc::new(AtomicBool::new(false));

        let (preview_tx, preview_rx) = bounded::<Arc<Vec<u8>>>(PREVIEW_QUEUE_CAP);
        let preview_rx_for_drain = preview_rx.clone();
        let (control_tx, control_rx) = bounded::<ControlCommand>(16);
        let (init_tx, init_rx) = std::sync::mpsc::sync_channel::<
            Result<(Vec<VideoFormat>, usize, Vec<PropertyInfo>, u32, u32, FormatSubtype), String>,
        >(1);

        let shared = Arc::new(SharedState {
            reader: Mutex::new(None),
            proc_amp: Mutex::new(None),
            camera_control: Mutex::new(None),
            preview_tx,
            preview_rx_for_drain,
            latest: latest.clone(),
            current_dims: current_dims.clone(),
            recorder_tx: recorder_tx.clone(),
            capturing: capturing.clone(),
            stop: stop.clone(),
            last_ts: Mutex::new(0),
            send_drops: AtomicU64::new(0),
            error_flag: error_flag.clone(),
            switching: AtomicBool::new(false),
            in_callback: AtomicUsize::new(0),
            read_pending: AtomicBool::new(false),
        });

        let shared_c = shared.clone();
        let control_join = thread::Builder::new()
            .name("mf-control".into())
            .spawn(move || {
                if let Err(e) =
                    mf_control_loop(camera_index, preferred, shared_c, control_rx, init_tx)
                {
                    log::error!("MF 控制线程退出: {}", e);
                }
            })
            .map_err(|e| anyhow!("启动控制线程失败: {}", e))?;

        let (available_formats, current_format_idx, initial_properties, w, h, subtype) = init_rx
            .recv_timeout(Duration::from_secs(10))
            .map_err(|e| anyhow!("等待初始化超时: {}", e))?
            .map_err(|e| anyhow!("初始化失败: {}", e))?;

        *current_dims.lock().unwrap() = (w, h);

        log::info!(
            "[TIMING] CaptureHandle 初始化（reader 创建 + SetMediaType）: {:?}",
            t_start.elapsed()
        );

        let decode_threads = match subtype {
            FormatSubtype::H264 => 1,
            _ => DECODER_THREADS,
        };

        let mut decode_joins = Vec::new();
        for i in 0..decode_threads {
            let rx = preview_rx.clone();
            let latest_c = latest.clone();
            let stop_c = stop.clone();
            let frames_c = frames.clone();
            let dims_c = current_dims.clone();
            let j = thread::Builder::new()
                .name(format!("preview-decoder-{}", i))
                .spawn(move || {
                    decode_loop(rx, latest_c, stop_c, frames_c, subtype, dims_c)
                })
                .map_err(|e| anyhow!("启动解码线程 {} 失败: {}", i, e))?;
            decode_joins.push(j);
        }
        drop(preview_rx);

        let t_first_frame = Instant::now();
        let deadline = Instant::now() + Duration::from_millis(FIRST_FRAME_WAIT_MS);
        while Instant::now() < deadline {
            if latest.lock().unwrap().is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        if latest.lock().unwrap().is_some() {
            log::info!(
                "[TIMING] 首个可解码帧就绪: {:?}",
                t_first_frame.elapsed()
            );
        } else {
            log::info!(
                "start_with_format 返回时首帧尚未就绪（>{}ms），由 UI 线程异步等待",
                FIRST_FRAME_WAIT_MS
            );
        }
        log::info!(
            "[TIMING] start_with_format 总计: {:?}",
            t_start.elapsed()
        );

        Ok(Self {
            latest,
            recorder_tx,
            capturing,
            stop,
            control_tx,
            _control_join: Some(control_join),
            _decode_joins: decode_joins,
            width: w,
            height: h,
            frames,
            recorder_dropped: Arc::new(AtomicU64::new(0)),
            available_formats,
            initial_properties,
            current_format_idx,
            current_subtype: subtype,
            current_dims,
            pending_switch: Mutex::new(None),
            error_flag,
        })
    }

    pub fn peek_latest(&self) -> Option<Arc<Frame>> {
        self.latest.lock().unwrap().clone()
    }

    pub fn attach_recorder(&self, tx: SyncSender<Arc<Vec<u8>>>) {
        *self.recorder_tx.lock().unwrap() = Some(tx);
        self.capturing.store(true, Ordering::Relaxed);
    }

    pub fn detach_recorder(&self) {
        self.capturing.store(false, Ordering::Relaxed);
        *self.recorder_tx.lock().unwrap() = None;
    }

    pub fn frames(&self) -> u64 {
        self.frames.load(Ordering::Relaxed)
    }

    /// 是否有采集错误（设备断开）。UI 侧用于状态栏提示。
    pub fn has_error(&self) -> bool {
        self.error_flag.load(Ordering::Relaxed)
    }

    pub fn stop(&mut self) {
        let t0 = Instant::now();
        self.stop.store(true, Ordering::Relaxed);
        self.error_flag.store(false, Ordering::Relaxed);
        self.detach_recorder();
        let _ = self.control_tx.try_send(ControlCommand::Quit);
        log::info!("[TIMING] stop() 发 Quit: {:?}", t0.elapsed());
    }

    pub fn read_properties(&self) -> Vec<PropertyInfo> {
        self.initial_properties.clone()
    }

    pub fn set_property(
        &self,
        kind: PropertyKind,
        id: i32,
        value: i32,
        manual: bool,
    ) -> Result<()> {
        self.control_tx
            .try_send(ControlCommand::SetProperty {
                kind,
                id,
                value,
                manual,
            })
            .map_err(|e| anyhow!("发送控制命令失败: {}", e))
    }

    /// 发起异步格式切换。只发送命令，不阻塞 UI 线程。
    /// 完成后通过 `poll_switch` 获取结果。
    pub fn request_format(&mut self, index: usize) -> std::result::Result<(), String> {
        {
            let g = self.pending_switch.lock().unwrap();
            if g.is_some() {
                return Err("上一次切换尚未完成".into());
            }
        }
        let new_fmt = self
            .available_formats
            .get(index)
            .ok_or_else(|| "格式索引越界".to_string())?
            .clone();
        if new_fmt.subtype != self.current_subtype {
            return Err(format!(
                "切换 {} -> {} 需要重新打开摄像头",
                self.current_subtype.name(),
                new_fmt.subtype.name()
            ));
        }
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        self.control_tx
            .send(ControlCommand::SwitchFormat { index, reply: tx })
            .map_err(|e| format!("发送切换命令失败: {}", e))?;
        *self.pending_switch.lock().unwrap() = Some(PendingSwitchState {
            target_idx: index,
            reply_rx: rx,
            started_at: Instant::now(),
        });
        Ok(())
    }

    /// 轮询异步切换结果。返回 `Some` 表示已完成（成功或失败）。
    pub fn poll_switch(&mut self) -> Option<SwitchOutcome> {
        let (target_idx, started_at, recv_result) = {
            let g = self.pending_switch.lock().unwrap();
            let state = g.as_ref()?;
            (
                state.target_idx,
                state.started_at,
                state.reply_rx.try_recv(),
            )
        };
        match recv_result {
            Ok(Ok(())) => {
                *self.pending_switch.lock().unwrap() = None;
                let fmt = self.available_formats.get(target_idx).cloned()?;
                self.width = fmt.width;
                self.height = fmt.height;
                self.current_format_idx = target_idx;
                *self.current_dims.lock().unwrap() = (fmt.width, fmt.height);
                log::info!("[TIMING] 异步切换完成: {:?}", started_at.elapsed());
                Some(SwitchOutcome::Success(fmt))
            }
            Ok(Err(e)) => {
                *self.pending_switch.lock().unwrap() = None;
                Some(SwitchOutcome::Failed(e))
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                if started_at.elapsed() > Duration::from_millis(SWITCH_REPLY_TIMEOUT_MS) {
                    *self.pending_switch.lock().unwrap() = None;
                    Some(SwitchOutcome::Failed("切换格式命令超时".into()))
                } else {
                    None
                }
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                *self.pending_switch.lock().unwrap() = None;
                Some(SwitchOutcome::Failed("控制线程已退出".into()))
            }
        }
    }

    /// 是否有切换正在进行中。UI 侧用于禁用格式/帧率下拉。
    pub fn is_switching(&self) -> bool {
        self.pending_switch.lock().unwrap().is_some()
    }

    /// 同步热切换格式。保留给特殊场景（测试、脚本）。
    /// 正常 UI 流程请用 `request_format` + `poll_switch`。
    #[allow(dead_code)]
    pub fn set_format(&mut self, index: usize) -> Result<VideoFormat> {
        let new_fmt = self
            .available_formats
            .get(index)
            .ok_or_else(|| anyhow!("格式索引越界"))?
            .clone();

        if new_fmt.subtype != self.current_subtype {
            return Err(anyhow!(
                "切换 {} -> {} 需要重新打开摄像头",
                self.current_subtype.name(),
                new_fmt.subtype.name()
            ));
        }

        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        self.control_tx
            .send(ControlCommand::SwitchFormat { index, reply: tx })
            .map_err(|e| anyhow!("发送切换命令失败: {}", e))?;

        match rx.recv_timeout(Duration::from_millis(SWITCH_REPLY_TIMEOUT_MS)) {
            Ok(Ok(())) => {
                self.width = new_fmt.width;
                self.height = new_fmt.height;
                self.current_format_idx = index;
                *self.current_dims.lock().unwrap() = (new_fmt.width, new_fmt.height);
                log::info!(
                    "格式切换成功: {} {}x{} @ {}fps（新帧将异步到达）",
                    self.current_subtype.name(),
                    new_fmt.width,
                    new_fmt.height,
                    new_fmt.fps
                );
                Ok(new_fmt)
            }
            Ok(Err(e)) => Err(anyhow!("{}", e)),
            Err(_) => Err(anyhow!("切换格式命令超时")),
        }
    }
}

impl Drop for CaptureHandle {
    fn drop(&mut self) {
        let t_all = Instant::now();
        self.stop();

        let t_ctrl = Instant::now();
        if let Some(j) = self._control_join.take() {
            let _ = j.join();
        }
        log::info!(
            "[TIMING] Drop: control thread join {:?}",
            t_ctrl.elapsed()
        );

        let n = self._decode_joins.len();
        let t_dec = Instant::now();
        for j in self._decode_joins.drain(..) {
            let _ = j.join();
        }
        log::info!(
            "[TIMING] Drop: {} 个解码线程 join {:?}",
            n,
            t_dec.elapsed()
        );
        log::info!("[TIMING] Drop 总计: {:?}", t_all.elapsed());
    }
}

// ---------------------------------------------------------------------------
// MF 控制线程
// ---------------------------------------------------------------------------

fn mf_control_loop(
    camera_index: u32,
    preferred: PreferredFormat,
    shared: Arc<SharedState>,
    control_rx: Receiver<ControlCommand>,
    init_tx: std::sync::mpsc::SyncSender<
        Result<(Vec<VideoFormat>, usize, Vec<PropertyInfo>, u32, u32, FormatSubtype), String>,
    >,
) -> Result<()> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(|e| anyhow!("CoInitializeEx 失败: {}", e))?;
        // 进程级只 MFStartup 一次，后续跨 subtype 切换跳过。
        // 配对 MFShutdown 在 main.rs 进程退出时调 mf::shutdown()。
        crate::mf::ensure_startup()?;

        let result = (|| -> Result<()> {
            let source = open_device_source(camera_index)?;

            let proc_amp: Option<IAMVideoProcAmp> = source.cast().ok();
            let camera_control: Option<IAMCameraControl> = source.cast().ok();
            *shared.proc_amp.lock().unwrap() = proc_amp;
            *shared.camera_control.lock().unwrap() = camera_control;

            let callback_impl = CaptureCallback {
                state: shared.clone(),
            };
            let unknown: IUnknown = callback_impl.into();
            let callback: IMFSourceReaderCallback = unknown.cast()?;

            let mut attrs_opt: Option<IMFAttributes> = None;
            MFCreateAttributes(&mut attrs_opt, 1)?;
            let attrs = attrs_opt.ok_or_else(|| anyhow!("MFCreateAttributes 返回 None"))?;
            attrs.SetUnknown(&MF_SOURCE_READER_ASYNC_CALLBACK, &callback)?;

            let reader: IMFSourceReader =
                MFCreateSourceReaderFromMediaSource(&source, &attrs)?;

            // P6.4：格式列表缓存。同一摄像头运行期内不变，切换 reader 时跳过枚举。
            let (media_types, video_formats) = {
                let mut g = format_cache().lock().unwrap();
                if let Some(cached) = g.get(&camera_index) {
                    log::debug!(
                        "设备暴露 {} 种格式（从缓存读取，跳过枚举）",
                        cached.1.len()
                    );
                    let mts: Vec<IMFMediaType> =
                        cached.0.iter().map(|s| s.0.clone()).collect();
                    (mts, cached.1.clone())
                } else {
                    let (mts, vfs) = collect_formats(&reader)?;
                    let send_mts: Vec<SendMediaType> =
                        mts.iter().map(|m| SendMediaType(m.clone())).collect();
                    g.insert(camera_index, (send_mts, vfs.clone()));
                    (mts, vfs)
                }
            };
            if video_formats.is_empty() {
                let _ = init_tx.send(Err("设备未暴露 MJPG/YUY2/H264 中任一种格式".into()));
                return Err(anyhow!("设备未暴露任何支持的格式"));
            }

            if should_log_formats(camera_index) {
                log::info!("设备暴露 {} 种格式:", video_formats.len());
                for (i, f) in video_formats.iter().enumerate() {
                    log::info!("  [{}] {}", i, f.label());
                }
            } else {
                log::debug!(
                    "设备暴露 {} 种格式（已打印过，略）",
                    video_formats.len()
                );
            }

            let selected_idx = match preferred {
                Some((sub, w, h, fps)) => {
                    match video_formats.iter().position(|f| {
                        f.subtype == sub
                            && f.width == w
                            && f.height == h
                            && f.fps == fps
                    }) {
                        Some(i) => i,
                        None => {
                            // preferred 指定的格式该设备不支持。
                            // 场景：配置里存的是同型号另一台设备的格式，换设备后不匹配。
                            // 回退到第一个 MJPEG 格式，保证用户至少能看到画面，
                            // 而不是「打开失败」一片空白。
                            let fallback = video_formats
                                .iter()
                                .position(|f| f.subtype == FormatSubtype::Mjpeg)
                                .unwrap_or(0);
                            let fallback_label = video_formats
                                .get(fallback)
                                .map(|f| f.label())
                                .unwrap_or_default();
                            log::warn!(
                                "设备不支持目标格式 {} {}x{} @ {}fps，回退到默认: {}",
                                sub.name(),
                                w,
                                h,
                                fps,
                                fallback_label
                            );
                            fallback
                        }
                    }
                }
                None => video_formats
                    .iter()
                    .position(|f| {
                        f.subtype == FormatSubtype::Mjpeg
                            && f.width == 2592
                            && f.height == 1944
                            && f.fps >= 29
                    })
                    .or_else(|| {
                        video_formats
                            .iter()
                            .position(|f| f.subtype == FormatSubtype::Mjpeg)
                    })
                    .unwrap_or(0),
            };

            let fmt = &video_formats[selected_idx];
            let (w, h, subtype) = (fmt.width, fmt.height, fmt.subtype);

            log::info!(
                "选中格式: {} (索引 {}/{}): {}x{} @ {}fps",
                subtype.name(),
                selected_idx,
                video_formats.len() - 1,
                w,
                h,
                fmt.fps
            );

            reader.SetCurrentMediaType(
                MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32,
                None,
                &media_types[selected_idx],
            )?;

            if let Ok(actual_mt) =
                reader.GetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32)
            {
                let size = actual_mt.GetUINT64(&MF_MT_FRAME_SIZE).unwrap_or(0);
                let aw = (size >> 32) as u32;
                let ah = (size & 0xFFFF_FFFF) as u32;
                let fr = actual_mt.GetUINT64(&MF_MT_FRAME_RATE).unwrap_or(0);
                let num = (fr >> 32) as u32;
                let den = (fr & 0xFFFF_FFFF) as u32;
                let afps = if den > 0 { num as f64 / den as f64 } else { 0.0 };
                log::info!(
                    "驱动实际接受: {}x{} @ {:.2}fps (原始 {} / {})",
                    aw, ah, afps, num, den
                );
                if (afps - fmt.fps as f64).abs() > 1.0 {
                    log::warn!(
                        "帧率协商不一致: 请求 {}fps, 实际 {:.2}fps",
                        fmt.fps, afps
                    );
                }
            }

            let initial_props = probe_all_properties(
                &shared.proc_amp.lock().unwrap(),
                &shared.camera_control.lock().unwrap(),
            );

            *shared.reader.lock().unwrap() = Some(reader);

            let switch_dims: Vec<(u32, u32)> = video_formats
                .iter()
                .map(|f| (f.width, f.height))
                .collect();

            let _ = init_tx.send(Ok((
                video_formats,
                selected_idx,
                initial_props,
                w,
                h,
                subtype,
            )));

            {
                let reader_guard = shared.reader.lock().unwrap();
                if let Some(r) = reader_guard.as_ref() {
                    shared.read_pending.store(true, Ordering::Release);
                    let _ = r.ReadSample(
                        MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32,
                        0,
                        None,
                        None,
                        None,
                        None,
                    );
                }
            }

            loop {
                if shared.stop.load(Ordering::Relaxed) {
                    break;
                }
                match control_rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(ControlCommand::SetProperty {
                        kind,
                        id,
                        value,
                        manual,
                    }) => {
                        let flag = if manual { 0x2 } else { 0x1 };
                        match kind {
                            PropertyKind::ProcAmp => {
                                if let Some(pa) = shared.proc_amp.lock().unwrap().as_ref() {
                                    let _ = pa.Set(id, value, flag);
                                }
                            }
                            PropertyKind::CameraControl => {
                                if let Some(cc) =
                                    shared.camera_control.lock().unwrap().as_ref()
                                {
                                    let _ = cc.Set(id, value, flag);
                                }
                            }
                        }
                    }
                    Ok(ControlCommand::SwitchFormat { index, reply }) => {
                        let stream_idx = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
                        let result: Result<(), String> = if let Some(mt) =
                            media_types.get(index)
                        {
                            shared.switching.store(true, Ordering::Release);

                            let deadline =
                                Instant::now() + Duration::from_millis(SWITCH_SETTLE_TIMEOUT_MS);
                            while shared.in_callback.load(Ordering::Acquire) > 0
                                && Instant::now() < deadline
                            {
                                thread::sleep(Duration::from_millis(2));
                            }

                            let deadline =
                                Instant::now() + Duration::from_millis(SWITCH_SETTLE_TIMEOUT_MS);
                            while shared.read_pending.load(Ordering::Acquire)
                                && Instant::now() < deadline
                            {
                                thread::sleep(Duration::from_millis(2));
                            }

                            let rv = {
                                let reader_guard = shared.reader.lock().unwrap();
                                match reader_guard.as_ref() {
                                    Some(r) => {
                                        let _ = r.SetStreamSelection(stream_idx, BOOL(0));
                                        let _ = r.Flush(stream_idx);
                                        let set_rv = r.SetCurrentMediaType(stream_idx, None, mt);
                                        let _ = r.SetStreamSelection(stream_idx, BOOL(1));

                                        if set_rv.is_ok() {
                                            match r.GetCurrentMediaType(stream_idx) {
                                                Ok(actual) => {
                                                    let actual_size = actual
                                                        .GetUINT64(&MF_MT_FRAME_SIZE)
                                                        .unwrap_or(0);
                                                    let aw = (actual_size >> 32) as u32;
                                                    let ah = (actual_size & 0xFFFF_FFFF) as u32;
                                                    log::info!(
                                                        "SetCurrentMediaType 成功，GetCurrentMediaType: {}x{}",
                                                        aw, ah
                                                    );
                                                }
                                                Err(e) => {
                                                    log::warn!(
                                                        "GetCurrentMediaType 失败（忽略）: {}",
                                                        e
                                                    );
                                                }
                                            }
                                            Ok(())
                                        } else {
                                            match set_rv {
                                                Err(e) => Err(e.to_string()),
                                                Ok(_) => unreachable!(),
                                            }
                                        }
                                    }
                                    None => Err("reader 不存在".to_string()),
                                }
                            };

                            shared.switching.store(false, Ordering::Release);

                            if rv.is_ok() {
                                if let Some(&(dw, dh)) = switch_dims.get(index) {
                                    let old = {
                                        let mut g = shared.current_dims.lock().unwrap();
                                        let old = *g;
                                        *g = (dw, dh);
                                        old
                                    };
                                    if subtype == FormatSubtype::Yuy2
                                        && old != (0, 0)
                                        && old != (dw, dh)
                                    {
                                        log::info!(
                                            "YUY2 解码尺寸切换: {}x{} -> {}x{}",
                                            old.0,
                                            old.1,
                                            dw,
                                            dh
                                        );
                                    }
                                }

                                let mut drained = 0u32;
                                while shared.preview_rx_for_drain.try_recv().is_ok() {
                                    drained += 1;
                                }
                                if drained > 0 {
                                    log::info!("切换后清空 preview 队列（{} 帧）", drained);
                                }
                                *shared.latest.lock().unwrap() = None;

                                let reader_guard = shared.reader.lock().unwrap();
                                if let Some(r) = reader_guard.as_ref() {
                                    if !shared.read_pending.swap(true, Ordering::AcqRel) {
                                        let _ = r.ReadSample(
                                            stream_idx,
                                            0,
                                            None,
                                            None,
                                            None,
                                            None,
                                        );
                                    }
                                }
                            }
                            rv
                        } else {
                            Err(format!("格式索引 {} 越界", index))
                        };
                        let _ = reply.send(result);
                    }
                    Ok(ControlCommand::Quit) => break,
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                }
            }

            shared.stop.store(true, Ordering::Relaxed);
            *shared.reader.lock().unwrap() = None;
            *shared.proc_amp.lock().unwrap() = None;
            *shared.camera_control.lock().unwrap() = None;
            Ok(())
        })();

        // 不在此处 MFShutdown。进程级配对由 main.rs 退出时调 mf::shutdown() 完成。
        // 跨 subtype 切换每次省 200ms。
        result
    }
}

unsafe fn open_device_source(camera_index: u32) -> Result<IMFMediaSource> {
    let mut attrs: Option<IMFAttributes> = None;
    MFCreateAttributes(&mut attrs, 1)?;
    let attrs = attrs.ok_or_else(|| anyhow!("MFCreateAttributes 返回 None"))?;
    attrs.SetGUID(
        &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
        &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
    )?;

    let mut devices_ptr: *mut Option<IMFActivate> = std::ptr::null_mut();
    let mut count: u32 = 0;
    MFEnumDeviceSources(&attrs, &mut devices_ptr, &mut count)?;

    if count == 0 || devices_ptr.is_null() {
        return Err(anyhow!("未找到任何视频捕获设备"));
    }

    let devices = std::slice::from_raw_parts(devices_ptr, count as usize);
    let activate = devices
        .get(camera_index as usize)
        .and_then(|d| d.as_ref())
        .ok_or_else(|| anyhow!("摄像头索引 {} 超出范围", camera_index))?;

    let source: IMFMediaSource = activate.ActivateObject()?;
    Ok(source)
}

unsafe fn collect_formats(
    reader: &IMFSourceReader,
) -> Result<(Vec<IMFMediaType>, Vec<VideoFormat>)> {
    let mut media_types = Vec::new();
    let mut video_formats = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut idx = 0u32;

    loop {
        let mt = match reader.GetNativeMediaType(
            MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32,
            idx,
        ) {
            Ok(mt) => mt,
            Err(_) => break,
        };
        idx += 1;

        let major = mt.GetGUID(&MF_MT_MAJOR_TYPE).unwrap_or_default();
        if major != MFMediaType_Video {
            continue;
        }
        let sub = mt.GetGUID(&MF_MT_SUBTYPE).unwrap_or_default();
        let Some(subtype) = FormatSubtype::from_guid(&sub) else {
            continue;
        };

        let size = mt.GetUINT64(&MF_MT_FRAME_SIZE).unwrap_or(0);
        let w = (size >> 32) as u32;
        let h = (size & 0xFFFF_FFFF) as u32;
        if w == 0 || h == 0 {
            continue;
        }
        let fr = mt.GetUINT64(&MF_MT_FRAME_RATE).unwrap_or(0);
        let num = (fr >> 32) as u32;
        let den = (fr & 0xFFFF_FFFF) as u32;
        let fps = if den > 0 { num / den } else { 0 };

        let fmt = VideoFormat {
            width: w,
            height: h,
            fps,
            subtype,
        };
        if !seen.insert(fmt.key()) {
            continue;
        }
        media_types.push(mt);
        video_formats.push(fmt);
    }

    Ok((media_types, video_formats))
}

// ---------------------------------------------------------------------------
// 属性自动探测
// ---------------------------------------------------------------------------

fn probe_all_properties(
    proc_amp: &Option<IAMVideoProcAmp>,
    camera_control: &Option<IAMCameraControl>,
) -> Vec<PropertyInfo> {
    let mut list = Vec::new();

    const PROC_AMP_NAMES: &[(&str, i32)] = &[
        ("亮度", 0),
        ("对比度", 1),
        ("色调", 2),
        ("饱和度", 3),
        ("锐度", 4),
        ("伽马", 5),
        ("彩色开关", 6),
        ("白平衡", 7),
        ("背光补偿", 8),
        ("增益", 9),
    ];

    const CAMERA_CONTROL_NAMES: &[(&str, i32)] = &[
        ("水平摇摄", 0),
        ("垂直摇摄", 1),
        ("旋转", 2),
        ("变焦", 3),
        ("曝光", 4),
        ("光圈", 5),
    ];

    if let Some(pa) = proc_amp {
        for (name, id) in PROC_AMP_NAMES {
            if let Some(info) = unsafe { probe_proc_amp(pa, *id, name) } {
                list.push(info);
            }
        }
    }
    if let Some(cc) = camera_control {
        for (name, id) in CAMERA_CONTROL_NAMES {
            if let Some(info) = unsafe { probe_camera_control(cc, *id, name) } {
                list.push(info);
            }
        }
    }
    list
}

unsafe fn probe_proc_amp(
    pa: &IAMVideoProcAmp,
    id: i32,
    name: &'static str,
) -> Option<PropertyInfo> {
    let mut min = 0i32;
    let mut max = 0i32;
    let mut step = 0i32;
    let mut default = 0i32;
    let mut caps = 0i32;
    if pa
        .GetRange(id, &mut min, &mut max, &mut step, &mut default, &mut caps)
        .is_err()
    {
        return None;
    }
    let mut current = default;
    let mut flags = 0i32;
    if pa.Get(id, &mut current, &mut flags).is_err() {
        current = default;
    }
    Some(PropertyInfo {
        kind: PropertyKind::ProcAmp,
        id,
        name,
        min,
        max,
        step: if step > 0 { step } else { 1 },
        default,
        current,
        supports_auto: caps & 0x1 != 0,
        supports_manual: caps & 0x2 != 0,
        is_auto: flags & 0x1 != 0,
    })
}

unsafe fn probe_camera_control(
    cc: &IAMCameraControl,
    id: i32,
    name: &'static str,
) -> Option<PropertyInfo> {
    let mut min = 0i32;
    let mut max = 0i32;
    let mut step = 0i32;
    let mut default = 0i32;
    let mut caps = 0i32;
    if cc
        .GetRange(id, &mut min, &mut max, &mut step, &mut default, &mut caps)
        .is_err()
    {
        return None;
    }
    let mut current = default;
    let mut flags = 0i32;
    if cc.Get(id, &mut current, &mut flags).is_err() {
        current = default;
    }
    Some(PropertyInfo {
        kind: PropertyKind::CameraControl,
        id,
        name,
        min,
        max,
        step: if step > 0 { step } else { 1 },
        default,
        current,
        supports_auto: caps & 0x1 != 0,
        supports_manual: caps & 0x2 != 0,
        is_auto: flags & 0x1 != 0,
    })
}

// ---------------------------------------------------------------------------
// 解码线程分派
// ---------------------------------------------------------------------------

fn decode_loop(
    rx: Receiver<Arc<Vec<u8>>>,
    latest: Arc<Mutex<Option<Arc<Frame>>>>,
    stop: Arc<AtomicBool>,
    frames: Arc<AtomicU64>,
    subtype: FormatSubtype,
    current_dims: Arc<Mutex<(u32, u32)>>,
) {
    match subtype {
        FormatSubtype::Mjpeg => decode_mjpeg(rx, latest, stop, frames),
        FormatSubtype::Yuy2 => decode_yuy2(rx, latest, stop, frames, current_dims),
        FormatSubtype::H264 => {
            let (iw, ih) = *current_dims.lock().unwrap();
            if let Err(e) = crate::h264_preview::run(
                rx,
                latest,
                stop,
                frames,
                current_dims,
                iw,
                ih,
            ) {
                log::error!("H264 解码线程退出: {}", e);
            }
        }
    }
}

fn decode_mjpeg(
    rx: Receiver<Arc<Vec<u8>>>,
    latest: Arc<Mutex<Option<Arc<Frame>>>>,
    stop: Arc<AtomicBool>,
    frames: Arc<AtomicU64>,
) {
    let mut rgb_work: Vec<u8> = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        let raw = match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let mut decoder = JpegDecoder::new_with_options(
            raw.as_slice(),
            DecoderOptions::new_fast().jpeg_set_out_colorspace(ColorSpace::RGB),
        );
        if decoder.decode_headers().is_err() {
            continue;
        }
        let (w, h) = match decoder.dimensions() {
            Some(d) => d,
            None => continue,
        };
        let size = w * h * 3;
        if rgb_work.len() != size {
            rgb_work.resize(size, 0);
        }
        if decoder.decode_into(&mut rgb_work).is_err() {
            continue;
        }
        let frame = Arc::new(Frame {
            width: w as u32,
            height: h as u32,
            rgb: rgb_work.clone(),
            at: Instant::now(),
        });
        frames.fetch_add(1, Ordering::Relaxed);
        *latest.lock().unwrap() = Some(frame);
    }
}

fn decode_yuy2(
    rx: Receiver<Arc<Vec<u8>>>,
    latest: Arc<Mutex<Option<Arc<Frame>>>>,
    stop: Arc<AtomicBool>,
    frames: Arc<AtomicU64>,
    current_dims: Arc<Mutex<(u32, u32)>>,
) {
    let mut w: u32 = 0;
    let mut h: u32 = 0;
    let mut expected: usize = 0;
    let mut rgb_work: Vec<u8> = Vec::new();

    while !stop.load(Ordering::Relaxed) {
        let raw = match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let (cur_w, cur_h) = *current_dims.lock().unwrap();
        if cur_w == 0 || cur_h == 0 {
            continue;
        }
        if (cur_w, cur_h) != (w, h) {
            w = cur_w;
            h = cur_h;
            expected = w as usize * h as usize * 2;
            rgb_work.resize(w as usize * h as usize * 3, 0);
        }

        let line_bytes = w as usize * 2;
        if raw.len() < expected || raw.len() > expected + line_bytes {
            continue;
        }

        yuy2_to_rgb(&raw[..expected], &mut rgb_work);
        let frame = Arc::new(Frame {
            width: w,
            height: h,
            rgb: rgb_work.clone(),
            at: Instant::now(),
        });
        frames.fetch_add(1, Ordering::Relaxed);
        *latest.lock().unwrap() = Some(frame);
    }
}

fn yuy2_to_rgb(src: &[u8], dst: &mut [u8]) {
    let mut out = 0;
    for chunk in src.chunks_exact(4) {
        let y0 = chunk[0] as i32;
        let u = chunk[1] as i32 - 128;
        let y1 = chunk[2] as i32;
        let v = chunk[3] as i32 - 128;
        let c0 = y0 - 16;
        let c1 = y1 - 16;
        let r0 = clamp_u8((298 * c0 + 409 * v + 128) >> 8);
        let g0 = clamp_u8((298 * c0 - 100 * u - 208 * v + 128) >> 8);
        let b0 = clamp_u8((298 * c0 + 516 * u + 128) >> 8);
        let r1 = clamp_u8((298 * c1 + 409 * v + 128) >> 8);
        let g1 = clamp_u8((298 * c1 - 100 * u - 208 * v + 128) >> 8);
        let b1 = clamp_u8((298 * c1 + 516 * u + 128) >> 8);
        if out + 6 <= dst.len() {
            dst[out] = r0;
            dst[out + 1] = g0;
            dst[out + 2] = b0;
            dst[out + 3] = r1;
            dst[out + 4] = g1;
            dst[out + 5] = b1;
            out += 6;
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