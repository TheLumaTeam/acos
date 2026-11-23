//! Local QEMU VM backend. Images are immutable bases; each computer gets a
//! private QCOW2 overlay and a QMP control socket in the host state directory.

use crate::{
    BackendError, CommandBackend, Computer, ComputerBackend, ComputerId, ComputerKind,
    ComputerState, CreateComputer, FileBackend, GuestCommandRequest, GuestCommandResult,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::Duration,
};

pub struct QemuBackend {
    images: PathBuf,
    state: PathBuf,
    executable: String,
    qemu_img: String,
    accelerator: String,
}

impl QemuBackend {
    pub fn new(images: PathBuf, state: PathBuf) -> Self {
        Self {
            images,
            state,
            executable: std::env::var("ACOS_QEMU").unwrap_or_else(|_| "qemu-system-aarch64".into()),
            qemu_img: std::env::var("ACOS_QEMU_IMG").unwrap_or_else(|_| "qemu-img".into()),
            accelerator: std::env::var("ACOS_QEMU_ACCEL").unwrap_or_else(|_| {
                if cfg!(target_os = "macos") {
                    "hvf"
                } else {
                    "kvm:tcg"
                }
                .into()
            }),
        }
    }

    fn dir(&self, id: &ComputerId) -> PathBuf {
        self.state.join(id.as_str())
    }
    fn disk(&self, id: &ComputerId) -> PathBuf {
        self.dir(id).join("disk.qcow2")
    }
    fn qmp(&self, id: &ComputerId) -> PathBuf {
        self.dir(id).join("qmp.sock")
    }
    fn qga(&self, id: &ComputerId) -> PathBuf {
        self.dir(id).join("qga.sock")
    }

