//! 开发板级启动编排。
//!
//! 这里负责一次性资源的所有权分配：PSRAM、RTOS、无线功能和屏幕所需的
//! 外设实例。具体寄存器和协议实现位于 `drivers`，业务任务位于 `features`。

use embassy_executor::Spawner;
use esp_hal::{
    interrupt::software::SoftwareInterruptControl, peripherals::Peripherals, ram,
    timer::timg::TimerGroup,
};

use crate::{
    drivers::{
        display::{self, DisplayPeripherals, St77916Display},
        touch::Cst816Touch,
    },
    features,
};

pub fn init(
    peripherals: Peripherals,
    spawner: Spawner,
) -> (
    St77916Display<'static>,
    Cst816Touch<'static>,
    esp_hal::rng::TrngSource<'static>,
) {
    crate::esp_info!("BOARD: initializing ESP32-S3 peripherals");

    // 堆区注册顺序决定全局分配器(空 caps)的 first-fit 落点。
    // 必须先注册 PSRAM(External),让应用侧(Slint / String / Vec)优先用 8MB 外部内存;
    // 100KB 内部 DRAM 全部留给 esp-radio 的 malloc_internal 与 WiFi/BLE DMA(它们只能用 Internal)。
    // 顺序错了会导致应用吃光内部 RAM,WiFi 连接/发包报 ESP_ERR_NO_MEM(257)。
    esp_alloc::psram_allocator!(
        peripherals.PSRAM,
        esp_hal::psram,
        esp_hal::psram::PsramConfig {
            mode: esp_hal::psram::PsramMode::OctalSpi,
            ..Default::default()
        }
    );
    crate::esp_info!("BOARD: PSRAM allocator initialized (registered first)");

    // 从 bootloader 回收的 RAM 与常规 DRAM,均为 Internal,供无线栈独占。
    esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);
    crate::esp_info!(
        "BOARD: internal heap registered, free_internal={} bytes",
        esp_alloc::HEAP.free_caps(esp_alloc::MemoryCapability::Internal.into())
    );

    let trng_source = esp_hal::rng::TrngSource::new(peripherals.RNG, peripherals.ADC1);
    crate::esp_debug!("BOARD: TRNG source acquired");

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);
    crate::esp_info!("BOARD: Embassy RTOS started");

    features::wifi_portal::start(spawner, peripherals.WIFI);
    features::bluetooth::start(spawner, peripherals.BT);
    crate::esp_info!("BOARD: WiFi and BLE tasks spawned");

    let (display, touch) = display::init(DisplayPeripherals {
        i2c0: peripherals.I2C0,
        gpio11: peripherals.GPIO11,
        gpio10: peripherals.GPIO10,
        spi2: peripherals.SPI2,
        gpio40: peripherals.GPIO40,
        gpio46: peripherals.GPIO46,
        gpio45: peripherals.GPIO45,
        gpio42: peripherals.GPIO42,
        gpio41: peripherals.GPIO41,
        gpio21: peripherals.GPIO21,
        gpio5: peripherals.GPIO5,
        gpio4: peripherals.GPIO4,
    });
    crate::esp_info!("BOARD: LCD and CST816S touch drivers initialized");
    (display, touch, trng_source)
}
