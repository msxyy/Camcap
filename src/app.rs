use crate::camera::list_cameras;
use crate::capture::{
    CaptureHandle, FormatSubtype, PropertyInfo, PropertyKind, VideoFormat,
};
use crate::photo;
use crate::recorder::RecorderHandle;
use eframe::egui;
use egui::{ColorImage, Key, TextureHandle, TextureOptions, ViewportCommand};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const TEXTURE_MIN_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Clone, Copy, PartialEq, Eq)]
enum DisplayMode {
    Dual,
    SingleA,
    SingleB,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CompareLayout {
    Horizontal,
    Vertical,
}

fn setup_fonts(ctx: &egui::Context) {
    let t0 = Instant::now();
    let mut fonts = egui::FontDefinitions::default();
    let candidates: &[&str] = &[
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\msyh.ttf",
        r"C:\Windows\Fonts\msyhbd.ttc",
        r"C:\Windows\Fonts\simhei.ttf",
        r"C:\Windows\Fonts\simsun.ttc",
    ];
    let mut loaded = false;
    for path in candidates {
        match std::fs::read(path) {
            Ok(data) => {
                fonts
                    .font_data
                    .insert("cn".to_owned(), egui::FontData::from_owned(data));
                if let Some(f) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                    f.insert(0, "cn".to_owned());
                }
                if let Some(f) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                    f.push("cn".to_owned());
                }
                log::info!("已加载中文字体: {}", path);
                loaded = true;
                break;
            }
            Err(e) => log::debug!("跳过 {}: {}", path, e),
        }
    }
    if !loaded {
        log::warn!("未找到任何中文字体，界面中文可能显示为方块");
    }
    ctx.set_fonts(fonts);
    log::info!("[TIMING] setup_fonts: {:?}", t0.elapsed());
}

struct Fps {
    last_tick: Instant,
    last_count: u64,
    value: f32,
}

impl Fps {
    fn new() -> Self {
        Self {
            last_tick: Instant::now(),
            last_count: 0,
            value: 0.0,
        }
    }
    fn update(&mut self, total: u64) {
        let elapsed = self.last_tick.elapsed().as_secs_f32();
        if elapsed >= 0.5 {
            self.value = (total - self.last_count) as f32 / elapsed;
            self.last_count = total;
            self.last_tick = Instant::now();
        }
    }
}

// ---------------------------------------------------------------------------
// 格式分组
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct FormatGroup {
    codec: FormatSubtype,
    width: u32,
    height: u32,
    fps_list: Vec<u32>,
    fps_to_idx: Vec<usize>,
}

impl FormatGroup {
    fn label(&self) -> String {
        format!("{}  {}x{}", self.codec.name(), self.width, self.height)
    }
}

fn build_groups(formats: &[VideoFormat]) -> Vec<FormatGroup> {
    let mut groups: Vec<FormatGroup> = Vec::new();
    for (i, f) in formats.iter().enumerate() {
        if let Some(g) = groups.iter_mut().find(|g| {
            g.codec == f.subtype && g.width == f.width && g.height == f.height
        }) {
            if !g.fps_list.iter().any(|&x| x == f.fps) {
                g.fps_list.push(f.fps);
                g.fps_to_idx.push(i);
            }
        } else {
            groups.push(FormatGroup {
                codec: f.subtype,
                width: f.width,
                height: f.height,
                fps_list: vec![f.fps],
                fps_to_idx: vec![i],
            });
        }
    }

    groups.sort_by(|a, b| {
        let codec_order = |s: FormatSubtype| match s {
            FormatSubtype::Mjpeg => 0,
            FormatSubtype::Yuy2 => 1,
            FormatSubtype::H264 => 2,
        };
        codec_order(a.codec)
            .cmp(&codec_order(b.codec))
            .then_with(|| (b.width * b.height).cmp(&(a.width * a.height)))
    });

    for g in &mut groups {
        let mut paired: Vec<(u32, usize)> = g
            .fps_list
            .iter()
            .copied()
            .zip(g.fps_to_idx.iter().copied())
            .collect();
        paired.sort_by(|a, b| b.0.cmp(&a.0));
        g.fps_list = paired.iter().map(|&(fps, _)| fps).collect();
        g.fps_to_idx = paired.iter().map(|&(_, idx)| idx).collect();
    }

    groups
}

fn locate_format(groups: &[FormatGroup], target_idx: usize) -> (usize, usize) {
    for (gi, g) in groups.iter().enumerate() {
        for (fi, &idx) in g.fps_to_idx.iter().enumerate() {
            if idx == target_idx {
                return (gi, fi);
            }
        }
    }
    (0, 0)
}

fn pick_fps_idx(group: &FormatGroup, target_fps: u32) -> usize {
    if let Some(i) = group.fps_list.iter().position(|&f| f == target_fps) {
        return i;
    }
    let mut best = 0;
    let mut best_diff = u32::MAX;
    for (i, &f) in group.fps_list.iter().enumerate() {
        let diff = if f > target_fps { f - target_fps } else { target_fps - f };
        if diff < best_diff {
            best_diff = diff;
            best = i;
        }
    }
    best
}

