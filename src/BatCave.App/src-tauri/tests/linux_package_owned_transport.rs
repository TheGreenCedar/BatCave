#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HostSupport {
    LinuxPackageTransport,
    UnsupportedHost,
}

fn host_support() -> HostSupport {
    if cfg!(target_os = "linux") {
        HostSupport::LinuxPackageTransport
    } else {
        HostSupport::UnsupportedHost
    }
}

#[test]
fn package_transport_probe_is_explicitly_linux_only() {
    if cfg!(target_os = "linux") {
        assert_eq!(host_support(), HostSupport::LinuxPackageTransport);
    } else {
        assert_eq!(host_support(), HostSupport::UnsupportedHost);
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use sha2::{Digest, Sha256};
    use std::ffi::CString;
    use std::fs::{self, File, OpenOptions};
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::os::fd::{AsRawFd, FromRawFd, RawFd};
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, ExitStatus, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, MutexGuard};
    use std::thread;
    use std::time::{Duration, Instant};

    const CONSUMER_FD: RawFd = 198;
    const APPIMAGE_LAUNCHER_MODE: &str = "BATCAVE_APPIMAGE_TRANSPORT_LAUNCHER";
    const MAX_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;
    const MAX_OUTPUT_BYTES: usize = 4096;
    const STEP_TIMEOUT: Duration = Duration::from_secs(120);
    const TERMINATION_GRACE: Duration = Duration::from_millis(500);
    const SETTLEMENT_TIMEOUT: Duration = Duration::from_secs(5);
    static PROBE_LOCK: Mutex<()> = Mutex::new(());
    static ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum PackageKind {
        Deb,
        AppImage,
    }

    impl PackageKind {
        fn directory(self) -> &'static str {
            match self {
                Self::Deb => "deb",
                Self::AppImage => "appimage",
            }
        }

        fn suffix(self) -> &'static str {
            match self {
                Self::Deb => ".deb",
                Self::AppImage => ".AppImage",
            }
        }

        fn memfd_name(self) -> &'static str {
            match self {
                Self::Deb => "batcave-linux-deb-transport",
                Self::AppImage => "batcave-linux-appimage-transport",
            }
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum FixedProbe {
        DebExtract,
        AppImageDescriptorPath,
        AppImageExecveat,
        AppImageFexecve,
    }

    impl FixedProbe {
        fn package_kind(self) -> PackageKind {
            match self {
                Self::DebExtract => PackageKind::Deb,
                Self::AppImageDescriptorPath | Self::AppImageExecveat | Self::AppImageFexecve => {
                    PackageKind::AppImage
                }
            }
        }
    }

    #[derive(Debug)]
    struct TransportOutcome {
        status: ExitStatus,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        output_bounded: bool,
        process_group_settled: bool,
        source_kind: &'static str,
        public_artifact_verified: bool,
        native_proven: bool,
        release_evidence_emitted: bool,
    }

    struct OwnedArtifact {
        descriptor: File,
        size: u64,
        sha256: [u8; 32],
        device: u64,
        inode: u64,
        required_seals: i32,
    }

    impl OwnedArtifact {
        fn validate(&self) -> Result<(), String> {
            let metadata = self
                .descriptor
                .metadata()
                .map_err(|error| format!("owned descriptor metadata failed: {error}"))?;
            if !metadata.is_file()
                || metadata.len() != self.size
                || metadata.dev() != self.device
                || metadata.ino() != self.inode
            {
                return Err("owned descriptor identity changed".to_string());
            }

            let flags = unsafe { libc::fcntl(self.descriptor.as_raw_fd(), libc::F_GETFL) };
            if flags < 0 || flags & libc::O_ACCMODE != libc::O_RDONLY {
                return Err("owned descriptor is not read-only".to_string());
            }
            let seals = unsafe { libc::fcntl(self.descriptor.as_raw_fd(), libc::F_GET_SEALS) };
            if seals < 0 || seals & self.required_seals != self.required_seals {
                return Err("owned descriptor seals changed".to_string());
            }

            let mut reader = self
                .descriptor
                .try_clone()
                .map_err(|error| format!("owned descriptor clone failed: {error}"))?;
            reader
                .seek(SeekFrom::Start(0))
                .map_err(|error| format!("owned descriptor rewind failed: {error}"))?;
            let mut hasher = Sha256::new();
            let copied = std::io::copy(&mut reader, &mut hasher)
                .map_err(|error| format!("owned descriptor rehash failed: {error}"))?;
            if copied != self.size || hasher.finalize().as_slice() != self.sha256 {
                return Err("owned descriptor bytes changed".to_string());
            }
            Ok(())
        }
    }

    struct PrivateRoot {
        path: PathBuf,
        removed: bool,
    }

    impl PrivateRoot {
        fn create(label: &str) -> Result<Self, String> {
            let sequence = ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "batcave-linux-package-transport-{}-{sequence}-{label}",
                std::process::id()
            ));
            fs::create_dir(&path)
                .map_err(|error| format!("private root creation failed: {error}"))?;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
                .map_err(|error| format!("private root permissions failed: {error}"))?;
            Ok(Self {
                path,
                removed: false,
            })
        }

        fn remove(mut self) -> Result<PathBuf, String> {
            fs::remove_dir_all(&self.path)
                .map_err(|error| format!("private root cleanup failed: {error}"))?;
            self.removed = true;
            Ok(self.path.clone())
        }
    }

    impl Drop for PrivateRoot {
        fn drop(&mut self) {
            if !self.removed {
                let _ = fs::remove_dir_all(&self.path);
            }
        }
    }

    struct BoundedOutput {
        bytes: Vec<u8>,
        overflowed: bool,
    }

    struct ProbeProcess {
        child: Child,
        process_group: libc::pid_t,
        stdout: Option<thread::JoinHandle<std::io::Result<BoundedOutput>>>,
        stderr: Option<thread::JoinHandle<std::io::Result<BoundedOutput>>>,
        settled: bool,
    }

    impl Drop for ProbeProcess {
        fn drop(&mut self) {
            if self.settled {
                return;
            }
            let _ = terminate_process_group(self.process_group, &mut self.child);
            self.settled =
                settle_process_group(self.process_group, &mut self.child).unwrap_or(false);
        }
    }

    struct SubreaperGuard {
        previous: i32,
    }

    impl SubreaperGuard {
        fn enable() -> Result<Self, String> {
            let mut previous = 0;
            if unsafe { libc::prctl(libc::PR_GET_CHILD_SUBREAPER, &mut previous) } != 0 {
                return Err(format!(
                    "subreaper state read failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            if previous == 0 && unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1) } != 0 {
                return Err(format!(
                    "subreaper enable failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(Self { previous })
        }
    }

    impl Drop for SubreaperGuard {
        fn drop(&mut self) {
            if self.previous == 0 {
                let _ = unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 0) };
            }
        }
    }

    fn lock_probes() -> MutexGuard<'static, ()> {
        PROBE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn bundle_artifact(kind: PackageKind) -> Result<PathBuf, String> {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("release")
            .join("bundle")
            .join(kind.directory());
        let mut matches = fs::read_dir(&directory)
            .map_err(|error| format!("bundle directory is unavailable: {error}"))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.ends_with(kind.suffix())
                            && name.contains(&format!("_{}_", env!("CARGO_PKG_VERSION")))
                    })
            })
            .collect::<Vec<_>>();
        matches.sort();
        if matches.len() != 1 {
            return Err(format!(
                "expected one version-bound {} bundle, found {}",
                kind.directory(),
                matches.len()
            ));
        }
        Ok(matches.remove(0))
    }

    fn acquire_owned_artifact(kind: PackageKind) -> Result<OwnedArtifact, String> {
        let source_path = bundle_artifact(kind)?;
        let inspected = fs::symlink_metadata(&source_path)
            .map_err(|error| format!("bundle metadata failed: {error}"))?;
        if !inspected.is_file()
            || inspected.file_type().is_symlink()
            || inspected.len() == 0
            || inspected.len() > MAX_ARTIFACT_BYTES
        {
            return Err("bundle must be a bounded regular non-link file".to_string());
        }

        let mut source = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(&source_path)
            .map_err(|error| format!("bundle open failed: {error}"))?;
        let opened = source
            .metadata()
            .map_err(|error| format!("opened bundle metadata failed: {error}"))?;
        if opened.dev() != inspected.dev()
            || opened.ino() != inspected.ino()
            || opened.len() != inspected.len()
        {
            return Err("bundle identity changed while opening".to_string());
        }

        let mut bytes = Vec::with_capacity(opened.len() as usize);
        source
            .read_to_end(&mut bytes)
            .map_err(|error| format!("bundle read failed: {error}"))?;
        if bytes.len() as u64 != opened.len() {
            return Err("bundle length changed while reading".to_string());
        }
        let sha256: [u8; 32] = Sha256::digest(&bytes).into();

        let name = CString::new(kind.memfd_name()).expect("fixed memfd name contains no NUL");
        let raw_fd = unsafe {
            libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING)
        };
        if raw_fd < 0 {
            return Err(format!(
                "memfd_create failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        let mut writable = unsafe { File::from_raw_fd(raw_fd) };
        writable
            .write_all(&bytes)
            .map_err(|error| format!("owned artifact write failed: {error}"))?;
        writable
            .flush()
            .map_err(|error| format!("owned artifact flush failed: {error}"))?;
        let mode = if kind == PackageKind::AppImage {
            0o500
        } else {
            0o400
        };
        if unsafe { libc::fchmod(writable.as_raw_fd(), mode) } != 0 {
            return Err(format!(
                "owned artifact permissions failed: {}",
                std::io::Error::last_os_error()
            ));
        }

        let required_seals =
            libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL;
        if unsafe { libc::fcntl(writable.as_raw_fd(), libc::F_ADD_SEALS, required_seals) } < 0 {
            return Err(format!(
                "owned artifact sealing failed: {}",
                std::io::Error::last_os_error()
            ));
        }

        let descriptor = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC)
            .open(format!("/proc/self/fd/{}", writable.as_raw_fd()))
            .map_err(|error| format!("read-only owned descriptor reopen failed: {error}"))?;
        let descriptor_metadata = descriptor
            .metadata()
            .map_err(|error| format!("owned descriptor metadata failed: {error}"))?;
        if descriptor_metadata.dev() == opened.dev() && descriptor_metadata.ino() == opened.ino() {
            return Err("owned artifact unexpectedly aliases the source".to_string());
        }
        let owned = OwnedArtifact {
            descriptor,
            size: bytes.len() as u64,
            sha256,
            device: descriptor_metadata.dev(),
            inode: descriptor_metadata.ino(),
            required_seals,
        };
        drop(writable);
        owned.validate()?;
        Ok(owned)
    }

    fn fixed_command(probe: FixedProbe, private_root: &Path) -> Result<Command, String> {
        let mut command = match probe {
            FixedProbe::DebExtract => {
                let payload_root = private_root.join("deb-payload");
                fs::create_dir(&payload_root)
                    .map_err(|error| format!("deb payload root creation failed: {error}"))?;
                let mut command = Command::new("/usr/bin/dpkg-deb");
                command
                    .arg("--extract")
                    .arg(format!("/proc/self/fd/{CONSUMER_FD}"))
                    .arg(payload_root);
                command
            }
            FixedProbe::AppImageDescriptorPath => {
                let mut command = Command::new(format!("/proc/self/fd/{CONSUMER_FD}"));
                command.arg("--appimage-offset");
                command
            }
            FixedProbe::AppImageExecveat | FixedProbe::AppImageFexecve => {
                let executable = std::env::current_exe()
                    .map_err(|error| format!("test executable lookup failed: {error}"))?;
                let test_name = match probe {
                    FixedProbe::AppImageExecveat => "linux::fixed_execveat_launcher_entry",
                    FixedProbe::AppImageFexecve => "linux::fixed_fexecve_launcher_entry",
                    _ => unreachable!(),
                };
                let mut command = Command::new(executable);
                command.arg("--exact").arg(test_name).arg("--nocapture");
                command
            }
        };
        command
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
            .env("LANG", "C")
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if matches!(
            probe,
            FixedProbe::AppImageExecveat | FixedProbe::AppImageFexecve
        ) {
            command.env(
                APPIMAGE_LAUNCHER_MODE,
                match probe {
                    FixedProbe::AppImageExecveat => "execveat",
                    FixedProbe::AppImageFexecve => "fexecve",
                    _ => unreachable!(),
                },
            );
        }
        Ok(command)
    }

    fn spawn_probe(
        artifact: &OwnedArtifact,
        probe: FixedProbe,
        private_root: &Path,
    ) -> Result<ProbeProcess, String> {
        if probe.package_kind() == PackageKind::AppImage {
            artifact.validate()?;
        }
        let descriptor = artifact.descriptor.as_raw_fd();
        let mut command = fixed_command(probe, private_root)?;
        unsafe {
            command.pre_exec(move || {
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::dup2(descriptor, CONSUMER_FD) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                let flags = libc::fcntl(CONSUMER_FD, libc::F_GETFD);
                if flags < 0
                    || libc::fcntl(CONSUMER_FD, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("fixed package probe spawn failed: {error}"))?;
        let process_group = child.id() as libc::pid_t;
        let stdout = child
            .stdout
            .take()
            .expect("piped package probe stdout is present after spawn");
        let stderr = child
            .stderr
            .take()
            .expect("piped package probe stderr is present after spawn");
        Ok(ProbeProcess {
            child,
            process_group,
            stdout: Some(thread::spawn(move || read_bounded(stdout))),
            stderr: Some(thread::spawn(move || read_bounded(stderr))),
            settled: false,
        })
    }

    fn run_probe(
        artifact: &OwnedArtifact,
        probe: FixedProbe,
        private_root: &Path,
    ) -> Result<TransportOutcome, String> {
        let _subreaper = SubreaperGuard::enable()?;
        let mut process = spawn_probe(artifact, probe, private_root)?;
        let status = match wait_for_child(&mut process.child, STEP_TIMEOUT)? {
            Some(status) => status,
            None => {
                terminate_process_group(process.process_group, &mut process.child)?;
                process.settled = settle_process_group(process.process_group, &mut process.child)?;
                if !process.settled {
                    return Err("timed-out package process group did not settle".to_string());
                }
                return Err("fixed package probe timed out".to_string());
            }
        };

        process.settled = settle_process_group(process.process_group, &mut process.child)?;
        if !process.settled {
            terminate_process_group(process.process_group, &mut process.child)?;
            process.settled = settle_process_group(process.process_group, &mut process.child)?;
        }

        let stdout = process
            .stdout
            .take()
            .expect("package probe owns its stdout reader")
            .join()
            .map_err(|_| "stdout reader panicked".to_string())?
            .map_err(|error| format!("stdout read failed: {error}"))?;
        let stderr = process
            .stderr
            .take()
            .expect("package probe owns its stderr reader")
            .join()
            .map_err(|_| "stderr reader panicked".to_string())?
            .map_err(|error| format!("stderr read failed: {error}"))?;
        artifact.validate()?;
        Ok(TransportOutcome {
            status,
            stdout: stdout.bytes,
            stderr: stderr.bytes,
            output_bounded: !stdout.overflowed && !stderr.overflowed,
            process_group_settled: process.settled,
            source_kind: "locally_built_bundle",
            public_artifact_verified: false,
            native_proven: false,
            release_evidence_emitted: false,
        })
    }

    fn read_bounded(mut reader: impl Read) -> std::io::Result<BoundedOutput> {
        let mut bytes = Vec::with_capacity(MAX_OUTPUT_BYTES);
        let mut overflowed = false;
        let mut buffer = [0_u8; 1024];
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            let remaining = MAX_OUTPUT_BYTES.saturating_sub(bytes.len());
            let retained = remaining.min(read);
            bytes.extend_from_slice(&buffer[..retained]);
            overflowed |= retained < read;
        }
        Ok(BoundedOutput { bytes, overflowed })
    }

    fn wait_for_child(child: &mut Child, timeout: Duration) -> Result<Option<ExitStatus>, String> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = child
                .try_wait()
                .map_err(|error| format!("package child wait failed: {error}"))?
            {
                return Ok(Some(status));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn terminate_process_group(
        process_group: libc::pid_t,
        child: &mut Child,
    ) -> Result<(), String> {
        signal_process_group(process_group, libc::SIGTERM)?;
        let deadline = Instant::now() + TERMINATION_GRACE;
        while process_group_exists(process_group) && Instant::now() < deadline {
            let _ = child.try_wait();
            reap_process_group(process_group)?;
            thread::sleep(Duration::from_millis(10));
        }
        if process_group_exists(process_group) {
            signal_process_group(process_group, libc::SIGKILL)?;
        }
        Ok(())
    }

    fn settle_process_group(process_group: libc::pid_t, child: &mut Child) -> Result<bool, String> {
        let deadline = Instant::now() + SETTLEMENT_TIMEOUT;
        loop {
            let _ = child
                .try_wait()
                .map_err(|error| format!("package child settlement failed: {error}"))?;
            reap_process_group(process_group)?;
            if !process_group_exists(process_group) {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn signal_process_group(process_group: libc::pid_t, signal: i32) -> Result<(), String> {
        let result = unsafe { libc::kill(-process_group, signal) };
        if result == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            Err(format!("process-group signal failed: {error}"))
        }
    }

    fn process_group_exists(process_group: libc::pid_t) -> bool {
        let result = unsafe { libc::kill(-process_group, 0) };
        if result == 0 {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }

    fn reap_process_group(process_group: libc::pid_t) -> Result<(), String> {
        loop {
            let mut status = 0;
            let result = unsafe { libc::waitpid(-process_group, &mut status, libc::WNOHANG) };
            if result > 0 {
                continue;
            }
            if result == 0 {
                return Ok(());
            }
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ECHILD) {
                return Ok(());
            }
            return Err(format!("process-group reap failed: {error}"));
        }
    }

    fn appimage_argv() -> (Vec<CString>, Vec<*const libc::c_char>) {
        let values = vec![
            CString::new("BatCave.Monitor.AppImage").expect("fixed argv contains no NUL"),
            CString::new("--appimage-offset").expect("fixed argv contains no NUL"),
        ];
        let mut pointers = values
            .iter()
            .map(|value| value.as_ptr())
            .collect::<Vec<_>>();
        pointers.push(std::ptr::null());
        (values, pointers)
    }

    fn appimage_env() -> (Vec<CString>, Vec<*const libc::c_char>) {
        let values = vec![
            CString::new("PATH=/usr/sbin:/usr/bin:/sbin:/bin")
                .expect("fixed environment contains no NUL"),
            CString::new("LANG=C").expect("fixed environment contains no NUL"),
            CString::new("LC_ALL=C").expect("fixed environment contains no NUL"),
        ];
        let mut pointers = values
            .iter()
            .map(|value| value.as_ptr())
            .collect::<Vec<_>>();
        pointers.push(std::ptr::null());
        (values, pointers)
    }

    #[test]
    fn fixed_execveat_launcher_entry() {
        if std::env::var(APPIMAGE_LAUNCHER_MODE).as_deref() != Ok("execveat") {
            return;
        }
        let (_argv_values, argv) = appimage_argv();
        let (_env_values, env) = appimage_env();
        let empty = CString::new("").expect("empty path contains no NUL");
        let result = unsafe {
            libc::execveat(
                CONSUMER_FD,
                empty.as_ptr(),
                argv.as_ptr().cast::<*mut libc::c_char>(),
                env.as_ptr().cast::<*mut libc::c_char>(),
                libc::AT_EMPTY_PATH,
            )
        };
        assert_eq!(
            result,
            0,
            "execveat failed: {}",
            std::io::Error::last_os_error()
        );
    }

    #[test]
    fn fixed_fexecve_launcher_entry() {
        if std::env::var(APPIMAGE_LAUNCHER_MODE).as_deref() != Ok("fexecve") {
            return;
        }
        let (_argv_values, argv) = appimage_argv();
        let (_env_values, env) = appimage_env();
        let result = unsafe { libc::fexecve(CONSUMER_FD, argv.as_ptr(), env.as_ptr()) };
        assert_eq!(
            result,
            0,
            "fexecve failed: {}",
            std::io::Error::last_os_error()
        );
    }

    fn assert_non_proof(outcome: &TransportOutcome) {
        assert_eq!(outcome.source_kind, "locally_built_bundle");
        assert!(!outcome.public_artifact_verified);
        assert!(!outcome.native_proven);
        assert!(!outcome.release_evidence_emitted);
        assert!(outcome.output_bounded);
        assert!(outcome.process_group_settled);
        assert!(
            outcome.status.success(),
            "fixed probe failed: {}",
            String::from_utf8_lossy(&outcome.stderr)
        );
    }

    #[test]
    #[ignore = "requires Linux deb/AppImage bundles built from this checkout"]
    fn built_deb_extracts_from_the_owned_descriptor_without_installing() {
        let _lock = lock_probes();
        let artifact = acquire_owned_artifact(PackageKind::Deb).expect("acquire built deb");
        let root = PrivateRoot::create("deb").expect("create private deb root");
        let outcome = run_probe(&artifact, FixedProbe::DebExtract, &root.path)
            .expect("run fixed dpkg-deb extraction");
        assert_non_proof(&outcome);
        assert!(outcome.stdout.is_empty());
        for binary in ["batcave-monitor", "batcave-monitor-cli"] {
            let path = root.path.join("deb-payload/usr/bin").join(binary);
            let metadata = fs::symlink_metadata(&path).expect("staged binary metadata");
            assert!(metadata.is_file());
            assert!(!metadata.file_type().is_symlink());
        }
        let removed = root.remove().expect("remove private deb root");
        assert!(!removed.exists());
    }

    #[test]
    #[ignore = "requires Linux deb/AppImage bundles built from this checkout"]
    fn built_appimage_runtime_accepts_all_closed_owned_descriptor_modes() {
        let _lock = lock_probes();
        let artifact = acquire_owned_artifact(PackageKind::AppImage).expect("acquire AppImage");
        for probe in [
            FixedProbe::AppImageDescriptorPath,
            FixedProbe::AppImageExecveat,
            FixedProbe::AppImageFexecve,
        ] {
            let root = PrivateRoot::create("appimage").expect("create private AppImage root");
            let outcome = run_probe(&artifact, probe, &root.path).expect("run AppImage probe");
            assert_non_proof(&outcome);
            let offset = String::from_utf8(outcome.stdout)
                .expect("AppImage offset is UTF-8")
                .trim()
                .parse::<u64>()
                .expect("AppImage offset is numeric");
            assert!(offset > 0 && offset < artifact.size);
            let removed = root.remove().expect("remove private AppImage root");
            assert!(!removed.exists());
        }
    }
}
