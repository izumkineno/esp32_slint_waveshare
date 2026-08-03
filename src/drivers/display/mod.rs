//! ST77916 QSPI display driver adapted for Slint line rendering.

#[path = "panel_init.rs"]
mod panel_init;

use crate::drivers::{tca9554, touch::Cst816Touch};
use core::ops::Range;
use esp_hal::{
    delay::Delay,
    gpio::{Level, Output, OutputConfig},
    i2c::master::{Config as I2cConfig, I2c},
    peripherals::{
        GPIO10, GPIO11, GPIO21, GPIO4, GPIO40, GPIO41, GPIO42, GPIO45, GPIO46, GPIO5, I2C0, SPI2,
    },
    spi::master::{Address, Command, Config as SpiConfig, DataMode, Spi},
    time::Rate,
    Blocking,
};
use slint::platform::software_renderer::Rgb565Pixel;

pub const LCD_WIDTH: usize = 360;
pub const LCD_HEIGHT: usize = 360;

const PIXELS_PER_SPI_CHUNK: usize = 32;

const LCD_OPCODE_WRITE_COMMAND: u8 = 0x02;
const LCD_OPCODE_READ_COMMAND: u8 = 0x0B;
const LCD_OPCODE_WRITE_COLOR: u8 = 0x32;

pub struct St77916Display<'d> {
    spi: Spi<'d, Blocking>,
    cs: Output<'d>,
    _backlight: Output<'d>,
}

pub struct DisplayPeripherals<'d> {
    pub i2c0: I2C0<'d>,
    pub gpio11: GPIO11<'d>,
    pub gpio10: GPIO10<'d>,
    pub spi2: SPI2<'d>,
    pub gpio40: GPIO40<'d>,
    pub gpio46: GPIO46<'d>,
    pub gpio45: GPIO45<'d>,
    pub gpio42: GPIO42<'d>,
    pub gpio41: GPIO41<'d>,
    pub gpio21: GPIO21<'d>,
    pub gpio5: GPIO5<'d>,
    pub gpio4: GPIO4<'d>,
}

pub fn init(parts: DisplayPeripherals<'static>) -> (St77916Display<'static>, Cst816Touch<'static>) {
    let delay = Delay::new();
    crate::esp_info!("DISPLAY: initializing I2C, touch, and ST77916 panel");

    let mut i2c = I2c::new(
        parts.i2c0,
        I2cConfig::default().with_frequency(Rate::from_khz(400)),
    )
    .unwrap()
    .with_sda(parts.gpio11)
    .with_scl(parts.gpio10);

    tca9554::configure(&mut i2c);
    crate::esp_debug!("DISPLAY: TCA9554 reset controller configured");

    // The LCD reset is controlled by TCA9554PWR EXIO2.
    delay.delay_millis(10);
    tca9554::write_output(&mut i2c, tca9554::LCD_RESET_BIT);
    delay.delay_millis(50);

    let spi = Spi::new(
        parts.spi2,
        // Use the low clock while probing the panel ID, then switch to 40 MHz.
        SpiConfig::default().with_frequency(Rate::from_khz(3_000)),
    )
    .unwrap()
    .with_sck(parts.gpio40)
    .with_sio0(parts.gpio46)
    .with_sio1(parts.gpio45)
    .with_sio2(parts.gpio42)
    .with_sio3(parts.gpio41);

    let cs = Output::new(parts.gpio21, Level::High, OutputConfig::default());
    let backlight = Output::new(parts.gpio5, Level::High, OutputConfig::default());

    let mut display = St77916Display {
        spi,
        cs,
        _backlight: backlight,
    };

    display.initialize_panel(&delay).unwrap();
    crate::esp_info!("DISPLAY: ST77916 panel initialized");

    // Keep LCD reset released while pulsing the touch reset on EXIO1.
    tca9554::write_output(&mut i2c, tca9554::LCD_RESET_BIT);
    delay.delay_millis(10);
    tca9554::write_output(&mut i2c, tca9554::LCD_RESET_BIT | tca9554::TOUCH_RESET_BIT);
    delay.delay_millis(50);

    let touch = Cst816Touch::new(i2c, parts.gpio4).unwrap();
    crate::esp_info!("DISPLAY: CST816S touch controller initialized");
    (display, touch)
}

