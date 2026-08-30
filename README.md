# ESP32-S3 Slint 示例

本工程基于 `esp_learn` 的 ESP32-S3-Touch-LCD-1.85C 硬件初始化代码，使用 [Slint](https://slint.dev/) 的 `no_std` 软件渲染器替换 `embedded-graphics` 作为 GUI。

目标硬件：

- ESP32-S3-Touch-LCD-1.85C
- ST77916 360 × 360 QSPI 圆形 LCD
- CST816S 电容触摸控制器
- PCF85063 RTC
- TCA9554PWR IO 扩展器
- 16 MB Flash、Octal PSRAM

## 工程结构

| 文件                                      | 作用                                                  |
| ----------------------------------------- | ----------------------------------------------------- |
| `src/main.rs`                           | 应用组合根、主循环和 Slint 窗口生命周期               |
| `bsp/src/lib.rs`                        | 独立板级支持 crate 的公共入口                         |
| `bsp/src/board/mod.rs`                  | 开发板外设所有权、PSRAM、RTOS 和功能启动编排          |
| `bsp/src/drivers/display/mod.rs`        | ST77916 QSPI 驱动和 Slint 行缓冲输出                  |
| `bsp/src/drivers/display/panel_init.rs` | ST77916 vendor 初始化命令流                           |
| `bsp/src/drivers/touch/mod.rs`          | CST816S 触摸驱动和共享 I2C 事务                       |
| `bsp/src/drivers/rtc.rs`                | PCF85063 RTC 初始化、BCD 解码和日期时间读取           |
| `bsp/src/drivers/tca9554.rs`            | TCA9554PWR I2C GPIO 扩展器和 LCD / 触摸复位线         |
| `bsp/src/features/wifi_portal.rs`      | WiFi SoftAP、DHCP、HTTP 配置门户和真实热点扫描        |
| `bsp/src/features/bluetooth.rs`         | BLE 广播、Central 扫描和 Security Manager 配对        |
| `bsp/src/features/config.rs`            | WiFi / BLE 命令、固定容量扫描快照和运行时配置状态     |
| `src/ui_logic/clock.rs`                 | 时钟页面数据、RTC 刷新和 UTC 偏移处理                 |
| `src/ui_logic/input.rs`                 | 触摸轮询、Slint 事件转换和滑动识别                    |
| `src/ui/platform.rs`                    | Slint Platform 和软件渲染行缓冲适配                   |
| `ui/main.slint`                         | Slint 声明式界面                                      |
| `docs/UI参考_360x360_clean.png`         | 360 × 360 表盘参考图底图,动态时间字段在 Slint 中覆盖 |
| `ui/fonts/NotoSansSC-UI.ttf`            | 从 Noto Sans SC 子集化的嵌入式中文字体                |
| `ui/fonts/OFL.txt`                      | 字体许可证                                            |
| `build.rs`                              | 编译`.slint` 文件并嵌入软件渲染资源                 |
| `Cargo.toml`                            | 应用与独立 BSP workspace 配置                         |
| `.cargo/config.toml`                    | Xtensa 目标、`build-std` 和 `espflash` 配置       |
| `Cargo.lock`                            | 与当前`esp` Rust toolchain 兼容的依赖锁定版本       |

代码按 BSP 与应用两层组织：

```text
bsp
  ├── drivers       LCD / touch / RTC / I2C 外设协议
  ├── features      WiFi 配网 / BLE 广播 / 共享配置
  ├── board         PSRAM / RTOS / 外设所有权与启动顺序
  └── logging.rs    设备监视器日志宏
src
  ├── ui_logic      时钟状态 / 触摸输入 / 页面行为
  └── ui            Slint Platform / 行缓冲渲染适配
```

`src/main.rs` 只负责组合 BSP、UI 和页面逻辑；板级驱动、无线任务、
共享状态与设备日志均由 `esp_slint_bsp` crate 提供。

可复用模块的边界、依赖、接入顺序和迁移检查清单见
[`docs/reusable-modules.md`](docs/reusable-modules.md)。

`esp_learn` 与 `esp_slint` 仍分别维护自己的 LCD vendor 初始化表：

- `../esp_learn/src/panel_init.rs`
- `bsp/src/drivers/display/panel_init.rs`

`bsp/src/drivers/touch/mod.rs` 维护 Slint 工程自己的 CST816S 驱动副本，
避免应用入口通过 `#[path]` 依赖另一个工程的目录结构。PCF85063 与 CST816S
继续共用同一条 I2C 总线。

## Slint 渲染流程

```text
CST816S touch
    ↓
Slint WindowEvent
    ↓
Slint UI scene
    ↓
MinimalSoftwareWindow + SoftwareRenderer
    ↓
LineBufferProvider
    ↓
ST77916 QSPI line write
```

工程使用 `RepaintBufferType::ReusedBuffer`，Slint 只把需要重绘的行写入单行缓冲区，再通过 ST77916 QSPI 刷新 LCD，不在应用入口栈上分配完整的 360 × 360 framebuffer。

`bsp/src/drivers/display/panel_init.rs` 使用紧凑二进制命令流保存初始化表。每条记录
包含命令字节、数据长度标志、参数数据，以及可选的 little-endian 延时值；运行时
由迭代器解码，不再为每条命令保存独立的 slice 指针和结构体元数据。

嵌入式软件渲染器不能依赖电脑的系统字体。中文界面使用 `Noto Sans SC` 的
UI 字符子集，并额外包含常用 CJK Unified Ideographs（`U+4E00–U+9FFF`），
覆盖扫描结果等运行时中文字符串；在 `ui/main.slint` 中设置为
`default-font-family`。`build.rs` 显式监听字体文件变化，Slint 编译时会把字体
字形表嵌入固件；因此开发板和桌面预览使用同一套中文字形，不再出现中文方框或
空白。

## 硬件连接

| 信号                       | 配置                              |
| -------------------------- | --------------------------------- |
| I2C SDA / SCL              | GPIO11 / GPIO10                   |
| TCA9554PWR                 | I2C 地址`0x20`                  |
| LCD reset                  | TCA9554PWR EXIO2                  |
| Touch reset                | TCA9554PWR EXIO1                  |
| LCD QSPI SCK               | GPIO40                            |
| LCD QSPI D0 / D1 / D2 / D3 | GPIO46 / GPIO45 / GPIO42 / GPIO41 |
| LCD CS                     | GPIO21                            |
| LCD backlight              | GPIO5                             |
| CST816S                    | I2C 地址`0x15`                  |
| PCF85063                   | I2C 地址`0x51`                  |
| CST816S INT                | GPIO4                             |
| LCD 分辨率                 | 360 × 360                        |

## Waveshare 官方硬件摘要

本工程对应 Waveshare `ESP32-S3-Touch-LCD-1.85C`。官方页面：
[https://docs.waveshare.net/ESP32-S3-Touch-LCD-1.85C/](https://docs.waveshare.net/ESP32-S3-Touch-LCD-1.85C/)

| 模块     | 官方规格 / 本工程用法                                                      |
| -------- | -------------------------------------------------------------------------- |
| 主控     | ESP32-S3，双核 Xtensa LX7，最高 240 MHz                                    |
| 存储     | 16 MB Flash、8 MB Octal PSRAM；PSRAM 同时承载 Slint 和无线动态分配         |
| 圆形屏幕 | 1.85 英寸、360 × 360、ST77916、QSPI、RGB565                               |
| 触摸     | CST816S 电容触摸，I2C 地址`0x15`，中断 GPIO4                             |
| RTC      | PCF85063，I2C 地址`0x51`                                                 |
| IO 扩展  | TCA9554PWR，I2C 地址`0x20`；LCD 复位 EXIO2，触摸复位 EXIO1               |
| 无线     | 2.4 GHz WiFi 与 Bluetooth LE；本工程使用`esp-radio`                      |
| 总线     | I2C GPIO11/GPIO10；LCD QSPI GPIO40、GPIO46/45/42/41、CS GPIO21；背光 GPIO5 |

### 板载 WiFi / 蓝牙配置门户

固件启动后可以提供一个用于配网的开放 SoftAP，但默认关闭。需要使用配置门户时，
先在设备的 `WiFi 控制` 页面开启 AP：

| 项目      | 值                      |
| --------- | ----------------------- |
| WiFi 名称 | `ESP32-S3-配置`       |
| WiFi 密码 | 无密码                  |
| 配置地址  | `http://192.168.4.1/` |

开启 AP 后，使用手机或电脑连接配置网络，然后在浏览器打开配置地址。网页可以提交
目标 WiFi 名称、密码、蓝牙广播名称以及蓝牙开关。WiFi 提交后，设备在后台尝试连接
目标网络；蓝牙名称在下一次广播周期使用。AP 和蓝牙默认关闭，重新上电后需要再次
开启。

Station 获取 DHCP 地址后，网络模块仅为 NTP 时间同步执行必要的 DNS 和 UDP/123
请求，不再周期性访问 `example.com` 或发送 HTTPS `GET /`，避免产生频繁的外部网络请求。
配置门户现在还提供两个扫描接口：

- `GET /api/wifi/scan`：请求 WiFi 扫描；
- `GET /api/wifi/results`：轮询 WiFi 扫描状态和结果；
- `GET /api/ble/scan`：请求 BLE 扫描；
- `GET /api/ble/results`：轮询 BLE 扫描状态和结果。

网页中的 WiFi / 蓝牙结果区域支持上下滚动，点击 WiFi 结果会自动填入目标
SSID。WiFi 扫描最多向无线驱动请求 8 个结果，屏幕和网页的固定容量快照仍保留
12 个槽位。扫描时保持当前 Station 或 AP+Station 模式，不再通过 `set_config`
重启 WiFi，避免 UI 与 BLE 共存时模式切换触发 `OutOfMemory`；Station 接口关闭时
扫描会直接失败，需重新开启 WiFi 后再试。

WiFi / Bluetooth LE 由 `esp-radio 0.18`、`esp-rtos 0.3` 和 Embassy 网络栈
提供。无线栈必须在启动时先注册动态堆、启动 `esp_rtos` 调度器，再初始化
Radio；本工程使用 64 KiB reclaimed heap、36 KiB 内部 heap 和 Octal PSRAM
allocator。WiFi 与 BLE 同时启用时通过 `esp-radio` 的 `coex` feature 共存，
无线任务不访问 Slint API，符合 `unsafe-single-threaded` 的单线程约束。
主程序本身也是 Embassy 的异步任务，循环末尾必须使用
`embassy_time::Timer::after_millis(...).await` 主动让出调度器；不能在异步主任务中
使用阻塞式 `esp_hal::delay::Delay`。此前主循环一直占用 executor，导致
`wifi_controller`、DHCP 和 BLE 任务没有运行，表现为手机关联 SoftAP 后无法获得
IP、WiFi 扫描无结果。本工程现在已改为异步定时器。

### 屏幕无线配置窗口

右滑进入菜单后，`WiFi 配置` 和 `蓝牙配置` 都提供独立的触摸窗口：

- WiFi：启动 `esp-radio` 硬件扫描，Station 接口开启时保持当前 AP/STA 模式不变；
  每次扫描最多请求 8 个热点，10 秒超时。屏幕 WiFi 列表使用 `Flickable`，结果
  超过可视区域时可以上下滑动，选择热点后进入大尺寸可滚动密码键盘。
- 蓝牙：启动 TrouBLE Central 被动扫描，扫描窗口为 5 秒，最多保存 12 个广播
  设备及地址 / RSSI。被动扫描可发现不响应主动扫描请求的设备；选择设备后进入
  六位数字配对码窗口。
- WiFi / BLE 扫描与配对结果通过 `bsp/src/features/config.rs` 的固定容量快照在无线
  任务和 Slint 主循环之间传递，后台任务不直接访问 Slint 对象。
- 功能菜单、WiFi / BLE 列表和密码键盘均支持触摸上下滑动；菜单按钮、导航按钮和
  键盘按钮在按下期间会改变背景和边框颜色，提供明确的点击反馈。
- AP 侧使用 `embassy-net::udp::UdpSocket` 和 `edge-dhcp` 协议层直接提供 DHCP，
  地址池为 `192.168.4.50–192.168.4.200`，因此连接 SoftAP 的设备可以正常获得
  `192.168.4.x` 地址并访问 `http://192.168.4.1/`。
  运行时通过网页提交的 WiFi 凭据和 BLE bond 仍不写入 Flash。若需设备开机自动连接，
  编辑 `bsp/src/features/config.rs` 中的 `BOOT_WIFI_SSID` 与 `BOOT_WIFI_PASSWORD`，然后重新
  编译并烧录；SSID 为空时关闭开机自动连接。凭据会直接编译进固件，请勿将真实密码提交
  到公共仓库。

配置示例：

```rust
pub(crate) const BOOT_WIFI_SSID: &str = "your-wifi-name";
pub(crate) const BOOT_WIFI_PASSWORD: &str = "your-wifi-password";
```

BLE Security Manager 使用 ESP32-S3 的 `RNG + ADC1` 熵源初始化随机种子，
`bsp/src/board/mod.rs` 保留 `TrngSource` 的生命周期，直到主循环结束。

## 环境要求

需要安装：

- ESP Rust toolchain，并可使用 `cargo +esp`
- Xtensa ESP32-S3 target
- `espflash`
- 已连接并能识别为 `COM3` 的开发板

查看串口：

```powershell
espflash list-ports --list-all-ports
```

如果开发板使用其他串口号，将下面命令中的 `COM3` 替换为实际端口。

## 编译

进入本工程目录：

```powershell
cd E:\Code\ESP32\esp_slint
```

检查格式：

```powershell
cargo +esp fmt --check
```

构建 ESP32-S3 release 固件：

```powershell
cargo +esp build --release
```

`.cargo/config.toml` 已经设置默认目标为：

```text
xtensa-esp32s3-none-elf
```

因此不需要额外传递 `--target`。构建脚本会先把 `ui/main.slint` 编译为 Rust 代码，并使用 `EmbedForSoftwareRenderer` 嵌入 Slint 软件渲染资源。板级驱动也可以独立检查：

```powershell
cargo +esp check -p esp_slint_bsp
cargo +esp check -p esp_slint_bsp --features full
```

固件产物：

```text
target/xtensa-esp32s3-none-elf/release/esp_slint
```

## 烧录到 COM3

先完成编译，然后执行：

```powershell
espflash flash --skip-update-check --port COM3 --chip esp32s3 --flash-size 16mb target/xtensa-esp32s3-none-elf/release/esp_slint
```

烧录命令会：

1. 连接 `COM3`；
2. 检测 ESP32-S3 芯片和 Flash；
3. 生成并写入 ESP-IDF 应用镜像；
4. 校验 Flash 内容；
5. 默认执行硬复位启动新固件。

Windows 命令行中建议使用 `/` 作为固件路径分隔符，避免某些 shell 将反斜杠解释为转义字符。

烧录并打开串口监视器：

```powershell
espflash flash --skip-update-check --port COM3 --chip esp32s3 --flash-size 16mb --monitor target/xtensa-esp32s3-none-elf/release/esp_slint
```

固件启用 `esp-println` 日志。应用层和 BSP 层日志通过 `esp_println::println!` 直接输出到
`espflash monitor` 使用的 UART / USB JTAG 通道，日志行包含
`[INFO][esp_slint::...]` 或 `[INFO][esp_slint_bsp::...]` 等级和模块前缀。`esp_println::logger::init_logger_from_env()`
仍保留用于 `esp-radio` 等依赖的 `log` 日志；`ESP_LOG` 只过滤依赖日志，不会隐藏应用层的
ESP 直出日志。串口监视器可以观察启动、板级初始化、DHCP、WiFi 连接与扫描、BLE 配对与扫描、
RTC 和 UI 状态；如果需要重新编译、烧录或使用其他串口工具，先关闭占用 `COM3` 的监视器。

也可以使用 `.cargo/config.toml` 中的 runner：

```powershell
cargo +esp run --release
```

该方式会执行配置的 `espflash flash --monitor --flash-size 16mb`，没有固定端口时可能会要求选择串口。需要固定使用 `COM3` 时，优先使用上面的显式 `espflash flash --port COM3` 命令。

## 界面功能

当前 UI 由 `ui/main.slint` 统一导出 `MainWindow`，并拆分为以下模块：

- `ui/state.slint`：集中维护 `AppState` 全局状态；`MainWindow` 通过双向属性绑定保持
  Rust 端属性 API 稳定，Rust 回调统一注册到导出的 `AppState` 全局单例；
- `ui/components/controls.slint`：共享的 `MenuItem`、`NavButton` 和 `KeyButton`；
- `ui/pages/home.slint` 与 `docs/UI参考_360x360_clean.png`：按 360 × 360 参考图复刻的时钟主页,仅日期、时间和秒数由运行时覆盖;
- `ui/pages/menu.slint`、`touch.slint`、`motion.slint`、`performance.slint`：基础功能页；
- `ui/pages/wifi_*.slint`、`ble_*.slint`：WiFi 和蓝牙控制、扫描、输入页面；
- `ui/pages/menu_shell.slint`：页面路由和左右滑动返回逻辑。
- 所有可独立预览的页面继承 `PageFrame`：以 360 × 360 窗口为基准，使用 356 × 356
  内圆、178px 圆角和 3px 边框，保留 ST77916 圆形 LCD 的实际显示比例；
- 底部导航统一上移到 `y: 270px`；WiFi / BLE 控制页使用紧凑按钮行，
  列表和键盘页面缩短可视区后保留 `Flickable` 滚动，避免控件被圆形边缘裁剪；
- WiFi / BLE 扫描列表使用 `for item[index] in model: MenuItem` 重复器，Rust 将扫描结果
  写入 `wifi-networks` / `ble-devices` 及对应 detail 数组，运行时只创建当前模型行数，
  不再预渲染 12 个隐藏项；

页面通过 `if root.menu-view == n: PageComponent {}` 条件实例化。Slint 会在运行时只
创建当前页面，切换页面时释放旧页面；所有 `.slint` 定义仍会在构建时编译进固件。

当前 UI 提供一个 360 × 360 圆形极简时钟主页和右滑进入的功能菜单：

- 主页参考圆形时钟布局：顶部显示日期和星期，中部显示 `HH:MM`，底部圆形区域显示秒数；
- 秒数使用 30 个环形点按两秒一格显示，已过去的点为深色、剩余的点为橙色；
  秒数数字使用七段样式，并与环形点同步更新；
- 时间初始读取 PCF85063；Station 通过 DHCP 获取 IP 后，使用 DHCP 下发的 DNS
  解析 NTP 域名，再通过 UDP/123 校时，并将结果写回 PCF85063；
- NTP 会依次尝试配置的多个时间服务器；单个服务器超时不会阻塞其他服务器；
- NTP 校时成功后每小时重新同步；Station 未连接、未获得 DHCP 地址或 DNS 不可达时，
  保持 RTC 当前时间并在 30 秒后自动重试；
- RTC 内部保存 UTC；时间设置页默认使用 `UTC+8`，可按小时调整 `UTC-12` 至 `UTC+14`，
  修改后立即刷新主页的本地时间显示；
- NTP 请求需要网络出口和上游防火墙允许 UDP/123，并允许返回流量；DNS 查询成功不代表
  UDP/123 可用。如果日志能解析 NTP 域名并出现 `sending NTP request`，随后所有服务器
  都出现 `NTP response timed out`，应先在路由器、手机热点或企业网关放行 UDP/123。
- 从主页向右滑动：进入功能菜单；
- `触摸计数`：打开触摸计数页面，点击卡片累计触摸次数；
- `动态演示`：打开使用 `animation-tick()` 驱动轨道运动的动画页面；
- `性能监视`：打开实时渲染 FPS 页面；
- `清零计数`：清零触摸计数；
- `WiFi 控制`：显示 AP 和目标网络连接状态，提供 AP 开关、WiFi 开关和断开连接按钮；
- `蓝牙控制`：显示广播状态，提供蓝牙开关和扫描入口；
- `时间设置`：调整 UTC 偏移时间，恢复默认值后回到 `UTC+8`；
- 功能菜单、扫描列表和键盘使用 `Flickable` 处理溢出内容；所有按钮在按下时有
  颜色反馈；在菜单或功能页向左滑动仍可返回时钟主页。

触摸事件由 CST816S 轮询获取，再转换为 Slint 的：

- `PointerPressed`
- `PointerMoved`
- `PointerReleased`
- `PointerExited`

主循环同时记录触摸起点和释放位置：水平位移至少 60 像素且垂直偏移不超过 100 像素时，识别为右滑或左滑，并切换 `menu-open` 状态。

## 单页面预览

每个 `ui/pages/*.slint` 都导出了可直接预览的组件。页面不再依赖
`menu-view` 路由条件，Viewer 可以直接指定组件：

```powershell
cd E:\Code\ESP32
cargo run --release --manifest-path slint/tools/viewer/Cargo.toml -- `
  --component WifiListPage `
  --screenshot C:/Temp/wifi-list.png `
  esp_slint/ui/pages/wifi_list.slint
```

可替换的组件和文件包括：

```text
HomePage          pages/home.slint
MenuPage          pages/menu.slint
TouchPage         pages/touch.slint
MotionPage        pages/motion.slint
PerformancePage   pages/performance.slint
WifiControlPage   pages/wifi_control.slint
WifiListPage      pages/wifi_list.slint
WifiPasswordPage  pages/wifi_password.slint
BleControlPage    pages/ble_control.slint
BleScanPage       pages/ble_scan.slint
BlePairPage       pages/ble_pair.slint
SettingsPage       pages/settings.slint
```

需要预览扫描结果时，通过导出的 `AppState` 全局单例注入数据：

```json
{
  "AppState": {
    "wifi-scan-state": 3,
    "wifi-scan-status": "扫描完成：2 个网络",
    "wifi-network-count": 2,
    "wifi-networks": ["家庭 WiFi", "办公室网络"],
    "wifi-network-details": ["-42 dBm · 加密", "-67 dBm · 开放"],
    "ble-scan-state": 3,
    "ble-scan-status": "扫描完成：2 个设备",
    "ble-device-count": 2,
    "ble-devices": ["无线键盘", "传感器"],
    "ble-device-details": [
      "AA:BB:CC:DD:EE:01 · -51 dBm",
      "AA:BB:CC:DD:EE:02 · -72 dBm"
    ]
  }
}
```

然后追加：

```powershell
--load-data C:/Temp/wifi-preview.json
```

## 依赖说明

Slint 依赖被固定为 `1.16.0`：

```toml
slint = "=1.16.0"
slint-build = "=1.16.0"
```

当前 `esp` toolchain 使用 Rust 1.88。较新的 Slint 版本会解析到要求 Rust 1.89–1.92 的传递依赖，因此不能直接使用浮动版本。`Cargo.lock` 已保存经过验证的兼容依赖版本。

Slint 的关键 feature：

- `compat-1-2`
- `unsafe-single-threaded`
- `libm`
- `renderer-software`

`unsafe-single-threaded` 要求所有 Slint API 都在当前单线程主循环中调用；本工程的
WiFi、DHCP 和 BLE 任务只访问无线状态，不访问 Slint 对象。

无线依赖固定为 `esp-radio 0.18.0`、`esp-rtos 0.3.0`、`embassy-net 0.8.0`
和 `trouble-host 0.6.0`；日志使用 `esp-println 0.17.0`。DHCP 使用
`edge-dhcp 0.7.0` 的无默认特性协议层，不再引入 `edge-nal` /
`edge-nal-embassy` UDP 适配。TrouBLE 额外启用 `scan`、`security` 和 `log`
features，并直接依赖 `bt-hci 0.8` 以访问扫描命令约束。这些版本与当前 Rust
1.88 / `esp-hal = 1.1.1` 兼容。不要直接切换到要求 Rust 1.95 的
`esp-radio 1.0.0-beta.0` API。

## 已验证命令

以下命令已在当前工程通过：

```powershell
cargo +esp fmt --check
cargo +esp check
cargo +esp build --release
cargo +esp metadata --no-deps --format-version 1
espflash save-image --skip-update-check --merge --chip esp32s3 --flash-size 16mb --skip-padding target/xtensa-esp32s3-none-elf/release/esp_slint <output.bin>
```

本轮修复覆盖：

- WiFi AP 和 STA 开关、目标网络连接状态、手动断开和自动重连控制；
- 蓝牙广播开关；关闭时拒绝新的 BLE 扫描和配对请求；
- WiFi 扫描保持当前 Station 或 AP+Station 模式，不通过 `set_config` 重启无线驱动，
  避免 UI 与 BLE 共存时模式切换触发 `OutOfMemory`；
- 屏幕功能菜单、WiFi / BLE 结果列表和密码键盘的 `Flickable` 溢出滚动；
- 60px × 38px 大尺寸并排密码键，支持字母、数字、符号、大小写、空格和删除；
- `MenuItem`、`NavButton` 和 `KeyButton` 的按下颜色反馈。

以下命令已通过：

```powershell
cargo +esp fmt --check
cargo +esp check
cargo +esp build --release
```

发布镜像尺寸会随 UI 字体和页面内容变化，必须以 `espflash save-image` 输出为准。
软件渲染器预览已检查 WiFi 控制、WiFi 列表和蓝牙控制页面；当前字体文件包含
21,180 个字形，新增开关、连接状态、扫描列表和滑动提示中文字符均已编译。

WiFi 列表入口会在当前 Station 或 AP+Station 模式下直接扫描，不会暂时关闭 SoftAP。
AP 默认关闭；需要使用网页配置时，先在设备 `WiFi 控制` 页面开启 AP。列表使用
Slint `Flickable` 显示动态数量的结果，超出可视区域时上下滑动浏览。

本轮已通过本地 `cargo +esp fmt --check`、`cargo +esp check` 和
`cargo +esp build --release`。设备端运行时验证仍需可用的 COM3 串口。

开启 AP 后，手机或电脑连接开放网络 `ESP32-S3-配置`，访问
`http://192.168.4.1/`，点击“扫描 WiFi”即可轮询扫描结果。
