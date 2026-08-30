# 可复用模块清单与接入说明

本文汇总当前工程中可迁移到其他 ESP32-S3 + Slint 项目的模块、接入顺序和硬件约束。内容以当前源码为准；构建、烧录和设备功能说明仍见项目根目录的 [README](../README.md)。

## 1. 复用边界

当前 workspace 包含应用 package 和独立的 `bsp` library crate。应用通过
`esp_slint_bsp` 依赖访问板级驱动、无线任务、共享状态和设备日志；UI 与页面逻辑
仍位于根 package。本文中的“复用”指将 BSP crate 作为路径依赖，或按实际边界迁移其源码。

1. 在同一 workspace 中通过 `esp_slint_bsp` 路径依赖组合现有模块；
2. 将 BSP crate 作为路径依赖迁移到结构相近的工程；
3. 若要调整板级协议或引脚，按实际硬件边界修改对应驱动和初始化顺序。

复用等级：

- **直接复用**：接口和依赖相对独立，通常只需调整模块路径；
- **组合复用**：需要按本文列出的文件组一起迁移；
- **硬件适配**：包含当前开发板的引脚、外设所有权或具体芯片协议，换板时必须修改。

## 2. 模块总览

| 模块 | 复用等级 | 主要入口 | 关键约束 |
| --- | --- | --- | --- |
| [`bsp/src/logging.rs`](../bsp/src/logging.rs) | 直接复用 | `esp_trace!`、`esp_debug!`、`esp_info!`、`esp_warn!`、`esp_error!` | 依赖 `esp-println`；日志不受 `ESP_LOG` 过滤 |
| [`bsp/src/drivers/display/panel_init.rs`](../bsp/src/drivers/display/panel_init.rs) | 组合复用 | `iter()`、`DEFAULT`、`NEW` | 仅适用于当前 ST77916 初始化表；接口当前为 `pub(crate)` |
| [`bsp/src/drivers/display/mod.rs`](../bsp/src/drivers/display/mod.rs) | 硬件适配 | `init()`、`St77916Display::write_line()` | 固定 ST77916、360 × 360、当前 QSPI 引脚和 RGB565 |
| [`bsp/src/drivers/touch/mod.rs`](../bsp/src/drivers/touch/mod.rs) | 硬件适配 | `Cst816Touch::new()`、`read()` | 固定 CST816S、I2C `0x15`、360 × 360 单点触摸 |
| [`bsp/src/drivers/rtc.rs`](../bsp/src/drivers/rtc.rs) | 组合复用 | `DateTime`、`init()`、`read_time()`、`write_time()` | PCF85063 通过触摸驱动持有的共享 I2C 总线访问 |
| [`bsp/src/drivers/tca9554.rs`](../bsp/src/drivers/tca9554.rs) | 硬件适配 | `configure()`、`write_output()` | 固定地址 `0x20` 和当前 LCD/触摸复位位 |
| [`src/ui/platform.rs`](../src/ui/platform.rs) | 组合复用 | `install_platform()`、`DisplayLineBuffer` | Slint software renderer；输出目标是 `St77916Display` |
| [`src/ui_logic/input.rs`](../src/ui_logic/input.rs) | 组合复用 | `poll_touch()`、`SwipeDirection` | 依赖 CST816S 和 `MinimalSoftwareWindow`；滑动阈值固定 |
| [`bsp/src/features/config.rs`](../bsp/src/features/config.rs) | 组合复用 | `request_*`、`take_*`、`copy_*`、`finish_*` | `critical-section` 固定容量状态通道；跨 crate 使用的快照 API 为 `pub` |
| [`bsp/src/features/time_sync.rs`](../bsp/src/features/time_sync.rs) | 组合复用 | Embassy task `run()` | 依赖可用的 Station `Stack`、DNS、UDP/123 和 `config` 时间戳通道 |
| [`bsp/src/features/wifi_portal.rs`](../bsp/src/features/wifi_portal.rs) | 组合复用 | `start()` | 同时包含 WiFi 控制、AP/STA、DHCP、HTTP 门户、扫描和 NTP 任务编排 |
| [`bsp/src/features/bluetooth.rs`](../bsp/src/features/bluetooth.rs) | 组合复用 | `start()` | TrouBLE Central + Peripheral；依赖全局 TRNG 和 `config` 状态通道 |
| [`src/ui_logic/clock.rs`](../src/ui_logic/clock.rs) | 硬件适配 | `initialize_rtc()`、`refresh_rtc()`、`apply_network_time()` | 直接依赖生成的 `MainWindow`、RTC、触摸总线和 UTC 配置 |
| [`ui/state.slint`](../ui/state.slint) | 组合复用 | `AppState`、`StateRoot`、`PageFrame` | 状态字段覆盖当前整套应用；页面基准固定为圆形 360 × 360 |
| [`ui/components/controls.slint`](../ui/components/controls.slint) | 直接复用 | `MenuItem`、`NavButton`、`KeyButton` | 尺寸和颜色按当前圆屏设计；可通过属性覆盖部分尺寸 |
| [`ui/pages/`](../ui/pages) | 组合复用 | 各文件导出的 `*Page` | 页面依赖 `AppState`、`PageFrame`，部分页面还依赖共享 controls |
| [`build.rs`](../build.rs) | 组合复用 | `slint_build::compile_with_config()` | Slint 资源按 software renderer 方式嵌入 |

