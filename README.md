# CamCap

Rust 编写的 Windows 摄像头预览 / 拍照 / 录制工具，风格参考 AMCap。
基于 Media Foundation（原生 `windows` crate 绑定），支持双摄像头同时预览、热切换格式、纯净模式、热插拔自动连接。

## 功能

- 枚举并选择摄像头（Windows 走 Media Foundation）
- **双画面并排预览**，方便同时对比两个摄像头
- 每个画面独立：选择设备 / 格式 / 帧率 / 打开 / 关闭 / 快照 / 录制
- 一键「打开全部」「关闭全部」「刷新设备」
- 格式与帧率**分开选择**：先选分辨率（约 15 项），再选该分辨率支持的帧率（约 7 项），避免 108 项长列表滚动
- 支持 **MJPG / YUY2 / H264** 三种原生格式
- 快照保存为 PNG；MJPG/YUY2 录制为 AVI，H264 录制为 MP4（用 `muxide` 直接封装，**无需 ffmpeg**）
- **AVI 录制帧率按实测回填**：驱动不严格按名义帧率吐帧时（如 MJPEG 名义 15fps 实际 30fps、YUY2 名义 10fps 实际 20fps），AVI 头写实测帧率，播放时长正确
- **热切换 UI 异步化**：切格式/帧率时 UI 不阻塞，状态栏显示「正在切换格式…」，完成后自动更新；期间可继续操作其它控件
- **配置持久化**：摄像头选择、格式偏好、显示模式、纯净布局、窗口大小自动保存到 `%APPDATA%\camcap\config.json`，下次启动恢复
- **热插拔自动连接**：启动自动打开已连接的摄像头；运行中插入自动打开；拔出显示「设备已断开」；插回自动重连。用户手动关闭的意图在 USB 物理连接变化后失效（拔插即重新自动打开）
- **同名设备区分**：两个 `FriendlyName` 相同的摄像头，ComboBox 显示 `Ingenic HD Web Camera [7&1af4834d]`，按 USB 端口级 SymbolicLink 区分
- 每个画面独立 FPS 统计、录制帧数/丢帧统计、**录制时长显示**
- **纯净模式**：双画面 / 单画面（仅 A / 仅 B）均支持。双画面下可左右/上下并排，单画面下全窗口只画一个画面；纯净模式下 `1`/`2`/`3` 快速切换画面；**左上角提示 3 秒后自动淡出**，鼠标移动或按键重新显示
- 设备参数浮窗：亮度 / 对比度 / 饱和度 / 白平衡 / 曝光 / 增益等（DirectShow `IAMVideoProcAmp` / `IAMCameraControl`），支持单参数「↺」恢复默认和**一键全部恢复默认**
- **录制中禁止切换设备 / 关闭 / 刷新**，避免 AVI 文件不完整

## 平台支持

目前 **仅 Windows**。设备枚举、格式枚举、异步采集、硬件 H264 解码全部依赖 Media Foundation。`camera.rs` 里的非 Windows 分支返回空列表，只是占位。

## 依赖

- Rust 1.75+
- Windows 10 / 11（Media Foundation 自带）
- 无需 ffmpeg

**编译注意**：`windows` crate 必须启用 `"implement"` feature（`#[implement]` 宏依赖），且必须显式依赖 `windows-core`（宏展开后引用 `::windows_core`）。`Cargo.toml` 里写：

```toml
[target.'cfg(windows)'.dependencies]
windows = { version = "0.58", features = [
    "implement",
    "Win32_Foundation",
    "Win32_Media_DirectShow",
    "Win32_Media_MediaFoundation",
    "Win32_System_Com",
] }
windows-core = "0.58"
```

## 运行

```powershell
cd D:\Rust\project\camcap
cargo run --release
```

日志同时输出到控制台和 `camcap-run.log`。输出文件默认放在 `./camcap-output/`（跟随启动时的工作目录）。

## 快捷键

| 按键 | 功能 |
|---|---|
| `P` | 进入/退出纯净模式。**在当前显示模式下生效**：双画面 → 双画面纯净；仅 A → 单画面纯净 A；仅 B → 单画面纯净 B |
| `1` / `2` / `3` | **仅纯净模式下**：切换到双画面 / 仅 A / 仅 B |
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

## 配置

配置保存在 `%APPDATA%\camcap\config.json`，当前版本 `version: 2`：

```json
{
  "version": 2,
  "slot_a": {
    "camera_index": 0,
    "camera_name": "Ingenic HD Web Camera",
    "symbolic_link": "\\\\?\\usb#vid_a108&pid_2240&mi_00#7&1af4834d&0&0000#{...}\\global",
    "preferred_format": { "subtype": 0, "width": 1920, "height": 1080, "fps": 30 }
  },
  "slot_b": {
    "camera_index": 1,
    "camera_name": "Ingenic HD Web Camera",
    "symbolic_link": "\\\\?\\usb#vid_a108&pid_2240&mi_00#7&2488dcd3&0&0000#{...}\\global",
    "preferred_format": { "subtype": 0, "width": 1920, "height": 1080, "fps": 30 }
  },
  "display_mode": 0,
  "compare_layout": 0,
  "window_size": [1280.0, 720.0]
}
```

| 字段 | 含义 |
|---|---|
| `slot_a.camera_index` | 摄像头索引（枚举顺序）。**仅作最后一层回退**，兼容 v1 配置 |
| `slot_a.camera_name` | 摄像头 `FriendlyName`（如 `Ingenic HD Web Camera`）。link 匹配失败时的第二层回退 |
| `slot_a.symbolic_link` | 摄像头 SymbolicLink（USB 端口级唯一）。**首选匹配键**，同名设备靠它区分 |
| `slot_a.preferred_format` | 上次使用的格式，点「打开」时优先用它。`subtype`：0=MJPEG, 1=YUY2, 2=H264 |
| `display_mode` | 0=双画面, 1=仅 A, 2=仅 B |
| `compare_layout` | 0=左右, 1=上下 |
| `window_size` | 窗口内尺寸（逻辑像素） |

**三层匹配回退**：加载配置时，对每个 slot 依次尝试 `symbolic_link` → `camera_name` → `camera_index`。任何一层匹配成功就用。三层都失败（设备真的不存在）则保持 `selected = 0`。

**v1 → v2 迁移**：读到 `version: 1` 的配置时，自动补 `camera_name` 和 `symbolic_link`（默认为空字符串），下一次保存写成 v2。旧配置不会丢，只是 `symbolic_link` 为空时按名字回退匹配。

**保存时机**：打开/关闭摄像头、切换格式、选设备、切显示模式、Tab 切布局后 500ms 内防抖保存；退出时强制保存一次。日志用 `trace` 级别，默认不输出，需要 `RUST_LOG=camcap=trace` 才可见。

