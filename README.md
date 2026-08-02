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

| 文件 | 作用 |
| --- | --- |
| `src/bin/main.rs` | 应用组合根、主循环和 Slint 窗口生命周期 |
| `src/board/mod.rs` | 开发板外设所有权、PSRAM、RTOS 和功能启动编排 |
| `src/drivers/display/mod.rs` | ST77916 QSPI 驱动和 Slint 行缓冲输出 |
| `src/drivers/display/panel_init.rs` | ST77916 vendor 初始化命令流 |
| `src/drivers/touch/mod.rs` | CST816S 触摸驱动和共享 I2C 事务 |
| `src/drivers/rtc.rs` | PCF85063 RTC 初始化、BCD 解码和日期时间读取 |
| `src/drivers/tca9554.rs` | TCA9554PWR I2C GPIO 扩展器和 LCD / 触摸复位线 |
| `src/features/wifi_portal.rs` | WiFi SoftAP、DHCP、HTTP 配置门户 |
| `src/features/bluetooth.rs` | BLE 广播和 GATT 服务 |
| `src/features/config.rs` | WiFi / BLE 功能之间的运行时配置状态 |
| `src/ui_logic/clock.rs` | 时钟页面数据和 RTC 刷新逻辑 |
| `src/ui_logic/input.rs` | 触摸轮询、Slint 事件转换和滑动识别 |
| `src/ui/platform.rs` | Slint Platform 和软件渲染行缓冲适配 |
| `ui/main.slint` | Slint 声明式界面 |
| `ui/fonts/NotoSansSC-UI.ttf` | 从 Noto Sans SC 子集化的嵌入式中文字体 |
| `ui/fonts/OFL.txt` | 字体许可证 |
| `build.rs` | 编译 `.slint` 文件并嵌入软件渲染资源 |
| `.cargo/config.toml` | Xtensa 目标、`build-std` 和 `espflash` 配置 |
| `Cargo.lock` | 与当前 `esp` Rust toolchain 兼容的依赖锁定版本 |

代码按四层组织：

```text
board
  ├── drivers       LCD / touch / RTC / I2C 外设协议
  ├── features      WiFi 配网 / BLE 广播 / 共享配置
  ├── ui_logic      时钟状态 / 触摸输入 / 页面行为
  └── ui            Slint Platform / 行缓冲渲染适配
```

`src/bin/main.rs` 只负责组合这些层，不再直接包含 LCD、触摸、RTC、网络或
Slint 渲染实现。

`esp_learn` 与 `esp_slint` 仍分别维护自己的 LCD vendor 初始化表：

- `esp_learn/src/panel_init.rs`
- `esp_slint/src/drivers/display/panel_init.rs`

`esp_slint/src/drivers/touch/mod.rs` 现在维护 Slint 工程自己的 CST816S 驱动副本，
避免入口文件通过 `#[path]` 依赖另一个工程的目录结构。PCF85063 与 CST816S
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

`src/drivers/display/panel_init.rs` 使用紧凑二进制命令流保存初始化表。每条记录
包含命令字节、数据长度标志、参数数据，以及可选的 little-endian 延时值；运行时
由迭代器解码，不再为每条命令保存独立的 slice 指针和结构体元数据。

嵌入式软件渲染器不能依赖电脑的系统字体。中文界面使用 `Noto Sans SC` 的
UI 字符子集，并在 `ui/main.slint` 中设置为 `default-font-family`。`build.rs`
显式监听字体文件变化，Slint 编译时会把字体字形表嵌入固件；因此开发板和
桌面预览使用同一套中文字形，不再出现中文方框或空白。

## 硬件连接

| 信号 | 配置 |
| --- | --- |
| I2C SDA / SCL | GPIO11 / GPIO10 |
| TCA9554PWR | I2C 地址 `0x20` |
| LCD reset | TCA9554PWR EXIO2 |
| Touch reset | TCA9554PWR EXIO1 |
| LCD QSPI SCK | GPIO40 |
| LCD QSPI D0 / D1 / D2 / D3 | GPIO46 / GPIO45 / GPIO42 / GPIO41 |
| LCD CS | GPIO21 |
| LCD backlight | GPIO5 |
| CST816S | I2C 地址 `0x15` |
| PCF85063 | I2C 地址 `0x51` |
| CST816S INT | GPIO4 |
| LCD 分辨率 | 360 × 360 |

## Waveshare 官方硬件摘要

本工程对应 Waveshare `ESP32-S3-Touch-LCD-1.85C`。官方页面：
<https://docs.waveshare.net/ESP32-S3-Touch-LCD-1.85C/>