    fn firmware(&self) -> Result<PathBuf, BackendError> {
        if let Some(path) = std::env::var_os("ACOS_QEMU_BIOS") {
            return Ok(PathBuf::from(path));
        }
        let executable = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|directory| directory.join(&self.executable))
            .find(|path| path.is_file())
            .ok_or_else(|| {
                backend(
                    "qemu_not_found",
                    format!("{} is not on PATH", self.executable),
                )
            })?;
        let executable_path = executable.canonicalize().map_err(io_error)?;
        let prefix = executable_path
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .ok_or_else(|| {
                backend(
                    "qemu_firmware_not_found",
                    "cannot determine QEMU installation prefix".into(),
                )
            })?;
        let firmware = prefix.join("share/qemu/edk2-aarch64-code.fd");
        if firmware.is_file() {
            Ok(firmware)
        } else {
            Err(backend(
                "qemu_firmware_not_found",
                format!(
                    "ARM64 UEFI firmware was not found at {}; set ACOS_QEMU_BIOS",
                    firmware.display()
                ),
            ))
        }
    }

    fn qga_request(
        &self,
        id: &ComputerId,
        request: serde_json::Value,
    ) -> Result<serde_json::Value, BackendError> {
        let stream = UnixStream::connect(self.qga(id)).map_err(|e| {
            backend(
                "guest_agent_unavailable",
                format!("cannot connect to guest agent: {e}"),
            )
        })?;
        stream.set_read_timeout(Some(Duration::from_secs(3))).ok();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|e| backend("guest_agent_protocol_error", e.to_string()))?;
        serde_json::to_writer(reader.get_mut(), &request)
            .map_err(|e| backend("guest_agent_protocol_error", e.to_string()))?;
        reader
            .get_mut()
            .write_all(b"\n")
            .map_err(|e| backend("guest_agent_protocol_error", e.to_string()))?;
        line.clear();
        reader
            .read_line(&mut line)
            .map_err(|e| backend("guest_agent_protocol_error", e.to_string()))?;
        let response: serde_json::Value = serde_json::from_str(&line)
            .map_err(|e| backend("guest_agent_protocol_error", e.to_string()))?;
        if let Some(error) = response.get("error") {
            return Err(backend("guest_agent_error", error.to_string()));
        }
        Ok(response.get("return").cloned().unwrap_or(response))
    }

    pub fn agent_info(&self, id: &ComputerId) -> Result<serde_json::Value, BackendError> {
        let mut info = serde_json::Map::new();
        for (key, command) in [
            ("agent", "guest-info"),
            ("os", "guest-get-osinfo"),
            ("hostname", "guest-get-host-name"),
            ("filesystems", "guest-get-fsinfo"),
            ("guest_time", "guest-get-time"),
        ] {
            let value = self.qga_request(id, serde_json::json!({"execute":command}));
            info.insert(key.into(), value.unwrap_or(serde_json::Value::Null));
        }
        Ok(serde_json::Value::Object(info))
    }

    fn validate_guest_path(path: &str) -> Result<(), BackendError> {
        if !path.starts_with("/workspace/")
            || path.split('/').any(|part| part == "..")
            || path.contains('\0')
        {
            return Err(backend(
                "invalid_guest_path",
                "file operations are restricted to /workspace and may not contain '..'".into(),
            ));
        }
        Ok(())
    }

    fn chown_workspace_file(&self, id: &ComputerId, path: &str) -> Result<(), BackendError> {
        let started = self.qga_request(id, serde_json::json!({"execute":"guest-exec","arguments":{"path":"/run/current-system/sw/bin/chown","arg":["agent:users","--",path]}}))?;
        let pid = started.get("pid").and_then(|v| v.as_u64()).ok_or_else(|| {
            backend(
                "guest_agent_protocol_error",
                "chown did not return a PID".into(),
            )
        })?;
        for _ in 0..100 {
            thread::sleep(Duration::from_millis(50));
            let status = self.qga_request(
                id,
                serde_json::json!({"execute":"guest-exec-status","arguments":{"pid":pid}}),
            )?;
            if status.get("exited").and_then(|v| v.as_bool()) == Some(true) {
                if status.get("exitcode").and_then(|v| v.as_i64()) == Some(0) {
                    return Ok(());
                }
                return Err(backend(
                    "guest_file_permission_error",
                    "could not assign workspace file to agent".into(),
                ));
            }
        }
        Err(backend(
            "guest_agent_timeout",
            "timed out assigning workspace file ownership".into(),
        ))
    }

    fn qmp_command(&self, id: &ComputerId, command: &str) -> Result<String, BackendError> {
        let path = self.qmp(id);
        let mut stream = UnixStream::connect(&path).map_err(|e| {
            backend(
                "qemu_unavailable",
                format!("cannot connect to QEMU monitor: {e}"),
            )
        })?;
        stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
        let mut input = [0_u8; 4096];
        let _ = stream.read(&mut input);
        stream
            .write_all(b"{\"execute\":\"qmp_capabilities\"}\r\n")
            .map_err(|e| backend("qemu_monitor_error", e.to_string()))?;
        let _ = stream.read(&mut input);
        stream
            .write_all(format!("{{\"execute\":\"{command}\"}}\r\n").as_bytes())
            .map_err(|e| backend("qemu_monitor_error", e.to_string()))?;
        let size = stream
            .read(&mut input)
            .map_err(|e| backend("qemu_monitor_error", e.to_string()))?;
        Ok(String::from_utf8_lossy(&input[..size]).into_owned())
    }

    fn observed(&self, id: &ComputerId, mut computer: Computer) -> Result<Computer, BackendError> {
        if !self.qmp(id).exists() {
            computer.state = ComputerState::Stopped;
            return Ok(computer);
        }
        let response = self.qmp_command(id, "query-status")?;
        computer.state = if response.contains("\"status\":\"running\"") {
            ComputerState::Running
        } else if response.contains("\"status\":\"paused\"") {
            ComputerState::Paused
        } else if response.contains("\"status\":\"shutdown\"") {
            ComputerState::Stopped
        } else {
            ComputerState::Unknown
        };
        Ok(computer)
    }

    fn read_metadata(&self, id: &ComputerId) -> Result<Computer, BackendError> {
        let path = self.dir(id).join("computer.json");
        let bytes = fs::read(path).map_err(|e| backend("computer_not_found", e.to_string()))?;
        serde_json::from_slice(&bytes).map_err(|e| backend("metadata_invalid", e.to_string()))
    }

    fn write_metadata(&self, computer: &Computer) -> Result<(), BackendError> {
        fs::create_dir_all(self.dir(&computer.id)).map_err(io_error)?;
        let bytes = serde_json::to_vec_pretty(computer)
            .map_err(|e| backend("metadata_invalid", e.to_string()))?;
        fs::write(self.dir(&computer.id).join("computer.json"), bytes).map_err(io_error)
    }
}

