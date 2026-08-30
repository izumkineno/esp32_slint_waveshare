# AGENTS.md — CubePet / ESP32-S3-Touch-LCD-1.85C 约束文档

> 本文件是 AI Agent 在本仓库工作的**最高优先级上下文**。任何代码、UI、驱动改动都必须满足本文约束。硬件细节以 `docs/` 为唯一真源，冲突时以 `docs/板载文档/` 为准。

---

## 1. 项目一句话定位

**CubePet** 是一台运行在 **Waveshare ESP32-S3-Touch-LCD-1.85C** 上的 **360×360 圆形触摸屏** Rust 固件 — `no_std` + `esp-hal 1.1.1` + `Slint 1.16.0 software-renderer` + `MinimalSoftwareWindow` 行缓冲。不是桌面/浏览器/手机 App，是**裸机嵌入式**工程，每一行像素、每一 KB RAM、每一 ms 延迟都受硬件约束。

详见 `docs/1_项目定位计划方案.md`。

---

## 2. 硬件铁律 — 改代码前必读

### 2.1 核心板

| 项目 | 值 | 说明 |
|---|---|---|
| 主控 | ESP32-S3R8 双核 Xtensa LX7 240MHz | `xtensa-esp32s3-none-elf` |
| Flash / PSRAM | 16 MB Flash + 8 MB Octal PSRAM | `heap_allocator!(reclaimed 64KB + 36KB)` 已在 `src/main.rs` 配置 |
| 屏幕 | **1.85" 圆形 LCD 360×360 RGB565 ST77916 QSPI** | 见 `docs/板载文档/05_st77916-lcd.md` |
| 触摸 | **CST816S 单点 I2C 0x15, INT=GPIO4, RESET=TCA9554 EXIO1** | 见 `docs/板载文档/06_cst816-touch.md` |
| RTC | PCF85063 I2C 0x51, 共享触摸的 I2C 总线 | 见 `docs/板载文档/07_pcf85063-rtc.md` |
| IO扩展 | TCA9554PWR I2C 0x20, EXIO1=触摸复位, EXIO2=LCD复位 | 见 `docs/板载文档/04_tca9554.md` |
| 总线 | I2C SDA=GPIO11 / SCL=GPIO10 @400kHz, QSPI SCK=40 D0=46 D1=45 D2=42 D3=41 CS=21 | |
| 背光 | GPIO5 (当前仅高电平输出, 需改 PWM 才可调亮度) | |
| 其他 | SD 1-bit (CLK14/CMD17/D016), 电池ADC GPIO8, 扬声器/麦克风 V1↔V2 不兼容 | 见 `docs/板载文档/01_board-reference.md` |

### 2.2 V1 / V2 音频不兼容 — 严禁跨版本照搬

| 资源 | V1 (本地 Demo) | V2 (官方 GitHub) |
|---|---|---|
| DAC | PCM5101 (BCLK48/WS38/DIN47) | ES8311 (MCLK2/BCLK48/WS38/DIN47, I2C 0x18) |
| 麦克风 | 数字麦克风 (BCLK15/WS2/DIN39) | ES7210 双麦 + 回声消除 (I2C 0x40, PA_CTRL=GPIO15) |

**规则**: 动音频前必须先确认板子是 V1 还是 V2 (看 `Rev2.0` 丝印/QC标签)，再按 `docs/板载文档/02_reusable-code.md` 选对应源码。V1 的 `Audio_PCM5101.*` / `MIC_MSM.*` 绝不能用于 V2。

### 2.3 360 圆屏的物理约束

- 控制器暴露 **方形 360×360** 地址空间，但**可视区是圆形**，中心 `(180,180)` 半径 `178`，四角不可达。
- 关键文字/按钮必须收进 **320 内切圆安全区**，否则被圆角裁切。
- 触摸坐标仍是 `0..360` 方形空间，命中判断需自行过滤圆外区域 (`canvas_contains`)。

---

## 3. 软件栈 — 不可替换的选型

