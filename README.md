# CamCap

Rust 编写的 Windows 摄像头预览 / 拍照 / 录制工具，风格参考 AMCap。
基于 Media Foundation（原生 `windows` crate 绑定），支持双摄像头同时预览、热切换格式、纯净模式。

## 功能

- 枚举并选择摄像头（Windows 走 Media Foundation）
- **双画面并排预览**，方便同时对比两个摄像头
- 每个画面独立：选择设备 / 格式 / 帧率 / 打开 / 关闭 / 快照 / 录制
- 一键「打开全部」「关闭全部」「刷新设备」
- 格式与帧率**分开选择**：先选分辨率（约 15 项），再选该分辨率支持的帧率（约 7 项），避免 108 项长列表滚动
- 支持 **MJPG / YUY2 / H264** 三种原生格式
- 快照保存为 PNG；MJPG/YUY2 录制为 AVI，H264 录制为 MP4（用 `muxide` 直接封装，**无需 ffmpeg**）
- 每个画面独立 FPS 统计、录制帧数/丢帧统计
- **纯净模式**：双画面 / 单画面（仅 A / 仅 B）均支持。双画面下可左右/上下并排，单画面下全窗口只画一个画面
- 设备参数浮窗：亮度 / 对比度 / 饱和度 / 白平衡 / 曝光 / 增益等（DirectShow `IAMVideoProcAmp` / `IAMCameraControl`），支持单参数「↺」恢复默认和**一键全部恢复默认**

## 平台支持

目前 **仅 Windows**。设备枚举、格式枚举、异步采集、硬件 H264 解码全部依赖 Media Foundation。`camera.rs` 里的非 Windows 分支返回空列表，只是占位。

## 依赖

- Rust 1.75+
- Windows 10 / 11（Media Foundation 自带）
- 无需 ffmpeg

## 运行

```powershell
cd D:\Rust\project\camcap
cargo run --release
```

日志同时输出到控制台和 `camcap-run.log`。输出文件默认放在 `./camcap-output/`。

## 快捷键

| 按键 | 功能 |
|---|---|
| `P` | 进入/退出纯净模式。**在当前显示模式下生效**：双画面 → 双画面纯净；仅 A → 单画面纯净 A；仅 B → 单画面纯净 B |
| `Tab` | **仅双画面纯净模式下**切换左右/上下；单画面纯净下无反应 |
| `F11` / `Alt+Enter` | 全屏 |
| `Esc` | 退出全屏 / 退出纯净模式 |

## 输出目录

```
camcap-output/
  photo-A-YYYYMMDD-HHMMSS.png
  photo-B-YYYYMMDD-HHMMSS.png
  recording-A-mjpg-YYYYMMDD-HHMMSS.avi
  recording-A-yuy2-YYYYMMDD-HHMMSS.avi
  recording-A-h264-YYYYMMDD-HHMMSS.mp4
  recording-B-...
```

## 格式切换行为

格式下拉和帧率下拉**选中即生效**，无需额外的「应用」按钮。内部两条路径：

1. **同 subtype 热切换**（如 MJPG 2592x1944 → MJPG 1920x1080）：先调 `IMFSourceReader::SetCurrentMediaType`。成功即视为切换完成，新帧异步到达。
2. **跨 subtype 或热切换失败**：关闭当前 reader，重新走 `CaptureHandle::start_with_format` 打开目标格式。耗时约 0.8–1.5s（含 reader 重建 + 首帧就绪）。

录制中格式下拉被禁用，避免录制文件分辨率跳变。

---

## 纯净模式

纯净模式隐藏所有 UI，只保留画面本身，适合对比画面、录制截图、演示。

按 `P` 进入/退出。**进入时不会改变当前显示模式**：

| 进入纯净前的显示模式 | 纯净模式表现 | 左上角提示 | Tab 行为 |
|---|---|---|---|
| 双画面 | 左右或上下并排画 A / B | `纯净对比 [左右/上下]  Tab 切换左右/上下  P / Esc 退出` | 切换左右/上下 |
| 仅 A | 全窗口只画画面 A | `纯净模式 [画面 A]  P / Esc 退出` | 无反应 |
| 仅 B | 全窗口只画画面 B | `纯净模式 [画面 B]  P / Esc 退出` | 无反应 |