fn io_error(error: std::io::Error) -> BackendError {
    backend("host_io_error", error.to_string())
}
fn backend(code: &str, message: String) -> BackendError {
    BackendError {
        code: code.into(),
        message,
    }
}
fn safe_image(root: &Path, name: &str) -> Result<PathBuf, BackendError> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name == "." || name == ".." {
        return Err(backend(
            "invalid_image",
            "image must be a filename inside ACOS_IMAGE_DIR".into(),
        ));
    }
    let root = root.canonicalize().map_err(io_error)?;
    let image = root.join(name).canonicalize().map_err(io_error)?;
    if !image.starts_with(&root) || !image.is_file() {
        return Err(backend(
            "invalid_image",
            "image is not a regular file inside ACOS_IMAGE_DIR".into(),
        ));
    }
    Ok(image)
}

impl ComputerBackend for QemuBackend {
    fn name(&self) -> &'static str {
        "qemu"
    }
    fn supports(&self, kind: ComputerKind) -> bool {
        kind == ComputerKind::VirtualMachine
    }

    fn create(&self, request: &CreateComputer) -> Result<Computer, BackendError> {
        let base = safe_image(&self.images, &request.image)?;
        fs::create_dir_all(self.dir(&request.id)).map_err(io_error)?;
        let disk = self.disk(&request.id);
        let output = Command::new(&self.qemu_img)
            .args(["create", "-f", "qcow2", "-F", "qcow2", "-b"])
            .arg(base)
            .arg(&disk)
            .arg("16G")
            .output()
            .map_err(io_error)?;
        if !output.status.success() {
            return Err(backend(
                "image_overlay_failed",
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }
        let computer = Computer {
            id: request.id.clone(),
            name: request.name.clone(),
            kind: request.kind,
            state: ComputerState::Stopped,
            resources: request.resources.clone(),
            backend: self.name().into(),
        };
        self.write_metadata(&computer)?;
        Ok(computer)
    }

    fn inspect(&self, id: &ComputerId) -> Result<Computer, BackendError> {
        let computer = self.read_metadata(id)?;
        self.observed(id, computer)
    }

    fn start(&self, id: &ComputerId) -> Result<Computer, BackendError> {
        let mut computer = self.read_metadata(id)?;
        if self.qmp(id).exists() {
            return self.observed(id, computer);
        }
        let qmp = self.qmp(id);
        let serial = self.dir(id).join("serial.log");
        let output = Command::new(&self.executable)
            .args([
                "-name",
                id.as_str(),
                "-machine",
                "virt",
                "-cpu",
                "host",
                "-accel",
            ])
            .arg(&self.accelerator)
            .arg("-bios")
            .arg(self.firmware()?)
            .args(["-smp"])
            .arg(computer.resources.vcpus.to_string())
            .args(["-m"])
            .arg(computer.resources.memory_mib.to_string())
            .arg("-drive")
            .arg(format!(
                "if=virtio,format=qcow2,file={}",
                self.disk(id).display()
            ))
            .args(["-display", "none", "-serial"])
            .arg(format!("file:{}", serial.display()))
            .args([
                "-netdev",
                "user,id=net0",
                "-device",
                "virtio-net-pci,netdev=net0",
            ])
            .arg("-qmp")
            .arg(format!("unix:{},server=on,wait=off", qmp.display()))
            .arg("-chardev")
            .arg(format!(
                "socket,path={},server=on,wait=off,id=qga0",
                self.dir(id).join("qga.sock").display()
            ))
            .args([
                "-device",
                "virtio-serial",
                "-device",
                "virtserialport,chardev=qga0,name=org.qemu.guest_agent.0",
            ])
            .arg("-daemonize")
            .output()
            .map_err(io_error)?;
        if !output.status.success() {
            return Err(backend(
                "qemu_start_failed",
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }
        thread::sleep(Duration::from_millis(150));
        computer.state = ComputerState::Running;
        self.write_metadata(&computer)?;
        self.observed(id, computer)
    }

    fn stop(&self, id: &ComputerId) -> Result<Computer, BackendError> {
        let mut computer = self.read_metadata(id)?;
        if self.qmp(id).exists() {
            let _ = self.qmp_command(id, "system_powerdown");
        }
        computer.state = ComputerState::Stopping;
        self.write_metadata(&computer)?;
        Ok(computer)
    }

    fn pause(&self, id: &ComputerId) -> Result<Computer, BackendError> {
        let mut computer = self.read_metadata(id)?;
        self.qmp_command(id, "stop")?;
        computer.state = ComputerState::Paused;
        self.write_metadata(&computer)?;
        Ok(computer)
    }

    fn resume(&self, id: &ComputerId) -> Result<Computer, BackendError> {
        let mut computer = self.read_metadata(id)?;
        self.qmp_command(id, "cont")?;
        computer.state = ComputerState::Running;
        self.write_metadata(&computer)?;
        Ok(computer)
    }

    fn delete(&self, id: &ComputerId) -> Result<(), BackendError> {
        if self.qmp(id).exists() {
            let _ = self.qmp_command(id, "quit");
        }
        fs::remove_dir_all(self.dir(id)).map_err(io_error)
    }
}

impl CommandBackend for QemuBackend {
    fn execute(
        &self,
        id: &ComputerId,
        request: &GuestCommandRequest,
    ) -> Result<GuestCommandResult, BackendError> {
        if request.argv.is_empty() || request.argv[0].is_empty() || request.argv.len() > 256 {
            return Err(backend(
                "invalid_command",
                "argv must contain 1-256 arguments".into(),
            ));
        }
        if request.argv.iter().any(|arg| arg.len() > 32_768) {
            return Err(backend(
                "invalid_command",
                "an argument exceeds 32 KiB".into(),
            ));
        }
        let timeout = request.timeout_ms.unwrap_or(30_000).clamp(1, 3_600_000);
        let output_limit = request
            .output_limit_bytes
            .unwrap_or(1_048_576)
            .clamp(1024, 10_485_760);
        let cwd = request.working_directory.as_deref().unwrap_or("/workspace");
        if !cwd.starts_with('/') || cwd.split('/').any(|part| part == "..") {
            return Err(backend(
                "invalid_working_directory",
                "working_directory must be an absolute guest path without '..'".into(),
            ));
        }
        let mut env = Vec::new();
        for (key, value) in &request.environment {
            if key.is_empty()
                || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                || key.starts_with("LD_")
                || value.len() > 16_384
            {
                return Err(backend(
                    "invalid_environment",
                    format!("invalid environment entry: {key}"),
                ));
            }
            env.push(format!("{key}={value}"));
        }
        let started = std::time::Instant::now();
        // QGA runs as root; drop privileges before invoking the guest process.
        let mut argv = vec![
            "-u".to_string(),
            "agent".to_string(),
            "--".to_string(),
            "/run/current-system/sw/bin/timeout".to_string(),
            "--foreground".to_string(),
            "--signal=TERM".to_string(),
            "--kill-after=2s".to_string(),
            format!("{timeout}ms"),
            "/usr/bin/env".to_string(),
            "-C".to_string(),
            cwd.to_string(),
        ];
        argv.extend(env);
        argv.push("--".to_string());
        argv.extend(request.argv.iter().cloned());
        let args =
            serde_json::json!({"path":"/run/wrappers/bin/sudo","arg":argv,"capture-output":true});
        let started_command = self.qga_request(
            id,
            serde_json::json!({"execute":"guest-exec","arguments":args}),
        )?;
        let pid = started_command
            .get("pid")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| {
                backend(
                    "guest_agent_protocol_error",
                    "guest-exec did not return a PID".into(),
                )
            })?;
        loop {
            if started.elapsed() > Duration::from_millis(timeout) {
                return Err(backend(
                    "command_timeout",
                    format!("command exceeded {timeout} ms"),
                ));
            }
            thread::sleep(Duration::from_millis(150));
            let status = self.qga_request(
                id,
                serde_json::json!({"execute":"guest-exec-status","arguments":{"pid":pid}}),
            )?;
            if status.get("exited").and_then(|v| v.as_bool()) == Some(true) {
                let stdout = status
                    .get("out-data")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let stderr = status
                    .get("err-data")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let stdout_bytes = STANDARD.decode(stdout).unwrap_or_default();
                let stderr_bytes = STANDARD.decode(stderr).unwrap_or_default();
                let truncated = stdout_bytes.len() + stderr_bytes.len() > output_limit;
                let mut remaining = output_limit;
                let out = &stdout_bytes[..stdout_bytes.len().min(remaining)];
                remaining -= out.len();
                let err = &stderr_bytes[..stderr_bytes.len().min(remaining)];
                return Ok(GuestCommandResult {
                    exit_code: status
                        .get("exitcode")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(-1) as i32,
                    stdout: STANDARD.encode(out),
                    stderr: STANDARD.encode(err),
                    duration_ms: started.elapsed().as_millis() as u64,
                    truncated,
                });
            }
        }
    }
}

impl FileBackend for QemuBackend {
    fn read_file(&self, id: &ComputerId, path: &str) -> Result<Vec<u8>, BackendError> {
        Self::validate_guest_path(path)?;
        let opened = self.qga_request(
            id,
            serde_json::json!({"execute":"guest-file-open","arguments":{"path":path,"mode":"rb"}}),
        )?;
        let handle = opened.as_u64().ok_or_else(|| {
            backend(
                "guest_agent_protocol_error",
                "guest-file-open returned no handle".into(),
            )
        })?;
        let result = (|| {
            let mut contents = Vec::new();
            loop {
                let chunk = self.qga_request(id, serde_json::json!({"execute":"guest-file-read","arguments":{"handle":handle,"count":65536}}))?;
                if let Some(encoded) = chunk.get("buf-b64").and_then(|v| v.as_str()) {
                    contents.extend(
                        STANDARD
                            .decode(encoded)
                            .map_err(|e| backend("guest_agent_protocol_error", e.to_string()))?,
                    );
                }
                if contents.len() > 10_485_760 {
                    return Err(backend(
                        "file_too_large",
                        "file exceeds the 10 MiB transfer limit".into(),
                    ));
                }
                if chunk.get("eof").and_then(|v| v.as_bool()) == Some(true) {
                    break;
                }
            }
            Ok(contents)
        })();
        let _ = self.qga_request(
            id,
            serde_json::json!({"execute":"guest-file-close","arguments":{"handle":handle}}),
        );
        result
    }

    fn write_file(&self, id: &ComputerId, path: &str, contents: &[u8]) -> Result<(), BackendError> {
        Self::validate_guest_path(path)?;
        if contents.len() > 10_485_760 {
            return Err(backend(
                "file_too_large",
                "file exceeds the 10 MiB transfer limit".into(),
            ));
        }
        let opened = self.qga_request(
            id,
            serde_json::json!({"execute":"guest-file-open","arguments":{"path":path,"mode":"wb"}}),
        )?;
        let handle = opened.as_u64().ok_or_else(|| {
            backend(
                "guest_agent_protocol_error",
                "guest-file-open returned no handle".into(),
            )
        })?;
        let result = (|| {
            for chunk in contents.chunks(48 * 1024) {
                let response = self.qga_request(id, serde_json::json!({"execute":"guest-file-write","arguments":{"handle":handle,"buf-b64":STANDARD.encode(chunk)}}))?;
                if response.get("count").and_then(|v| v.as_u64()) != Some(chunk.len() as u64) {
                    return Err(backend(
                        "guest_file_write_failed",
                        "guest agent wrote fewer bytes than requested".into(),
                    ));
                }
            }
            Ok(())
        })();
        let close = self.qga_request(
            id,
            serde_json::json!({"execute":"guest-file-close","arguments":{"handle":handle}}),
        );
        result?;
        close?;
        self.chown_workspace_file(id, path)
    }
}