// ---------------------------------------------------------------------------
// CameraSlot
// ---------------------------------------------------------------------------

struct CameraSlot {
    selected: usize,
    capture: Option<CaptureHandle>,
    recorder: Option<RecorderHandle>,
    texture: Option<TextureHandle>,
    fps: Fps,
    last_rendered_seq: u64,
    last_texture_update: Instant,
    props: Vec<PropertyInfo>,
    props_loaded: bool,
    show_props: bool,
    groups: Vec<FormatGroup>,
    group_idx: usize,
    fps_idx: usize,
    pending_switch: bool,
}

impl CameraSlot {
    fn new() -> Self {
        Self {
            selected: 0,
            capture: None,
            recorder: None,
            texture: None,
            fps: Fps::new(),
            last_rendered_seq: 0,
            last_texture_update: Instant::now(),
            props: Vec::new(),
            props_loaded: false,
            show_props: false,
            groups: Vec::new(),
            group_idx: 0,
            fps_idx: 0,
            pending_switch: false,
        }
    }

    fn is_open(&self) -> bool {
        self.capture.is_some()
    }
    fn is_recording(&self) -> bool {
        self.recorder.is_some()
    }

    fn open(&mut self, camera_index: u32) -> Result<(u32, u32), String> {
        self.open_with_preferred(camera_index, None)
    }

    fn open_with_preferred(
        &mut self,
        camera_index: u32,
        preferred: Option<(FormatSubtype, u32, u32, u32)>,
    ) -> Result<(u32, u32), String> {
        let t0 = Instant::now();
        self.close();
        match CaptureHandle::start_with_format(camera_index, preferred) {
            Ok(c) => {
                let (w, h) = (c.width, c.height);
                self.groups = build_groups(&c.available_formats);
                let (gi, fi) = locate_format(&self.groups, c.current_format_idx);
                self.group_idx = gi;
                self.fps_idx = fi;
                self.pending_switch = false;
                self.capture = Some(c);
                self.fps = Fps::new();
                self.last_rendered_seq = 0;
                self.last_texture_update = Instant::now();
                self.props = Vec::new();
                self.props_loaded = false;
                self.show_props = false;
                log::info!("[TIMING] open_with_preferred 总计: {:?}", t0.elapsed());
                Ok((w, h))
            }
            Err(e) => {
                log::warn!(
                    "[TIMING] open_with_preferred 失败（{:?}）: {}",
                    t0.elapsed(),
                    e
                );
                Err(format!("打开失败: {}", e))
            }
        }
    }

    fn close(&mut self) {
        if let Some(mut r) = self.recorder.take() {
            if let Some(c) = self.capture.as_ref() {
                c.detach_recorder();
            }
            r.stop();
        }
        if let Some(mut c) = self.capture.take() {
            c.stop();
        }
        self.texture = None;
        self.props.clear();
        self.props_loaded = false;
        self.show_props = false;
        self.groups.clear();
        self.group_idx = 0;
        self.fps_idx = 0;
        self.pending_switch = false;
        self.last_texture_update = Instant::now();
    }

    fn reconcile(&mut self) {
        if let Some(cap) = self.capture.as_ref() {
            let cur_idx = cap.current_format_idx;
            let (gi, fi) = locate_format(&self.groups, cur_idx);
            let need_sync = self
                .groups
                .get(self.group_idx)
                .and_then(|g| g.fps_to_idx.get(self.fps_idx).copied())
                != Some(cur_idx);
            if need_sync && self.pending_switch {
                self.group_idx = gi;
                self.fps_idx = fi;
                self.pending_switch = false;
                log::warn!(
                    "UI 与驱动状态不一致，已回滚到实际格式索引 {}",
                    cur_idx
                );
            }
        }
    }

    fn toggle_recording(
        &mut self,
        output_dir: &Path,
        tag: &str,
    ) -> Result<Option<PathBuf>, String> {
        if let Some(mut r) = self.recorder.take() {
            if let Some(c) = self.capture.as_ref() {
                c.detach_recorder();
            }
            r.stop();
            return Ok(None);
        }

        let Some(cap) = self.capture.as_ref() else {
            return Err("请先打开摄像头".into());
        };
        let (w, h) = (cap.width, cap.height);
        let subtype = cap.current_subtype;

        let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let ext = match subtype {
            FormatSubtype::H264 => "mp4",
            _ => "avi",
        };
        let tag_codec = match subtype {
            FormatSubtype::Mjpeg => "mjpg",
            FormatSubtype::Yuy2 => "yuy2",
            FormatSubtype::H264 => "h264",
        };
        let path = output_dir.join(format!(
            "recording-{}-{}-{}.{}",
            tag, tag_codec, ts, ext
        ));

        let (rec, tx) = RecorderHandle::start(path.clone(), w, h, 30, 30, subtype)
            .map_err(|e| e.to_string())?;
        cap.attach_recorder(tx);
        self.recorder = Some(rec);
        Ok(Some(path))
    }