**退出纯净后保持进入前的显示模式**：在「仅 A」按 `P` 进入单画面纯净，再按 `P` 退出，仍然是「仅 A」，不会跳回双画面。

如果单画面纯净下对应 slot 未打开，画面中央显示「画面 A 未打开」/「画面 B 未打开」，不会黑屏无提示。

---

## 性能优化记录

本节记录从「能用」到「跟手」的几轮性能优化，目的是给后续维护者一个明确的改动意图索引。
所有数字来自开发机（Intel HD Graphics Gen11，USB UVC 摄像头，1080p/4K 输出）。

### 优化前的基线

| 操作 | 耗时 | 备注 |
|---|---|---|
| 启动（main → UI 可交互） | ~270 ms | 可以接受 |
| 首次打开摄像头 | 1.4–2.3 s | 阻塞 UI |
| 同 subtype 切换格式（如 1920→1280） | **100% 失败**，回退重启 ~2.0–5.6 s | `SetCurrentMediaType` 返回 `MF_E_INVALIDREQUEST` |
| 跨 subtype 切换（MJPEG↔YUY2↔H264） | 重启 ~2.0–2.3 s | 正常兜底 |

用户感知：点一下“打开”要等两秒，点下拉切格式要么失败要么等更久。

### 优化后（当前）

| 操作 | 耗时 | 备注 |
|---|---|---|
| 启动 | ~200–340 ms | 波动来自 eframe 初始化 |
| 首次打开（可交互时间） | **~0.5–0.8 s** | 画面 1s 内异步出现 |
| 同 subtype 热切换 | **~0.5–0.7 s** | `SetCurrentMediaType` 成功即返回（本机实测 553–711ms） |
| 跨 subtype 切换 | ~0.8–1.5 s | 走重启，符合预期 |
| H264 热切换（含 16 对齐 padding） | ~0.5–0.7 s | 不再误判失败；padding 不再出现在画面里；首帧延迟取决于 GOP |
| YUY2 跨分辨率热切换 | ~0.5–0.7 s | 画面继续刷新，解码端尺寸自动同步 |

---

### P1：首帧短等待（打开路径异步化）

**问题**：`CaptureHandle::start_with_format` 把“首帧到达”作为打开成功的必要条件，`deadline = 5s`，实际 1.0–1.2s 才到，于是 UI 线程被同步卡住 2s。

**改动**（`capture.rs`）：
- 新增 `FIRST_FRAME_WAIT_MS = 300`
- 首帧只短等 300ms；等不到也返回成功
- `latest` 保持 `None`，由 UI 线程的 `pump_texture` 自动接上
- `app.rs` 的 `paint_video` 加 `is_open` 参数：打开中无首帧时显示“画面 X 正在等待首帧…”

**收益**：打开从 2.2s 降到 0.5–0.8s 可交互，画面 1s 内异步填充，不再阻塞 UI 线程。

---

### P0：热切换正确性修复（三连修）

#### P0.1：`SetCurrentMediaType` 前必须解除流

**问题**：reader 工作在**异步回调模式**（`MF_SOURCE_READER_ASYNC_CALLBACK`），回调结尾一直 `ReadSample`。此时直接调 `SetCurrentMediaType` 会因存在挂起的异步读请求返回 `MF_E_INVALIDREQUEST`（`0xC00D36B2`），导致同 subtype 热切换 100% 失败。

**改动**（`capture.rs::mf_control_loop` 的 `SwitchFormat` 分支）：
1. `switching = true`（回调会在开头直接 return，不处理帧也不续读）
2. 等 `in_callback == 0`（处理中的回调全部退出）
3. 等 `read_pending == false`（挂起的 ReadSample 回调返回后清标志且不续读，流水线真正停住）
4. 独占 reader：`SetStreamSelection(false)` → `Flush` → `SetCurrentMediaType` → `SetStreamSelection(true)`
5. **先** `switching = false`，**再**重启 `ReadSample`

#### P0.2：`switching=false` 必须发生在 `ReadSample` 之前

**问题**：如果先 `ReadSample` 再清 `switching`，回调一触发就被 `switching` 短路 return，之后**再也没人续读**，流水线彻底断裂（日志里表现为切换后 `0x0`，再也没有新帧）。