| 模块 | 官方规格 / 本工程用法 |
| --- | --- |
| 主控 | ESP32-S3，双核 Xtensa LX7，最高 240 MHz |
| 存储 | 16 MB Flash、8 MB Octal PSRAM；PSRAM 同时承载 Slint 和无线动态分配 |
| 圆形屏幕 | 1.85 英寸、360 × 360、ST77916、QSPI、RGB565 |
| 触摸 | CST816S 电容触摸，I2C 地址 `0x15`，中断 GPIO4 |
| RTC | PCF85063，I2C 地址 `0x51` |
| IO 扩展 | TCA9554PWR，I2C 地址 `0x20`；LCD 复位 EXIO2，触摸复位 EXIO1 |
| 无线 | 2.4 GHz WiFi 与 Bluetooth LE；本工程使用 `esp-radio` |
| 总线 | I2C GPIO11/GPIO10；LCD QSPI GPIO40、GPIO46/45/42/41、CS GPIO21；背光 GPIO5 |

### 板载 WiFi / 蓝牙配置门户

固件启动后会创建一个用于配网的 SoftAP：

| 项目 | 值 |
| --- | --- |
| WiFi 名称 | `ESP32-S3-配置` |
| WiFi 密码 | `esp32s3-config` |
| 配置地址 | `http://192.168.4.1/` |

使用手机或电脑连接配置网络，然后在浏览器打开配置地址。网页可以提交目标
WiFi 名称、密码、蓝牙广播名称以及蓝牙开关。WiFi 提交后，设备在后台尝试
连接目标网络；蓝牙名称在下一次广播周期使用。当前配置保存在运行内存中，
重新上电后需要再次配置。

WiFi / Bluetooth LE 由 `esp-radio 0.18`、`esp-rtos 0.3` 和 Embassy 网络栈
提供。无线栈必须在启动时先注册动态堆、启动 `esp_rtos` 调度器，再初始化
Radio；本工程使用 64 KiB reclaimed heap、36 KiB 内部 heap 和 Octal PSRAM
allocator。WiFi 与 BLE 同时启用时通过 `esp-radio` 的 `coex` feature 共存，
无线任务不访问 Slint API，符合 `unsafe-single-threaded` 的单线程约束。

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

因此不需要额外传递 `--target`。构建脚本会先把 `ui/main.slint` 编译为 Rust 代码，并使用 `EmbedForSoftwareRenderer` 嵌入 Slint 软件渲染资源。

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

本工程当前没有启用串口日志，监视器主要用于观察启动时的串口输出；如果需要重新编译、烧录或使用其他串口工具，先关闭占用 `COM3` 的监视器。

也可以使用 `.cargo/config.toml` 中的 runner：

```powershell
cargo +esp run --release
```

该方式会执行配置的 `espflash flash --monitor --flash-size 16mb`，没有固定端口时可能会要求选择串口。需要固定使用 `COM3` 时，优先使用上面的显式 `espflash flash --port COM3` 命令。

## 界面功能

当前 `ui/main.slint` 提供一个 360 × 360 圆形时钟主页和右滑进入的功能菜单：

- 主页：显示 PCF85063 提供的 `HH:MM:SS` 和 `YYYY-MM-DD`；
- 从主页向右滑动：进入功能菜单；
- `触摸计数`：打开触摸计数页面，点击卡片累计触摸次数；
- `动态演示`：打开使用 `animation-tick()` 驱动轨道运动的动画页面；
- `性能监视`：打开实时渲染 FPS 页面；
- `清零计数`：清零触摸计数；
- `WiFi 配置`：显示板载配置网络、密码和网页地址；
- `蓝牙配置`：显示蓝牙配置网页和广播生效说明；
- 在菜单或功能页向左滑动：返回时钟主页；页面按钮可返回功能菜单。

触摸事件由 CST816S 轮询获取，再转换为 Slint 的：

- `PointerPressed`
- `PointerMoved`
- `PointerReleased`
- `PointerExited`

主循环同时记录触摸起点和释放位置：水平位移至少 60 像素且垂直偏移不超过 100 像素时，识别为右滑或左滑，并切换 `menu-open` 状态。

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

无线依赖固定为 `esp-radio = 0.18.0`、`esp-rtos = 0.3.0`、`embassy-net = 0.8.0`
和 `trouble-host = 0.6.0`，这些版本与当前 Rust 1.88 / `esp-hal = 1.1.1`
兼容。不要直接切换到要求 Rust 1.95 的 `esp-radio 1.0.0-beta.0` API。

## 已验证命令

以下命令已在当前工程通过：

```powershell
cargo +esp fmt --check
cargo +esp build --release
cargo +esp metadata --no-deps --format-version 1
```

本次已修复中文字体缺失，完成中文时钟主页、六项功能菜单、板载 HTTP 配置
门户以及 WiFi / BLE 无线初始化。已通过格式检查、ESP32-S3 release 固件构建、
Slint 语法检查和软件渲染器截图验证；尚未重新烧录到 COM3。
