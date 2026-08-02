//! PCF85063 RTC driver for the ESP32-S3-Touch-LCD-1.85C board.
//!
//! The RTC shares the board I2C bus with the CST816S controller. The touch
//! driver owns that bus, so this module uses its small register-transaction
//! adapter instead of creating a second I2C peripheral instance.

use crate::drivers::touch::Cst816Touch;
use esp_hal::i2c::master::Error;

pub const PCF85063_ADDRESS: u8 = 0x51;

const CONTROL_1_REG: u8 = 0x00;
const CONTROL_1_CAP_SEL: u8 = 0x01;
const TIME_START_REG: u8 = 0x04;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub weekday: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl DateTime {
    pub fn is_valid(self) -> bool {
        self.year >= 1970
            && (1..=12).contains(&self.month)
            && (1..=31).contains(&self.day)
            && self.weekday < 7
            && self.hour < 24
            && self.minute < 60
            && self.second < 60
    }
}

pub fn init(touch: &mut Cst816Touch<'_>) -> Result<(), Error> {
    touch.write_register(PCF85063_ADDRESS, &[CONTROL_1_REG, CONTROL_1_CAP_SEL])
}

pub fn read_time(touch: &mut Cst816Touch<'_>) -> Result<DateTime, Error> {
    let mut data = [0u8; 7];
    touch.write_read_register(PCF85063_ADDRESS, &[TIME_START_REG], &mut data)?;

    Ok(DateTime {
        second: bcd_to_dec(data[0] & 0x7f),
        minute: bcd_to_dec(data[1] & 0x7f),
        hour: bcd_to_dec(data[2] & 0x3f),
        day: bcd_to_dec(data[3] & 0x3f),
        weekday: bcd_to_dec(data[4] & 0x07),
        month: bcd_to_dec(data[5] & 0x1f),
        year: 1970 + u16::from(bcd_to_dec(data[6])),
    })
}

fn bcd_to_dec(value: u8) -> u8 {
    (value >> 4) * 10 + (value & 0x0f)
}