```
UI:        Slint 1.16.0  renderer-software + MinimalSoftwareWindow (行缓冲)
           ui/main.slint  360×360, clip:true, 混合渲染 (复杂用图片/简单用绘制)
Platform:  src/ui/platform.rs  EspPlatform + DisplayLineBuffer
App:       src/ui_logic/  clock / pet_animator / input (+ 待扩展 settings/ai/audio)
BSP:       bsp/src/drivers/  display / touch / tca9554 / rtc  (+ features/ 待扩展 wifi/ble/sd)
HAL:       esp-hal 1.1.1 + esp-alloc + esp-bootloader-esp-idf + esp-println
Toolchain: esp toolchain (rust-toolchain.toml channel="esp"), target xtensa-esp32s3-none-elf, build-std=[core,alloc]
Build:     build.rs  slint_build::EmbedForSoftwareRenderer + linkall.x
Runner:    espflash flash --monitor --flash-size 16mb  (.cargo/config.toml)
```

**版本锁定**: `slint = "=1.16.0"`、`esp-hal = "1.1.1"` 精确版本，升级需全量回归。

---

## 4. 关键约束 — 违反即 Bug

### 4.1 渲染：行缓冲，不是帧缓冲

- **唯一正确路径**: `MinimalSoftwareWindow` + 单行 `DisplayLineBuffer` (`[Rgb565Pixel; 360]` 单数组) + `St77916Display::write_line()` 按行 `0x32/0x002C00` QSPI 写入。
- **禁止**: 创建 `360×360` 全帧 `framebuffer` (会吃掉 253KB RAM) 与 Slint 行渲染同时挂载同一 SPI；`esp_learn` 的 `embedded-graphics` 全帧路径与当前 Slint 路径二选一，见 `docs/板载文档/02_reusable-code.md §3`。
- **动画开销**: 单帧宠物绘制 <40% 屏幕面积；`property animation` 优先，位图帧序列每状态 <60KB；`window.draw_if_needed()` 仅重绘脏区，不要强制全屏 `request_redraw`。
- **字体**: 默认仅 ASCII；CJK 需外挂 TTF 并预烘焙 (见 `ui/main.slint` 底部隐藏 Text 预留)。

### 4.2 内存与并发

- `no_std` + `no_main` + `extern crate alloc`，堆仅 ~100KB (64KB reclaimed + 36KB)，**严禁**每帧 `alloc` / 大 `Vec` / `String` 拼接。
- `slint` 开启 `unsafe-single-threaded`，**所有 Slint API 必须在主循环单线程调用**，禁止从 Embassy task / 中断直接调 `MainWindow::set_*`。
- 跨任务共享状态用 `critical_section::Mutex<RefCell<...>>` 或 `embassy-sync` channel (参考 `bsp/src/features/config.rs`)，不要用 `std::sync`。
- `#[deny(clippy::mem_forget)]` — 禁用 `mem::forget` 于 `esp_hal` 类型。

### 4.3 触摸与输入

- **单点 only** (`points` 0/1)，无多指；手势仅 **轻触 + 水平滑动 ≥60px** (`src/ui_logic/input.rs`)。
- `Cst816Touch` 持有共享 I2C，RTC 通过 `touch` 的寄存器事务访问总线，不要创建第二个 I2C 实例。
- 触摸轮询在主循环 `poll_touch` 完成，`Ok(None)` 与 I2C 错误均视为抬起；不要阻塞等待 `INT` 中断。

### 4.4 时钟与 RTC

- `clock::initialize_rtc(&mut touch)` → `clock::refresh_rtc(&window, &mut touch)` 1Hz 刷新，失败时 UI 显示 `--:--` 降级，不可 `panic`。
- 时区/制式通过 `clock::refresh/apply` + NVS 持久化，NTP 校准走 `bsp/src/features/time_sync.rs` Embassy task (多源、失败 30s 重试、成功每小时一次)。

### 4.5 初始化顺序 — 错序即黑屏

