//! 开发板级启动编排。
//!
//! 这里负责一次性资源的所有权分配：PSRAM、RTOS、无线功能和屏幕所需的
//! 外设实例。具体寄存器和协议实现位于 `drivers`，业务任务位于 `features`。

use embassy_executor::Spawner;
use esp_hal::{
    interrupt::software::SoftwareInterruptControl, peripherals::Peripherals,
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
) -> (St77916Display<'static>, Cst816Touch<'static>) {
    // Slint and the radio services use dynamic allocation. Register PSRAM before
    // starting any RTOS task so every service sees the complete heap.
    esp_alloc::psram_allocator!(
        peripherals.PSRAM,
        esp_hal::psram,
        esp_hal::psram::PsramConfig {
            mode: esp_hal::psram::PsramMode::OctalSpi,
            ..Default::default()
        }
    );

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    features::wifi_portal::start(spawner, peripherals.WIFI);
    features::bluetooth::start(spawner, peripherals.BT);

    display::init(DisplayPeripherals {
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
    })
}