[`src/main.rs`](../src/main.rs) 是应用组合根；[`bsp/src/board/mod.rs`](../bsp/src/board/mod.rs)
是 BSP 的板级组合根。二者分别负责应用生命周期和硬件初始化，不应互相复制。

## 3. 基础设施模块

### 3.1 直出日志

[`bsp/src/logging.rs`](../bsp/src/logging.rs) 提供五个带日志等级和 `module_path!()` 的宏。输出直接走 `esp_println::println!`，与 `espflash monitor` 使用同一 UART 或 USB JTAG 通道。

适合复用的场景：

- `no_std` 应用需要始终可见的设备诊断日志；
- 第三方依赖继续使用 `log` facade，而应用日志不希望被 `ESP_LOG` 隐藏。

接入要求：

- 保留 `esp-println` 依赖；
- 应用 crate 添加 `esp_slint_bsp` 路径依赖并从该 crate 导入日志宏；
- 若需要运行时等级过滤，应另加过滤层，当前宏不会过滤。

### 3.2 Slint 构建脚本

[`build.rs`](../build.rs) 使用 `EmbedForSoftwareRenderer` 编译 [`ui/main.slint`](../ui/main.slint)，并监听 `ui` 目录和表盘图片变化。迁移到其他软件渲染项目时，应同步保留：

- `slint-build = "=1.16.0"`；
- `EmbedResourcesKind::EmbedForSoftwareRenderer`；
- 资源文件的 `cargo:rerun-if-changed`；
- 当前 ESP 链接脚本参数。

如果新项目不使用当前表盘图片，应删除对应的监听路径，而不是保留失效引用。

## 4. 硬件驱动模块

### 4.1 ST77916 显示与初始化命令流

复用文件组：

- [`bsp/src/drivers/display/mod.rs`](../bsp/src/drivers/display/mod.rs)
- [`bsp/src/drivers/display/panel_init.rs`](../bsp/src/drivers/display/panel_init.rs)
- [`bsp/src/drivers/tca9554.rs`](../bsp/src/drivers/tca9554.rs)
- [`bsp/src/drivers/touch/mod.rs`](../bsp/src/drivers/touch/mod.rs)

`display::init(DisplayPeripherals)` 一次性取得 I2C、SPI 和 GPIO 所有权，返回：

```rust
(St77916Display<'static>, Cst816Touch<'static>)
```

初始化流程包含 TCA9554 配置、LCD/触摸复位、低速读取面板 ID、选择 `DEFAULT` 或 `NEW` vendor 命令流、切换至 40 MHz QSPI，以及创建触摸驱动。