```
400kHz I2C (GPIO11/10) → TCA9554 配置 → EXIO2 LCD复位脉冲 → 3MHz 读 ST77916 0x04 选 DEFAULT/NEW 初始化表
→ 软件复位 0x01 + 120ms → 切 40MHz QSPI → EXIO1 触摸复位 + 创建 CST816S → PCF85063 init
→ install_platform(MinimalSoftwareWindow 360×360) → MainWindow::new() → 主循环
```

不要跳过 TCA9554 就调 `ST77916_Init`，不要只复制 `panel_init.rs` 而省略 `display::init()` 的选表与复位时序。

---

## 5. 目录契约

```
cubepet/
├── src/
│   ├── main.rs              # 唯一入口: heap/外设/窗口/主循环 16ms tick
│   ├── ui/  platform.rs     # EspPlatform + DisplayLineBuffer (行缓冲核心)
│   │        mod.rs
│   └── ui_logic/ clock.rs / input.rs / pet_animator.rs
├── ui/
│   ├── main.slint           # 360×360 Window 唯一真源 (禁止拆多文件后忘记 build.rs)
│   └── assets/  frame.png / doro_body.png / mat.png / ... (已切片资源)
├── bsp/
│   ├── src/drivers/ display/ (mod.rs + panel_init.rs [+framebuffer.rs feature]) / touch/ / rtc.rs / tca9554.rs
│   ├── src/features/ config.rs / wifi_portal.rs / time_sync.rs / bluetooth.rs (成组迁移, 勿单文件复制)
│   ├── src/board/mod.rs     # allocator/TRNG/RTOS/Radio 启动顺序
│   └── Cargo.toml           # feature: embedded-graphics / wifi / ble
├── docs/
│   ├── 1_项目定位计划方案.md  # 产品/MVP/UI/架构总纲
│   └── 板载文档/ 00-14_*.md   # 硬件唯一真源 (01板级/02复用边界/03-13各驱动)
├── examples/  panel_init.rs / st77916.rs / cst816.rs / embedded-graphics-board.rs
├── vendor/ESP32-S3-Touch-LCD-1.85C-Demo/  # V1 原厂 Demo (勿直接用于 V2)
├── tools/slice_doro_atlas.py
├── build.rs / Cargo.toml / rust-toolchain.toml / .cargo/config.toml
└── AGENTS.md                # 本文件
```

---

## 6. UI 开发规则 (360 圆屏)

1. **尺寸锁死**: `MainWindow { width:360px; height:360px; }` + 根 `Rectangle { width:360px; height:360px; clip:true; }`，不要改尺寸或移除 `clip`。
2. **混合渲染**: 复杂插画用 `Image @image-url("assets/*.png")` (需 `EmbedForSoftwareRenderer`)，简单+动态用 `Rectangle/Text` 绘制 (圆环/秒点/胶囊/状态行)，不要把宠物每帧都做全屏位图。
3. **坐标系**: 顶层 `x/y` 均为绝对像素，居中构图；新增元素先确认是否落在 320 安全区内。
4. **回调**: Slint `callback xxx()` 在 Rust 侧用 `Rc<Cell<bool>>` 标志位在主循环消费，不要在回调闭包里做 I2C/SPI/网络阻塞操作。
5. **新增页面**: 保持 `Home + Settings Stack + Overlay` 三层 (方案 §5.1)，不要引入 `esp_slint` 的深层 `MenuShell + 多Page`。
6. **资源**: 新增图片放 `ui/assets/`，`build.rs` 会自动 `rerun-if-changed=ui` 并嵌入；超大资源走 SD (`/sdcard`) 而非 Flash 嵌入。

---

## 7. Agent 工作流

### 7.1 接任务前必做

1. 读 `docs/板载文档/01_board-reference.md` 确认引脚/地址/版本边界。
2. 读 `docs/板载文档/02_reusable-code.md` 确认**成组迁移**清单，不要单文件复制 `wifi_portal.rs` / `rtc.rs` / `panel_init.rs`。
3. 读 `ui/main.slint` 与 `src/main.rs` 主循环，确认改动不破坏行缓冲与 16ms tick。