**注意**：配置只恢复选择状态，**不会自动打开摄像头**。启动自动打开由「热插拔自动连接」机制负责，见下一节。

删除 `%APPDATA%\camcap\config.json` 即可恢复到首次运行状态。

---

## 热插拔自动连接

软件每 2 秒轮询一次设备列表，按 `SymbolicLink` 对比前后差异，处理插入/拔出事件。核心规则：

| 场景 | 行为 |
|---|---|
| 启动时已连接 1 个摄像头 | 自动打开 A |
| 启动时已连接 2 个及以上 | 自动打开 A 和 B（按枚举顺序） |
| 启动时 0 个 | 不打开 |
| 运行中插入新设备 | 自动打开到空闲 slot（A 优先） |
| 运行中拔出设备 | 显示「画面 X 设备已断开」，画面冻结在最后一帧 |
| 拔出的设备插回（同一个 USB 口） | 自动重连，日志 `设备自动重连（link 匹配）` |
| 拔出的设备插回（换了 USB 口） | 自动重连，日志 `设备自动重连（name 回退，可能换了 USB 口）` |
| 用户手动关闭 + 设备不拔 | **不自动重连**，尊重用户意图 |
| 用户手动关闭 + 设备拔插 | 拔插瞬间清除「关闭意图」，插回时自动打开 |
| 用户手动关闭的是已断开的设备 | 插回时自动打开（断开状态下的「关闭」不算「用户不想用」） |

**「用户关闭意图」的生命周期**：只在 USB 物理连接不变时有效。一旦设备被拔出（`SymbolicLink` 从列表中消失），该 slot 的 `user_closed` 标记就被清除。插回时按「新设备插入」处理，正常自动打开。

**同名设备区分**：两个 `FriendlyName` 相同的摄像头（如 `Ingenic HD Web Camera`），VID/PID 也可能相同（同型号芯片），只有 `SymbolicLink` 里的 USB 端口哈希段（`7&xxxxxxxx`）不同。ComboBox 显示 `Ingenic HD Web Camera [7&1af4834d]` 和 `Ingenic HD Web Camera [7&2488dcd3]`，用户可区分。

**已知局限**：`SymbolicLink` 反映的是 **USB 物理端口位置**，不是设备 ID。把设备从 USB 口 A 换到 USB 口 B，`SymbolicLink` 会变。此时回退到按 `FriendlyName` 匹配，如果两个同名设备都在，可能匹配到另一个，导致 A/B 分配对调或格式回退到默认。这是 Windows 层面无法解决的——同型号无唯一序列号的 UVC 设备，`SymbolicLink` 已是能做到的最细粒度。

**格式回退**：如果 `preferred_format` 指定的格式该设备不支持（如配置里存的是 500 万像素设备的 `2592x1944`，当前设备最大只支持 `1920x1080`），`mf_control_loop` 会**回退到第一个 MJPEG 格式**并打 `warn` 日志，而不是直接失败。保证用户至少能看到画面，手动切到想要的格式即可。

---

## 格式切换行为

格式下拉和帧率下拉**选中即生效**，无需额外的「应用」按钮。内部两条路径：

1. **同 subtype 热切换**（如 MJPG 1920x1080 → MJPG 1280x720）：**异步**发送 `SetCurrentMediaType` 命令，成功即视为切换完成，新帧异步到达。UI 不阻塞，状态栏显示「正在切换格式…」，格式/帧率下拉在切换期间禁用。完成后通过 `poll_switch` 通知 UI。
2. **跨 subtype 或热切换失败**：关闭当前 reader，重新走 `CaptureHandle::start_with_format` 打开目标格式。耗时约 1.2–1.4s（含 reader Release + 重建 + 首帧就绪）。**跨 subtype 走同步重启**，因为要销毁并重建 reader，无法在现有 reader 上异步完成。

录制中格式下拉被禁用，避免录制文件分辨率跳变。录制中设备下拉、关闭按钮、刷新设备按钮也被禁用，避免 AVI 文件不完整。

---

## 纯净模式

纯净模式隐藏所有 UI，只保留画面本身，适合对比画面、录制截图、演示。

按 `P` 进入/退出。**进入时不会改变当前显示模式**：

| 进入纯净前的显示模式 | 纯净模式表现 | 左上角提示 | 快捷键行为 |
|---|---|---|---|
| 双画面 | 左右或上下并排画 A / B | `纯净对比 [左右/上下]  Tab 切换左右/上下  1/2/3 切换画面  P / Esc 退出` | `Tab` 切布局；`1/2/3` 切画面 |
| 仅 A | 全窗口只画画面 A | `纯净模式 [画面 A]  1/2/3 切换画面  P / Esc 退出` | `1/2/3` 切画面；`Tab` 无反应 |
| 仅 B | 全窗口只画画面 B | `纯净模式 [画面 B]  1/2/3 切换画面  P / Esc 退出` | `1/2/3` 切画面；`Tab` 无反应 |

**左上角提示延迟淡出**：进入纯净模式时提示立即显示，**3 秒后开始淡出，0.5 秒内消失**。淡出后画面完全无遮挡，适合录屏/截图。

| 时机 | 提示行为 |
|---|---|
| 按 `P` 进入纯净 | 立即显示，3 秒后淡出 |
| 移动鼠标 | 重新显示，3 秒后淡出 |
| 按 `Tab` / `1` / `2` / `3` / `F11` | 重新显示，3 秒后淡出 |
| 按 `P` / `Esc` 退出 | 提示立即消失，下次进入重新计时 |
| 鼠标持续移动 | 每次移动都重置 3 秒计时，提示一直显示 |

**退出纯净后保持进入前的显示模式**：在「仅 A」按 `P` 进入单画面纯净，再按 `P` 退出，仍然是「仅 A」，不会跳回双画面。

如果单画面纯净下对应 slot 未打开，画面中央显示「画面 A 未打开」/「画面 B 未打开」，不会黑屏无提示。

---

## 性能优化记录

本节记录从「能用」到「跟手」的几轮性能优化，目的是给后续维护者一个明确的改动意图索引。
所有数字来自开发机（Intel HD Graphics Gen11，USB UVC 摄像头，1080p 输出）。

### 优化前的基线

| 操作 | 耗时 | 备注 |
|---|---|---|
| 启动（main → UI 可交互） | ~270 ms | 可以接受 |
| 首次打开摄像头 | 1.4–2.3 s | 阻塞 UI |
| 同 subtype 切换格式（如 1920→1280） | **100% 失败**，回退重启 ~2.0–5.6 s | `SetCurrentMediaType` 返回 `MF_E_INVALIDREQUEST` |
| 跨 subtype 切换（MJPEG↔YUY2↔H264） | 重启 ~2.0–2.3 s | 正常兜底 |
| AVI 录制低帧率流 | 播放时长错误（快 2–6 倍） | 帧率写死 30fps |
| 热切换 UI 响应 | **冻结 0.5–0.8s** | 同步 `recv_timeout` 阻塞 UI 线程 |
| 每次启动 | 选择/格式全部重置 | 无持久化 |
| 纯净模式提示 | 一直显示，遮挡画面 | 无延迟淡出 |
| 启动自动打开（2 摄像头） | **~1s 阻塞 UI** | 串行同步打开 |

