use std::{
    collections::BTreeMap,
    ffi::{c_char, c_int, c_void, CStr},
    io,
    mem::size_of,
    ptr,
};

use crate::contracts::{
    MetricLimitationCode, MetricQuality, MetricQualityInfo, MetricSource, ProcessSample,
    SystemMetricQuality, SystemMetricsSnapshot,
};

const KERN_SUCCESS: i32 = 0;
const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const K_CF_NUMBER_SINT64_TYPE: isize = 4;
const IO_REGISTRY_PATH_BYTES: usize = 512;
const DISK_BASELINE_PENDING: &str =
    "Waiting for a stable IOKit physical-device baseline after storage topology changed.";

type CfTypeRef = *const c_void;
type CfStringRef = *const c_void;
type CfDictionaryRef = *const c_void;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(name: *const c_char) -> CfDictionaryRef;
    fn IOServiceGetMatchingServices(
        main_port: u32,
        matching: CfDictionaryRef,
        existing: *mut u32,
    ) -> i32;
    fn IOIteratorNext(iterator: u32) -> u32;
    fn IOObjectRelease(object: u32) -> i32;
    fn IORegistryEntryGetRegistryEntryID(entry: u32, entry_id: *mut u64) -> i32;
    fn IORegistryEntryGetPath(entry: u32, plane: *const c_char, path: *mut c_char) -> i32;
    fn IORegistryEntryCreateCFProperty(
        entry: u32,
        key: CfStringRef,
        allocator: *const c_void,
        options: u32,
    ) -> CfTypeRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCreateWithCString(
        allocator: *const c_void,
        value: *const c_char,
        encoding: u32,
    ) -> CfStringRef;
    fn CFDictionaryGetValue(dictionary: CfDictionaryRef, key: *const c_void) -> *const c_void;
    fn CFDictionaryGetTypeID() -> usize;
    fn CFNumberGetTypeID() -> usize;
    fn CFNumberGetValue(number: *const c_void, number_type: isize, value: *mut c_void) -> u8;
    fn CFGetTypeID(value: CfTypeRef) -> usize;
    fn CFRelease(value: CfTypeRef);
}

struct IoObject(u32);

impl Drop for IoObject {
    fn drop(&mut self) {
        if self.0 != 0 {
            unsafe {
                IOObjectRelease(self.0);
            }
        }
    }
}

struct CfOwned(CfTypeRef);

impl Drop for CfOwned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CFRelease(self.0);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RawDiskDevice {
    registry_id: u64,
    registry_path: String,
    read_bytes: Option<u64>,
    write_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HostDiskAggregate {
    device_ids: Vec<u64>,
    read_bytes: u64,
    write_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HostDiskCollection {
    Available(HostDiskAggregate),
    Unavailable {
        limitation: MetricLimitationCode,
        message: String,
    },
}

#[derive(Debug, Default)]
pub struct MacosSystemCollector {
    last_disk_device_ids: Option<Vec<u64>>,
}

impl MacosSystemCollector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enrich(&mut self, snapshot: &mut SystemMetricsSnapshot, _processes: &[ProcessSample]) {
        self.enrich_with_disk_collection(snapshot, collect_host_disk());
    }

    fn enrich_with_disk_collection(
        &mut self,
        snapshot: &mut SystemMetricsSnapshot,
        disk: HostDiskCollection,
    ) {
        snapshot.disk_read_bps = 0;
        snapshot.disk_write_bps = 0;

        let disk_quality = match disk {
            HostDiskCollection::Available(disk) => {
                snapshot.disk_read_total_bytes = disk.read_bytes;
                snapshot.disk_write_total_bytes = disk.write_bytes;
                let topology_is_stable = self
                    .last_disk_device_ids
                    .as_ref()
                    .is_some_and(|previous| previous == &disk.device_ids);
                self.last_disk_device_ids = Some(disk.device_ids);
                if topology_is_stable {
                    MetricQualityInfo::new(MetricQuality::Native, MetricSource::Iokit)
                } else {
                    MetricQualityInfo::new(MetricQuality::Held, MetricSource::Iokit)
                        .with_limitation(
                            MetricLimitationCode::PendingBaseline,
                            DISK_BASELINE_PENDING,
                        )
                }
            }
            HostDiskCollection::Unavailable {
                limitation,
                message,
            } => {
                snapshot.disk_read_total_bytes = 0;
                snapshot.disk_write_total_bytes = 0;
                self.last_disk_device_ids = None;
                MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Iokit)
                    .with_limitation(limitation, &message)
            }
        };

        snapshot.quality = Some(SystemMetricQuality {
            cpu: Some(MetricQualityInfo::new(
                MetricQuality::Estimated,
                MetricSource::Sysinfo,
            )),
            kernel_cpu: Some(
                MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Sysinfo)
                    .with_limitation(
                        MetricLimitationCode::UnsupportedMetric,
                        "Kernel CPU is unavailable from the macOS system collector.",
                    ),
            ),
            logical_cpu: Some(MetricQualityInfo::new(
                MetricQuality::Estimated,
                MetricSource::Sysinfo,
            )),
            memory: Some(MetricQualityInfo::new(
                MetricQuality::Native,
                MetricSource::Sysinfo,
            )),
            swap: Some(MetricQualityInfo::new(
                MetricQuality::Estimated,
                MetricSource::Sysinfo,
            )),
            disk: Some(disk_quality),
            network: Some(MetricQualityInfo::new(
                MetricQuality::Native,
                MetricSource::Sysinfo,
            )),
        });
    }
}