### 7.2 开发循环

```bash
# 1. 构建检查 (host 需 esp toolchain + xtensa target)
cargo check
cargo build              # dev: opt-level "s" 已配置
# 2. 烧录到板子 (16MB flash, 自动进 monitor)
cargo run                # = espflash flash --monitor --flash-size 16mb
# 3. 例程验证 (可选)
cargo run --example panel_init --features embedded-graphics
```

- 不要在未安装 `esp` toolchain 时尝试 `cargo build` (会报 `xtensa-esp32s3-none-elf` 缺失)。
- 烧录前确认板子已进下载模式 (USB-C, GPIO19/20)。
- `build.rs` 失败多为 `slint_build` 或 `linkall.x` 缺失，检查 `ui/main.slint` 路径与 `slint-build = "=1.16.0"`。

### 7.3 验证门槛

- **显示**: 烧录后 2s 内亮屏且 `frame.png` 背景 + `HH:MM` 可读；黑屏先查 TCA9554/复位时序/QSPI 速率。
- **触摸**: 轻触宠物有 `pet-tapped` log / 动画；滑动 ≥60px 触发 `swipe right/left`；无响应查 I2C 地址 0x15 / GPIO4 上拉。
- **RTC**: `refresh_rtc` 成功则时间走字，失败则 `--:--` 且有 `[WARN] RTC unavailable`，不 panic。
- 任何驱动 `Result::Err` 必须降级显示/静默，不可 `unwrap` 导致整机卡死。

---

## 8. 禁止事项 (MUST NOT)

- ❌ 创建 `360×360` 全帧 `[[u16;360];360]` 与 Slint 行缓冲并存。
- ❌ 在 Embassy task / 中断中调用 `slint::` / `MainWindow::set_*`。
- ❌ 使用 `std`、`std::sync`、`tokio`、或假设有文件系统/OS。
- ❌ 跨 V1/V2 复用音频/麦克风代码 (PCM5101 ↔ ES8311/ES7210)。
- ❌ 单文件复制 `display/panel_init.rs` / `wifi_portal.rs` / `rtc.rs` 而丢弃配套的选表/总线所有权/初始化顺序。
- ❌ 把关键 UI 放在圆屏四角 (会被物理边框遮挡)。
- ❌ 每帧 `alloc` / 大 `format!` / 阻塞 `Delay::delay_millis(>20)` 卡住 60Hz 主循环。
- ❌ 修改 `bsp/` 引脚定义 (GPIO40/46/45/42/41/21/5/4/11/10) 而不同步改 `docs/` 与 `examples/`。
- ❌ 新增依赖时引入 `default-features = true` 导致二进制膨胀 (Flash 16MB 但 RAM 极紧)。

---

## 9. 文档维护

- 新增/重命名 `docs/**` 后执行 `oma docs verify --no-urls` (外链检查用 `oma docs verify`)。
- 硬件相关改动必须同步更新 `docs/板载文档/` 对应章节，并在 `docs/板载文档/02_reusable-code.md` 登记复用边界。
- 本文件变更需保持与 `docs/1_项目定位计划方案.md` 的 MVP 定义一致。

---

## 10. 快速索引

- 总纲: `docs/1_项目定位计划方案.md`
- 板级: `docs/板载文档/01_board-reference.md`
- 复用边界: `docs/板载文档/02_reusable-code.md`
- 显示: `docs/板载文档/05_st77916-lcd.md`
- 触摸: `docs/板载文档/06_cst816-touch.md`
- RTC: `docs/板载文档/07_pcf85063-rtc.md`
- SD/Flash/音频/麦克风/电池/无线: `08`/`09`/`10`/`11`/`12`/`13`
- 圆屏画板: `docs/板载文档/14_embedded-graphics.md`
- UI 资产切片: `tools/slice_doro_atlas.py`
