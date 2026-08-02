//! TCA9554PWR I2C GPIO expander used for board reset lines.

use esp_hal::{i2c::master::I2c, Blocking};

pub const ADDRESS: u8 = 0x20;
pub const LCD_RESET_BIT: u8 = 1 << 1;
pub const TOUCH_RESET_BIT: u8 = 1 << 0;

const OUTPUT_REG: u8 = 0x01;
const CONFIG_REG: u8 = 0x03;

pub fn configure(i2c: &mut I2c<'_, Blocking>) {
    i2c.write(ADDRESS, &[CONFIG_REG, 0x00]).unwrap();
    i2c.write(ADDRESS, &[OUTPUT_REG, 0x00]).unwrap();
}

pub fn write_output(i2c: &mut I2c<'_, Blocking>, value: u8) {
    i2c.write(ADDRESS, &[OUTPUT_REG, value]).unwrap();
}