    fn snapshot(&self, output_dir: &Path, tag: &str) -> Result<PathBuf, String> {
        let cap = self.capture.as_ref().ok_or("请先打开摄像头")?;
        let frame = cap.peek_latest().ok_or("暂无可用帧")?;
        let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let path = output_dir.join(format!("photo-{}-{}.png", tag, ts));
        photo::save_frame(&frame, &path).map_err(|e| e.to_string())?;
        Ok(path)
    }

    fn pump_texture(&mut self, ctx: &egui::Context, tag: &str) {
        let (seq, frame_opt) = {
            let Some(cap) = self.capture.as_ref() else {
                return;
            };
            self.fps.update(cap.frames());
            (cap.frames(), cap.peek_latest())
        };
        if self.last_rendered_seq == seq {
            return;
        }
        if self.last_texture_update.elapsed() < TEXTURE_MIN_INTERVAL {
            return;
        }
        let Some(frame) = frame_opt else { return };
        self.last_rendered_seq = seq;
        self.last_texture_update = Instant::now();

        let size = [frame.width as usize, frame.height as usize];
        let img = ColorImage::from_rgb(size, &frame.rgb);
        match self.texture.as_mut() {
            Some(tex) => tex.set(img, TextureOptions::LINEAR),
            None => {
                self.texture = Some(ctx.load_texture(
                    format!("cam-{}", tag),
                    img,
                    TextureOptions::LINEAR,
                ));
            }
        }
    }

    /// 短格式标签：`MJPEG 2592x1944 @ 30`
    fn short_format_label(&self) -> String {
        let Some(cap) = self.capture.as_ref() else {
            return "未打开".to_string();
        };
        if let Some(f) = cap.available_formats.get(cap.current_format_idx) {
            format!(
                "{} {}x{} @ {}",
                f.subtype.name(),
                f.width,
                f.height,
                f.fps
            )
        } else {
            cap.current_subtype.name().to_string()
        }
    }
}

// ---------------------------------------------------------------------------
// 应用格式切换
// ---------------------------------------------------------------------------