`St77916Display::write_line(line, range, pixels)` 接收 Slint `Rgb565Pixel` 行片段。它会：

- 校验行号、范围和像素数量；无效范围直接返回 `Ok(())`；
- 设置 ST77916 写入窗口；
- 将 RGB565 转为 big-endian 字节；
- 每 32 像素分块，通过 Quad Data Mode 写入。

当前硬件映射：

| 资源 | 当前值 |
| --- | --- |
| LCD | ST77916，360 × 360，RGB565 |
| QSPI SCK | GPIO40 |
| QSPI D0/D1/D2/D3 | GPIO46/GPIO45/GPIO42/GPIO41 |
| LCD CS | GPIO21 |
| 背光 | GPIO5 |
| I2C SDA/SCL | GPIO11/GPIO10，400 kHz |
| 触摸中断 | GPIO4 |
| LCD/触摸复位 | TCA9554 `LCD_RESET_BIT` / `TOUCH_RESET_BIT` |

换屏或换板时必须一起检查 `DisplayPeripherals`、分辨率常量、panel 初始化表、SPI 模式、时钟、像素字节序和复位时序。

### 4.2 紧凑 panel 初始化表

[`bsp/src/drivers/display/panel_init.rs`](../bsp/src/drivers/display/panel_init.rs) 将 vendor 命令编码为紧凑字节流：

```text
command, data_len | 0x80_if_delay, data..., delay_ms_le_if_present
```

`InitCommandIter` 在运行时按需解码，避免为每条命令保存独立 slice 指针和结构体元数据。这个编码器思路可以迁移到其他面板，但 `DEFAULT`、`NEW` 数据本身只适用于当前 ST77916 变体。迁移到其他芯片时应保留迭代器格式，替换命令表并重新验证所有长度和延时字段。

### 4.3 CST816S 触摸

[`bsp/src/drivers/touch/mod.rs`](../bsp/src/drivers/touch/mod.rs) 提供：

- `Cst816Touch::new(i2c, interrupt_pin)`：读取 chip ID、关闭自动休眠并持有 I2C；
- `Cst816Touch::read()`：读取第一个触点，返回 `Result<Option<TouchPoint>, Error>`；
- `TouchPoint { x, y }`：当前有效范围为 `0..360`。

该驱动只处理单点触摸。超出范围的坐标会被丢弃。当前实现由触摸对象独占 I2C，并通过 crate 内部的 register transaction 方法给 RTC 复用；若新项目使用独立 I2C bus manager，应先把总线抽象从 `Cst816Touch` 中拆出，再迁移 RTC。

### 4.4 PCF85063 RTC 与日期时间

[`bsp/src/drivers/rtc.rs`](../bsp/src/drivers/rtc.rs) 包含两层能力：

1. `DateTime` 的纯时间转换：`is_valid()`、`from_unix_seconds()`、`to_unix_seconds()`、`with_utc_offset()`；
2. PCF85063 I2C 操作：`init()`、`read_time()`、`write_time()`。

`DateTime` 支持当前 RTC 映射使用的 `1970..=2069`。它适合单独提取成无硬件依赖的时间模型；提取时应把日期辅助函数一并迁移。

RTC 地址固定为 `0x51`。硬件访问参数目前是 `&mut Cst816Touch`，因为触摸驱动持有共享 I2C。此签名是当前工程的板级折中，不是通用 RTC 驱动接口。

### 4.5 TCA9554 GPIO 扩展器

[`bsp/src/drivers/tca9554.rs`](../bsp/src/drivers/tca9554.rs) 只封装当前板需要的两项操作：

- `configure()`：将全部 IO 配为输出并清零；
- `write_output()`：一次写入完整输出寄存器。

当前错误处理使用 `unwrap()`，适合启动阶段失败即终止的板级初始化。若要作为通用驱动复用，应改为返回 I2C 错误，并增加按位更新时的输出 shadow，避免调用方覆盖其他输出位。