用户感知：点一下“打开”要等两秒，点下拉切格式要么失败要么等更久，录出来的低帧率视频播放飞快，切格式时窗口卡住，每次启动都要重新选设备，纯净模式左上角文字一直挡着。

### 优化后（当前）

| 操作 | 耗时 | 备注 |
|---|---|---|
| 启动 | ~185–360 ms | 波动来自 eframe 初始化 |
| 首次打开（可交互时间） | **~0.5–0.8 s** | 画面 1s 内异步出现 |
| 同 subtype 热切换 | **~0.56–0.70 s（异步）** | UI 不阻塞，可继续操作 |
| 跨 subtype 切换 | **~1.17–1.42 s** | 走重启；平均 1.28s |
| H264 热切换（含 16 对齐 padding） | ~0.56–0.70 s（异步） | padding 不再出现在画面里 |
| YUY2 跨分辨率热切换 | ~0.57–0.70 s（异步） | 画面继续刷新，解码端尺寸自动同步 |
| AVI 录制任意帧率 | 播放时长 ≈ 实际录制时长 | 实测帧率回填，误差 < 5% |
| 配置加载 | < 1 ms | 启动时读取一次 |
| 热插拔检测间隔 | 2 s | 轮询开销 < 50ms，不阻塞 UI |
| 拔插后自动重连 | ~0.45–0.55 s | 含 reader 创建 + SetMediaType + 首帧等待 |
| 纯净模式提示 | 3 s 后淡出 | 画面完全无遮挡 |
| 启动自动打开（2 摄像头） | **~15ms 启动 + 后台 1s 填充** | UI 先出现，画面异步 |

---

### P1：首帧短等待（打开路径异步化）

**问题**：`CaptureHandle::start_with_format` 把“首帧到达”作为打开成功的必要条件，`deadline = 5s`，实际 1.0–1.2s 才到，于是 UI 线程被同步卡住 2s。

**改动**（`capture.rs`）：
- 新增 `FIRST_FRAME_WAIT_MS`
- 首帧只短等；等不到也返回成功
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

**问题**：切换前 preview 队列里可能积压着十几帧旧分辨率数据。切换成功后解码线程要花 1–2s 消化旧帧，期间新帧全部被 `try_send` 丢弃（队列满），`latest` 长时间为空 → UI 侧看不到新画面。

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

**改动**（`capture.rs`）：
- `SharedState` 新增 `current_dims: Arc<Mutex<(u32, u32)>>`，作为「当前生效采集尺寸」的共享真值
- `CaptureHandle` 持有同一个 `Arc`，切换成功后同步更新
- `start_with_format` 在 `init_rx` 收到 `(w, h)` 后写入，再 clone 给解码线程
- `mf_control_loop` 的 `SwitchFormat` 成功分支在 **drain 之前** 写入新尺寸
- `decode_yuy2` 每帧读 `current_dims`，尺寸变化时 `resize` 缓冲并重算 `expected`
- 长度判据改为上下界：
  - `raw.len() < expected`：切换前残留的旧小尺寸帧，丢弃
  - `raw.len() > expected + 一行`：切换前残留的旧大尺寸帧，丢弃
  - `[expected, expected + 一行]`：正常帧，允许驱动加少量行尾 padding

**日志去重**：尺寸切换日志由控制线程统一打（控制线程只有一条，天然去重），`decode_yuy2` 内不再输出。日志形如：

```
YUY2 解码尺寸切换: 1920x1080 -> 1280x720
```

**收益**：YUY2 跨分辨率热切换后画面继续刷新，不再冻结在切换前那一帧。

---

### P3：H264 可见尺寸与 buffer 尺寸分离（底部绿边修复）

**问题**：H264 解码器输出的 NV12 buffer 高度按 16 对齐 padding：请求 1920x1080 实际 buffer 是 1920x1088，请求 2592x1944 实际 buffer 是 2592x1952。640x480 因为 `480 % 16 == 0` 没有 padding。

P3 之前，`STREAM_CHANGE` 分支把 `display_h` 同步成 buffer 高度，NV12→RGB 按 `display_h` 生成 RGB，底部多出的 8 行（未初始化的 UV 数据）被转换出来，表现为**画面底部一条绿边**。

**改动**（`h264_preview.rs`）：
- `H264Decoder` 把尺寸拆成两组：
  - `visible_w/visible_h`：输出 RGB 的目标尺寸，**不含 padding**，由 `current_dims` 驱动
  - `buffer_w/buffer_h/stride`：解码器实际给的 NV12 buffer 尺寸，**只用于读取**
- `STREAM_CHANGE` 分支**只更新 `buffer_*`**，不再动 `visible_*`
- 新增 `set_visible_dims()`，`run()` 每帧前从 `current_dims` 读可见尺寸并同步
- `extract_rgb` 按 `buffer_*` + `stride` 读 NV12，但只输出前 `visible_h` 行 RGB
- `run()` 签名增加 `current_dims / initial_w / initial_h`

**日志变化**：

```
H264 输出布局: visible=1920x1080, buffer=1920x1080, stride=1920
H264 重协商布局: visible=1920x1080, buffer=1920x1088, stride=1920
```

**收益**：H264 所有分辨率底部绿边消失。

**已知极小竞态**：热切换瞬间，如果解码线程在控制线程更新 `current_dims` 之后、drain 之前刚好取到一帧旧分辨率的帧，`visible_*` 已是新值但该帧 SPS 是旧的，可能输出一帧错乱画面。窗口在微秒级，之后自动恢复。

---

### P4：AVI 录制实测帧率回填

**问题**：`app.rs::toggle_recording` 里调用 `RecorderHandle::start(..., 30, 30, subtype)`，第 4 个参数（名义帧率）**写死 30fps**。而驱动的实际输出帧率和名义设置经常不一致：

| 格式 | 名义帧率 | 实际输出 |
|---|---|---|
| MJPEG | 15 / 20 / 30 | **始终 ~30fps**（sensor 走硬件编码通道） |
| YUY2 | 10 | **~20fps**（驱动按名义的 2 倍吐帧） |
| YUY2 | 30 | ~29.8fps |

AVI 头 `avih.dwMicroSecPerFrame` 和 `strh.dwScale/dwRate` 都按名义帧率写，播放器按此速率播放，导致：

