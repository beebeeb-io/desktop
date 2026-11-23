//! The Windows user's regional format (Settings > Time & language > Region > Regional format), read once at
//! start: the decimal separator, the order of a short date and whether times are 12- or 24-hour. Sizes and dates
//! follow it (`source::format_size`, `ui::file_list::format_date`); the words stay English.
//!
//! Tests and board frames see en-GB, the board's own format, unless a test pins another region on its thread
//! ([`pin`]), so no board frame moves with the PC the tests run on.

/// The order of day, month and year in the region's short date.
// Only Windows reads a region (`from_values`); elsewhere the app itself only ever has the board's `Dmy`.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DateOrder {
    /// 28 Oct 2026 (en-GB, nl-NL, de-DE).
    Dmy,
    /// Oct 28, 2026 (en-US).
    Mdy,
    /// 2026 Oct 28 (ja-JP, sv-SE, hu-HU).
    Ymd,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Region {
    /// "627.8 MB" or "627,8 MB".
    pub decimal: char,
    pub date_order: DateOrder,
    /// "5:19 PM" rather than "17:19".
    pub hour12: bool,
}

impl Region {
    /// The board's format.
    pub const EN_GB: Region = Region {
        decimal: '.',
        date_order: DateOrder::Dmy,
        hour12: false,
    };
    #[cfg(test)]
    pub const NL_NL: Region = Region {
        decimal: ',',
        date_order: DateOrder::Dmy,
        hour12: false,
    };
    #[cfg(test)]
    pub const EN_US: Region = Region {
        decimal: '.',
        date_order: DateOrder::Mdy,
        hour12: true,
    };

    /// From the region's own values: the decimal separator (`sDecimal`), the short date's pattern (`sShortDate`,
    /// "d-M-yyyy") and the short time's (`sShortTime`, "HH:mm" or "h:mm tt"). What is missing or unreadable keeps
    /// the board's.
    #[cfg_attr(not(any(windows, test)), allow(dead_code))]
    pub fn from_values(decimal: Option<&str>, short_date: Option<&str>, short_time: Option<&str>) -> Region {
        let mut region = Region::EN_GB;
        if let Some(sep) = decimal.and_then(|d| d.trim().chars().next()) {
            region.decimal = sep;
        }
        if let Some(pattern) = short_date {
            // The first of d, M and y in the pattern says the order; quoted literals are not pattern letters.
            let mut quoted = false;
            for c in pattern.chars() {
                match c {
                    '\'' => quoted = !quoted,
                    'd' | 'M' | 'y' if !quoted => {
                        region.date_order = match c {
                            'd' => DateOrder::Dmy,
                            'M' => DateOrder::Mdy,
                            _ => DateOrder::Ymd,
                        };
                        break;
                    }
                    _ => {}
                }
            }
        }
        if let Some(pattern) = short_time {
            if pattern.contains('h') {
                region.hour12 = true;
            } else if pattern.contains('H') {
                region.hour12 = false;
            }
        }
        region
    }
}

/// The region sizes and dates follow.
pub fn current() -> Region {
    #[cfg(test)]
    {
        PINNED.with(|p| p.get())
    }
    #[cfg(not(test))]
    {
        static REGION: std::sync::OnceLock<Region> = std::sync::OnceLock::new();
        *REGION.get_or_init(read)
    }
}

#[cfg(test)]
thread_local! {
    static PINNED: std::cell::Cell<Region> = const { std::cell::Cell::new(Region::EN_GB) };
}

/// Run `f` with `region` as this thread's region (tests only); en-GB again afterwards.
#[cfg(test)]
pub fn pin<R>(region: Region, f: impl FnOnce() -> R) -> R {
    struct Back;
    impl Drop for Back {
        fn drop(&mut self) {
            PINNED.with(|p| p.set(Region::EN_GB));
        }
    }
    PINNED.with(|p| p.set(region));
    let _back = Back;
    f()
}

#[cfg(all(windows, not(test)))]
fn read() -> Region {
    const KEY: &str = "Control Panel\\International";
    let decimal = reg_string(KEY, "sDecimal");
    let date = reg_string(KEY, "sShortDate");
    let time = reg_string(KEY, "sShortTime");
    Region::from_values(decimal.as_deref(), date.as_deref(), time.as_deref())
}

#[cfg(all(not(windows), not(test)))]
fn read() -> Region {
    Region::EN_GB
}

/// A string value under HKEY_CURRENT_USER, or `None` when it is not there or cannot be read.
#[cfg(all(windows, not(test)))]
fn reg_string(key: &str, name: &str) -> Option<String> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};

    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (key_w, name_w) = (wide(key), wide(name));
    // The region's values are short ("dd/MM/yyyy"); 128 UTF-16 units is ample.
    let mut buffer = [0u16; 128];
    let mut size = std::mem::size_of_val(&buffer) as u32;
    // SAFETY: both names are NUL-terminated; `buffer` is `size` bytes long and outlives the call.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key_w.as_ptr(),
            name_w.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let units = (size as usize / 2).min(buffer.len());
    let text = String::from_utf16_lossy(&buffer[..units]);
    let text = text.trim_end_matches('\0').to_string();
    (!text.is_empty()).then_some(text)
}