**改动**：把 `switching.store(false)` 挪到 `ReadSample` 之前，且 `ReadSample` 带 `read_pending` 去重。

#### P0.3：切换时 drain preview 队列 + 清空 latest

**问题**：切换前 preview 队列里可能积压着十几帧旧分辨率数据（2592×1944 MJPEG 每帧约 4MB）。切换成功后解码线程要花 1–2s 消化旧帧，期间新帧全部被 `try_send` 丢弃（队列满），`latest` 长时间为空 → UI 侧看不到新画面。

**改动**：
- `SharedState` 新增 `preview_rx_for_drain: Receiver<Arc<Vec<u8>>>`（clone 共享消息）
- 切换成功后 `try_recv` 直到空，一次清掉积压
- 同时 `*shared.latest.lock() = None`，让 UI 立即显示“等待首帧”

**收益**：日志里会出现 `切换后清空 preview 队列（N 帧）`，随后新帧立即进入解码线程。

---

### P2：YUY2 跨分辨率热切换尺寸同步

**问题**：`decode_yuy2` 的 `width/height` 在 `start_with_format` 时按初始格式固定传入，热切换只更新了 `CaptureHandle.width/height` 和 `current_format_idx`，**没有通知解码线程**。于是：

- 1920x1080 → 1280x720 后，新帧 `raw.len() = 1,843,200`，`expected = 4,147,200`
- 原判据 `raw.len() < expected` 成立 → 每一帧都 `continue`
- `latest` 永远停在切换前的旧帧，UI 画面卡住
- 但 `SetCurrentMediaType` 成功、`预览端累计丢 0`，日志看起来「切换成功」，具有欺骗性

同分辨率只改帧率（如 1280x720@5 → 1280x720@10）不受影响，所以问题只在 YUY2 跨分辨率热切换时出现。MJPEG 从 JPEG 头读尺寸、H264 从解码器读尺寸，都没有这个问题。

**改动**（`capture.rs`）：
- `SharedState` 新增 `current_dims: Arc<Mutex<(u32, u32)>>`，作为「当前生效采集尺寸」的共享真值
- `CaptureHandle` 持有同一个 `Arc`，`set_format` 成功后同步更新（幂等兜底）
- `start_with_format` 在 `init_rx` 收到 `(w, h)` 后写入，再 clone 给解码线程
- `mf_control_loop` 的 `SwitchFormat` 成功分支在 **drain 之前** 写入新尺寸——解码线程从 drain 那一刻起就用新 expected 判断
- `decode_yuy2` 每帧读 `current_dims`，尺寸变化时 `resize` 缓冲并重算 `expected`
- 长度判据改为上下界：
  - `raw.len() < expected`：切换前残留的旧小尺寸帧，或异常帧，丢弃
  - `raw.len() > expected + 一行`：切换前残留的旧大尺寸帧，丢弃（否则用新 expected 截断旧大帧会得到一帧错乱画面）
  - `[expected, expected + 一行]`：正常帧，允许驱动加少量行尾 padding，截取前 `expected` 字节

**日志去重**：尺寸切换日志由控制线程统一打（控制线程只有一条，天然去重），`decode_yuy2` 内不再输出，避免 `DECODER_THREADS=2` 个解码线程各打一次。日志形如：

```
YUY2 解码尺寸切换: 1920x1080 -> 1280x720
```

位置在 `SetCurrentMediaType 成功` 之后、`格式切换成功` 之前，与 `current_dims` 的写入顺序一致。

**收益**：YUY2 跨分辨率热切换后画面继续刷新，不再冻结在切换前那一帧；`1920x1080 ↔ 640x480 ↔ 1280x720` 来回切都正常。已实测确认。

---

### P3：H264 可见尺寸与 buffer 尺寸分离（底部绿边修复）

**问题**：H264 解码器输出的 NV12 buffer 高度按 16 对齐 padding：请求 2592x1944 实际 buffer 是 2592x1952，请求 1920x1080 实际 buffer 是 1920x1088。640x480 因为 `480 % 16 == 0` 没有 padding，所以只有它正常。

P3 之前，`STREAM_CHANGE` 分支把 `display_h` 同步成 buffer 高度：

```rust
self.display_w = bw;   // 当时是为了让 Frame 尺寸和 NV12→RGB 一致
self.display_h = bh;   // 但 bh 含 padding → padding 行被当成可见图像输出
```