- YUY2 名义 10fps 实际 20fps：录 22 秒得 428 帧，标 10fps，播放 **42.8 秒**（慢一倍）
- MJPEG 名义 15fps 实际 30fps：录 12 秒得 357 帧，标 15fps，播放 **23.8 秒**（慢一倍）

**改动**（`recorder.rs`）：

1. `AviWriter` 新增 `first_ts: Option<Instant>`、`last_ts: Option<Instant>`，`write_frame` 每帧记录时间戳。
2. `finish()` 返回 `Result<(u32, f32)>`（帧数 + 实测帧率），用 `(last_ts - first_ts)` 算实测帧率：
   ```rust
   let intervals = (frame_count - 1) as u64;
   let dw_rate = (intervals * 1_000_000_000 / elapsed_us) as u32;
   let dw_scale = 1000u32;  // 保留 3 位小数
   ```
3. 回填三个字段：
   - `avih.dwMicroSecPerFrame`（offset 32）
   - `strh.dwScale` / `strh.dwRate`（offset 128 / 132）
   - `avih.dwMaxBytesPerSec`（offset 36）
4. `RecorderHandle::start` 的 AVI 分支更新日志。

**日志变化**：

```
YUY2 直通录制已就绪: 1280x720 @ 10fps, 输出 ...
AVI 完成: 428 帧, 实测 20.23fps (dwScale=1000, dwRate=20232)
录制已保存: ... (428 帧, 20.23fps)
```

**验证数据**：

| 格式 | 名义 | 录制时长 | 帧数 | 实测 | 播放时长 | 误差 |
|---|---|---|---|---|---|---|
| YUY2 1280x720 | 10fps | 22s | 428 | 20.23fps | 21.2s | 4% |
| YUY2 640x480 | 30fps | 19s | 562 | 29.76fps | 18.9s | <1% |
| MJPEG 1920x1080 | 15fps | 12s | 357 | 29.76fps | 12.0s | 0% |

---

### P5：热切换 UI 异步化

**问题**：`app.rs::apply_format` 直接调 `slot.capture.set_format()`，`set_format` 内部 `rx.recv_timeout(3000ms)` 同步阻塞 UI 线程。虽然实际耗时只有 0.5–0.8s，但 egui 是单线程绘制，这期间鼠标悬停、按钮点击、窗口重绘全部停摆。

**改动**：

`capture.rs`：
- `CaptureHandle` 新增 `pending_switch: Mutex<Option<PendingSwitchState>>`
- 新增 `request_format(index)`：只发送 `ControlCommand::SwitchFormat`，**不 recv**，立即返回
- 新增 `poll_switch()`：`try_recv` 检查 reply channel，返回 `SwitchOutcome::Success(fmt)` / `Failed(e)` / `None`（进行中）
- 新增 `is_switching()`：UI 用于禁用格式/帧率下拉

`app.rs`：
- `apply_format` 同 subtype 分支改为 `request_format` + 立即返回，状态栏显示「正在切换格式…」
- `CameraSlot` 新增 `poll_switch()`：调 `cap.poll_switch()`，成功时更新 `preferred_format`，失败时回滚 `group_idx/fps_idx`
- `update` 开头轮询两个 slot 的 `poll_switch()`
- 格式/帧率下拉的 `add_enabled_ui` 条件加 `!is_switching`

**收益**：

| 维度 | 同步（之前） | 异步（现在） |
|---|---|---|
| UI 阻塞 | 0.5–0.8s 完全冻结 | 0，可继续操作 |
| 切换期间能做什么 | 什么都不能做 | 打开/关闭/参数/纯净模式/切换显示模式 |

---

### P6：跨 subtype 重启优化（P6.1 / P6.2 / P6.4）

**问题**：跨 subtype 切换走重启路径（`close()` + `start_with_format`），实测 1.36–1.70s。加分段计时后拆解：

```
open_with_preferred 1410ms
  = Drop 760ms (54%)
      ├─ reader Release   555ms (39%)  ← 最大头
      ├─ MFShutdown       204ms (14%)
      └─ 解码 join        <1ms
  + start_with_format 649ms (46%)
      ├─ CaptureHandle 初始化  341ms
      └─ 首帧等待              308ms  ← 纯浪费
```

**P6.1：首帧等待 300ms → 100ms**

**改动**（`capture.rs`）：

```rust
const FIRST_FRAME_WAIT_MS: u64 = 100;
```

首帧由 `pump_texture` 异步接上，UI 不阻塞。日志显示实际等待稳定在 106–108ms，每次省 ~200ms。

**P6.2：`MFShutdown` 延后到进程退出**

**问题**：每次 `mf_control_loop` 退出都调 `MFShutdown`，跨 subtype 切换时重复 Startup/Shutdown 每次浪费 200ms。

**改动**：

新建 `src/mf.rs`：

```rust
static MF_INIT: OnceLock<()> = OnceLock::new();

pub fn ensure_startup() -> Result<()> {
    if MF_INIT.get().is_none() {
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL)?; }
        let _ = MF_INIT.set(());
    }
    Ok(())
}

pub fn shutdown() {
    if MF_INIT.get().is_some() {
        unsafe { let _ = MFShutdown(); }
    }
}
```

- `camera.rs::list_cameras` 和 `capture.rs::mf_control_loop` 开头调 `mf::ensure_startup()`
- `main.rs` 的 `run_native` 返回后调一次 `mf::shutdown()`

**收益**：`Drop: control thread join` 从 750ms 降到 530–790ms。

**P6.4：格式列表缓存**

**问题**：每次 `start_with_format` 都调 `collect_formats`，遍历 94 种格式，每次 30–80ms。同一摄像头格式列表不变。

**改动**（`capture.rs`）：

```rust
// IMFMediaType 底层是 IUnknown(NonNull<c_void>)，windows-rs 默认不实现 Send/Sync。
// 但 mf_control_loop 里 CoInitializeEx(COINIT_MULTITHREADED) 是 MTA，
// COM 对象在 MTA 下可跨线程 AddRef/Release，这是 COM 规范保证的。
struct SendMediaType(IMFMediaType);
unsafe impl Send for SendMediaType {}
unsafe impl Sync for SendMediaType {}

static FORMAT_CACHE: OnceLock<Mutex<HashMap<u32, (Vec<SendMediaType>, Vec<VideoFormat>)>>> =
    OnceLock::new();
```

`mf_control_loop` 里枚举前先查缓存，命中则跳过 `collect_formats`。

**收益**：日志出现 `设备暴露 94 种格式（从缓存读取，跳过枚举）`。实际收益被不同格式间 `SetCurrentMediaType` 的耗时差异掩盖，但在同一格式多次切换时有 30–80ms 收益。

**P6.3（未做）：reader Release 后台化**