## 5. Slint 平台与输入适配

### 5.1 Software renderer 平台

[`src/ui/platform.rs`](../src/ui/platform.rs) 提供：

- `install_platform(Rc<MinimalSoftwareWindow>)`：注册单窗口 Slint platform；
- `DisplayLineBuffer`：实现 `LineBufferProvider<TargetPixel = Rgb565Pixel>`，将每个重绘行直接交给 `St77916Display::write_line()`。

最小渲染组合如下：

```rust
let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
ui::install_platform(window.clone());
window.set_size(PhysicalSize::new(360, 360));

let mut line_buffer = [Rgb565Pixel(0); 360];
window.draw_if_needed(|renderer| {
    renderer.render_by_line(ui::DisplayLineBuffer {
        display: &mut display,
        buffer: &mut line_buffer,
    });
});
```

当前实现不分配完整 framebuffer。迁移时，行缓冲长度必须覆盖显示宽度；显示驱动和 Slint renderer 必须使用相同像素格式。

### 5.2 触摸事件与滑动识别

[`src/ui_logic/input.rs`](../src/ui_logic/input.rs) 的 `poll_touch()` 将 CST816S 数据转换为：

- `PointerPressed`
- `PointerMoved`
- `PointerReleased`
- `PointerExited`

调用方需跨循环保存 `last_touch` 和 `touch_start`。释放时，水平位移至少 60 像素且垂直偏移不超过 100 像素，返回 `SwipeDirection::Left` 或 `Right`。触摸读取错误目前按“触摸结束”处理；若项目需要区分 I2C 故障和真实释放，应调整返回契约。

## 6. 后台任务共享状态

[`bsp/src/features/config.rs`](../bsp/src/features/config.rs) 是 UI 主循环与 Embassy 无线任务之间的无 Slint 依赖桥梁。所有共享值存放在 `critical_section::Mutex<RefCell<_>>` 中，跨任务只复制固定容量结构，不把 Slint 对象传入后台任务。

### 6.1 通道类型

| 通道 | UI/调用方入口 | 后台任务入口 | 状态读取 |
| --- | --- | --- | --- |
| WiFi 凭据 | `request_wifi_credentials()` | `take_wifi_command()` | WiFi status 单独读取 |
| AP/Station/断开控制 | `request_wifi_ap_state()`、`request_wifi_station_state()`、`request_wifi_disconnect()` | `take_wifi_control()` | `copy_wifi_status()` |
| WiFi 扫描 | `request_wifi_scan()` | `take_wifi_scan_request()`、`finish_wifi_scan()`、`fail_wifi_scan()` | `copy_wifi_scan()` |
| BLE 扫描 | `request_ble_scan()` | `take_ble_scan_request()`、`store_ble_scan_entry()`、`finish_ble_scan()`、`fail_ble_scan()` | `copy_ble_scan()` |
| BLE 配对 | `request_ble_pairing()`、`request_ble_pair_confirmation()` | `take_ble_pair_request()`、`take_ble_pair_confirmation()`、`set_ble_pair_state()` | `copy_ble_pair_state()` |
| BLE 设置 | `set_ble_enabled()`、门户更新名称 | BLE task 读取 | `copy_ble_enabled()`、`copy_ble_name()` |
| NTP 时间 | NTP task `publish_time_sync()` | UI 主循环 `take_time_sync()` | 单槽、消费即清空 |
| UTC 偏移 | `adjust_utc_offset()`、`reset_utc_offset()` | 时钟 UI 读取 | `utc_offset_hours()` |

### 6.2 固定容量和状态机

- WiFi/BLE 快照最多保存 `MAX_SCAN_RESULTS = 12` 条；
- SSID、BLE 名称最多 32 字节，WiFi 密码最多 64 字节；
- WiFi 驱动单次扫描请求最多 8 个结果；
- 状态使用 `u8` 常量表示 `IDLE / REQUESTED / RUNNING / READY / FAILED` 等阶段；
- `request_*`、`take_*` 形成单消费者命令通道，重复请求可能合并为同一个状态；
- 快照是 `Clone + Copy` 固定数组，不在无线任务和 UI 主循环之间传递动态容器。