NV12→RGB 按 `display_h` 生成 RGB，于是底部多出的 8 行（未初始化的 UV 数据）被转换出来，表现为**画面底部一条绿边**。

当时这样改是为了让老版 `set_format` 的「等一帧 Frame 尺寸匹配目标」判据能通过。后来判据改成「`SetCurrentMediaType` 成功即成功」，这个同步的原始目的消失了，只剩副作用。

**改动**（`h264_preview.rs`）：
- `H264Decoder` 把尺寸拆成两组：
  - `visible_w/visible_h`：输出 RGB 的目标尺寸，**不含 padding**，由 `current_dims` 驱动
  - `buffer_w/buffer_h/stride`：解码器实际给的 NV12 buffer 尺寸，**只用于读取**
- `STREAM_CHANGE` 分支**只更新 `buffer_*`**，不再动 `visible_*`
- 新增 `set_visible_dims()`，`run()` 每帧前从 `current_dims` 读可见尺寸并同步；变化时打一条 `H264 可见尺寸切换` 日志
- `extract_rgb` 按 `buffer_*` + `stride` 读 NV12，但只输出前 `visible_h` 行 RGB
- `current_dimensions()` 返回 `visible_*`，`Frame.width/height` 与输出 RGB 一致
- `run()` 签名增加 `current_dims / initial_w / initial_h`，H264 与 YUY2 共用同一个尺寸真值（`capture.rs::decode_loop` 里从 `current_dims` 读初始值传入）

**日志变化**：

```
H264 输出布局: visible=2592x1944, buffer=2592x1944, stride=2592
H264 重协商布局: visible=2592x1944, buffer=2592x1952, stride=2592
```

热切到 1920x1080：

```
H264 可见尺寸切换: 2592x1944 -> 1920x1080
H264 重协商布局: visible=1920x1080, buffer=1920x1088, stride=1920
```

640x480（无 padding）：

```
H264 可见尺寸切换: 1920x1080 -> 640x480
H264 重协商布局: visible=640x480, buffer=640x480, stride=640
```

**收益**：H264 所有分辨率底部绿边消失；`visible` 始终等于用户选的分辨率，`buffer` 只在读取 NV12 时使用；热切换分辨率时 `visible` 随 `current_dims` 更新，`buffer` 随重协商更新，两者互不干扰。

**验证状态**：已在 `2592x1944`（buffer 1952）、`1920x1080`（buffer 1088）、`640x480`（buffer 480，无 padding）三种分辨率下实测确认底部绿边消失，`visible` 与 `buffer` 日志分离正确。

**已知极小竞态**：热切换瞬间，如果解码线程在控制线程更新 `current_dims` 之后、drain 之前刚好取到一帧旧分辨率的帧，`visible_*` 已是新值但该帧 SPS 是旧的，可能输出一帧错乱画面。窗口在微秒级，之后自动恢复。彻底消除需要给每帧带上 SPS 尺寸，工作量不成比例，当前不处理。

---

### H264：重协商时同步尺寸（P3 前的中间状态，供维护者理解演进）

> 这一节记录 P3 之前的改动，**已被 P3 取代**。保留是为了说明为什么代码里曾经出现 `display_h = buffer_h`，以及它如何演变成绿边问题。维护者不应再按本节实现。

**当时的改动**（`h264_preview.rs`）：
- `pump_output` 的 `STREAM_CHANGE` 分支同步 `display_w = bw; display_h = bh`
- 新增 `H264Decoder::current_dimensions()`，`run()` 里用它构造 `Frame`
- NV12→RGB 转换和 `Frame` 尺寸完全一致

**当时的目的**：老版 `set_format` 靠 `Frame` 尺寸判定切换是否成功，H264 有 padding 导致 `Frame.height` 永远对不上目标分辨率，所以把 `display_*` 同步成 `buffer_*`，让 `Frame` 尺寸至少和 NV12→RGB 一致、不越界。

**遗留的副作用**：`display_h` 含 padding，NV12→RGB 把 padding 行也输出，底部出现绿边。P3 把 `display_*` 拆成 `visible_*` 和 `buffer_*` 后才彻底解决。

---

### 切换判据重构：从“等帧”到“看 SetMediaType”

