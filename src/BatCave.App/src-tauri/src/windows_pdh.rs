#![cfg_attr(not(windows), allow(dead_code, unused_imports))]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskRates {
    pub read_bps: u64,
    pub write_bps: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PdhSample {
    Ready(DiskRates),
    Held(String),
}

#[cfg(windows)]
use std::{ffi::OsStr, iter::once, os::windows::ffi::OsStrExt, ptr::null};

#[cfg(windows)]
use windows_sys::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterValue,
    PdhOpenQueryW, PDH_FMT_COUNTERVALUE, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY,
};

const ERROR_SUCCESS: u32 = 0;
const READ_COUNTER: &str = r"\PhysicalDisk(_Total)\Disk Read Bytes/sec";
const WRITE_COUNTER: &str = r"\PhysicalDisk(_Total)\Disk Write Bytes/sec";

#[cfg(windows)]
pub struct PdhDiskSampler {
    query: PDH_HQUERY,
    read_counter: PDH_HCOUNTER,
    write_counter: PDH_HCOUNTER,
    warmed: bool,
}

#[cfg(windows)]
unsafe impl Send for PdhDiskSampler {}

#[cfg(windows)]
impl PdhDiskSampler {
    pub fn new() -> Result<Self, String> {
        let mut query = 0 as PDH_HQUERY;
        let status = unsafe { PdhOpenQueryW(null(), 0, &mut query) };
        if status != ERROR_SUCCESS {
            return Err(format!("pdh_open_query_failed:{status}"));
        }

        let read_counter = match add_counter(query, READ_COUNTER) {
            Ok(counter) => counter,
            Err(error) => {
                unsafe {
                    PdhCloseQuery(query);
                }
                return Err(error);
            }
        };
        let write_counter = match add_counter(query, WRITE_COUNTER) {
            Ok(counter) => counter,
            Err(error) => {
                unsafe {
                    PdhCloseQuery(query);
                }
                return Err(error);
            }
        };

        Ok(Self {
            query,
            read_counter,
            write_counter,
            warmed: false,
        })
    }

    pub fn sample(&mut self) -> Result<PdhSample, String> {
        let status = unsafe { PdhCollectQueryData(self.query) };
        if status != ERROR_SUCCESS {
            return Err(format!("pdh_collect_failed:{status}"));
        }

        if !self.warmed {
            self.warmed = true;
            return Ok(PdhSample::Held(
                "PDH disk counters are warming up.".to_string(),
            ));
        }

        Ok(PdhSample::Ready(DiskRates {
            read_bps: sample_counter(self.read_counter)?,
            write_bps: sample_counter(self.write_counter)?,
        }))
    }
}

#[cfg(windows)]
impl Drop for PdhDiskSampler {
    fn drop(&mut self) {
        if !self.query.is_null() {
            unsafe {
                PdhCloseQuery(self.query);
            }
        }
    }
}

#[cfg(not(windows))]
pub struct PdhDiskSampler;

#[cfg(not(windows))]
impl PdhDiskSampler {
    pub fn new() -> Result<Self, String> {
        Err("pdh_disk_sampler_requires_windows".to_string())
    }

    pub fn sample(&mut self) -> Result<PdhSample, String> {
        Err("pdh_disk_sampler_requires_windows".to_string())
    }
}

#[cfg(windows)]
fn add_counter(query: PDH_HQUERY, counter_path: &str) -> Result<PDH_HCOUNTER, String> {
    let counter_path = wide(counter_path);
    let mut counter = 0 as PDH_HCOUNTER;
    let status = unsafe { PdhAddEnglishCounterW(query, counter_path.as_ptr(), 0, &mut counter) };
    if status == ERROR_SUCCESS {
        Ok(counter)
    } else {
        Err(format!("pdh_add_counter_failed:{status}"))
    }
}

#[cfg(windows)]
fn sample_counter(counter: PDH_HCOUNTER) -> Result<u64, String> {
    let mut value = PDH_FMT_COUNTERVALUE::default();
    let mut counter_type = 0_u32;
    let status = unsafe {
        PdhGetFormattedCounterValue(counter, PDH_FMT_DOUBLE, &mut counter_type, &mut value)
    };
    if status != ERROR_SUCCESS {
        return Err(format!("pdh_format_counter_failed:{status}"));
    }
    if value.CStatus != ERROR_SUCCESS {
        return Err(format!("pdh_counter_status_failed:{}", value.CStatus));
    }

    let raw = unsafe { value.Anonymous.doubleValue };
    Ok(rate_to_u64(raw))
}

fn rate_to_u64(value: f64) -> u64 {
    if !value.is_finite() || value <= 0.0 {
        return 0;
    }

    value.round().min(u64::MAX as f64) as u64
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_to_u64_clamps_invalid_or_negative_values() {
        assert_eq!(rate_to_u64(f64::NAN), 0);
        assert_eq!(rate_to_u64(-1.0), 0);
        assert_eq!(rate_to_u64(0.49), 0);
        assert_eq!(rate_to_u64(1.51), 2);
    }
}