**问题**：`Drop: control thread join` 里 **530–790ms 是 `IMFSourceReader` 的 COM Release**（停流 → 释放 buffer → 通知驱动），是当前最大单点。

**潜在改法**：控制线程退出前把 reader 取出来丢到后台线程 drop，`Drop: control thread join` 从 530ms 降到 <5ms，跨 subtype 总耗时从 1.28s 降到 0.6–0.9s。

**为什么不做**：

1. 后台 Release 和新 reader 创建可能竞争驱动资源。
2. 进程退出时后台线程可能没跑完，理论上系统兜底，但极端情况可能留未释放资源。
3. 连续快速切换时可能同时有 2–3 个 reader 在后台 Release。

当前 1.28s 平均可接受，不冒这个险。

---

### P7：热插拔自动连接 + SymbolicLink 设备匹配

**问题**：两个核心缺陷：

1. **启动不自动打开**：每次启动都要手动点「打开」。
2. **同名设备无法区分**：两个 `Ingenic HD Web Camera`，VID/PID 都是 `a108:2240`，`FriendlyName` 相同，ComboBox 里两行一模一样，配置按索引存导致拔插后错位。

**探测确认**：新增 `examples/probe_devices.rs`，读取每个设备的 `FRIENDLY_NAME` 和 `SYMBOLIC_LINK`。结果：

```
[0] FriendlyName : Ingenic HD Web Camera
    SymbolicLink : \\?\usb#vid_a108&pid_2240&mi_00#7&1af4834d&0&0000#{...}
[1] FriendlyName : Ingenic HD Web Camera
    SymbolicLink : \\?\usb#vid_a108&pid_2240&mi_00#7&2488dcd3&0&0000#{...}
```

**VID/PID 相同，只有 SymbolicLink 里的 `7&xxxxxxxx` 段不同**。这是 USB 拓扑路径哈希，按物理端口唯一。确认可以用 `SymbolicLink` 区分同名设备。

**改动**：

`camera.rs`：
- `CameraInfo` 加 `symbolic_link: String`
- `list_cameras` 读 `MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK`

`config.rs`：
- `SlotConfig` 加 `camera_name` / `symbolic_link`（`serde(default)`）
- `AppConfig::default().version = 2`
- `load()` 里 v1→v2 打日志迁移

`capture.rs`：
- `preferred` 格式匹配失败时**回退到第一个 MJPEG**，不再 `return Err`
- 之前的行为是「启动自动打开 B 失败: 设备不支持目标格式 MJPEG 2592x1944 @ 30fps」，现在会回退到 1920x1080 并打 warn

`app.rs`：
- 新增 `CameraEntry { index, name, link }`
- `CameraSlot` 加 `connected_link: Option<String>`
- `known_device_names` → `known_device_links`，热插拔按 link 对比
- `poll_hotplug`：link 优先匹配重连，name 回退
- ComboBox 显示 `name [7&xxxxxxxx]`
- 启动自动打开：按设备数分配 A/B
- 用户关闭时 `user_closed = true`，拔插事件清除
- `short_link` 提取 `7&xxxxxxxx` 段

**四层匹配策略**：

1. `symbolic_link` 匹配 → 命中就用
2. `camera_name` 匹配 → 命中就用
3. `camera_index` 匹配 → 兼容 v1 配置
4. `preferred_format` 不支持 → 回退到第一个 MJPEG

**收益**：

- 启动自动打开，用户不用手动点
- 运行中插入/拔出自动处理
- 同名设备 ComboBox 可区分，配置按设备而非 slot 存
- 用户关闭意图在 USB 物理连接不变时有效，拔插后失效
- `preferred_format` 不支持时回退默认，不再打开失败

**验证数据**：

| 场景 | 日志证据 |
|---|---|
| 启动自动打开 | `启动自动打开 A` + `启动自动打开 B` |
| 格式不支持回退 | `设备不支持目标格式 MJPEG 2592x1944 @ 30fps，回退到默认: MJPEG 1920x1080 @ 30fps` |
| 拔插按 link 匹配重连 | `设备自动重连（link 匹配）` |
| 换口按 name 回退重连 | `设备自动重连（name 回退，可能换了 USB 口）` |
| 用户关闭的设备被拔出 | `用户关闭的设备被拔出，清除关闭意图: 7&1af4834d` |
| 插回自动打开 | `新设备自动打开` |

---

### P8：纯净模式提示延迟淡出

**问题**：纯净模式下左上角的快捷键提示一直显示，遮挡画面。录屏、截图、演示时很碍事。

**改动**（`app.rs`）：

新增常量：

```rust
const PURE_HINT_DURATION: Duration = Duration::from_secs(3);
const PURE_HINT_FADE_DURATION: f32 = 0.5;
```

`CamCapApp` 新增字段：

```rust
/// 纯净模式提示的显示截止时刻。`Some(t)` 且 `Instant::now() < t` 时显示。
/// 最后 `PURE_HINT_FADE_DURATION` 秒用透明度渐变淡出。
pure_hint_until: Option<Instant>,
```

新增方法：

```rust
fn toggle_pure_compare(&mut self) {
    self.pure_compare = !self.pure_compare;
    if self.pure_compare {
        self.pure_hint_until = Some(Instant::now() + PURE_HINT_DURATION);
    } else {
        self.pure_hint_until = None;
    }
}

fn pure_hint_alpha(&self) -> f32 {
    let Some(t) = self.pure_hint_until else { return 0.0; };
    let now = Instant::now();
    if now >= t { return 0.0; }
    let remaining = t.duration_since(now).as_secs_f32();
    if remaining < PURE_HINT_FADE_DURATION {
        remaining / PURE_HINT_FADE_DURATION
    } else {
        1.0
    }
}
```

`update` 里检测交互：

```rust
if self.pure_compare {
    let had_interaction = ctx.input(|i| {
        let pointer_moved = i.pointer.delta() != egui::Vec2::ZERO;
        let any_key_pressed = i.events.iter().any(|e| {
            matches!(e, egui::Event::Key { pressed: true, .. })
        });
        pointer_moved || any_key_pressed
    });
    if had_interaction {
        self.pure_hint_until = Some(Instant::now() + PURE_HINT_DURATION);
    }
}
```

`paint_hint` 加 `alpha: f32` 参数，文字和背景透明度都按 alpha 缩放；`alpha <= 0.0` 直接 return。

纯净模式渲染时：

```rust
let hint_alpha = self.pure_hint_alpha();
// ...
if hint_alpha > 0.0 {
    paint_hint(ui, &hint, hint_alpha);
}
```

**刷新节奏**：纯净模式且提示淡出中时，`ctx.request_repaint_after(Duration::from_millis(50))`，保证淡出动画 20fps 流畅。

**收益**：

