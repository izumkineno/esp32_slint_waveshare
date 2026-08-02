# ESP32-S3 Slint 示例

本工程基于 `esp_learn` 的 ESP32-S3-Touch-LCD-1.85C 硬件初始化代码，使用 [Slint](https://slint.dev/) 的 `no_std` 软件渲染器替换 `embedded-graphics` 作为 GUI。

目标硬件：

- ESP32-S3-Touch-LCD-1.85C
- ST77916 360 × 360 QSPI 圆形 LCD
- CST816S 电容触摸控制器
- TCA9554PWR IO 扩展器
- 16 MB Flash、Octal PSRAM

## 工程结构

| 文件 | 作用 |
| --- | --- |
| `src/bin/main.rs` | ESP32-S3 入口、Slint Platform、触摸事件循环 |
| `src/st77916.rs` | ST77916 QSPI 驱动和 Slint 行缓冲输出 |
| `src/panel_init.rs` | 压缩后的 ST77916 vendor 初始化命令流 |
| `ui/main.slint` | Slint 声明式界面 |
| `build.rs` | 编译 `.slint` 文件并嵌入软件渲染资源 |
| `.cargo/config.toml` | Xtensa 目标、`build-std` 和 `espflash` 配置 |
| `Cargo.lock` | 与当前 `esp` Rust toolchain 兼容的依赖锁定版本 |

为避免 LCD vendor 初始化表跨工程共享，两个工程分别维护自己的本地文件：

- `esp_learn/src/panel_init.rs`
- `esp_slint/src/panel_init.rs`

Slint 工程仍复用 `esp_learn/src/cst816.rs` 作为 CST816S 触摸驱动，因此两个工程仍需要位于同一个仓库目录下。

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

`src/panel_init.rs` 使用紧凑二进制命令流保存初始化表。每条记录包含命令字节、数据长度标志、参数数据，以及可选的 little-endian 延时值；运行时由迭代器解码，不再为每条命令保存独立的 slice 指针和结构体元数据。

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
| CST816S INT | GPIO4 |
| LCD 分辨率 | 360 × 360 |

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

当前 `ui/main.slint` 提供一个 360 × 360 圆形控制界面：

- `Touch dashboard`：触摸中央信息卡片时增加计数；
- `Touch count`：显示累计触摸次数；
- `Touch received`：显示最近一次触摸状态；
- `CLEAR`：清零计数并恢复 `Ready` 状态。
- `FPS`：顶部徽标显示最近约 1 秒内完成的 Slint 渲染帧数；
- `ANIMATE`：切换到使用 `animation-tick()` 驱动轨道运动的小动画页面；
- `BACK`：从动画页面返回控制面板。

触摸事件由 CST816S 轮询获取，再转换为 Slint 的：

- `PointerPressed`
- `PointerMoved`
- `PointerReleased`
- `PointerExited`

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

`unsafe-single-threaded` 要求所有 Slint API 都在当前单线程主循环中调用；本工程没有在中断或其他线程中访问 Slint。

## 已验证命令

以下命令已在当前工程通过：

```powershell
cargo +esp fmt --check
cargo +esp build --release
cargo +esp metadata --no-deps --format-version 1
```

固件已实际烧录到 `COM3`，ESP32-S3 识别正常，Flash 大小为 16 MB，烧录完成后运行正常。