迁移这一模块时，应同时迁移所有生产者和消费者，避免只复制状态结构而遗漏状态转换。若新工程允许泛型或更清晰的类型边界，可将重复状态机提取为枚举；当前代码优先使用固定布局和简单临界区访问。

## 7. 无线功能模块

### 7.1 WiFi 配置门户

[`bsp/src/features/wifi_portal.rs`](../bsp/src/features/wifi_portal.rs) 的 `start(spawner, WIFI)` 一次启动：

- `esp-radio` WiFi controller；
- AP 与 Station 两个 Embassy network runner；
- AP 静态地址和 DHCP server；
- HTTP 配置门户；
- Station NTP 同步任务。

默认状态：SoftAP 关闭、Station 开启；仅当编译期 SSID 非空时自动连接。运行时提交的凭据不写入 Flash。

门户常量与接口：

| 项目 | 值 |
| --- | --- |
| AP SSID | `ESP32-S3-配置` |
| AP 地址 | `192.168.4.1/24` |
| DHCP 地址池 | `192.168.4.50..=192.168.4.200` |
| 配置提交 | `POST /config` |
| WiFi 扫描 | `GET /api/wifi/scan` |
| WiFi 结果 | `GET /api/wifi/results` |
| BLE 扫描 | `GET /api/ble/scan` |
| BLE 结果 | `GET /api/ble/results` |

当前 HTTP server 使用单连接循环和固定缓冲区，适合设备配置页，不是通用 Web server。WiFi 扫描依赖 Station 接口处于开启状态；扫描时保留当前 AP/STA 模式，不通过重新配置 controller 来重启无线硬件。

### 7.2 Bluetooth LE

[`bsp/src/features/bluetooth.rs`](../bsp/src/features/bluetooth.rs) 同时实现：

- Peripheral 广播和 Battery Service GATT server；
- Central 被动扫描；
- Security Manager 数字配对流程。

当前限制：

- 最大连接数 1；
- L2CAP channel 数 3；
- 扫描窗口 5 秒；
- 默认关闭 BLE；关闭时拒绝新的扫描和配对请求；
- 广播名称最大 32 字节，并按 advertising data 空间进一步截断；
- 需要提前初始化并保持 ESP TRNG source 生命周期。

该模块依赖 `esp-radio`、`trouble-host`、`bt-hci`、Embassy 和 [`config.rs`](../bsp/src/features/config.rs)。只复制 `bluetooth.rs` 会缺少命令通道、扫描快照和板级随机源初始化。

### 7.3 NTP 时间同步

[`bsp/src/features/time_sync.rs`](../bsp/src/features/time_sync.rs) 导出 Embassy task `run(Stack<'static>)`。任务在网络 link 和 DHCP 配置就绪后依次尝试多个 NTP server：

- DNS 超时 5 秒；
- NTP 响应超时 8 秒；
- 全部失败后 30 秒重试；
- 成功后每小时同步；
- 只接受 NTP server/broadcast mode 且 stratum 非 0 的至少 48 字节响应；
- 成功结果通过 `config::publish_time_sync()` 交给 UI/RTC 层。

复用时需要上游 DNS 和 UDP/123 可用。该任务只产生 Unix timestamp，不直接操作 RTC，因此也可将发布端替换为其他时间消费者。

## 8. Slint 状态、控件和页面

### 8.1 状态与圆屏基类

[`ui/state.slint`](../ui/state.slint) 导出：

- `AppState`：Rust、页面和后台状态映射的全局契约；
- `StateRoot`：把 `AppState` 属性双向绑定到组件 root；
- `PageFrame`：356 × 356、178px 圆角、3px 边框的圆屏页面基类。