- 进入纯净模式 3 秒后提示自动淡出，画面完全无遮挡
- 移动鼠标或按键时提示重新出现，3 秒后再淡出
- 退出纯净模式时状态清空，下次进入重新计时

---

### P9：启动自动打开异步化

**问题**：P7 加的启动自动打开在 `CamCapApp::new` 里同步执行，两个摄像头串行打开，总耗时 ~1s 全部阻塞在 UI 线程。用户双击 exe 后要等 1 秒多才看到窗口，体感像卡住。

**日志证据**：
[TIMING] CamCapApp::new: 1007ms
[TIMING] 启动总耗时（main → UI 就绪）: 1161ms
启动自动打开 A: 572ms
启动自动打开 B: 420ms


**改动**（`app.rs`）：

- `CamCapApp` 新增 `pending_auto_open` / `auto_open_rx` / `auto_open_tx` / `auto_open_in_progress`
- `CameraSlot` 拆出 `install_capture(entry, c)`，从 `open_with_preferred` 的 `Ok` 分支抽出来，供异步路径复用
- `CamCapApp::new` **不再打开摄像头**，只记录待打开列表 + 创建 channel
- `update` 第一帧启动后台线程，串行打开 A/B，每个结果通过 `auto_open_rx.send()` 回传；全部完成后发 sentinel `None`
- `update` 每帧 `try_recv` 收集结果，`Ok(cap)` 调 `slot.install_capture()`，`Err` 打 warn
- 用户在打开完成前手动操作（关闭/重新打开）该 slot 时，`user_took_over` 检测丢弃后台结果，`CaptureHandle::Drop` 清理
- `request_repaint_after` 在 `auto_open_in_progress` 期间 50ms 刷一次，尽快发现结果

**收益**：

| 指标 | 修复前 | 修复后 |
|---|---|---|
| `CamCapApp::new` | ~1s | **~15ms** |
| `启动总耗时` | ~1.1s | **~150–290ms** |
| A 画面出现 | ~1.1s | ~500ms |
| B 画面出现 | ~1.1s | ~900ms |
| 启动期间 UI 可操作 | 否 | **是** |

**边界情况**：

| 场景 | 行为 |
|---|---|
| 启动后用户立刻点「关闭全部」 | `close_both` 设 `user_closed = true`，后台结果被丢弃 |
| 启动后用户手动打开某 slot | `capture.is_some()` 为 true，后台结果被丢弃 |
| 窗口关闭时后台线程还在跑 | `tx.send` 返回 Err，后台线程立即退出 |
| 0 个设备 | 不启动后台线程 |
| 1 个设备 | 只打开 A |



### 设备断开提示

**问题**：`OnReadSample` 里 `if hrstatus.is_err() { return Ok(()); }`，错误被静默吞掉。摄像头拔掉，画面冻结在最后一帧，UI 无任何提示。

**改动**：

`capture.rs`：
- `SharedState` 加 `error_flag: Arc<AtomicBool>`
- `OnReadSample` 开头 `hrstatus.is_err()` 时置位并打 `log::warn!("采集错误 0x{:08X}，设备可能已断开")`，直接 return 不续读
- `CaptureHandle::has_error()` 返回标志
- `CaptureHandle::stop()` 时清零

`app.rs`：
- `CameraSlot::has_error()` 转发
- 状态栏 `TopBottomPanel::bottom("status")` 里优先显示红色 `画面 A 设备已断开` / `画面 A / B 设备已断开`

**收益**：用户拔线后左下角状态栏立即显示断开提示，知道发生了什么。

**局限**：只处理 `hrstatus.is_err()`。有些驱动拔线后不报错、只停回调，这种情况检测不到。可加超时检测（`last_ts` 超过 N 秒没更新判定断开），但低帧率格式误报率高，当前不做。

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
- H264 解码 buffer 有 **16 对齐 padding**，`Frame.width/height` 永远对不上

三类情况导致热切换全部误判失败，回退重启。

**改动**（`capture.rs`）：
- 判据改为：**`SetCurrentMediaType` 成功即成功**
- 新帧异步到达，UI 侧通过 `pump_texture` 自然接上
- 切换耗时从“等帧 2.5s”降到“SetMediaType 完成即返回”的 0.5–0.7s

---

### 日志去重：格式列表只打一次

**问题**：切换格式时若跨 subtype 走重启路径会重新进 `mf_control_loop`，每次都打印 100+ 行格式列表，刷屏。

**改动**（`capture.rs`）：
- 新增 `should_log_formats(camera_index)`，进程级 `OnceLock<Mutex<HashSet<u32>>>` 去重
- 首次打开某摄像头 → 完整打印 N 种格式
- 后续切换/重启 → 只输出一行 `DEBUG 设备暴露 N 种格式（已打印过，略）`

---

### 设备参数：一键恢复默认

**改动**（`app.rs::render_props_window`）：
- 参数浮窗顶部加「全部恢复默认」按钮
- 一键把所有参数的 `current` 设为 `default`，再逐个 `set_property(kind, id, default, !is_auto)` 发给控制线程
- 原 `pending: Option<...>` 改为 `Vec<...>`，一帧内多条命令不再互相覆盖
- 成功时打 `已一键恢复 N 个参数默认值`

**语义说明**：`default` 来自 DirectShow `GetRange` 返回的出厂默认值，不是「打开那一刻的当前值」。

---

### 纯净模式：单画面支持 + 1/2/3 快速切换

**问题**：原 `update` 里 `pure_compare` 被硬编码为只在 `DisplayMode::Dual` 下生效，且按 `P` 时强制 `display_mode = Dual`。

**改动**（`app.rs`）：
- 新增 `render_pure_single(ui, slot, tag)`：全窗口只画一个 slot 的预览
- `update` 里 P 键和工具栏纯净按钮**不再强制切到 Dual**
- `Tab` 的条件加 `display_mode == Dual`，单画面纯净下无反应
- 纯净模式下新增 `1` / `2` / `3` 快捷键，分别切 `Dual` / `SingleA` / `SingleB`
- CentralPanel 的 pure 分支按 `mode` 分三种

**收益**：
- 双画面 / 仅 A / 仅 B 三种显示模式下都能直接按 `P` 进入对应纯净形态
- 纯净模式下可直接用 `1/2/3` 切换画面，不需要先退出
- 退出纯净后保持进入前的 `display_mode`

---

### 录制中禁用危险操作 + 录制时长显示

**问题**：录制中点「关闭」「刷新设备」或切设备下拉，`close()` 会 `stop()` + `join` recorder 线程，AVI 的 `finish()` 可能正在回填字节，极端情况下 AVI 文件不完整（缺 `idx1`）。