fn apply_format(
    slot: &mut CameraSlot,
    cameras: &[(u32, String)],
    tag: &str,
    status: &mut String,
    error: &mut Option<String>,
    prev_group_idx: usize,
    prev_fps_idx: usize,
) {
    let Some(cap) = slot.capture.as_ref() else {
        slot.group_idx = prev_group_idx;
        slot.fps_idx = prev_fps_idx;
        return;
    };
    let Some(group) = slot.groups.get(slot.group_idx).cloned() else {
        slot.group_idx = prev_group_idx;
        slot.fps_idx = prev_fps_idx;
        return;
    };
    let Some(&target_idx) = group.fps_to_idx.get(slot.fps_idx) else {
        slot.group_idx = prev_group_idx;
        slot.fps_idx = prev_fps_idx;
        return;
    };

    if target_idx == cap.current_format_idx {
        slot.pending_switch = false;
        return;
    }

    let target_subtype = group.codec;
    let target_w = group.width;
    let target_h = group.height;
    let target_fps = group.fps_list[slot.fps_idx];
    let same_subtype = target_subtype == cap.current_subtype;
    let cur_idx = cap.current_format_idx;

    if same_subtype {
        log::info!(
            "[{}] 尝试热切换: cur_idx={} target_idx={} target={}x{}@{}",
            tag, cur_idx, target_idx, target_w, target_h, target_fps
        );
        let t_hot = Instant::now();
        slot.pending_switch = true;
        let set_result = slot.capture.as_mut().map(|c| c.set_format(target_idx));
        match set_result {
            Some(Ok(f)) => {
                log::info!(
                    "[TIMING] [{}] 热切换成功: {:?}",
                    tag,
                    t_hot.elapsed()
                );
                *status = format!("[{}] 格式已切换: {}", tag, f.label());
                slot.last_rendered_seq = 0;
                slot.last_texture_update = Instant::now();
                slot.props_loaded = false;
                slot.pending_switch = false;
                *error = None;
                return;
            }
            Some(Err(e)) => {
                log::warn!(
                    "[TIMING] [{}] 热切换失败（{:?}），回退到重启: {}",
                    tag,
                    t_hot.elapsed(),
                    e
                );
            }
            None => {
                slot.group_idx = prev_group_idx;
                slot.fps_idx = prev_fps_idx;
                slot.pending_switch = false;
                return;
            }
        }
    }

    let cam_idx = cameras.get(slot.selected).map(|(i, _)| *i).unwrap_or(0);
    let preferred = Some((target_subtype, target_w, target_h, target_fps));
    log::info!(
        "[{}] 重启 reader: target={} {}x{}@{}",
        tag, target_subtype.name(), target_w, target_h, target_fps
    );
    let t_restart = Instant::now();
    match slot.open_with_preferred(cam_idx, preferred) {
        Ok((w, h)) => {
            log::info!(
                "[TIMING] [{}] 重启成功: {:?}",
                tag,
                t_restart.elapsed()
            );
            *status = format!(
                "[{}] 已重启为 {} ({}x{})",
                tag, target_subtype.name(), w, h
            );
            *error = None;
        }
        Err(e) => {
            log::error!(
                "[TIMING] [{}] 重启失败（{:?}）: {}",
                tag,
                t_restart.elapsed(),
                e
            );
            *error = Some(format!("重启失败: {}", e));
            slot.group_idx = prev_group_idx;
            slot.fps_idx = prev_fps_idx;
        }
    }
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

pub struct CamCapApp {
    cameras: Vec<(u32, String)>,
    slots: [CameraSlot; 2],
    status: String,
    error: Option<String>,
    output_dir: PathBuf,
    display_mode: DisplayMode,
    compare_layout: CompareLayout,
    pure_compare: bool,
}

impl CamCapApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_fonts(&cc.egui_ctx);
        let output_dir = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("camcap-output");
        let _ = std::fs::create_dir_all(&output_dir);

        let cameras: Vec<(u32, String)> = match list_cameras() {
            Ok(list) => list.into_iter().map(|c| (c.index, c.name)).collect(),
            Err(e) => {
                log::error!("枚举摄像头失败: {}", e);
                Vec::new()
            }
        };

        let status = if cameras.is_empty() {
            "未检测到摄像头".into()
        } else {
            format!("检测到 {} 个摄像头", cameras.len())
        };

        Self {
            cameras,
            slots: [CameraSlot::new(), CameraSlot::new()],
            status,
            error: None,
            output_dir,
            display_mode: DisplayMode::Dual,
            compare_layout: CompareLayout::Horizontal,
            pure_compare: false,
        }
    }

    fn refresh_cameras(&mut self) {
        match list_cameras() {
            Ok(list) => {
                self.status = format!("检测到 {} 个摄像头", list.len());
                self.cameras = list.into_iter().map(|c| (c.index, c.name)).collect();
                self.error = None;
            }
            Err(e) => self.error = Some(format!("枚举失败: {}", e)),
        }
    }

    fn open_both(&mut self) {
        if self.cameras.is_empty() {
            self.error = Some("没有可用摄像头".into());
            return;
        }

        let (idx_a, name_a) = self.cameras[self.slots[0].selected].clone();
        match self.slots[0].open(idx_a) {
            Ok((w, h)) => {
                self.status = format!("[A] {} ({}x{})", name_a, w, h);
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }

        if self.cameras.len() > 1 && self.slots[1].selected == self.slots[0].selected {
            self.slots[1].selected = (self.slots[0].selected + 1) % self.cameras.len();
        }
        let (idx_b, name_b) = self.cameras[self.slots[1].selected].clone();
        match self.slots[1].open(idx_b) {
            Ok((w, h)) => {
                self.status = format!("[B] {} ({}x{})", name_b, w, h);
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }

    fn close_both(&mut self) {
        for s in self.slots.iter_mut() {
            s.close();
        }
        self.status = "已全部关闭".into();
    }
}

// ---------------------------------------------------------------------------
// 单画面渲染
// ---------------------------------------------------------------------------

fn render_slot(
    ui: &mut egui::Ui,
    slot: &mut CameraSlot,
    cameras: &[(u32, String)],
    tag: &str,
    output_dir: &Path,
    status: &mut String,
    error: &mut Option<String>,
) {
    slot.reconcile();

    let is_open = slot.is_open();
    let is_recording = slot.is_recording();

    // ---------- 第一行：设备 / 格式 / 帧率 / 打开-关闭 / 参数 ----------
    ui.horizontal(|ui| {
        // 摄像头
        let current_cam = cameras
            .get(slot.selected)
            .map(|(_, n)| n.clone())
            .unwrap_or_else(|| "无".into());
        egui::ComboBox::from_id_salt(format!("combo-{}", tag))
            .selected_text(current_cam)
            .width(120.0)
            .show_ui(ui, |ui| {
                for (i, (_, name)) in cameras.iter().enumerate() {
                    ui.selectable_value(&mut slot.selected, i, name);
                }
            });

        // 格式/分辨率
        let groups_snapshot: Vec<FormatGroup> = slot.groups.clone();
        let current_group_label = groups_snapshot
            .get(slot.group_idx)
            .map(|g| g.label())
            .unwrap_or_else(|| "—".into());

        ui.add_enabled_ui(is_open && !is_recording, |ui| {
            let mut new_group_idx = slot.group_idx;
            egui::ComboBox::from_id_salt(format!("pin-combo-{}", tag))
                .selected_text(current_group_label)
                .width(190.0)
                .show_ui(ui, |ui| {
                    for (i, g) in groups_snapshot.iter().enumerate() {
                        ui.selectable_value(&mut new_group_idx, i, g.label());
                    }
                });

            if new_group_idx != slot.group_idx {
                let old_fps = slot
                    .groups
                    .get(slot.group_idx)
                    .and_then(|g| g.fps_list.get(slot.fps_idx).copied())
                    .unwrap_or(30);
                let prev_group_idx = slot.group_idx;
                let prev_fps_idx = slot.fps_idx;
                slot.group_idx = new_group_idx;
                if let Some(g) = slot.groups.get(new_group_idx) {
                    slot.fps_idx = pick_fps_idx(g, old_fps);
                } else {
                    slot.fps_idx = 0;
                }
                apply_format(
                    slot,
                    cameras,
                    tag,
                    status,
                    error,
                    prev_group_idx,
                    prev_fps_idx,
                );
            }
        });

        // 帧率
        let fps_list: Vec<u32> = slot
            .groups
            .get(slot.group_idx)
            .map(|g| g.fps_list.clone())
            .unwrap_or_default();
        let current_fps = fps_list.get(slot.fps_idx).copied().unwrap_or(0);

        ui.add_enabled_ui(is_open && !is_recording && !fps_list.is_empty(), |ui| {
            let mut new_fps_idx = slot.fps_idx;
            egui::ComboBox::from_id_salt(format!("fps-combo-{}", tag))
                .selected_text(if current_fps > 0 {
                    format!("{}fps", current_fps)
                } else {
                    "—".to_string()
                })
                .width(64.0)
                .show_ui(ui, |ui| {
                    for (i, &f) in fps_list.iter().enumerate() {
                        ui.selectable_value(&mut new_fps_idx, i, format!("{} fps", f));
                    }
                });

            if new_fps_idx != slot.fps_idx {
                let prev_group_idx = slot.group_idx;
                let prev_fps_idx = slot.fps_idx;
                slot.fps_idx = new_fps_idx;
                apply_format(
                    slot,
                    cameras,
                    tag,
                    status,
                    error,
                    prev_group_idx,
                    prev_fps_idx,
                );
            }
        });

        // 打开/关闭 合一
        let toggle_label = if is_open { "关闭" } else { "打开" };
        if ui
            .add(egui::Button::new(toggle_label).min_size(egui::vec2(50.0, 24.0)))
            .clicked()
        {
            if is_open {
                slot.close();
                *status = format!("[{}] 已关闭", tag);
            } else if let Some((idx, name)) = cameras.get(slot.selected).cloned() {
                match slot.open(idx) {
                    Ok((w, h)) => {
                        *status = format!("[{}] {} ({}x{})", tag, name, w, h);
                        *error = None;
                    }
                    Err(e) => *error = Some(e),
                }
            }
        }

        // 参数
        ui.add_enabled_ui(is_open, |ui| {
            let resp = ui
                .add(egui::Button::new("⚙").min_size(egui::vec2(28.0, 24.0)))
                .on_hover_text("设备参数（亮度/对比度/曝光等）");
            if resp.clicked() {
                slot.show_props = !slot.show_props;
            }
            if slot.show_props {
                ui.visuals_mut().override_text_color =
                    Some(egui::Color32::from_rgb(100, 180, 255));
            }
        });
    });

    // ---------- 第二行：拍照 / 录制 / FPS / 格式 / REC ----------
    ui.horizontal(|ui| {
        // 拍照
        let photo_resp = ui.add_enabled(
            is_open,
            egui::Button::new("📷 拍照").min_size(egui::vec2(66.0, 24.0)),
        );
        if photo_resp.clicked() {
            match slot.snapshot(output_dir, tag) {
                Ok(p) => {
                    *status = format!("[{}] 已保存 {}", tag, p.display());
                    *error = None;
                }
                Err(e) => *error = Some(e),
            }
        }

        // 录制
        if is_recording {
            let resp = ui.add(
                egui::Button::new("⏹ 停止")
                    .fill(egui::Color32::from_rgb(180, 50, 50))
                    .min_size(egui::vec2(74.0, 24.0)),
            );
            if resp.clicked() {
                match slot.toggle_recording(output_dir, tag) {
                    Ok(_) => {
                        *status = format!("[{}] 录制已停止", tag);
                        *error = None;
                    }
                    Err(e) => *error = Some(e),
                }
            }
        } else {
            let resp = ui.add_enabled(
                is_open,
                egui::Button::new("⏺ 录制")
                    .fill(egui::Color32::from_rgb(50, 130, 60))
                    .min_size(egui::vec2(74.0, 24.0)),
            );
            if resp.clicked() {
                match slot.toggle_recording(output_dir, tag) {
                    Ok(Some(p)) => {
                        *status = format!("[{}] 录制中 {}", tag, p.display());
                        *error = None;
                    }
                    Ok(None) => {}
                    Err(e) => *error = Some(e),
                }
            }
        }

        ui.separator();

        // FPS + 格式，紧凑显示
        ui.label(
            egui::RichText::new(format!("FPS {:.0}", slot.fps.value))
                .color(egui::Color32::from_gray(180)),
        );
        ui.label(
            egui::RichText::new(slot.short_format_label())
                .small()
                .color(egui::Color32::from_gray(160)),
        );

        // REC 状态右对齐
        ui.with_layout(
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                if let Some(rec) = slot.recorder.as_ref() {
                    let d = rec.dropped.load(std::sync::atomic::Ordering::Relaxed);
                    let w = rec.written.load(std::sync::atomic::Ordering::Relaxed);
                    ui.label(
                        egui::RichText::new(format!("● 写 {} / 丢 {}", w, d))
                            .color(egui::Color32::from_rgb(230, 60, 60))
                            .small(),
                    );
                }
            },
        );
    });

    ui.separator();

    // ---------- 预览区 ----------
    let avail = ui.available_size();
    if avail.y < 50.0 {
        ui.centered_and_justified(|ui| {
            ui.label("窗口过小，请扩大窗口以显示预览");
        });
        return;
    }
    let (rect, _) = ui.allocate_exact_size(avail, egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, 4.0, egui::Color32::from_black_alpha(50));
    paint_video(ui, slot.texture.as_ref(), rect, tag, is_open);
}

fn paint_video(
    ui: &egui::Ui,
    tex: Option<&TextureHandle>,
    rect: egui::Rect,
    tag: &str,
    is_open: bool,
) {
    ui.painter().rect_filled(rect, 0.0, egui::Color32::BLACK);
    let Some(tex) = tex else {
        let msg = if is_open {
            format!("画面 {} 正在等待首帧…", tag)
        } else {
            format!("画面 {} 未打开", tag)
        };
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            msg,
            egui::FontId::proportional(18.0),
            egui::Color32::GRAY,
        );
        return;
    };
    let img_size = tex.size_vec2();
    if img_size.x <= 0.0 || img_size.y <= 0.0 {
        return;
    }
    let scale = (rect.width() / img_size.x).min(rect.height() / img_size.y);
    let display = img_size * scale;
    let pos = rect.center() - display / 2.0;
    let img_rect = egui::Rect::from_min_size(pos, display);
    ui.painter().image(
        tex.id(),
        img_rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
}

/// 在左上角画一条半透明底的提示文本。
/// 双画面纯净和单画面纯净共用。
fn paint_hint(ui: &egui::Ui, text: &str) {
    let pos = ui.max_rect().left_top() + egui::vec2(12.0, 12.0);
    let galley = ui.painter().layout_no_wrap(
        text.to_string(),
        egui::FontId::proportional(14.0),
        egui::Color32::from_white_alpha(200),
    );
    let bg = egui::Rect::from_min_size(pos, galley.size()).expand(6.0);
    ui.painter()
        .rect_filled(bg, 4.0, egui::Color32::from_black_alpha(120));
    ui.painter().galley(pos, galley, egui::Color32::WHITE);
}

// ---------------------------------------------------------------------------
// 纯净对比
// ---------------------------------------------------------------------------

fn render_pure_compare(ui: &mut egui::Ui, slots: &[CameraSlot; 2], layout: CompareLayout) {
    let rect = ui.available_rect_before_wrap();
    ui.painter()
        .rect_filled(rect, 0.0, egui::Color32::BLACK);

    match layout {
        CompareLayout::Horizontal => {
            let half = rect.width() / 2.0;
            let rect_a = egui::Rect::from_min_size(rect.min, egui::vec2(half, rect.height()));
            let rect_b = egui::Rect::from_min_size(
                egui::pos2(rect.min.x + half, rect.min.y),
                egui::vec2(half, rect.height()),
            );
            paint_video(ui, slots[0].texture.as_ref(), rect_a, "A", slots[0].is_open());
            paint_video(ui, slots[1].texture.as_ref(), rect_b, "B", slots[1].is_open());
            ui.painter().line_segment(
                [
                    egui::pos2(rect.min.x + half, rect.min.y),
                    egui::pos2(rect.min.x + half, rect.max.y),
                ],
                egui::Stroke::new(1.0_f32, egui::Color32::from_gray(80)),
            );
        }
        CompareLayout::Vertical => {
            let half = rect.height() / 2.0;
            let rect_a = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), half));
            let rect_b = egui::Rect::from_min_size(
                egui::pos2(rect.min.x, rect.min.y + half),
                egui::vec2(rect.width(), half),
            );
            paint_video(ui, slots[0].texture.as_ref(), rect_a, "A", slots[0].is_open());
            paint_video(ui, slots[1].texture.as_ref(), rect_b, "B", slots[1].is_open());
            ui.painter().line_segment(
                [
                    egui::pos2(rect.min.x, rect.min.y + half),
                    egui::pos2(rect.max.x, rect.min.y + half),
                ],
                egui::Stroke::new(1.0_f32, egui::Color32::from_gray(80)),
            );
        }
    }

    ui.allocate_rect(rect, egui::Sense::hover());
}