`AppState` 的属性按时钟、菜单、WiFi、BLE 分组；callback 负责将扫描、开关、输入、配对、清零和 UTC 调整请求送回 Rust。复用单个页面时，至少保留该页面读取的属性和触发的 callback，或在页面中改为显式输入属性和 callback。

### 8.2 共享控件

[`ui/components/controls.slint`](../ui/components/controls.slint) 提供：

| 组件 | 输入 | 回调 | 用途 |
| --- | --- | --- | --- |
| `MenuItem` | `label`、`detail` | `activated()` | 扫描结果和菜单列表行 |
| `NavButton` | `button-width`、`label` | `activated()` | 页面底部导航或主要动作 |
| `KeyButton` | `value`、`button-width` | `pressed(string)` | WiFi 密码和 BLE 数字键盘 |

三个控件均包含按下态颜色反馈。`KeyButton` 会将 `SHIFT`、`SYM`、`SPACE`、`BACK` 显示为中文标签，但回调仍返回原始控制值。

### 8.3 可独立预览页面

当前可复用/预览的导出组件：

| 组件 | 文件 |
| --- | --- |
| `HomePage` | [`ui/pages/home.slint`](../ui/pages/home.slint) |
| `MenuPage` | [`ui/pages/menu.slint`](../ui/pages/menu.slint) |
| `TouchPage` | [`ui/pages/touch.slint`](../ui/pages/touch.slint) |
| `MotionPage` | [`ui/pages/motion.slint`](../ui/pages/motion.slint) |
| `PerformancePage` | [`ui/pages/performance.slint`](../ui/pages/performance.slint) |
| `WifiControlPage` | [`ui/pages/wifi_control.slint`](../ui/pages/wifi_control.slint) |
| `WifiListPage` | [`ui/pages/wifi_list.slint`](../ui/pages/wifi_list.slint) |
| `WifiPasswordPage` | [`ui/pages/wifi_password.slint`](../ui/pages/wifi_password.slint) |
| `BleControlPage` | [`ui/pages/ble_control.slint`](../ui/pages/ble_control.slint) |
| `BleScanPage` | [`ui/pages/ble_scan.slint`](../ui/pages/ble_scan.slint) |
| `BlePairPage` | [`ui/pages/ble_pair.slint`](../ui/pages/ble_pair.slint) |
| `SettingsPage` | [`ui/pages/settings.slint`](../ui/pages/settings.slint) |

[`ui/pages/menu_shell.slint`](../ui/pages/menu_shell.slint) 是当前页面路由器，`menu-view` 使用 `0..=10` 选择页面，并处理向左滑动返回主页。迁移或增删页面时，路由编号、菜单入口和 Rust 侧跳转值必须一起更新。

单页面预览示例：

```powershell
cargo run --release --manifest-path ../slint/tools/viewer/Cargo.toml -- `
  --component WifiListPage `
  ui/pages/wifi_list.slint
```

