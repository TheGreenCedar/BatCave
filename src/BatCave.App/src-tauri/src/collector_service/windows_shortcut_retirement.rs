use std::{
    ffi::{c_void, OsString},
    mem::size_of,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    ptr,
};

use windows_sys::{
    core::GUID,
    Win32::{
        Foundation::{
            CloseHandle, GetLastError, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, HANDLE,
            PROPERTYKEY,
        },
        Storage::FileSystem::{
            CreateFileW, FileDispositionInfo, GetFileInformationByHandle,
            GetFinalPathNameByHandleW, ReadFile, SetFileInformationByHandle,
            BY_HANDLE_FILE_INFORMATION, DELETE, FILE_ATTRIBUTE_DIRECTORY,
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
            OPEN_EXISTING, READ_CONTROL,
        },
        System::{
            Com::{
                CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
                StructuredStorage::{PropVariantClear, PropVariantToStringAlloc, PROPVARIANT},
                CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
            },
            Variant::VT_LPWSTR,
        },
        UI::Shell::{
            FOLDERID_CommonPrograms, FOLDERID_PublicDesktop, SHCreateMemStream,
            SHGetKnownFolderPath,
        },
    },
};

const SHORTCUT_NAME: &str = "BatCave Monitor.lnk";
const APP_USER_MODEL_ID: &str = "dev.batcave.monitor";
const SHORTCUT_MAX_BYTES: u64 = 1024 * 1024;
const COM_TEXT_CAPACITY: usize = 32 * 1024;
const SLGP_RAWPATH: u32 = 4;

const CLSID_SHELL_LINK: GUID = GUID::from_u128(0x00021401_0000_0000_c000_000000000046);
const IID_SHELL_LINK_W: GUID = GUID::from_u128(0x000214f9_0000_0000_c000_000000000046);
#[cfg(test)]
const IID_PERSIST_FILE: GUID = GUID::from_u128(0x0000010b_0000_0000_c000_000000000046);
const IID_PERSIST_STREAM: GUID = GUID::from_u128(0x00000109_0000_0000_c000_000000000046);
const IID_PROPERTY_STORE: GUID = GUID::from_u128(0x886d8eeb_8cf2_4446_8d02_cdba1dbdcf99);
const PKEY_APP_USER_MODEL_ID: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
    pid: 5,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LegacyShortcutLocation {
    PublicDesktop,
    CommonPrograms,
}

impl LegacyShortcutLocation {
    const ALL: [Self; 2] = [Self::PublicDesktop, Self::CommonPrograms];