**问题**：最初 `CaptureHandle::set_format` 的判据是“等到一帧分辨率匹配目标的 `Frame` 才算成功”，但——

- 部分驱动切换后 **1.8s** 才吐第一帧（超过常见等待阈值）
- H264 **需要等 IDR** 才能解码，切换后可能几秒内没有可解码帧
- H264 解码 buffer 有 **16 对齐 padding**（例如请求 2592×1944，实际 buffer 是 2592×1952），`Frame.width/height` 永远对不上

三类情况导致热切换全部误判失败，回退重启。

**改动**（`capture.rs::set_format`）：
- 判据改为：**`SetCurrentMediaType` 成功即成功**
- 新帧异步到达，UI 侧通过 `pump_texture` 自然接上
- 切换耗时从“等帧 2.5s”降到“SetMediaType 完成即返回”的 0.5–0.7s
- `SWITCH_REPLY_TIMEOUT_MS = 3000` 只用于控制线程通信超时兜底

同时在 `mf_control_loop` 里加日志：`SetCurrentMediaType 成功，GetCurrentMediaType: WxH`，便于排错。

---

### 日志去重：格式列表只打一次

**问题**：切换格式时若跨 subtype 走重启路径会重新进 `mf_control_loop`，每次都打印 100+ 行格式列表，刷屏。

**改动**（`capture.rs`）：
- 新增 `should_log_formats(camera_index)`，进程级 `OnceLock<Mutex<HashSet<u32>>>` 去重
- 首次打开某摄像头 → 完整打印 108 种格式
- 后续切换/重启 → 只输出一行 `DEBUG 设备暴露 108 种格式（已打印过，略）`
- 程序重启后下次启动会重新打印一次

---

### 设备参数：一键恢复默认

**改动**（`app.rs::render_props_window`）：
- 参数浮窗顶部加「全部恢复默认」按钮
- 一键把所有参数的 `current` 设为 `default`，再逐个 `set_property(kind, id, default, !is_auto)` 发给控制线程
- 原 `pending: Option<...>` 改为 `Vec<...>`，一帧内多条命令不再互相覆盖（顺带修了原有一帧多控件变化的隐患）
- 成功时打 `已一键恢复 N 个参数默认值`
- 自动/手动状态保持不变（语义等同逐个点「↺」）

**语义说明**：`default` 来自 DirectShow `GetRange` 返回的出厂默认值，不是「打开那一刻的当前值」。若要恢复到打开时的完整状态（值 + 自动/手动模式），需在 `PropertyInfo` 里额外记录 `initial_current` / `initial_is_auto` 两个快照字段，当前未实现。

**验证状态**：已在设备参数浮窗中点击「全部恢复默认」实测，日志出现 `已一键恢复 11 个参数默认值`（本机 16 项候选中 11 项被驱动支持），各滑块回到默认值。

---

### 纯净模式：单画面支持

**问题**：原 `update` 里 `pure_compare` 被硬编码为只在 `DisplayMode::Dual` 下生效，且按 `P` 时强制 `display_mode = Dual`。用户想看单画面纯净时必须先切回双画面，退出后也回不到原来的单画面状态。

**改动**（`app.rs`）：
- 新增 `paint_hint(ui, text)`：在左上角画一条半透明底的提示文本，双画面/单画面纯净共用
- 新增 `render_pure_single(ui, slot, tag)`：全窗口只画一个 slot 的预览，复用 `paint_video`
- `update` 里 P 键和工具栏纯净按钮**不再强制切到 Dual**
- `Tab` 的条件加 `display_mode == Dual`，单画面纯净下无反应
- CentralPanel 的 pure 分支按 `mode` 分三种：`Dual` → `render_pure_compare`，`SingleA` / `SingleB` → `render_pure_single`
- 单画面纯净下 `paint_hint` 显示 `纯净模式 [画面 A]  P / Esc 退出`

**收益**：
- 双画面 / 仅 A / 仅 B 三种显示模式下都能直接按 `P` 进入对应纯净形态
- 退出纯净后保持进入前的 `display_mode`，不再跳回双画面
- 未打开画面的 slot 在单画面纯净下显示「画面 A 未打开」，不会黑屏无提示