**改动**（`app.rs`）：
- 设备下拉、关闭按钮、工具栏「刷新设备」在录制中禁用，悬停提示原因
- `CameraSlot` 新增 `recording_started: Option<Instant>`，录制中第二行右侧显示 `● REC 00:12  写 N / 丢 M`
- 超过 1 小时自动变 `hh:mm:ss`

---

## 关键设计约束（维护者须知）

1. **异步 reader 的所有格式切换必须在 `switching` 窗口内做，且窗口内绝不 `ReadSample`**。
2. **`switching=false` 必须在 `ReadSample` 之前**，否则流水线断裂，症状是“切换后 0 帧”。
3. **切换成功后必须 drain preview 队列**，否则旧帧堵塞新帧。
4. **不要用 `Frame` 尺寸做热切换判据**。H264 有 buffer padding，跨格式跨分辨率时不可靠。
5. **跨 subtype 切换（MJPEG↔YUY2↔H264）直接走重启**，不要在 `request_format` / `set_format` 里强行热切。
6. **YUY2 解码线程的尺寸必须每帧从 `current_dims` 读取，不能构造时固定**。否则跨分辨率热切换后新帧会被长度判据全部跳过，画面卡在切换前那一帧（见 P2）。
7. **`current_dims` 的写入必须在 drain preview 队列之前**。解码线程从新尺寸生效那一刻开始丢弃旧尺寸残留帧，顺序错了会看到一帧错乱画面。
8. **H264 的 `visible_*` 和 `buffer_*` 必须分开**。`visible_*` 由 `current_dims` 驱动、是最终输出 RGB 的尺寸；`buffer_*` 只用于读取 NV12。把两者混用会把 16 对齐 padding 输出成绿边（见 P3）。
9. **纯净模式不改变 `display_mode`**。进入纯净只是隐藏 UI，退出后必须回到进入前的显示模式。
10. **AVI 录制帧率必须按实测回填，不能用名义帧率**。驱动实际输出和名义设置经常不一致（见 P4）。
11. **异步切换期间 `reconcile` 必须跳过检查**。`pending_switch == true` 时 `current_format_idx` 尚未更新是正常的，此时回滚 `group_idx/fps_idx` 会导致切换完成后 UI 显示旧格式。
12. **配置保存必须防抖**。`update` 每帧都可能触发 dirty，但只在距上次保存 > 500ms 时才真正写文件。日志用 `trace` 级别，避免刷屏。
13. **`MFStartup` / `MFShutdown` 进程级配对，不在每次 reader 关闭时调**。`camera.rs` / `capture.rs` 开头调 `mf::ensure_startup()`（只首次真正 Startup），`main.rs` 进程退出时调 `mf::shutdown()`（见 P6.2）。
14. **`FORMAT_CACHE` 的 `SendMediaType` 依赖 MTA**。`mf_control_loop` 里 `CoInitializeEx(COINIT_MULTITHREADED)` 是 MTA，COM 对象可跨线程。如果未来改成 STA，必须去掉 `SendMediaType` 的 `unsafe impl Send + Sync`（见 P6.4）。
15. **`SymbolicLink` 是设备匹配的首选键，不是打开设备的参数**。打开设备仍然用 `index`（`MFEnumDeviceSources` 返回顺序）。`symbolic_link` 只用于配置持久化和热插拔检测（见 P7）。
16. **`user_closed` 的生命周期绑定 USB 物理连接**。设备拔插（`SymbolicLink` 从列表消失）时清除 `user_closed`，插回时按「新设备插入」处理。不要改成持久化到配置，否则用户换设备时会出现「明明插上了却不自动打开」的困惑。
17. **热插拔轮询按 `SymbolicLink` 对比，不按 `FriendlyName`**。同名设备的 `FriendlyName` 相同，按名字对比会漏检/误判（见 P7）。
18. **`preferred_format` 匹配失败必须回退到默认格式，不能直接失败**。配置里的格式可能是同型号另一台设备的，换设备后不匹配。回退到第一个 MJPEG 保证用户至少能看到画面（见 P7）。
19. **纯净模式提示必须延迟淡出**。进入纯净后提示 3 秒自动淡出，鼠标移动/按键重新显示。不要改成一直显示（挡画面）或完全移除（用户忘记快捷键）。淡出期间 `request_repaint_after(50ms)` 保证动画流畅（见 P8）。
20. **启动自动打开必须在后台线程做，不能阻塞 `CamCapApp::new`**。两个摄像头串行打开 ~1s，全部阻塞会让 UI 推迟 1 秒才出现。异步化后 UI ~150ms 出现，画面异步填充。后台结果和用户手动操作冲突时，用户优先（`user_took_over` 检测）。

---

## 硬件 / 固件注意事项

### UVC 配置里的尺寸描述符错误会导致格式虚报

某些 UVC 摄像头（尤其是白牌/定制固件）的 `VS_FORMAT_*` 描述符里 `wWidth/wHeight` 字段可能错位（大小端搞反、offset 偏移 2 字节等）。Windows UVC 驱动解析这些描述符后，`GetNativeMediaType` 会返回**明显不可能的分辨率**，例如：

```
[0] MJPEG 9284x19120 @ 30fps    ← 1.77 亿像素，USB 带宽差 8 倍
[0] MJPEG 5188x1504 @ 30fps     ← 3.45:1 比例，不常见
```

后果：

1. 格式下拉里出现一堆无意义的分辨率，用户困惑。
2. 选中某些虚报格式后，`SetCurrentMediaType` 可能返回 `ERROR_ARITHMETIC_OVERFLOW`（`0x80070216`）。
3. 极端情况下 `MFCreateSourceReaderFromMediaSource` 直接失败。

**这是固件问题，不是代码问题。** 正确的修复方式是用厂商工具重新烧录正确的 UVC 配置。修复后格式列表从 101 种降到 94 种（或对应数量），最大分辨率等于 sensor 上限。

### 同名设备 + 同 VID/PID 是常见现象

批量采购的 UVC 摄像头，`FriendlyName` 和 VID/PID 通常完全相同（如都是 `Ingenic HD Web Camera`，`a108:2240`）。但不同批次的固件配置可能不同（一个支持 500 万像素，一个最大 1080p）。

这种情况下：
- **不能用 `FriendlyName` 区分**：名字一样
- **不能用 VID/PID 区分**：同型号芯片
- **只能用 `SymbolicLink` 区分**：USB 端口级唯一

`SymbolicLink` 是 USB 拓扑路径的哈希，**反映物理端口位置**，不是设备 ID。换 USB 口后 `SymbolicLink` 会变，此时回退到按 `FriendlyName` 匹配。如果两个同名设备都在，可能匹配到另一个。这是 Windows 层面无法解决的——除非设备有唯一序列号（大部分白牌 UVC 没有）。

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