    fn folder_id(self) -> &'static GUID {
        match self {
            Self::PublicDesktop => &FOLDERID_PublicDesktop,
            Self::CommonPrograms => &FOLDERID_CommonPrograms,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::PublicDesktop => "public_desktop",
            Self::CommonPrograms => "common_programs",
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct ShortcutContract {
    target: String,
    arguments: String,
    icon_path: String,
    icon_index: i32,
    working_directory: String,
    show_command: i32,
    hotkey: u16,
    description: String,
    app_user_model_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ShortcutIdentity {
    attributes: u32,
    hardlink_count: u32,
    file_index: u64,
    size: u64,
}

impl ShortcutIdentity {
    fn from_info(info: &BY_HANDLE_FILE_INFORMATION) -> Self {
        Self {
            attributes: info.dwFileAttributes,
            hardlink_count: info.nNumberOfLinks,
            file_index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
            size: (u64::from(info.nFileSizeHigh) << 32) | u64::from(info.nFileSizeLow),
        }
    }

    fn validate(self) -> Result<(), String> {
        if self.attributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) != 0
            || self.hardlink_count != 1
            || self.file_index == 0
            || !(1..=SHORTCUT_MAX_BYTES).contains(&self.size)
        {
            return Err("installer_shortcut_identity_invalid".to_string());
        }
        Ok(())
    }
}

#[derive(Debug)]
struct OwnedHandle(HANDLE);

impl OwnedHandle {
    fn new(handle: HANDLE, context: &str) -> Result<Self, String> {
        if invalid_handle(handle) {
            Err(last_error(context))
        } else {
            Ok(Self(handle))
        }
    }

    fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

#[derive(Debug)]
struct PinnedDirectory {
    path: PathBuf,
    handle: OwnedHandle,
    volume_serial: u32,
    file_index: u64,
}

impl PinnedDirectory {
    fn open(path: &Path) -> Result<Self, String> {
        let path_wide = wide_path(path);
        let handle = OwnedHandle::new(
            unsafe {
                CreateFileW(
                    path_wide.as_ptr(),
                    FILE_READ_ATTRIBUTES | READ_CONTROL,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    ptr::null(),
                    OPEN_EXISTING,
                    FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                    ptr::null_mut(),
                )
            },
            "installer_shortcut_ancestry_open_failed",
        )?;
        let info = file_information(handle.raw(), "installer_shortcut_ancestry_info_failed")?;
        if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0
            || info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || !fixed_path_eq(
                &final_path(&handle, "installer_shortcut_ancestry_path_failed")?,
                path,
            )
        {
            return Err("installer_shortcut_ancestry_identity_invalid".to_string());
        }
        Ok(Self {
            path: path.to_path_buf(),
            handle,
            volume_serial: info.dwVolumeSerialNumber,
            file_index: file_index(&info),
        })
    }

    fn revalidate(&self) -> Result<(), String> {
        let info = file_information(
            self.handle.raw(),
            "installer_shortcut_ancestry_revalidate_info_failed",
        )?;
        if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0
            || info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || info.dwVolumeSerialNumber != self.volume_serial
            || file_index(&info) != self.file_index
            || !fixed_path_eq(
                &final_path(
                    &self.handle,
                    "installer_shortcut_ancestry_revalidate_path_failed",
                )?,
                &self.path,
            )
        {
            return Err("installer_shortcut_ancestry_changed".to_string());
        }
        Ok(())
    }
}

struct PinnedLocation {
    location: LegacyShortcutLocation,
    root: PathBuf,
    ancestry: Vec<PinnedDirectory>,
}

impl PinnedLocation {
    fn open(location: LegacyShortcutLocation) -> Result<Self, String> {
        let root = known_folder_path(location)?;
        let ancestry = pin_ancestry(&root)?;
        Ok(Self {
            location,
            root,
            ancestry,
        })
    }

    fn shortcut_path(&self) -> PathBuf {
        shortcut_path_for_root(&self.root)
    }

    fn revalidate(&self) -> Result<(), String> {
        for directory in &self.ancestry {
            directory.revalidate()?;
        }
        let after = known_folder_path(self.location)?;
        if !fixed_path_eq(&after, &self.root) {
            return Err(format!(
                "installer_shortcut_{}_known_folder_changed",
                self.location.label()
            ));
        }
        Ok(())
    }
}

fn shortcut_path_for_root(root: &Path) -> PathBuf {
    root.join(SHORTCUT_NAME)
}

#[derive(Debug)]
struct PinnedShortcut {
    path: PathBuf,
    handle: OwnedHandle,
    volume_serial: u32,
    identity: ShortcutIdentity,
    creation_time: u64,
    last_write_time: u64,
    bytes: Vec<u8>,
}

impl PinnedShortcut {
    fn open(path: &Path) -> Result<Option<Self>, String> {
        let path_wide = wide_path(path);
        let raw = unsafe {
            CreateFileW(
                path_wide.as_ptr(),
                0x8000_0000 | READ_CONTROL | FILE_READ_ATTRIBUTES | DELETE,
                FILE_SHARE_READ,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT,
                ptr::null_mut(),
            )
        };
        if invalid_handle(raw) {
            let error = unsafe { GetLastError() };
            if missing_path_error(error) {
                return Ok(None);
            }
            return Err(format!("installer_shortcut_open_failed:{error}"));
        }
        let handle = OwnedHandle(raw);
        let info = file_information(handle.raw(), "installer_shortcut_info_failed")?;
        let identity = ShortcutIdentity::from_info(&info);
        identity.validate()?;
        if !fixed_path_eq(
            &final_path(&handle, "installer_shortcut_path_failed")?,
            path,
        ) {
            return Err("installer_shortcut_path_invalid".to_string());
        }
        let bytes = read_locked_file(
            handle.raw(),
            identity.size,
            "installer_shortcut_read_failed",
        )?;
        Ok(Some(Self {
            path: path.to_path_buf(),
            handle,
            volume_serial: info.dwVolumeSerialNumber,
            identity,
            creation_time: filetime(info.ftCreationTime),
            last_write_time: filetime(info.ftLastWriteTime),
            bytes,
        }))
    }

    fn validate_contract(&self, monitor_path: &Path) -> Result<(), String> {
        let contract = read_shortcut_contract(&self.bytes)?;
        validate_shortcut_contract(&contract, monitor_path)
    }

    fn revalidate(&self) -> Result<(), String> {
        let info = file_information(
            self.handle.raw(),
            "installer_shortcut_revalidate_info_failed",
        )?;
        if info.dwVolumeSerialNumber != self.volume_serial
            || ShortcutIdentity::from_info(&info) != self.identity
            || filetime(info.ftCreationTime) != self.creation_time
            || filetime(info.ftLastWriteTime) != self.last_write_time
            || !fixed_path_eq(
                &final_path(&self.handle, "installer_shortcut_revalidate_path_failed")?,
                &self.path,
            )
        {
            return Err("installer_shortcut_changed".to_string());
        }
        Ok(())
    }

    fn mark_for_deletion(&self) -> Result<(), String> {
        let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
        if unsafe {
            SetFileInformationByHandle(
                self.handle.raw(),
                FileDispositionInfo,
                (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                size_of::<FILE_DISPOSITION_INFO>() as u32,
            )
        } == 0
        {
            return Err(last_error("installer_shortcut_delete_failed"));
        }
        Ok(())
    }
}

pub(super) fn retire_shared_legacy_shortcuts(monitor_path: &Path) -> Result<(), String> {
    let install_root = monitor_path
        .parent()
        .ok_or_else(|| "installer_shortcut_monitor_parent_missing".to_string())?;
    if monitor_path.file_name().and_then(|name| name.to_str()) != Some("batcave-monitor.exe")
        || install_root.file_name().and_then(|name| name.to_str()) != Some("BatCave Monitor")
    {
        return Err("installer_shortcut_monitor_path_invalid".to_string());
    }
    let locations = LegacyShortcutLocation::ALL
        .into_iter()
        .map(PinnedLocation::open)
        .collect::<Result<Vec<_>, _>>()?;

    for location in &locations {
        retire_one(location, monitor_path)?;
    }
    for location in &locations {
        require_absent(location)?;
    }
    Ok(())
}

fn retire_one(location: &PinnedLocation, monitor_path: &Path) -> Result<(), String> {
    let path = location.shortcut_path();
    let Some(shortcut) = PinnedShortcut::open(&path)? else {
        return require_absent(location);
    };
    shortcut.validate_contract(monitor_path)?;
    shortcut.revalidate()?;
    location.revalidate()?;
    shortcut.mark_for_deletion()?;
    drop(shortcut);
    require_absent(location)
}

fn require_absent(location: &PinnedLocation) -> Result<(), String> {
    location.revalidate()?;
    let path = location.shortcut_path();
    match PinnedShortcut::open(&path)? {
        None => {}
        Some(_) => {
            return Err(format!(
                "installer_shortcut_{}_still_present",
                location.location.label()
            ))
        }
    }
    location.revalidate()?;
    match PinnedShortcut::open(&path)? {
        None => Ok(()),
        Some(_) => Err(format!(
            "installer_shortcut_{}_appeared",
            location.location.label()
        )),
    }
}

fn pin_ancestry(root: &Path) -> Result<Vec<PinnedDirectory>, String> {
    let mut paths = root
        .ancestors()
        .filter(|path| !path.as_os_str().is_empty())
        .collect::<Vec<_>>();
    paths.reverse();
    if paths.len() > 16 || paths.last().copied() != Some(root) {
        return Err("installer_shortcut_ancestry_invalid".to_string());
    }
    paths.into_iter().map(PinnedDirectory::open).collect()
}

fn known_folder_path(location: LegacyShortcutLocation) -> Result<PathBuf, String> {
    let mut raw = ptr::null_mut();
    let result =
        unsafe { SHGetKnownFolderPath(location.folder_id(), 0, ptr::null_mut(), &mut raw) };
    if result < 0 || raw.is_null() {
        return Err(format!(
            "installer_shortcut_{}_known_folder_failed:{result:#010x}",
            location.label()
        ));
    }
    let value = read_nul_terminated_wide(raw, COM_TEXT_CAPACITY).map(PathBuf::from);
    unsafe { CoTaskMemFree(raw.cast()) };
    value
}

#[repr(C)]
struct UnknownVtable {
    query_interface: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
struct ShellLinkWVtable {
    unknown: UnknownVtable,
    get_path: unsafe extern "system" fn(*mut c_void, *mut u16, i32, *mut c_void, u32) -> i32,
    get_id_list: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
    set_id_list: unsafe extern "system" fn(*mut c_void, *const c_void) -> i32,
    get_description: unsafe extern "system" fn(*mut c_void, *mut u16, i32) -> i32,
    set_description: unsafe extern "system" fn(*mut c_void, *const u16) -> i32,
    get_working_directory: unsafe extern "system" fn(*mut c_void, *mut u16, i32) -> i32,
    set_working_directory: unsafe extern "system" fn(*mut c_void, *const u16) -> i32,
    get_arguments: unsafe extern "system" fn(*mut c_void, *mut u16, i32) -> i32,
    set_arguments: unsafe extern "system" fn(*mut c_void, *const u16) -> i32,
    get_hotkey: unsafe extern "system" fn(*mut c_void, *mut u16) -> i32,
    set_hotkey: unsafe extern "system" fn(*mut c_void, u16) -> i32,
    get_show_command: unsafe extern "system" fn(*mut c_void, *mut i32) -> i32,
    set_show_command: unsafe extern "system" fn(*mut c_void, i32) -> i32,
    get_icon_location: unsafe extern "system" fn(*mut c_void, *mut u16, i32, *mut i32) -> i32,
    set_icon_location: unsafe extern "system" fn(*mut c_void, *const u16, i32) -> i32,
    set_relative_path: unsafe extern "system" fn(*mut c_void, *const u16, u32) -> i32,
    resolve: unsafe extern "system" fn(*mut c_void, *mut c_void, u32) -> i32,
    set_path: unsafe extern "system" fn(*mut c_void, *const u16) -> i32,
}

#[cfg(test)]
#[repr(C)]
struct PersistFileVtable {
    unknown: UnknownVtable,
    get_class_id: unsafe extern "system" fn(*mut c_void, *mut GUID) -> i32,
    is_dirty: unsafe extern "system" fn(*mut c_void) -> i32,
    load: unsafe extern "system" fn(*mut c_void, *const u16, u32) -> i32,
    save: unsafe extern "system" fn(*mut c_void, *const u16, i32) -> i32,
    save_completed: unsafe extern "system" fn(*mut c_void, *const u16) -> i32,
    get_current_file: unsafe extern "system" fn(*mut c_void, *mut *mut u16) -> i32,
}

#[repr(C)]
struct PersistStreamVtable {
    unknown: UnknownVtable,
    get_class_id: unsafe extern "system" fn(*mut c_void, *mut GUID) -> i32,
    is_dirty: unsafe extern "system" fn(*mut c_void) -> i32,
    load: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    save: unsafe extern "system" fn(*mut c_void, *mut c_void, i32) -> i32,
    get_size_max: unsafe extern "system" fn(*mut c_void, *mut u64) -> i32,
}

#[repr(C)]
struct PropertyStoreVtable {
    unknown: UnknownVtable,
    get_count: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32,
    get_at: unsafe extern "system" fn(*mut c_void, u32, *mut PROPERTYKEY) -> i32,
    get_value: unsafe extern "system" fn(*mut c_void, *const PROPERTYKEY, *mut PROPVARIANT) -> i32,
    set_value:
        unsafe extern "system" fn(*mut c_void, *const PROPERTYKEY, *const PROPVARIANT) -> i32,
    commit: unsafe extern "system" fn(*mut c_void) -> i32,
}

struct ComPtr(*mut c_void);

impl ComPtr {
    fn query(&self, iid: &GUID, context: &str) -> Result<Self, String> {
        let mut value = ptr::null_mut();
        let vtable = unsafe { &**(self.0.cast::<*const UnknownVtable>()) };
        let result = unsafe { (vtable.query_interface)(self.0, iid, &mut value) };
        if result < 0 || value.is_null() {
            Err(format!("{context}:{result:#010x}"))
        } else {
            Ok(Self(value))
        }
    }

    fn shell_link(&self) -> &ShellLinkWVtable {
        unsafe { &**(self.0.cast::<*const ShellLinkWVtable>()) }
    }

    #[cfg(test)]
    fn persist_file(&self) -> &PersistFileVtable {
        unsafe { &**(self.0.cast::<*const PersistFileVtable>()) }
    }

    fn persist_stream(&self) -> &PersistStreamVtable {
        unsafe { &**(self.0.cast::<*const PersistStreamVtable>()) }
    }

    fn property_store(&self) -> &PropertyStoreVtable {
        unsafe { &**(self.0.cast::<*const PropertyStoreVtable>()) }
    }
}

impl Drop for ComPtr {
    fn drop(&mut self) {
        if !self.0.is_null() {
            let vtable = unsafe { &**(self.0.cast::<*const UnknownVtable>()) };
            unsafe { (vtable.release)(self.0) };
        }
    }
}

struct ComApartment;

impl ComApartment {
    fn initialize() -> Result<Self, String> {
        let result = unsafe { CoInitializeEx(ptr::null(), COINIT_MULTITHREADED as u32) };
        if result < 0 {
            Err(format!(
                "installer_shortcut_com_initialize_failed:{result:#010x}"
            ))
        } else {
            Ok(Self)
        }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

fn read_shortcut_contract(bytes: &[u8]) -> Result<ShortcutContract, String> {
    if bytes.is_empty() || bytes.len() > SHORTCUT_MAX_BYTES as usize {
        return Err("installer_shortcut_bytes_invalid".to_string());
    }
    let _apartment = ComApartment::initialize()?;
    let mut raw = ptr::null_mut();
    let result = unsafe {
        CoCreateInstance(
            &CLSID_SHELL_LINK,
            ptr::null_mut(),
            CLSCTX_INPROC_SERVER,
            &IID_SHELL_LINK_W,
            &mut raw,
        )
    };
    if result < 0 || raw.is_null() {
        return Err(format!("installer_shortcut_create_failed:{result:#010x}"));
    }
    let shell = ComPtr(raw);
    let stream = ComPtr(unsafe { SHCreateMemStream(bytes.as_ptr(), bytes.len() as u32) });
    if stream.0.is_null() {
        return Err("installer_shortcut_stream_create_failed".to_string());
    }
    let persist = shell.query(
        &IID_PERSIST_STREAM,
        "installer_shortcut_persist_stream_query_failed",
    )?;
    let result = unsafe { (persist.persist_stream().load)(persist.0, stream.0) };
    if result < 0 {
        return Err(format!(
            "installer_shortcut_stream_load_failed:{result:#010x}"
        ));
    }
    let target = shell_link_text(|buffer, capacity| unsafe {
        (shell.shell_link().get_path)(shell.0, buffer, capacity, ptr::null_mut(), SLGP_RAWPATH)
    })?;
    let description = shell_link_text(|buffer, capacity| unsafe {
        (shell.shell_link().get_description)(shell.0, buffer, capacity)
    })?;
    let working_directory = shell_link_text(|buffer, capacity| unsafe {
        (shell.shell_link().get_working_directory)(shell.0, buffer, capacity)
    })?;
    let arguments = shell_link_text(|buffer, capacity| unsafe {
        (shell.shell_link().get_arguments)(shell.0, buffer, capacity)
    })?;
    let mut hotkey = u16::MAX;
    let result = unsafe { (shell.shell_link().get_hotkey)(shell.0, &mut hotkey) };
    if result < 0 {
        return Err(format!("installer_shortcut_hotkey_failed:{result:#010x}"));
    }
    let mut show_command = i32::MIN;
    let result = unsafe { (shell.shell_link().get_show_command)(shell.0, &mut show_command) };
    if result < 0 {
        return Err(format!(
            "installer_shortcut_show_command_failed:{result:#010x}"
        ));
    }
    let mut icon_index = i32::MIN;
    let icon_path = shell_link_text(|buffer, capacity| unsafe {
        (shell.shell_link().get_icon_location)(shell.0, buffer, capacity, &mut icon_index)
    })?;
    let property_store = shell.query(
        &IID_PROPERTY_STORE,
        "installer_shortcut_property_store_query_failed",
    )?;
    let app_user_model_id = property_string(
        &property_store,
        &PKEY_APP_USER_MODEL_ID,
        "installer_shortcut_app_id",
    )?;
    Ok(ShortcutContract {
        target,
        arguments,
        icon_path,
        icon_index,
        working_directory,
        show_command,
        hotkey,
        description,
        app_user_model_id,
    })
}

fn shell_link_text(read: impl FnOnce(*mut u16, i32) -> i32) -> Result<String, String> {
    let mut buffer = vec![u16::MAX; COM_TEXT_CAPACITY];
    let result = read(buffer.as_mut_ptr(), buffer.len() as i32);
    if result < 0 {
        return Err(format!("installer_shortcut_property_failed:{result:#010x}"));
    }
    let Some(end) = buffer.iter().position(|value| *value == 0) else {
        return Err("installer_shortcut_property_unbounded".to_string());
    };
    String::from_utf16(&buffer[..end])
        .map_err(|_| "installer_shortcut_property_utf16_invalid".to_string())
}

fn property_string(store: &ComPtr, key: &PROPERTYKEY, context: &str) -> Result<String, String> {
    let mut value = PROPVARIANT::default();
    let result = unsafe { (store.property_store().get_value)(store.0, key, &mut value) };
    if result < 0 {
        unsafe { PropVariantClear(&mut value) };
        return Err(format!("{context}_read_failed:{result:#010x}"));
    }
    let text = if unsafe { value.Anonymous.Anonymous.vt } != VT_LPWSTR {
        Err(format!("{context}_type_invalid"))
    } else {
        let mut raw = ptr::null_mut();
        let conversion = unsafe { PropVariantToStringAlloc(&value, &mut raw) };
        let converted = if conversion < 0 || raw.is_null() {
            Err(format!("{context}_conversion_failed:{conversion:#010x}"))
        } else {
            read_nul_terminated_wide(raw, COM_TEXT_CAPACITY)
        };
        if !raw.is_null() {
            unsafe { CoTaskMemFree(raw.cast()) };
        }
        converted
    };
    unsafe { PropVariantClear(&mut value) };
    text
}

fn validate_shortcut_contract(
    contract: &ShortcutContract,
    monitor_path: &Path,
) -> Result<(), String> {
    let install_root = monitor_path
        .parent()
        .ok_or_else(|| "installer_shortcut_monitor_parent_missing".to_string())?;
    if !fixed_path_eq(Path::new(&contract.target), monitor_path)
        || !contract.arguments.is_empty()
        || !contract.icon_path.is_empty()
        || contract.icon_index != 0
        || !fixed_path_eq(Path::new(&contract.working_directory), install_root)
        || contract.show_command != 1
        || contract.hotkey != 0
        || !contract.description.is_empty()
        || contract.app_user_model_id != APP_USER_MODEL_ID
    {
        return Err("installer_shortcut_contract_invalid".to_string());
    }
    Ok(())
}

fn read_nul_terminated_wide(value: *const u16, capacity: usize) -> Result<String, String> {
    let mut length = 0_usize;
    while length < capacity && unsafe { *value.add(length) } != 0 {
        length += 1;
    }
    if length == capacity {
        return Err("installer_shortcut_wide_text_unbounded".to_string());
    }
    String::from_utf16(unsafe { std::slice::from_raw_parts(value, length) })
        .map_err(|_| "installer_shortcut_wide_text_utf16_invalid".to_string())
}

fn read_locked_file(handle: HANDLE, size: u64, context: &str) -> Result<Vec<u8>, String> {
    if !(1..=SHORTCUT_MAX_BYTES).contains(&size) {
        return Err("installer_shortcut_bytes_invalid".to_string());
    }
    let mut bytes = vec![0_u8; size as usize];
    let mut offset = 0_usize;
    while offset < bytes.len() {
        let mut read = 0_u32;
        if unsafe {
            ReadFile(
                handle,
                bytes[offset..].as_mut_ptr().cast(),
                (bytes.len() - offset) as u32,
                &mut read,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(last_error(context));
        }
        if read == 0 || read as usize > bytes.len() - offset {
            return Err(format!("{context}:truncated"));
        }
        offset += read as usize;
    }
    Ok(bytes)
}

fn file_information(handle: HANDLE, context: &str) -> Result<BY_HANDLE_FILE_INFORMATION, String> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    if unsafe { GetFileInformationByHandle(handle, &mut info) } == 0 {
        return Err(last_error(context));
    }
    Ok(info)
}

fn final_path(handle: &OwnedHandle, context: &str) -> Result<PathBuf, String> {
    let required = unsafe { GetFinalPathNameByHandleW(handle.raw(), ptr::null_mut(), 0, 0) };
    if required == 0 {
        return Err(last_error(context));
    }
    let mut buffer = vec![0_u16; required as usize + 1];
    let written = unsafe {
        GetFinalPathNameByHandleW(handle.raw(), buffer.as_mut_ptr(), buffer.len() as u32, 0)
    };
    if written == 0 || written as usize >= buffer.len() {
        return Err(last_error(context));
    }
    Ok(strip_verbatim_path_prefix(PathBuf::from(
        OsString::from_wide(&buffer[..written as usize]),
    )))
}

fn strip_verbatim_path_prefix(path: PathBuf) -> PathBuf {
    let text = path.as_os_str().to_string_lossy();
    if text
        .get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(r"\\?\UNC\"))
    {
        return PathBuf::from(format!(r"\\{}", &text[8..]));
    }
    if text
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(r"\\?\"))
    {
        return PathBuf::from(&text[4..]);
    }
    path
}

fn fixed_path_eq(left: &Path, right: &Path) -> bool {
    left.as_os_str()
        .to_string_lossy()
        .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
}

fn wide_path(value: &Path) -> Vec<u16> {
    value
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn file_index(info: &BY_HANDLE_FILE_INFORMATION) -> u64 {
    (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow)
}

fn filetime(value: windows_sys::Win32::Foundation::FILETIME) -> u64 {
    (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
}

fn invalid_handle(handle: HANDLE) -> bool {
    handle.is_null() || handle == (-1_isize as HANDLE)
}

fn missing_path_error(error: u32) -> bool {
    matches!(error, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND)
}

fn last_error(context: &str) -> String {
    format!("{context}:{}", unsafe { GetLastError() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::windows::fs::symlink_file};

    fn require_hresult(result: i32, context: &str) -> Result<(), String> {
        if result < 0 {
            Err(format!("{context}:{result:#010x}"))
        } else {
            Ok(())
        }
    }

    fn write_test_shortcut(path: &Path, contract: &ShortcutContract) -> Result<(), String> {
        let _apartment = ComApartment::initialize()?;
        let mut raw = ptr::null_mut();
        require_hresult(
            unsafe {
                CoCreateInstance(
                    &CLSID_SHELL_LINK,
                    ptr::null_mut(),
                    CLSCTX_INPROC_SERVER,
                    &IID_SHELL_LINK_W,
                    &mut raw,
                )
            },
            "test_shortcut_create_failed",
        )?;
        if raw.is_null() {
            return Err("test_shortcut_create_null".to_string());
        }
        let shell = ComPtr(raw);
        let target = wide_path(Path::new(&contract.target));
        require_hresult(
            unsafe { (shell.shell_link().set_path)(shell.0, target.as_ptr()) },
            "test_shortcut_target_failed",
        )?;
        let arguments = wide_text(&contract.arguments);
        require_hresult(
            unsafe { (shell.shell_link().set_arguments)(shell.0, arguments.as_ptr()) },
            "test_shortcut_arguments_failed",
        )?;
        let working_directory = wide_path(Path::new(&contract.working_directory));
        require_hresult(
            unsafe {
                (shell.shell_link().set_working_directory)(shell.0, working_directory.as_ptr())
            },
            "test_shortcut_working_directory_failed",
        )?;
        let description = wide_text(&contract.description);
        require_hresult(
            unsafe { (shell.shell_link().set_description)(shell.0, description.as_ptr()) },
            "test_shortcut_description_failed",
        )?;
        let icon = wide_path(Path::new(&contract.icon_path));
        require_hresult(
            unsafe {
                (shell.shell_link().set_icon_location)(shell.0, icon.as_ptr(), contract.icon_index)
            },
            "test_shortcut_icon_failed",
        )?;
        require_hresult(
            unsafe { (shell.shell_link().set_show_command)(shell.0, contract.show_command) },
            "test_shortcut_show_command_failed",
        )?;
        require_hresult(
            unsafe { (shell.shell_link().set_hotkey)(shell.0, contract.hotkey) },
            "test_shortcut_hotkey_failed",
        )?;

        let property_store = shell.query(&IID_PROPERTY_STORE, "test_shortcut_store_failed")?;
        let mut app_id = wide_text(&contract.app_user_model_id);
        let mut value = PROPVARIANT::default();
        value.Anonymous.Anonymous.vt = VT_LPWSTR;
        value.Anonymous.Anonymous.Anonymous.pwszVal = app_id.as_mut_ptr();
        require_hresult(
            unsafe {
                (property_store.property_store().set_value)(
                    property_store.0,
                    &PKEY_APP_USER_MODEL_ID,
                    &value,
                )
            },
            "test_shortcut_app_id_failed",
        )?;
        require_hresult(
            unsafe { (property_store.property_store().commit)(property_store.0) },
            "test_shortcut_store_commit_failed",
        )?;

        let persist = shell.query(&IID_PERSIST_FILE, "test_shortcut_persist_failed")?;
        let output = wide_path(path);
        require_hresult(
            unsafe { (persist.persist_file().save)(persist.0, output.as_ptr(), 1) },
            "test_shortcut_save_failed",
        )
    }

    fn wide_text(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn canonical_fixture_root(root: &tempfile::TempDir) -> PathBuf {
        strip_verbatim_path_prefix(
            fs::canonicalize(root.path()).expect("canonicalize shortcut fixture root"),
        )
    }

    fn relocated_monitor_path() -> PathBuf {
        PathBuf::from(r"D:\Apps\BatCave Monitor\batcave-monitor.exe")
    }

    fn exact_contract() -> ShortcutContract {
        let monitor_path = relocated_monitor_path();
        ShortcutContract {
            target: monitor_path.to_string_lossy().into_owned(),
            arguments: String::new(),
            icon_path: String::new(),
            icon_index: 0,
            working_directory: monitor_path
                .parent()
                .expect("monitor parent")
                .to_string_lossy()
                .into_owned(),
            show_command: 1,
            hotkey: 0,
            description: String::new(),
            app_user_model_id: APP_USER_MODEL_ID.to_string(),
        }
    }

    #[test]
    fn retirement_scope_is_exactly_the_two_shared_tauri_locations() {
        assert_eq!(
            LegacyShortcutLocation::ALL
                .into_iter()
                .map(LegacyShortcutLocation::label)
                .collect::<Vec<_>>(),
            vec!["public_desktop", "common_programs"]
        );
        let public = LegacyShortcutLocation::PublicDesktop.folder_id();
        assert_eq!(public.data1, FOLDERID_PublicDesktop.data1);
        assert_eq!(public.data2, FOLDERID_PublicDesktop.data2);
        assert_eq!(public.data3, FOLDERID_PublicDesktop.data3);
        assert_eq!(public.data4, FOLDERID_PublicDesktop.data4);
        let programs = LegacyShortcutLocation::CommonPrograms.folder_id();
        assert_eq!(programs.data1, FOLDERID_CommonPrograms.data1);
        assert_eq!(programs.data2, FOLDERID_CommonPrograms.data2);
        assert_eq!(programs.data3, FOLDERID_CommonPrograms.data3);
        assert_eq!(programs.data4, FOLDERID_CommonPrograms.data4);
        assert_eq!(SHORTCUT_NAME, "BatCave Monitor.lnk");
        assert_eq!(
            shortcut_path_for_root(Path::new(r"D:\Relocated Public\Desktop")),
            PathBuf::from(r"D:\Relocated Public\Desktop\BatCave Monitor.lnk")
        );
        assert_eq!(
            shortcut_path_for_root(Path::new(r"E:\Machine Data\Start Menu\Programs")),
            PathBuf::from(r"E:\Machine Data\Start Menu\Programs\BatCave Monitor.lnk")
        );
    }

    #[test]
    fn verbatim_unc_handle_paths_match_redirected_known_folders() {
        let redirected = PathBuf::from(r"\\server\share\Machine Data\Public Desktop");
        let handle_path = PathBuf::from(r"\\?\UNC\server\share\Machine Data\Public Desktop");
        let lower_handle_path = PathBuf::from(r"\\?\unc\server\share\Machine Data\Public Desktop");

        for observed in [handle_path, lower_handle_path] {
            let canonical = strip_verbatim_path_prefix(observed);
            assert_eq!(canonical, redirected);
            assert!(fixed_path_eq(&canonical, &redirected));
            assert_eq!(
                shortcut_path_for_root(&canonical),
                PathBuf::from(r"\\server\share\Machine Data\Public Desktop\BatCave Monitor.lnk")
            );
        }
        assert_eq!(
            strip_verbatim_path_prefix(PathBuf::from(r"\\?\D:\Relocated\Public Desktop")),
            PathBuf::from(r"D:\Relocated\Public Desktop")
        );
        assert_eq!(
            strip_verbatim_path_prefix(redirected.clone()),
            redirected,
            "an already canonical UNC path must remain unchanged"
        );
    }

    #[test]
    fn exact_legacy_shortcut_contract_accepts_a_verified_relocated_install_root() {
        assert_eq!(
            validate_shortcut_contract(&exact_contract(), &relocated_monitor_path()),
            Ok(())
        );
    }

    #[test]
    fn every_mutable_legacy_shortcut_field_is_fail_closed() {
        let cases = [
            ShortcutContract {
                target: r"C:\Temp\batcave-monitor.exe".to_string(),
                ..exact_contract()
            },
            ShortcutContract {
                arguments: "--unexpected".to_string(),
                ..exact_contract()
            },
            ShortcutContract {
                icon_path: relocated_monitor_path().to_string_lossy().into_owned(),
                ..exact_contract()
            },
            ShortcutContract {
                icon_index: 1,
                ..exact_contract()
            },
            ShortcutContract {
                working_directory: r"C:\Temp".to_string(),
                ..exact_contract()
            },
            ShortcutContract {
                show_command: 0,
                ..exact_contract()
            },
            ShortcutContract {
                hotkey: 1,
                ..exact_contract()
            },
            ShortcutContract {
                description: "foreign".to_string(),
                ..exact_contract()
            },
            ShortcutContract {
                app_user_model_id: "foreign.app".to_string(),
                ..exact_contract()
            },
        ];
        for contract in cases {
            assert_eq!(
                validate_shortcut_contract(&contract, &relocated_monitor_path()),
                Err("installer_shortcut_contract_invalid".to_string())
            );
        }
    }

    #[test]
    fn shortcut_identity_rejects_reparse_directory_hardlink_and_size_attacks() {
        let valid = ShortcutIdentity {
            attributes: 0,
            hardlink_count: 1,
            file_index: 1,
            size: 1024,
        };
        assert_eq!(valid.validate(), Ok(()));

        for identity in [
            ShortcutIdentity {
                attributes: FILE_ATTRIBUTE_REPARSE_POINT,
                ..valid
            },
            ShortcutIdentity {
                attributes: FILE_ATTRIBUTE_DIRECTORY,
                ..valid
            },
            ShortcutIdentity {
                hardlink_count: 2,
                ..valid
            },
            ShortcutIdentity {
                file_index: 0,
                ..valid
            },
            ShortcutIdentity { size: 0, ..valid },
            ShortcutIdentity {
                size: SHORTCUT_MAX_BYTES + 1,
                ..valid
            },
        ] {
            assert_eq!(
                identity.validate(),
                Err("installer_shortcut_identity_invalid".to_string())
            );
        }
    }

    #[test]
    fn native_com_roundtrip_deletes_only_the_exact_validated_link_handle() {
        let root = tempfile::tempdir().expect("create shortcut fixture root");
        let shortcut_path = canonical_fixture_root(&root).join(SHORTCUT_NAME);
        write_test_shortcut(&shortcut_path, &exact_contract()).expect("write exact shortcut");

        let shortcut = PinnedShortcut::open(&shortcut_path)
            .expect("open exact shortcut")
            .expect("exact shortcut present");
        assert_eq!(
            read_shortcut_contract(&shortcut.bytes).expect("read exact shortcut bytes"),
            exact_contract()
        );
        shortcut
            .validate_contract(&relocated_monitor_path())
            .expect("validate exact shortcut");
        shortcut.revalidate().expect("revalidate exact shortcut");
        shortcut.mark_for_deletion().expect("mark exact shortcut");
        drop(shortcut);

        assert!(
            PinnedShortcut::open(&shortcut_path)
                .expect("reopen deleted shortcut")
                .is_none(),
            "the exact original name must be absent after the validated handle closes"
        );
    }

    #[test]
    fn hostile_contract_and_hardlink_are_rejected_without_deletion() {
        let root = tempfile::tempdir().expect("create hostile shortcut fixture root");
        let root_path = canonical_fixture_root(&root);
        let hostile_path = root_path.join("hostile.lnk");
        write_test_shortcut(
            &hostile_path,
            &ShortcutContract {
                arguments: "--unexpected".to_string(),
                ..exact_contract()
            },
        )
        .expect("write hostile shortcut");
        let hostile = PinnedShortcut::open(&hostile_path)
            .expect("open hostile shortcut")
            .expect("hostile shortcut present");
        assert_eq!(
            hostile.validate_contract(&relocated_monitor_path()),
            Err("installer_shortcut_contract_invalid".to_string())
        );
        drop(hostile);
        assert!(
            hostile_path.is_file(),
            "a foreign contract must be preserved"
        );

        let valid_path = root_path.join("hardlinked.lnk");
        let alias_path = root_path.join("hardlinked-alias.lnk");
        write_test_shortcut(&valid_path, &exact_contract()).expect("write hardlink fixture");
        fs::hard_link(&valid_path, &alias_path).expect("create hardlink alias");
        assert_eq!(
            PinnedShortcut::open(&valid_path).expect_err("hardlinked shortcut must fail closed"),
            "installer_shortcut_identity_invalid"
        );
        assert!(valid_path.is_file());
        assert!(alias_path.is_file());
    }

    #[test]
    fn recreated_hostile_collision_is_detected_and_preserved_by_the_repeated_gate() {
        let root = tempfile::tempdir().expect("create recreated shortcut fixture root");
        let shortcut_path = canonical_fixture_root(&root).join(SHORTCUT_NAME);
        write_test_shortcut(&shortcut_path, &exact_contract()).expect("write original shortcut");
        let original = PinnedShortcut::open(&shortcut_path)
            .expect("open original shortcut")
            .expect("original shortcut present");
        original
            .validate_contract(&relocated_monitor_path())
            .expect("original contract");
        original.mark_for_deletion().expect("retire original");
        drop(original);

        let recreated = ShortcutContract {
            arguments: "--hostile-recreation".to_string(),
            ..exact_contract()
        };
        write_test_shortcut(&shortcut_path, &recreated).expect("recreate hostile shortcut");
        let collision = PinnedShortcut::open(&shortcut_path)
            .expect("open recreated collision")
            .expect("recreated collision present");
        assert_eq!(
            collision.validate_contract(&relocated_monitor_path()),
            Err("installer_shortcut_contract_invalid".to_string())
        );
        drop(collision);
        assert!(shortcut_path.is_file(), "the recreated object must remain");
    }

    #[test]
    fn reparse_shortcut_is_rejected_without_touching_its_target_when_supported() {
        let root = tempfile::tempdir().expect("create reparse fixture root");
        let root_path = canonical_fixture_root(&root);
        let target = root_path.join("target.lnk");
        let reparse = root_path.join("reparse.lnk");
        write_test_shortcut(&target, &exact_contract()).expect("write reparse target");
        if symlink_file(&target, &reparse).is_err() {
            return;
        }
        assert_eq!(
            PinnedShortcut::open(&reparse).expect_err("reparse shortcut must fail closed"),
            "installer_shortcut_identity_invalid"
        );
        assert!(target.is_file());
        assert!(reparse.exists());
    }
}
