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

    pub fn from_unix_seconds(timestamp: u64) -> Option<Self> {
        const SECONDS_PER_DAY: u64 = 86_400;

        let mut days = timestamp / SECONDS_PER_DAY;
        let seconds_today = timestamp % SECONDS_PER_DAY;
        let mut year = 1970u16;

        while days >= u64::from(days_in_year(year)) {
            days -= u64::from(days_in_year(year));
            year = year.checked_add(1)?;
            if year > 2069 {
                return None;
            }
        }

        let mut month = 1u8;
        while days >= u64::from(days_in_month(year, month)) {
            days -= u64::from(days_in_month(year, month));
            month += 1;
        }

        Some(Self {
            year,
            month,
            day: days as u8 + 1,
            weekday: ((timestamp / SECONDS_PER_DAY + 4) % 7) as u8,
            hour: (seconds_today / 3_600) as u8,
            minute: ((seconds_today % 3_600) / 60) as u8,
            second: (seconds_today % 60) as u8,
        })
    }
    pub fn to_unix_seconds(self) -> Option<u64> {
        if !self.is_valid() || self.year > 2069 {
            return None;
        }

        const SECONDS_PER_DAY: u64 = 86_400;
        let mut days = 0u64;
        let mut year = 1970u16;
        while year < self.year {
            days += u64::from(days_in_year(year));
            year += 1;
        }

        let mut month = 1u8;
        while month < self.month {
            days += u64::from(days_in_month(self.year, month));
            month += 1;
        }
        days += u64::from(self.day - 1);

        days.checked_mul(SECONDS_PER_DAY)?
            .checked_add(u64::from(self.hour) * 3_600)?
            .checked_add(u64::from(self.minute) * 60)?
            .checked_add(u64::from(self.second))
    }

    pub fn with_utc_offset(self, offset_hours: i8) -> Option<Self> {
        let timestamp = self.to_unix_seconds()?;
        let offset_seconds = i64::from(offset_hours) * 3_600;
        let adjusted = if offset_seconds >= 0 {
            timestamp.checked_add(offset_seconds as u64)?
        } else {
            timestamp.checked_sub((-offset_seconds) as u64)?
        };
        Self::from_unix_seconds(adjusted)
    }
}

pub fn init(touch: &mut Cst816Touch<'_>) -> Result<(), Error> {
    match touch.write_register(PCF85063_ADDRESS, &[CONTROL_1_REG, CONTROL_1_CAP_SEL]) {
        Ok(()) => {
            crate::esp_info!(
                "RTC: PCF85063 initialized at address 0x{:02X}",
                PCF85063_ADDRESS
            );
            Ok(())
        }
        Err(error) => {
            crate::esp_warn!("RTC: initialization failed: {:?}", error);
            Err(error)
        }
    }
}

pub fn read_time(touch: &mut Cst816Touch<'_>) -> Result<DateTime, Error> {
    let mut data = [0u8; 7];
    if let Err(error) = touch.write_read_register(PCF85063_ADDRESS, &[TIME_START_REG], &mut data) {
        crate::esp_warn!("RTC: time read failed: {:?}", error);
        return Err(error);
    }

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

pub fn write_time(touch: &mut Cst816Touch<'_>, datetime: DateTime) -> Result<(), Error> {
    let year = datetime.year.saturating_sub(1970) as u8;
    let data = [
        TIME_START_REG,
        dec_to_bcd(datetime.second),
        dec_to_bcd(datetime.minute),
        dec_to_bcd(datetime.hour),
        dec_to_bcd(datetime.day),
        dec_to_bcd(datetime.weekday),
        dec_to_bcd(datetime.month),
        dec_to_bcd(year),
    ];
    crate::esp_info!(
        "RTC: writing time {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        datetime.year,
        datetime.month,
        datetime.day,
        datetime.hour,
        datetime.minute,
        datetime.second
    );
    match touch.write_register(PCF85063_ADDRESS, &data) {
        Ok(()) => {
            crate::esp_info!("RTC: time write completed");
            Ok(())
        }
        Err(error) => {
            crate::esp_warn!("RTC: time write failed: {:?}", error);
            Err(error)
        }
    }
}

fn bcd_to_dec(value: u8) -> u8 {
    (value >> 4) * 10 + (value & 0x0f)
}

fn dec_to_bcd(value: u8) -> u8 {
    ((value / 10) << 4) | (value % 10)
}

fn is_leap_year(year: u16) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_year(year: u16) -> u16 {
    if is_leap_year(year) {
        366
    } else {
        365
    }
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 30,
    }
}