500 万像素 H264 在 Intel HD Graphics Gen11 上硬件解码速度约 **20fps**，采集是 **30fps**，中间差 10fps。预览队列满后 `try_send` 失败，丢的帧可能是 P 帧也可能是 IDR。丢 P 帧 → 后续参考断裂 → 黑帧/花屏；丢 IDR → 整个 GOP 无法解码 → 全黑。

视频编码的参考帧结构决定这个问题**无法在消费端彻底消除**。

#### 推荐用法

**H264 500 万像素场景下，选 `2592x1944 @20fps` 或更低帧率。** 采集 20fps、解码 20fps，队列永不堆积。

如果需要 30fps 录制且必须 500 万像素，接受预览会闪，或者用 **MJPG 2592x1944 @30fps** 格式——MJPG 每帧独立编码，丢帧不影响后续帧。

#### 未来可能的优化方向（未实现）

把 NV12 解码输出直接作为两张纹理（Y 平面 R8 + UV 平面 RG8）上传到 GPU，由 shader 做 YUV→RGB 转换。CPU 不再处理 500 万像素的颜色转换，解码线程可以从 47ms/帧压到 25ms/帧以下，达到 30fps。代价是 `eframe` 需要从 `glow` 切到 `wgpu`，工作量约 2~3 天。

---

## 已知限制

- **跨 subtype 切换走同步重启（~1.17–1.42s）**，UI 会被阻塞。当前主要耗时在 `IMFSourceReader` 的 COM Release（530–790ms）和 `MFCreateSourceReader` + `SetCurrentMediaType`（520–548ms），都是 MF/驱动黑盒。理论上可通过「reader Release 后台化」降到 0.6–0.9s，但风险较高（后台 Release 和新 reader 创建可能竞争驱动资源），当前不做。
- H264 热切换后首帧延迟取决于驱动的 GOP 周期（本机观测约 1.6–2.1s），期间 UI 显示“正在等待首帧…”，属于预期行为。日志里的 `采集端帧间隔 N ms（真丢帧）` 在切换后首次出现时，实际是「首帧到达延迟」，不是真丢帧。
- 录制 H264 时若源端 IDR 间隔较长，首个 IDR 到达前写不了 mp4（`recorder.rs` 已有兜底）。
- **低帧率格式的帧间隔误报**：`YUY2 1920x1080@5`（实际 ~9fps，113ms 间隔）、`H264 640x480@5`（实际 ~15fps，67ms 间隔）等格式下，`FRAME_GAP_WARN_TICKS = 600_000`（60ms）固定阈值导致每帧都打 `采集端帧间隔 N ms（真丢帧）` 警告。`预览端累计丢 0` 说明帧都被正常消费。后续可把阈值改为 `1.5 × 名义帧间隔`。
- **H264 热切换瞬间的极小竞态**：见 P3 末尾，微秒级窗口，输出一帧错乱后自动恢复。
- **MJPEG 名义帧率设置无效**：本机 sensor 的 MJPEG 走硬件编码通道，固定 30fps 输出。P4 已用实测回填修正播放时长，但实际帧率不变。
- **配置不自动打开摄像头**：配置只恢复选择状态，启动自动打开由「热插拔自动连接」机制负责。
- **`SymbolicLink` 反映 USB 物理端口，换口后配置回退到按名字匹配**。两个同名设备都在时，可能 A/B 分配对调或格式回退到默认。这是 Windows 层面无法解决的——同型号无唯一序列号的 UVC 设备，`SymbolicLink` 已是能做到的最细粒度。
- **设备断开检测只处理 `hrstatus.is_err()`**。有些驱动拔线后不报错、只停回调，这种情况检测不到。可加 `last_ts` 超时检测，但低帧率格式误报率高，当前不做。
- **热插拔轮询间隔 2 秒**。拔插后最多 2 秒才响应。更短间隔会增加 UI 开销（每次轮询约 10–50ms），更长会让用户觉得「插上了没反应」。

## 项目结构

```
src/
  main.rs          # 入口，日志 tee 到 camcap-run.log，加载配置，
                   # 进程退出时 mf::shutdown()
  app.rs           # eGUI 界面 + CameraSlot 状态 + 异步切换轮询
                   # + 设备参数浮窗 + 双画面/单画面纯净模式 + 配置保存
                   # + 热插拔轮询 + SymbolicLink 设备匹配
                   # + 纯净模式提示延迟淡出
  camera.rs        # Media Foundation 设备枚举，读 FriendlyName + SymbolicLink
  capture.rs       # CaptureHandle：reader 创建、异步回调、格式/参数控制、
                   # 解码线程调度、current_dims 共享尺寸、
                   # request_format/poll_switch 异步切换接口、
                   # 格式列表缓存、preferred 格式回退、error_flag 断开检测
  config.rs        # 配置持久化（serde_json，%APPDATA%\camcap\config.json），
                   # v1→v2 迁移，camera_name / symbolic_link 字段
  mf.rs            # Media Foundation 进程级初始化（ensure_startup / shutdown）
  h264_preview.rs  # H264 硬件解码器（IMFTranform + NV12→RGB24），
                   # visible（输出尺寸）与 buffer（含 16 对齐 padding）分离
  recorder.rs      # MJPG/YUY2 → AVI（实测帧率回填）、H264 → MP4（muxide）
  photo.rs         # 快照保存

examples/
  probe_devices.rs # 独立探测程序，列出所有设备的 FriendlyName + SymbolicLink
                   # 用法：cargo run --example probe_devices
```

## 性能参考（Intel HD Graphics Gen11 + 1080p UVC 摄像头）

| 场景 | 启动耗时 | 首帧就绪 | 格式切换（重启路径） |
|---|---|---|---|
| MJPG 1920x1080 @30 | 字体加载 ~200ms | ~350ms | ~400ms |
| H264 1920x1080 @20 | 字体加载 ~200ms | ~450ms（含解码器初始化） | ~500ms |

日志里带 `[TIMING]` 前缀的行记录各阶段耗时，运行一次后 `grep TIMING camcap-run.log` 查看。

| 路径 | 实测耗时 | 备注 |
|---|---|---|
| 同 subtype 热切换 | 565–700ms | 异步，UI 不阻塞 |
| 跨 subtype 重启 | 1.17–1.42s | 同步，UI 阻塞；平均 1.28s |
| 其中 `reader Release` | 530–790ms | 最大单点，不可控 |
| 其中 `CaptureHandle 初始化` | 520–548ms | MF/驱动黑盒 |
| 其中 `首帧等待` | ~107ms | P6.1 已从 300ms 降 |
| 拔插后自动重连 | 450–550ms | 含 reader 创建 + SetMediaType + 首帧等待 |

## 许可

本项目采用 MIT 许可证，详见 [LICENSE](LICENSE)。

Copyright (c) 2026 msxyy