impl St77916Display<'_> {
    fn qspi_address(opcode: u8, command: u8, mode: DataMode) -> Address {
        Address::_32Bit(((opcode as u32) << 24) | ((command as u32) << 8), mode)
    }

    fn send_command(&mut self, command: u8, data: &[u8]) -> Result<(), esp_hal::spi::Error> {
        self.cs.set_low();
        let result = self.spi.half_duplex_write(
            // ST77916 command and parameter phases use one data line.
            DataMode::SingleTwoDataLines,
            Command::None,
            Self::qspi_address(
                LCD_OPCODE_WRITE_COMMAND,
                command,
                DataMode::SingleTwoDataLines,
            ),
            0,
            data,
        );
        self.cs.set_high();
        result
    }

    fn read_register(&mut self, register: u8, data: &mut [u8]) -> Result<(), esp_hal::spi::Error> {
        self.cs.set_low();
        let result = self.spi.half_duplex_read(
            DataMode::SingleTwoDataLines,
            Command::None,
            Self::qspi_address(
                LCD_OPCODE_READ_COMMAND,
                register,
                DataMode::SingleTwoDataLines,
            ),
            0,
            data,
        );
        self.cs.set_high();
        result
    }

    fn initialize_panel(&mut self, delay: &Delay) -> Result<(), esp_hal::spi::Error> {
        let mut register_data = [0u8; 4];
        let command_stream = if self.read_register(0x04, &mut register_data).is_ok()
            && register_data == [0x00, 0x02, 0x7F, 0x7F]
        {
            panel_init::NEW
        } else {
            panel_init::DEFAULT
        };

        self.send_command(0x01, &[])?;
        delay.delay_millis(120);
        self.spi
            .apply_config(&SpiConfig::default().with_frequency(Rate::from_khz(40_000)))
            .unwrap();

        self.send_command(0x36, &[0x00])?;
        self.send_command(0x3A, &[0x55])?;

        for init_command in panel_init::iter(command_stream) {
            self.send_command(init_command.command, init_command.data)?;
            if init_command.delay_ms != 0 {
                delay.delay_millis(init_command.delay_ms as u32);
            }
        }

        self.send_command(0x29, &[])?;
        crate::esp_info!("DISPLAY: panel command stream completed");
        Ok(())
    }

    fn set_window(
        &mut self,
        x_start: u16,
        y_start: u16,
        x_end: u16,
        y_end: u16,
    ) -> Result<(), esp_hal::spi::Error> {
        self.send_command(
            0x2A,
            &[
                (x_start >> 8) as u8,
                x_start as u8,
                (x_end >> 8) as u8,
                x_end as u8,
            ],
        )?;
        self.send_command(
            0x2B,
            &[
                (y_start >> 8) as u8,
                y_start as u8,
                (y_end >> 8) as u8,
                y_end as u8,
            ],
        )
    }

    /// Sends one rendered Slint line to the LCD.
    pub fn write_line(
        &mut self,
        line: usize,
        range: Range<usize>,
        pixels: &[Rgb565Pixel],
    ) -> Result<(), esp_hal::spi::Error> {
        if line >= LCD_HEIGHT
            || range.start >= range.end
            || range.end > LCD_WIDTH
            || pixels.len() != range.len()
        {
            return Ok(());
        }

        self.set_window(
            range.start as u16,
            line as u16,
            (range.end - 1) as u16,
            line as u16,
        )?;

        self.cs.set_low();
        let command_result = self.spi.half_duplex_write(
            DataMode::SingleTwoDataLines,
            Command::None,
            Self::qspi_address(LCD_OPCODE_WRITE_COLOR, 0x2C, DataMode::SingleTwoDataLines),
            0,
            &[],
        );
        if let Err(error) = command_result {
            crate::esp_warn!("DISPLAY: color command failed: {:?}", error);
            self.cs.set_high();
            return Err(error);
        }

        for chunk in pixels.chunks(PIXELS_PER_SPI_CHUNK) {
            let mut bytes = [0u8; PIXELS_PER_SPI_CHUNK * 2];
            for (index, pixel) in chunk.iter().enumerate() {
                let raw = pixel.0.to_be_bytes();
                bytes[index * 2] = raw[0];
                bytes[index * 2 + 1] = raw[1];
            }

            if let Err(error) = self.spi.half_duplex_write(
                DataMode::Quad,
                Command::None,
                Address::None,
                0,
                &bytes[..chunk.len() * 2],
            ) {
                crate::esp_warn!("DISPLAY: pixel write failed: {:?}", error);
                self.cs.set_high();
                return Err(error);
            }
        }

        self.cs.set_high();
        Ok(())
    }
}