/// Sum of per-interface byte counters from the kernel route table and IFMIB
/// data, replacing sysinfo's `Networks` refresh (which paid for a `getifaddrs`
/// per interface via `if_indextoname` plus one more for addresses).
/// The 64-bit counters come from IFMIB_IFDATA; the RTM_IFINFO2 `ifm_data`
/// counters are 32-bit-capped for unprivileged callers (sysinfo issue #1378).
pub(crate) fn interface_byte_totals() -> io::Result<(u64, u64)> {
    let mut received = 0_u64;
    let mut transmitted = 0_u64;
    for index in route_interface_indices()? {
        // An interface whose IFMIB read fails contributes 0, matching sysinfo.
        if let Ok((rx, tx)) = interface_counters(index) {
            received = received.saturating_add(rx);
            transmitted = transmitted.saturating_add(tx);
        }
    }
    Ok((received, transmitted))
}

fn sysctl_route(mib: &mut [c_int], buffer: *mut c_void, size: &mut usize) -> io::Result<()> {
    let result = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as u32,
            buffer,
            size,
            ptr::null_mut(),
            0,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn route_interface_indices() -> io::Result<Vec<u16>> {
    let mut mib = [libc::CTL_NET, libc::PF_ROUTE, 0, 0, libc::NET_RT_IFLIST2, 0];
    let mut size = 0_usize;
    sysctl_route(&mut mib, ptr::null_mut(), &mut size)?;
    let mut capacity = size.saturating_add(size / 4).max(4096);
    for _ in 0..4 {
        let mut buffer = vec![0_u8; capacity];
        let mut written = capacity;
        match sysctl_route(&mut mib, buffer.as_mut_ptr().cast(), &mut written) {
            Ok(()) => {
                buffer.truncate(written);
                return Ok(interface_indices_from_route_dump(&buffer));
            }
            // The table grew between the size probe and the read; retry bigger.
            Err(error) if error.raw_os_error() == Some(libc::ENOMEM) => {
                capacity = capacity.saturating_mul(2);
            }
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::from_raw_os_error(libc::ENOMEM))
}

fn interface_indices_from_route_dump(buffer: &[u8]) -> Vec<u16> {
    // Walk the shared rt_msghdr prefix (msglen/version/type/addrs/flags/index);
    // if_msghdr itself carries a trailing if_data, so records of other types can
    // be shorter than size_of::<if_msghdr>().
    const HDR_PREFIX: usize = 16;
    let mut indices = Vec::new();
    let mut offset = 0_usize;
    while offset + HDR_PREFIX <= buffer.len() {
        let msglen = usize::from(u16::from_ne_bytes(
            buffer[offset..offset + 2].try_into().unwrap_or_default(),
        ));
        if msglen < HDR_PREFIX || offset + msglen > buffer.len() {
            break;
        }
        let msg_type = buffer[offset + 3];
        if i32::from(msg_type) == libc::RTM_IFINFO2 && msglen >= size_of::<libc::if_msghdr2>() {
            let index = u16::from_ne_bytes(
                buffer[offset + 12..offset + 14]
                    .try_into()
                    .unwrap_or_default(),
            );
            indices.push(index);
        }
        offset += msglen;
    }
    indices
}

fn interface_counters(index: u16) -> io::Result<(u64, u64)> {
    let mut mib = [
        libc::CTL_NET,
        libc::PF_LINK,
        libc::NETLINK_GENERIC,
        libc::IFMIB_IFDATA,
        c_int::from(index),
        libc::IFDATA_GENERAL,
    ];
    let mut data: libc::ifmibdata = unsafe { std::mem::zeroed() };
    let mut size = size_of::<libc::ifmibdata>();
    sysctl_route(
        &mut mib,
        (&mut data as *mut libc::ifmibdata).cast::<c_void>(),
        &mut size,
    )?;
    Ok((data.ifmd_data.ifi_ibytes, data.ifmd_data.ifi_obytes))
}

fn collect_host_disk() -> HostDiskCollection {
    match collect_iokit_devices().and_then(aggregate_physical_devices) {
        Ok(aggregate) => HostDiskCollection::Available(aggregate),
        Err(message) => HostDiskCollection::Unavailable {
            limitation: MetricLimitationCode::CollectorFailure,
            message,
        },
    }
}

fn collect_iokit_devices() -> Result<Vec<RawDiskDevice>, String> {
    let matching = unsafe { IOServiceMatching(c"IOBlockStorageDriver".as_ptr()) };
    if matching.is_null() {
        return Err("IOKit could not create the physical block-driver query.".to_string());
    }

    let mut iterator = 0_u32;
    let result = unsafe { IOServiceGetMatchingServices(0, matching, &mut iterator) };
    if result != KERN_SUCCESS {
        return Err(format!(
            "IOKit physical block-driver enumeration failed with code {result}."
        ));
    }
    let iterator = IoObject(iterator);
    let mut devices = Vec::new();

    loop {
        let service = unsafe { IOIteratorNext(iterator.0) };
        if service == 0 {
            break;
        }
        let service = IoObject(service);
        let registry_id = registry_entry_id(service.0)?;
        let registry_path = registry_entry_path(service.0)?;
        let (read_bytes, write_bytes) = disk_statistics(service.0)?;
        devices.push(RawDiskDevice {
            registry_id,
            registry_path,
            read_bytes: Some(read_bytes),
            write_bytes: Some(write_bytes),
        });
    }

    Ok(devices)
}

fn registry_entry_id(entry: u32) -> Result<u64, String> {
    let mut value = 0_u64;
    let result = unsafe { IORegistryEntryGetRegistryEntryID(entry, &mut value) };
    if result == KERN_SUCCESS && value != 0 {
        Ok(value)
    } else {
        Err(format!(
            "IOKit block-driver identity lookup failed with code {result}."
        ))
    }
}

fn registry_entry_path(entry: u32) -> Result<String, String> {
    let mut path = [0_i8; IO_REGISTRY_PATH_BYTES];
    let result = unsafe {
        IORegistryEntryGetPath(
            entry,
            c"IOService".as_ptr(),
            path.as_mut_ptr().cast::<c_char>(),
        )
    };
    if result != KERN_SUCCESS {
        return Err(format!(
            "IOKit block-driver path lookup failed with code {result}."
        ));
    }
    let path = unsafe { CStr::from_ptr(path.as_ptr().cast::<c_char>()) };
    Ok(path.to_string_lossy().into_owned())
}

fn disk_statistics(entry: u32) -> Result<(u64, u64), String> {
    let statistics_key = cf_string(c"Statistics")?;
    let statistics =
        unsafe { IORegistryEntryCreateCFProperty(entry, statistics_key.0, ptr::null(), 0) };
    if statistics.is_null() {
        return Err("An IOKit physical block driver did not publish statistics.".to_string());
    }
    let statistics = CfOwned(statistics);
    if unsafe { CFGetTypeID(statistics.0) } != unsafe { CFDictionaryGetTypeID() } {
        return Err("An IOKit physical block driver published malformed statistics.".to_string());
    }

    Ok((
        dictionary_u64(statistics.0, c"Bytes (Read)")?,
        dictionary_u64(statistics.0, c"Bytes (Write)")?,
    ))
}

fn dictionary_u64(dictionary: CfDictionaryRef, key: &CStr) -> Result<u64, String> {
    let key = cf_string(key)?;
    let number = unsafe { CFDictionaryGetValue(dictionary, key.0) };
    if number.is_null() || unsafe { CFGetTypeID(number) } != unsafe { CFNumberGetTypeID() } {
        return Err("An IOKit physical block driver omitted a byte counter.".to_string());
    }
    let mut value = 0_i64;
    let converted = unsafe {
        CFNumberGetValue(
            number,
            K_CF_NUMBER_SINT64_TYPE,
            (&mut value as *mut i64).cast::<c_void>(),
        )
    };
    if converted == 0 || value < 0 {
        return Err(
            "An IOKit physical block driver published an invalid byte counter.".to_string(),
        );
    }
    Ok(value as u64)
}

fn cf_string(value: &CStr) -> Result<CfOwned, String> {
    let value = unsafe {
        CFStringCreateWithCString(ptr::null(), value.as_ptr(), K_CF_STRING_ENCODING_UTF8)
    };
    if value.is_null() {
        Err("CoreFoundation could not allocate an IOKit property key.".to_string())
    } else {
        Ok(CfOwned(value))
    }
}

fn aggregate_physical_devices(devices: Vec<RawDiskDevice>) -> Result<HostDiskAggregate, String> {
    let mut physical = BTreeMap::<u64, (u64, u64)>::new();
    for device in devices {
        if is_disk_image_path(&device.registry_path) {
            continue;
        }
        let (Some(read_bytes), Some(write_bytes)) = (device.read_bytes, device.write_bytes) else {
            return Err(
                "An IOKit physical block driver did not publish complete byte counters."
                    .to_string(),
            );
        };
        match physical.get(&device.registry_id) {
            Some(previous) if *previous != (read_bytes, write_bytes) => {
                return Err(
                    "IOKit returned conflicting counters for one physical device.".to_string(),
                );
            }
            Some(_) => {}
            None => {
                physical.insert(device.registry_id, (read_bytes, write_bytes));
            }
        }
    }

    if physical.is_empty() {
        return Err(
            "No eligible IOKit physical block driver published host disk counters.".to_string(),
        );
    }

    let mut read_bytes = 0_u64;
    let mut write_bytes = 0_u64;
    for (read, write) in physical.values().copied() {
        read_bytes = read_bytes
            .checked_add(read)
            .ok_or_else(|| "IOKit physical disk read counters overflowed.".to_string())?;
        write_bytes = write_bytes
            .checked_add(write)
            .ok_or_else(|| "IOKit physical disk write counters overflowed.".to_string())?;
    }
    Ok(HostDiskAggregate {
        device_ids: physical.keys().copied().collect(),
        read_bytes,
        write_bytes,
    })
}

fn is_disk_image_path(path: &str) -> bool {
    path.contains("/IOHDIXController/") || path.contains("DiskImages")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    struct AttachedDiskImage {
        device: String,
        directory: std::path::PathBuf,
    }

    impl Drop for AttachedDiskImage {
        fn drop(&mut self) {
            let _ = Command::new("hdiutil")
                .args(["detach", self.device.as_str()])
                .output();
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    fn system() -> SystemMetricsSnapshot {
        SystemMetricsSnapshot {
            cpu_percent: 0.0,
            kernel_cpu_percent: 0.0,
            logical_cpu_percent: vec![],
            memory_used_bytes: 1,
            memory_total_bytes: 2,
            memory_available_bytes: Some(1),
            swap_used_bytes: Some(0),
            swap_total_bytes: Some(0),
            process_count: 1,
            disk_read_total_bytes: 0,
            disk_write_total_bytes: 0,
            disk_read_bps: 0,
            disk_write_bps: 0,
            network_received_total_bytes: 0,
            network_transmitted_total_bytes: 0,
            network_received_bps: 0,
            network_transmitted_bps: 0,
            memory_accounting: None,
            quality: None,
        }
    }

    fn device(id: u64, path: &str, read: u64, write: u64) -> RawDiskDevice {
        RawDiskDevice {
            registry_id: id,
            registry_path: path.to_string(),
            read_bytes: Some(read),
            write_bytes: Some(write),
        }
    }

    fn available(ids: &[u64], read: u64, write: u64) -> HostDiskCollection {
        HostDiskCollection::Available(HostDiskAggregate {
            device_ids: ids.to_vec(),
            read_bytes: read,
            write_bytes: write,
        })
    }

    #[test]
    fn physical_device_aggregate_deduplicates_apfs_and_excludes_disk_images() {
        let aggregate = aggregate_physical_devices(vec![
            device(
                10,
                "IOService:/AppleARMPE/ans/IOBlockStorageDriver",
                100,
                50,
            ),
            device(
                10,
                "IOService:/AppleARMPE/ans/IOBlockStorageDriver",
                100,
                50,
            ),
            device(20, "IOService:/AppleUSB/IOBlockStorageDriver", 40, 30),
            device(
                30,
                "IOService:/IOResources/IOHDIXController/DiskImages/IOBlockStorageDriver",
                9_999,
                9_999,
            ),
        ])
        .expect("physical aggregate");

        assert_eq!(aggregate.device_ids, vec![10, 20]);
        assert_eq!(aggregate.read_bytes, 140);
        assert_eq!(aggregate.write_bytes, 80);
    }

    #[test]
    fn incomplete_physical_device_counters_make_host_disk_unavailable() {
        let mut incomplete = device(10, "IOService:/physical", 100, 50);
        incomplete.write_bytes = None;

        assert!(aggregate_physical_devices(vec![incomplete])
            .expect_err("incomplete device must fail closed")
            .contains("complete byte counters"));
    }

    #[test]
    fn host_disk_availability_requires_a_stable_device_identity_baseline() {
        let mut collector = MacosSystemCollector::new();
        let mut first = system();
        collector.enrich_with_disk_collection(&mut first, available(&[10], 1_000, 500));
        let first_disk = first.quality.unwrap().disk.unwrap();
        assert_eq!(first_disk.quality, MetricQuality::Held);
        assert_eq!(first_disk.source, Some(MetricSource::Iokit));
        assert_eq!(
            first_disk.limitation_code,
            Some(MetricLimitationCode::PendingBaseline)
        );

        let mut second = system();
        collector.enrich_with_disk_collection(&mut second, available(&[10], 1_200, 600));
        let second_disk = second.quality.unwrap().disk.unwrap();
        assert_eq!(second_disk.quality, MetricQuality::Native);
        assert_eq!(second.disk_read_total_bytes, 1_200);
        assert_eq!(second.disk_write_total_bytes, 600);

        let mut attached = system();
        collector.enrich_with_disk_collection(&mut attached, available(&[10, 20], 9_000, 4_000));
        let attached_disk = attached.quality.unwrap().disk.unwrap();
        assert_eq!(attached_disk.quality, MetricQuality::Held);
        assert_eq!(attached.disk_read_bps, 0);
        assert_eq!(attached.disk_write_bps, 0);
    }

    #[test]
    fn host_disk_is_explicitly_unavailable_when_iokit_fails_closed() {
        let mut collector = MacosSystemCollector::new();
        let mut snapshot = system();
        collector.enrich_with_disk_collection(
            &mut snapshot,
            HostDiskCollection::Unavailable {
                limitation: MetricLimitationCode::CollectorFailure,
                message: "fixture failure".to_string(),
            },
        );

        let disk = snapshot.quality.unwrap().disk.unwrap();
        assert_eq!(disk.quality, MetricQuality::Unavailable);
        assert_eq!(disk.source, Some(MetricSource::Iokit));
        assert_eq!(snapshot.disk_read_total_bytes, 0);
        assert_eq!(snapshot.disk_write_total_bytes, 0);
    }

    #[test]
    fn native_iokit_source_reports_a_physical_device() {
        let HostDiskCollection::Available(aggregate) = collect_host_disk() else {
            panic!("native IOKit host disk source must be available on the macOS test host");
        };
        assert!(!aggregate.device_ids.is_empty());
        assert!(aggregate.read_bytes > 0);
    }

    #[test]
    #[ignore = "mounts a temporary local DMG for native collector evidence"]
    fn native_dmg_fixture_does_not_change_the_physical_aggregate() {
        let before_raw = collect_iokit_devices().expect("IOKit devices before DMG");
        let before = aggregate_physical_devices(before_raw.clone()).expect("physical devices");
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("batcave-iokit-dmg-{}-{nonce}", std::process::id()));
        fs::create_dir(&directory).expect("create DMG fixture directory");
        let image = directory.join("fixture.dmg");
        let create = Command::new("hdiutil")
            .args(["create", "-size", "8m", "-fs", "HFS+", "-volname"])
            .arg("BatCaveDiskFixture")
            .arg(&image)
            .output()
            .expect("run hdiutil create");
        assert!(
            create.status.success(),
            "hdiutil create failed: {}",
            String::from_utf8_lossy(&create.stderr)
        );
        let attach = Command::new("hdiutil")
            .args(["attach", "-nobrowse", "-readonly", "-nomount"])
            .arg(&image)
            .output()
            .expect("run hdiutil attach");
        assert!(
            attach.status.success(),
            "hdiutil attach failed: {}",
            String::from_utf8_lossy(&attach.stderr)
        );
        let attach_output = String::from_utf8(attach.stdout).expect("UTF-8 hdiutil output");
        let device = attach_output
            .split_whitespace()
            .find(|value| value.starts_with("/dev/disk"))
            .expect("attached device path")
            .to_string();
        let _guard = AttachedDiskImage { device, directory };

        let after_raw = collect_iokit_devices().expect("IOKit devices with DMG");
        let after = aggregate_physical_devices(after_raw.clone()).expect("physical devices");

        assert!(after_raw.len() > before_raw.len());
        assert!(after_raw
            .iter()
            .any(|device| is_disk_image_path(&device.registry_path)));
        assert_eq!(after.device_ids, before.device_ids);
        println!(
            "raw_drivers_before={} raw_drivers_with_dmg={} physical_ids={:?}",
            before_raw.len(),
            after_raw.len(),
            after.device_ids
        );
    }

    fn route_record(msg_type: u8, index: u16, tail: &[u8]) -> Vec<u8> {
        let mut record = Vec::new();
        if i32::from(msg_type) == libc::RTM_IFINFO2 {
            let mut header: libc::if_msghdr2 = unsafe { std::mem::zeroed() };
            header.ifm_msglen = (size_of::<libc::if_msghdr2>() + tail.len()) as u16;
            header.ifm_type = msg_type;
            header.ifm_index = index;
            record.extend_from_slice(unsafe {
                std::slice::from_raw_parts(
                    (&header as *const libc::if_msghdr2).cast::<u8>(),
                    size_of::<libc::if_msghdr2>(),
                )
            });
        } else {
            let mut header: libc::if_msghdr = unsafe { std::mem::zeroed() };
            header.ifm_msglen = (size_of::<libc::if_msghdr>() + tail.len()) as u16;
            header.ifm_type = msg_type;
            record.extend_from_slice(unsafe {
                std::slice::from_raw_parts(
                    (&header as *const libc::if_msghdr).cast::<u8>(),
                    size_of::<libc::if_msghdr>(),
                )
            });
        }
        record.extend_from_slice(tail);
        record
    }

    #[test]
    fn route_dump_walker_extracts_ifinfo2_indices() {
        let mut buffer = Vec::new();
        // A non-IFINFO2 record first: the walker must skip it.
        buffer.extend_from_slice(&route_record(libc::RTM_NEWADDR as u8, 0, &[0; 24]));
        buffer.extend_from_slice(&route_record(libc::RTM_IFINFO2 as u8, 4, &[]));
        buffer.extend_from_slice(&route_record(libc::RTM_IFINFO2 as u8, 9, &[0; 40]));
        buffer.extend_from_slice(&route_record(libc::RTM_IFINFO2 as u8, 12, &[0; 8]));
        // Trailing garbage shorter than a header is ignored.
        buffer.extend_from_slice(&[0xff, 0x00]);

        assert_eq!(interface_indices_from_route_dump(&buffer), vec![4, 9, 12]);
        assert!(interface_indices_from_route_dump(&[]).is_empty());
    }

    #[test]
    #[ignore = "live host check: compares against sysinfo Networks totals"]
    fn live_interface_totals_match_sysinfo_networks() {
        let (rx, tx) = interface_byte_totals().expect("interface totals read");
        let networks = sysinfo::Networks::new_with_refreshed_list();
        let (sys_rx, sys_tx) = networks.iter().fold((0_u64, 0_u64), |(r, t), (_, data)| {
            (r + data.total_received(), t + data.total_transmitted())
        });
        println!("native rx={rx} tx={tx}; sysinfo rx={sys_rx} tx={sys_tx}");
        assert!(rx.abs_diff(sys_rx) <= 1024 * 1024, "rx {rx} vs {sys_rx}");
        assert!(tx.abs_diff(sys_tx) <= 1024 * 1024, "tx {tx} vs {sys_tx}");
    }
}
