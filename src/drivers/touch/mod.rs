//! CST816S single-point touch driver for the ESP32-S3-Touch-LCD-1.85C board.

use esp_hal::{
    gpio::{Input, InputConfig, InputPin, Pull},
    i2c::master::{Error, I2c},
    Blocking,
};

pub const CST816S_ADDRESS: u8 = 0x15;
pub const TOUCH_WIDTH: u16 = 360;
pub const TOUCH_HEIGHT: u16 = 360;

const DATA_START_REG: u8 = 0x02;
const CHIP_ID_REG: u8 = 0xA7;
const AUTO_SLEEP_REG: u8 = 0xFE;
const POINT_NUM_MAX: u8 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TouchPoint {
    pub x: u16,
    pub y: u16,
}

pub struct Cst816Touch<'d> {
    i2c: I2c<'d, Blocking>,
    _interrupt: Input<'d>,
}

impl<'d> Cst816Touch<'d> {
    /// Creates a CST816S driver after the controller reset pulse is complete.
    pub fn new(
        mut i2c: I2c<'d, Blocking>,
        interrupt_pin: impl InputPin + 'd,
    ) -> Result<Self, Error> {
        // Match the ESP-IDF driver: verify the controller is present and disable
        // its automatic sleep mode before the application starts polling it.
        let mut chip_id = [0u8; 1];
        i2c.write_read(CST816S_ADDRESS, &[CHIP_ID_REG], &mut chip_id)?;
        crate::esp_debug!("TOUCH: CST816S chip id=0x{:02X}", chip_id[0]);
        i2c.write(CST816S_ADDRESS, &[AUTO_SLEEP_REG, 1])?;
        crate::esp_info!("TOUCH: automatic sleep disabled");

        let interrupt = Input::new(interrupt_pin, InputConfig::default().with_pull(Pull::Up));

        Ok(Self {
            i2c,
            _interrupt: interrupt,
        })
    }

    pub(crate) fn write_register(&mut self, address: u8, data: &[u8]) -> Result<(), Error> {
        self.i2c.write(address, data)
    }

    pub(crate) fn write_read_register(
        &mut self,
        address: u8,
        register: &[u8],
        data: &mut [u8],
    ) -> Result<(), Error> {
        self.i2c.write_read(address, register, data)
    }

    /// Reads the first CST816S contact, if one is currently present.
    ///
    /// The ESP-IDF driver reads register `0x02` and limits the controller's
    /// reported point count to one. The same wire format is used here:
    /// `[points, x_high, x_low, y_high, y_low]`.
    pub fn read(&mut self) -> Result<Option<TouchPoint>, Error> {
        let mut data = [0u8; 5];
        self.i2c
            .write_read(CST816S_ADDRESS, &[DATA_START_REG], &mut data)?;

        let point_count = data[0].min(POINT_NUM_MAX);
        if point_count == 0 {
            return Ok(None);
        }

        let x = (((data[1] & 0x0F) as u16) << 8) | data[2] as u16;
        let y = (((data[3] & 0x0F) as u16) << 8) | data[4] as u16;

        if x >= TOUCH_WIDTH || y >= TOUCH_HEIGHT {
            return Ok(None);
        }

        Ok(Some(TouchPoint { x, y }))
    }
}