扫描列表等动态页面需要通过 Slint Viewer 的 `--load-data` 给导出的 `AppState` 注入数据；完整示例见 [README 的“单页面预览”章节](../README.md#单页面预览)。

## 9. 推荐复用组合

### 9.1 仅复用圆屏 Slint UI

迁移：

- [`ui/state.slint`](../ui/state.slint)
- [`ui/components/controls.slint`](../ui/components/controls.slint)
- 需要的 [`ui/pages/`](../ui/pages) 页面
- 字体和页面直接引用的图片资源

随后缩减 `AppState`，只保留已迁移页面需要的属性和 callback。不要直接带入不再使用的 WiFi/BLE 状态。

### 9.2 复用 ST77916 + CST816S 渲染链

迁移：

- [`bsp/src/drivers/display`](../bsp/src/drivers/display)、[`bsp/src/drivers/touch`](../bsp/src/drivers/touch)、[`bsp/src/drivers/tca9554.rs`](../bsp/src/drivers/tca9554.rs)
- [`src/ui/platform.rs`](../src/ui/platform.rs)
- [`src/ui_logic/input.rs`](../src/ui_logic/input.rs)
- [`bsp/src/logging.rs`](../bsp/src/logging.rs)

需要保持 RGB565、360 像素行缓冲、`RepaintBufferType::ReusedBuffer` 和当前设备引脚一致；换板时以 `DisplayPeripherals` 为集中修改点。

### 9.3 复用 RTC + NTP 校时

硬件侧迁移 `bsp/src/drivers/rtc`；网络侧迁移 `bsp/src/features/time_sync`。两者当前通过 `bsp/src/features/config` 的单槽 Unix timestamp 通道解耦。若不使用触摸驱动持有共享 I2C，应先把 RTC 参数改为独立 I2C 抽象。

### 9.4 复用完整无线配置能力

至少一起迁移：

- [`bsp/src/features/config.rs`](../bsp/src/features/config.rs)
- [`bsp/src/features/wifi_portal.rs`](../bsp/src/features/wifi_portal.rs)
- [`bsp/src/features/time_sync.rs`](../bsp/src/features/time_sync.rs)
- [`bsp/src/features/bluetooth.rs`](../bsp/src/features/bluetooth.rs)
- [`bsp/src/board/mod.rs`](../bsp/src/board/mod.rs) 中的 allocator、TRNG、RTOS 启动顺序

WiFi 与 BLE 共存依赖 `esp-radio` 的 `coex` feature。后台无线任务不得调用 Slint API。

## 10. 初始化与主循环顺序

当前经过验证的组合顺序：

1. 初始化 `esp_println` logger 和 ESP32-S3 peripherals；
2. **先注册 PSRAM allocator**，再注册供无线栈使用的 internal/reclaimed heap；
3. 创建并保留 `TrngSource`；
4. 启动 `esp_rtos`；
5. 启动 WiFi、Embassy network、NTP 和 BLE tasks；
6. 初始化 TCA9554、ST77916、CST816S；
7. 安装 Slint platform，再创建 `MainWindow`；
8. 主循环中处理共享状态、NTP 时间、Slint timers、触摸和按行渲染；
9. 循环末尾使用 `embassy_time::Timer::after_millis(...).await` 主动让出 executor。

不能把第 9 步替换为阻塞式 delay。否则无线 controller、DHCP、HTTP、NTP 和 BLE tasks 得不到执行机会。

## 11. 版本与并发约束

当前组合使用：

- Rust 1.88，目标 `xtensa-esp32s3-none-elf`；
- `esp-hal = 1.1.1`；
- `esp-radio = 0.18.0`，启用 `wifi`、`ble`、`coex`；
- `esp-rtos = 0.3.0`；
- `embassy-net = 0.8.0`；
- `trouble-host = 0.6.0`；
- `slint = 1.16.0`、`slint-build = 1.16.0`。

Slint 启用了 `unsafe-single-threaded`。所有 Slint API 必须留在当前单线程 UI 主循环；无线任务只能通过 [`config.rs`](../bsp/src/features/config.rs) 交换普通数据。

## 12. 迁移检查清单

- [ ] 明确是同型号硬件复用，还是需要改引脚/总线/芯片协议。
- [ ] 按“推荐复用组合”迁移完整文件组，没有漏掉状态通道或初始化表。
- [ ] 检查所有 `pub(crate)` 和私有接口，只开放真实需要的边界。
- [ ] 保持 PSRAM、internal heap、TRNG、RTOS 的启动顺序。
- [ ] 保持 Slint 单线程约束，后台任务不持有 UI 对象。
- [ ] 检查固定容量：SSID 32、密码 64、扫描快照 12。
- [ ] 检查 360 × 360、RGB565、行缓冲长度和触摸坐标范围。
- [ ] 替换编译期 WiFi 凭据；不要把真实密码提交到仓库。
- [ ] 若需要断电保存配置，新增 Flash/NVS 持久化；当前运行时配置不会持久化。
- [ ] 执行格式检查、目标检查和 release 构建。

验证命令：

```powershell
cargo +esp fmt --check
cargo +esp check
cargo +esp build --release
```