/// 单画面纯净模式：全窗口只画一个 slot 的预览。
fn render_pure_single(ui: &mut egui::Ui, slot: &CameraSlot, tag: &str) {
    let rect = ui.available_rect_before_wrap();
    // paint_video 内部会先填黑，再按比例居中画纹理 / 显示“未打开”提示
    paint_video(ui, slot.texture.as_ref(), rect, tag, slot.is_open());
    ui.allocate_rect(rect, egui::Sense::hover());
}

// ---------------------------------------------------------------------------
// 参数浮窗
// ---------------------------------------------------------------------------

fn render_props_window(ctx: &egui::Context, slot: &mut CameraSlot, tag: &str) {
    if !slot.show_props || !slot.is_open() {
        return;
    }

    if !slot.props_loaded {
        if let Some(cap) = slot.capture.as_ref() {
            slot.props = cap.read_properties();
            slot.props_loaded = true;
        }
    }

    let mut open = slot.show_props;
    egui::Window::new(format!("设备参数 · 画面 {}", tag))
        .id(egui::Id::new(format!("props-{}", tag)))
        .open(&mut open)
        .resizable(true)
        .default_width(360.0)
        .min_width(320.0)
        .max_height(ctx.available_rect().height() * 0.8)
        .show(ctx, |ui| {
            if slot.props.is_empty() {
                ui.label("设备未报告任何可调属性");
                return;
            }

            // ---------- 一键恢复默认 ----------
            ui.horizontal(|ui| {
                if ui
                    .button("全部恢复默认")
                    .on_hover_text("把所有参数重置为驱动默认值")
                    .clicked()
                {
                    // 先改 UI 侧状态，再统一发送，避免边改边发时
                    // 控制线程处理顺序和 UI 显示不一致。
                    for prop in slot.props.iter_mut() {
                        prop.current = prop.default;
                    }
                    if let Some(cap) = slot.capture.as_ref() {
                        for prop in slot.props.iter() {
                            let manual = !prop.is_auto;
                            if let Err(e) =
                                cap.set_property(prop.kind, prop.id, prop.default, manual)
                            {
                                log::warn!(
                                    "恢复默认失败 {}（id={}）: {}",
                                    prop.name,
                                    prop.id,
                                    e
                                );
                            }
                        }
                        log::info!("已一键恢复 {} 个参数默认值", slot.props.len());
                    }
                }
                ui.label(
                    egui::RichText::new("（数值回到驱动默认，自动/手动状态保持不变）")
                        .small()
                        .color(egui::Color32::from_gray(140)),
                );
            });
            ui.separator();

            // ---------- 单个参数 ----------
            // 改成 Vec：一帧内可能同时有多条命令要发
            // （例如拖动滑块 + 勾选自动，或一键默认）。
            let mut pending: Vec<(PropertyKind, i32, i32, bool)> = Vec::new();

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for prop in slot.props.iter_mut() {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.set_min_width(300.0);

                            if prop.supports_auto && prop.supports_manual {
                                let mut is_auto = prop.is_auto;
                                if ui
                                    .checkbox(&mut is_auto, "自动")
                                    .on_hover_text("切换自动/手动模式")
                                    .changed()
                                {
                                    prop.is_auto = is_auto;
                                    pending.push((
                                        prop.kind,
                                        prop.id,
                                        prop.current,
                                        !is_auto,
                                    ));
                                }
                            }

                            ui.label(prop.name);

                            let slider =
                                egui::Slider::new(&mut prop.current, prop.min..=prop.max)
                                    .step_by(prop.step as f64)
                                    .show_value(true);
                            let resp = ui.add_enabled(!prop.is_auto, slider);
                            if resp.changed() {
                                pending.push((prop.kind, prop.id, prop.current, true));
                            }

                            if ui
                                .small_button("↺")
                                .on_hover_text(format!("默认 {}", prop.default))
                                .clicked()
                            {
                                prop.current = prop.default;
                                pending.push((
                                    prop.kind,
                                    prop.id,
                                    prop.default,
                                    !prop.is_auto,
                                ));
                            }
                        });
                    }
                });

            for (kind, id, value, manual) in pending {
                if let Some(cap) = slot.capture.as_ref() {
                    let _ = cap.set_property(kind, id, value, manual);
                }
            }
        });

    slot.show_props = open;
}