**验证状态**：已在「仅 A」「仅 B」下按 `P` 实测，全窗口只画对应画面、提示文本正确、`Tab` 无反应、`Esc` 退出后仍是「仅 A」/「仅 B」。

---

## 关键设计约束（维护者须知）

1. **异步 reader 的所有格式切换必须在 `switching` 窗口内做，且窗口内绝不 `ReadSample`**。
2. **`switching=false` 必须在 `ReadSample` 之前**，否则流水线断裂，症状是“切换后 0 帧”。
3. **切换成功后必须 drain preview 队列**，否则旧帧堵塞新帧。
4. **不要用 `Frame` 尺寸做热切换判据**。H264 有 buffer padding，跨格式跨分辨率时不可靠。
5. **跨 subtype 切换（MJPEG↔YUY2↔H264）直接走重启**，不要在 `set_format` 里强行热切。
6. **YUY2 解码线程的尺寸必须每帧从 `current_dims` 读取，不能构造时固定**。否则跨分辨率热切换后新帧会被长度判据全部跳过，画面卡在切换前那一帧（见 P2）。
7. **`current_dims` 的写入必须在 drain preview 队列之前**。解码线程从新尺寸生效那一刻开始丢弃旧尺寸残留帧，顺序错了会看到一帧错乱画面。
8. **H264 的 `visible_*` 和 `buffer_*` 必须分开**。`visible_*` 由 `current_dims` 驱动、是最终输出 RGB 的尺寸；`buffer_*` 只用于读取 NV12。把两者混用会把 16 对齐 padding 输出成绿边（见 P3）。
9. **纯净模式不改变 `display_mode`**。进入纯净只是隐藏 UI，退出后必须回到进入前的显示模式。`Tab` 只在双画面纯净下生效，单画面纯净下应无反应。

---

## 已知问题

### ⚠️ H264 500 万像素 @30fps 预览会闪

#### 现象

- **MJPG 任意分辨率、任意帧率**：预览正常
- **YUY2 任意分辨率**：预览正常
- **H264 ≤ 1920x1080 @30fps**：预览正常
- **H264 2592x1944 @20fps 及以下**：预览正常
- **H264 2592x1944 @30fps**：预览画面**周期性黑一下亮一下**，录制文件不受影响

#### 根因

500 万像素 H264 在 Intel HD Graphics Gen11 上硬件解码速度约 **20fps**，采集是 **30fps**，中间差 10fps。

- 预览队列（`crossbeam::bounded(16)`）满后 `try_send` 失败，新帧被丢弃
- 丢的帧可能是 **P 帧**（前向预测帧），也可能恰好是 **IDR 关键帧**
- 丢 P 帧 → 后续 P 帧参考断裂 → 解码器输出黑帧/花屏
- 丢 IDR → 整个 GOP（约 60 帧 = 2 秒）无法解码 → 全黑，直到下一个 IDR

视频编码的参考帧结构决定这个问题**无法在消费端彻底消除**：要么解码 30fps 不丢帧（硬件跟不上），要么丢帧必黑（编码决定的）。这也是为什么 PotPlayer 能正常播放同样的码流——它是背压消费，宁可延迟累积也不丢帧。

#### 已验证无效的方案

- 加大预览队列容量（16 → 64）：黑帧频率从每秒 50 次降到每秒 9 次，但仍肉眼可见
- 队列满时优先保留 IDR：只能把「持续黑屏」变成「偶发 1~2 秒黑 + 恢复」，无法完全消除
- 把 RGB24→Color32 转换从 GUI 线程挪到解码线程：GUI 不再卡顿，但解码线程仍是 20fps
- 垃圾帧检测跳过黑帧：治标不治本，画面变成「稀疏正常帧 + 大量跳过」，一顿一顿

#### 推荐用法

**H264 500 万像素场景下，选 `2592x1944 @20fps` 或更低帧率的格式。** 采集 20fps、解码 20fps，队列永不堆积，不丢帧、不黑屏，预览和录制都是 20fps 稳定。

如果需要 30fps 录制且必须 500 万像素，接受预览会闪，或者用 **MJPG 2592x1944 @30fps** 格式——MJPG 每帧独立编码，丢帧不影响后续帧，预览和录制都是 30fps。代价是 MJPG 文件体积比 H264 大 3~5 倍（500 万像素 MJPG 约 30~50 MB/s，H264 约 8~15 MB/s）。

