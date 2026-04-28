// Copyright 2026 l5y
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! I²C / HD44780 hardware adapters and LCD initialisation for the binary.
//!
//! Lives outside the library so the pure formatting helpers in `lcd_odroid`
//! remain hardware-free and unit-testable.

use std::time::Duration;

use embedded_hal::blocking::delay::{DelayMs, DelayUs};
use embedded_hal::blocking::i2c::Write as I2cWrite02;
use embedded_hal_1::i2c::I2c as _;
use hd44780_driver::{Cursor, CursorBlink, Display, DisplayMode, HD44780, bus::DataBus};
use lcd_odroid::{LcdDisplay, info};
use linux_embedded_hal::{I2CError, I2cdev};

/// Default Linux I²C bus device path. Override with `i2c_bus` in `config.toml`.
pub const I2C_BUS_DEFAULT: &str = "/dev/i2c-0";
/// Default HD44780 backpack address on the I²C bus. Override with `i2c_addr`
/// in `config.toml`.
pub const I2C_ADDR_DEFAULT: u8 = 0x27;

/// Adapter that wraps [`linux_embedded_hal::I2cdev`] and re-exposes it through
/// the embedded-hal 0.2 [`I2cWrite02`] trait that `hd44780-driver` 0.4 requires.
///
/// `linux-embedded-hal` 0.4 dropped its embedded-hal 0.2 impls when it migrated
/// to 1.0; this shim forwards each 0.2 `write` call to the underlying 1.0 `I2c`
/// implementation so the existing driver keeps working unchanged.
pub struct I2cAdapter(pub I2cdev);

impl I2cWrite02 for I2cAdapter {
    type Error = I2CError;

    fn write(&mut self, address: u8, bytes: &[u8]) -> Result<(), Self::Error> {
        self.0.write(address, bytes)
    }
}

/// Adapter that re-exposes [`linux_embedded_hal::Delay`] semantics through the
/// embedded-hal 0.2 [`DelayUs`]/[`DelayMs`] traits that `hd44780-driver` 0.4
/// requires. The driver only ever asks for `u16` µs and `u8` ms delays, so
/// only those two width specialisations are implemented.
pub struct DelayAdapter;

impl DelayUs<u16> for DelayAdapter {
    fn delay_us(&mut self, us: u16) {
        std::thread::sleep(Duration::from_micros(u64::from(us)));
    }
}

impl DelayMs<u8> for DelayAdapter {
    fn delay_ms(&mut self, ms: u8) {
        std::thread::sleep(Duration::from_millis(u64::from(ms)));
    }
}

/// Concrete LCD that wraps [`HD44780`] and a [`DelayAdapter`].
///
/// Owns both so the runner code can use a single `&mut impl LcdDisplay`
/// rather than threading two separate mutable references.
pub struct I2cLcd<B: DataBus> {
    lcd: HD44780<B>,
    delay: DelayAdapter,
}

impl<B: DataBus> LcdDisplay for I2cLcd<B> {
    fn write_line(&mut self, pos: u8, text: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.lcd
            .set_cursor_pos(pos, &mut self.delay)
            .map_err(|_| "cursor")?;
        self.lcd
            .write_str(text, &mut self.delay)
            .map_err(|_| "write")?;
        Ok(())
    }
}

/// Opens the given I²C bus, brings the HD44780 panel up at `addr` in 4-line
/// mode, and returns an [`LcdDisplay`] handle ready for the runner to write
/// into.
///
/// # Errors
/// Returns an error if the I²C device cannot be opened, or if any HD44780
/// initialisation step (reset, clear, display-mode set) fails.
pub fn init_lcd(bus: &str, addr: u8) -> Result<impl LcdDisplay, Box<dyn std::error::Error>> {
    let i2c = I2cAdapter(I2cdev::new(bus)?);
    let mut delay = DelayAdapter;
    let mut lcd_inner = HD44780::new_i2c(i2c, addr, &mut delay).map_err(|_| "lcd init")?;
    lcd_inner.reset(&mut delay).map_err(|_| "reset")?;
    lcd_inner.clear(&mut delay).map_err(|_| "clear")?;
    lcd_inner
        .set_display_mode(
            DisplayMode {
                display: Display::On,
                cursor_visibility: Cursor::Invisible,
                cursor_blink: CursorBlink::Off,
            },
            &mut delay,
        )
        .map_err(|_| "mode")?;
    info!("LCD initialized on {bus} @ 0x{addr:02X}");
    Ok(I2cLcd {
        lcd: lcd_inner,
        delay,
    })
}