// ---------------------------------------------------------------------------
// App 主循环
// ---------------------------------------------------------------------------

impl eframe::App for CamCapApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let is_full_before = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));

        let toggle_fullscreen = ctx.input(|i| {
            i.key_pressed(Key::F11) || (i.key_pressed(Key::Enter) && i.modifiers.alt)
        });
        if toggle_fullscreen {
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(!is_full_before));
        }

        // P 键：切换纯净模式。
        // 不再强制切到 Dual。当前是「仅 A」就进入单画面纯净 A，
        // 当前是「双画面」就进入双画面纯净。
        if ctx.input(|i| i.key_pressed(Key::P)) {
            self.pure_compare = !self.pure_compare;
        }

        // Tab：只在双画面纯净模式下切换左右/上下。
        // 单画面纯净没有并排布局，Tab 不做任何事。
        if self.pure_compare
            && self.display_mode == DisplayMode::Dual
            && ctx.input(|i| i.key_pressed(Key::Tab))
        {
            self.compare_layout = match self.compare_layout {
                CompareLayout::Horizontal => CompareLayout::Vertical,
                CompareLayout::Vertical => CompareLayout::Horizontal,
            };
        }

        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            if is_full_before {
                ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
            } else if self.pure_compare {
                self.pure_compare = false;
            }
        }

        match self.display_mode {
            DisplayMode::Dual => {
                self.slots[0].pump_texture(ctx, "A");
                self.slots[1].pump_texture(ctx, "B");
            }
            DisplayMode::SingleA => self.slots[0].pump_texture(ctx, "A"),
            DisplayMode::SingleB => self.slots[1].pump_texture(ctx, "B"),
        }

        let show_chrome = !is_full_before && !self.pure_compare;

        if show_chrome {
            egui::TopBottomPanel::top("toolbar")
                .exact_height(72.0)
                .show(ctx, |ui| {
                    ui.add_space(2.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("打开全部").clicked() {
                            self.open_both();
                        }
                        if ui.button("关闭全部").clicked() {
                            self.close_both();
                        }
                        if ui.button("刷新设备").clicked() {
                            self.refresh_cameras();
                        }

                        ui.separator();

                        ui.label("显示:");
                        ui.selectable_value(
                            &mut self.display_mode,
                            DisplayMode::Dual,
                            "双画面",
                        );
                        ui.selectable_value(
                            &mut self.display_mode,
                            DisplayMode::SingleA,
                            "仅 A",
                        );
                        ui.selectable_value(
                            &mut self.display_mode,
                            DisplayMode::SingleB,
                            "仅 B",
                        );

                        ui.separator();

                        let pure_label = if self.pure_compare {
                            "退出纯净 (P)"
                        } else {
                            "纯净对比 (P)"
                        };
                        let pure_btn = egui::Button::new(pure_label)
                            .fill(egui::Color32::from_rgb(50, 90, 160));
                        if ui.add(pure_btn).clicked() {
                            // 同样不再强制切到 Dual
                            self.pure_compare = !self.pure_compare;
                        }

                        if ui.button("全屏 (F11)").clicked() {
                            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(true));
                        }

                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "摄像头: {}",
                                        self.cameras.len()
                                    ))
                                    .small()
                                    .color(egui::Color32::from_gray(160)),
                                );
                            },
                        );
                    });
                });

            egui::TopBottomPanel::bottom("status")
                .exact_height(26.0)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(&self.status);
                        if let Some(err) = &self.error {
                            ui.separator();
                            ui.colored_label(
                                egui::Color32::from_rgb(220, 80, 80),
                                err,
                            );
                        }
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "输出: {}",
                                        self.output_dir.display()
                                    ))
                                    .small()
                                    .color(egui::Color32::from_gray(140)),
                                );
                            },
                        );
                    });
                });
        }

        let pure = self.pure_compare;
        let mode = self.display_mode;
        let layout = self.compare_layout;

        let mut panel = egui::CentralPanel::default();
        if pure {
            panel = panel.frame(egui::Frame::none());
        }

        panel.show(ctx, |ui| {
            if pure {
                // 纯净模式：按当前 display_mode 分三种
                match mode {
                    DisplayMode::Dual => {
                        render_pure_compare(ui, &self.slots, layout);
                        let layout_name = match layout {
                            CompareLayout::Horizontal => "左右",
                            CompareLayout::Vertical => "上下",
                        };
                        let hint = format!(
                            "纯净对比 [{}]   Tab 切换左右/上下   P / Esc 退出",
                            layout_name
                        );
                        paint_hint(ui, &hint);
                    }
                    DisplayMode::SingleA => {
                        render_pure_single(ui, &self.slots[0], "A");
                        paint_hint(ui, "纯净模式 [画面 A]   P / Esc 退出");
                    }
                    DisplayMode::SingleB => {
                        render_pure_single(ui, &self.slots[1], "B");
                        paint_hint(ui, "纯净模式 [画面 B]   P / Esc 退出");
                    }
                }
            } else {
                let status = &mut self.status;
                let error = &mut self.error;
                let cameras = &self.cameras;
                let output_dir = &self.output_dir;
                let slots = &mut self.slots;

                match mode {
                    DisplayMode::Dual => {
                        ui.columns(2, |cols| {
                            render_slot(
                                &mut cols[0],
                                &mut slots[0],
                                cameras,
                                "A",
                                output_dir,
                                status,
                                error,
                            );
                            render_slot(
                                &mut cols[1],
                                &mut slots[1],
                                cameras,
                                "B",
                                output_dir,
                                status,
                                error,
                            );
                        });
                    }
                    DisplayMode::SingleA => {
                        render_slot(
                            ui,
                            &mut slots[0],
                            cameras,
                            "A",
                            output_dir,
                            status,
                            error,
                        );
                    }
                    DisplayMode::SingleB => {
                        render_slot(
                            ui,
                            &mut slots[1],
                            cameras,
                            "B",
                            output_dir,
                            status,
                            error,
                        );
                    }
                }
            }
        });

        if show_chrome {
            let slots = &mut self.slots;
            if mode == DisplayMode::Dual || mode == DisplayMode::SingleA {
                render_props_window(ctx, &mut slots[0], "A");
            }
            if mode == DisplayMode::Dual || mode == DisplayMode::SingleB {
                render_props_window(ctx, &mut slots[1], "B");
            }
        }

        if self.slots[0].is_open() || self.slots[1].is_open() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }
}