#### 未来可能的优化方向（未实现）

把 NV12 解码输出直接作为两张纹理（Y 平面 R8 + UV 平面 RG8）上传到 GPU，由 shader 做 YUV→RGB 转换。这样 CPU 不再处理 500 万像素的颜色转换和 20MB `Color32` 分配，解码线程可以从 47ms/帧压到 25ms/帧以下，达到 30fps。代价是 `eframe` 需要从 `glow` 后端切到 `wgpu`，并写自定义渲染管线，工作量约 2~3 天。当前版本不包含这个改动。

---

## 已知限制

- 跨 subtype 切换走重启（~0.8–1.5s），这是 MF 对 `SetCurrentMediaType` 跨类型的内部拆重建开销，不做优化。
- H264 热切换后首帧延迟取决于驱动的 GOP 周期（本机观测约 1.8s），期间 UI 显示“正在等待首帧…”，属于预期行为。
- 录制 H264 时若源端 IDR 间隔较长，首个 IDR 到达前写不了 mp4（`recorder.rs` 已有兜底，等待首个 IDR）。
- **YUY2 `1920x1080@5` 的帧间隔误报**：该格式下驱动按约 10fps 回传样本（`LLTimestamp` 增量约 94ms），而 `FRAME_GAP_WARN_TICKS = 600_000`（60ms）是固定阈值，导致每帧都打 `采集端帧间隔 94ms（真丢帧）` 警告。`预览端累计丢 0` 说明帧都被正常消费，不影响画面。后续可把阈值改为 `1.5 × 名义帧间隔`，按格式的 `fps` 动态计算，避免低帧率格式被误报。
- **AVI 录制帧率写死 30fps**（`app.rs` 里 `RecorderHandle::start(..., 30, 30, subtype)`）：YUY2/MJPG 低帧率流录成 AVI 时 `avih`/`strh` 标 30fps，实际帧间隔更大，播放会偏快。H264 走 MP4 用实时 PTS，不受影响。修法是从 `cap.available_formats[cap.current_format_idx].fps` 取实际帧率传入。
- **H264 热切换瞬间的极小竞态**：见 P3 末尾，微秒级窗口，输出一帧错乱后自动恢复，当前不处理。
- **热切换期间 UI 同步阻塞**：`set_format` 内部 `recv_timeout` 会阻塞 UI 线程 0.5–0.7s，期间窗口不可交互。后续可改为异步切换（UI 只发命令、立即返回，切换完成由 `pump_texture` 自然接上）。

## 项目结构

```
src/
  main.rs          # 入口，日志 tee 到 camcap-run.log
  app.rs           # eGUI 界面 + CameraSlot 状态 + 格式切换逻辑 + 设备参数浮窗
                   # + 双画面/单画面纯净模式
  camera.rs        # Media Foundation 设备枚举
  capture.rs       # CaptureHandle：reader 创建、异步回调、格式/参数控制、
                   # 解码线程调度、current_dims 共享尺寸（YUY2/H264 跨分辨率同步）
  h264_preview.rs  # H264 硬件解码器（IMFTranform + NV12→RGB24），
                   # visible（输出尺寸）与 buffer（含 16 对齐 padding）分离
  recorder.rs      # MJPG/YUY2 → AVI、H264 → MP4（muxide）
  photo.rs         # 快照保存
```

## 性能参考（Intel HD Graphics Gen11 + 500 万像素 UVC 摄像头）

| 场景 | 启动耗时 | 首帧就绪 | 格式切换（重启路径） |
|---|---|---|---|
| MJPG 2592x1944 @30 | 字体加载 ~200ms | ~350ms | ~400ms |
| H264 2592x1944 @20 | 字体加载 ~200ms | ~450ms（含解码器初始化） | ~500ms |

日志里带 `[TIMING]` 前缀的行记录各阶段耗时，运行一次后 `grep TIMING camcap-run.log` 查看。
同 subtype 热切换实测稳定在 553–711ms（含 `SetCurrentMediaType` + `Flush` 的驱动开销），与上表的「重启路径」不是同一路径，不要混淆。H264 的 16 对齐 padding 不再出现在画面里（P3），但热切换耗时不受影响。

## 许可

MIT