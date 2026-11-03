// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
//! One-shot, user-authorized macOS disk writes. The graphical process stays unprivileged.

use crate::{
    drives,
    ops::{self, Boot, Job, Msg},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    fs::{self, File},
    io::{self, Read, Write},
    net::Shutdown,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            fs::{FileTypeExt, MetadataExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError, Sender},
    },
    time::{Duration, Instant},
};

pub const WORKER_ARG: &str = "--rustfus-authorized-write";
const MAX_FRAME: usize = 1024 * 1024;
const WAIT: Duration = Duration::from_millis(100);
const AUTH_TIMEOUT: Duration = Duration::from_secs(180);
const CANCELLED: &str = "Cancelled";
const DISK_PRIVACY_ERROR: &str = "macOS blocked raw disk access";
pub const PRIVACY_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles";

pub fn is_disk_privacy_error(message: &str) -> bool {
    message.starts_with(DISK_PRIVACY_ERROR)
}

pub(crate) fn disk_open_error(device: &str, error: &io::Error) -> String {
    disk_open_error_for_uid(device, error, effective_uid())
}

fn disk_open_error_for_uid(device: &str, error: &io::Error, uid: u32) -> String {
    match (uid, error.raw_os_error()) {
        (0, Some(libc::EPERM)) => format!(
            "{DISK_PRIVACY_ERROR} to {device}: {error}.\n\n\
             The disk worker is already running with administrator privileges. macOS privacy or device-security restrictions can still deny access.\n\n\
             In System Settings → Privacy & Security, check Rustfus's Removable Volumes access under Files & Folders, if listed. If raw disk access remains blocked, allow Rustfus under Full Disk Access, then quit and reopen Rustfus. For cargo/Terminal launches, check the launching Terminal app instead.\n\n\
             If access is already allowed, close apps using this USB drive and reconnect it. Rustfus will not bypass macOS security or retry the write automatically."
        ),
        (0, Some(libc::EACCES)) => format!(
            "Cannot open {device}: {error}. The disk worker already has administrator privileges; check the device's access restrictions."
        ),
        (_, Some(libc::EPERM | libc::EACCES)) => format!(
            "Cannot open {device}: {error}. Start the write from the Rustfus app to request macOS administrator authorization, or run the command-line write with sudo."
        ),
        _ => format!("Cannot open {device}: {error}"),
    }
}
const APPLESCRIPT: &str = r#"on run argv
    set executablePath to item 1 of argv
    set socketPath to item 2 of argv
    set sessionToken to item 3 of argv
    set callerUID to item 4 of argv
    set commandText to quoted form of executablePath & " --rustfus-authorized-write " & quoted form of socketPath & " " & quoted form of sessionToken & " " & quoted form of callerUID
    do shell script commandText with administrator privileges
end run"#;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Hello {
    token: String,
}

// SCM_RIGHTS transfers the caller's already-open read-only image, not a path
// the privileged worker could resolve to a different, root-readable file.
fn send_image(stream: &UnixStream, file: &File) -> io::Result<()> {
    let mut byte = [0u8];
    let mut vector = libc::iovec {
        iov_base: byte.as_mut_ptr().cast(),
        iov_len: 1,
    };
    let mut control = [0usize; 8];
    // SAFETY: All msghdr/iovec/control buffers live through sendmsg. The sole
    // SCM_RIGHTS payload is one valid descriptor, and its length fits control.
    unsafe {
        let mut message: libc::msghdr = std::mem::zeroed();
        message.msg_iov = &mut vector;
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen = libc::CMSG_SPACE(std::mem::size_of::<libc::c_int>() as _) as _;
        let header = libc::CMSG_FIRSTHDR(&message);
        (*header).cmsg_level = libc::SOL_SOCKET;
        (*header).cmsg_type = libc::SCM_RIGHTS;
        (*header).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<libc::c_int>() as _) as _;
        std::ptr::write_unaligned(
            libc::CMSG_DATA(header).cast::<libc::c_int>(),
            file.as_raw_fd(),
        );
        if libc::sendmsg(stream.as_raw_fd(), &message, 0) != 1 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

fn receive_image(stream: &UnixStream) -> io::Result<File> {
    let mut byte = [0u8];
    let mut vector = libc::iovec {
        iov_base: byte.as_mut_ptr().cast(),
        iov_len: 1,
    };
    // Only accept one descriptor; do not leave room for an unowned extra fd.
    let mut control = [0usize; 3];
    // SAFETY: recvmsg writes only into the provided live buffers. A descriptor
    // is adopted exactly once after validating the ancillary header and payload.
    unsafe {
        let mut message: libc::msghdr = std::mem::zeroed();
        message.msg_iov = &mut vector;
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen = libc::CMSG_LEN(std::mem::size_of::<libc::c_int>() as _) as _;
        if libc::recvmsg(stream.as_raw_fd(), &mut message, 0) != 1 {
            return Err(io::Error::other("No image descriptor received"));
        }
        let header = libc::CMSG_FIRSTHDR(&message);
        if header.is_null()
            || (*header).cmsg_level != libc::SOL_SOCKET
            || (*header).cmsg_type != libc::SCM_RIGHTS
            || (*header).cmsg_len as usize
                != libc::CMSG_LEN(std::mem::size_of::<libc::c_int>() as _) as usize
        {
            return Err(io::Error::other("Invalid image descriptor message"));
        }
        let fd = std::ptr::read_unaligned(libc::CMSG_DATA(header).cast::<libc::c_int>());
        let file = File::from_raw_fd(fd);
        if message.msg_flags & libc::MSG_CTRUNC != 0
            || !file.metadata()?.is_file()
            || libc::fcntl(fd, libc::F_GETFL) & libc::O_ACCMODE != libc::O_RDONLY
        {
            return Err(io::Error::other("Image must be a regular read-only file"));
        }
        libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
        Ok(file)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    job: Job,
    language: String,
}

fn error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn send<T: Serialize>(writer: &mut impl Write, value: &T) -> io::Result<()> {
    let data = serde_json::to_vec(value).map_err(io::Error::other)?;
    if data.len() > MAX_FRAME {
        return Err(io::Error::other("Authorization message is too large"));
    }
    writer.write_all(&(data.len() as u32).to_be_bytes())?;
    writer.write_all(&data)?;
    writer.flush()
}

fn receive<T: DeserializeOwned>(reader: &mut impl Read) -> io::Result<T> {
    let mut size = [0; 4];
    reader.read_exact(&mut size)?;
    let size = u32::from_be_bytes(size) as usize;
    if size == 0 || size > MAX_FRAME {
        return Err(io::Error::other("Invalid authorization message size"));
    }
    let mut data = vec![0; size];
    reader.read_exact(&mut data)?;
    serde_json::from_slice(&data).map_err(io::Error::other)
}

fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
    let mut uid = 0;
    let mut gid = 0;
    // SAFETY: getpeereid writes to two valid uid_t/gid_t outputs for this open socket.
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(uid)
}

fn effective_uid() -> u32 {
    // SAFETY: geteuid takes no pointers and has no preconditions.
    unsafe { libc::geteuid() }
}

fn validate_device_path(path: &str) -> Result<(), String> {
    let disk = path
        .strip_prefix("/dev/disk")
        .ok_or("Authorization requires an external whole disk")?;
    if disk.is_empty() || !disk.bytes().all(|c| c.is_ascii_digit()) || disk == "0" {
        return Err("Authorization requires an external whole disk".into());
    }
    Ok(())
}

fn validate_job_shape(job: &Job) -> Result<(), String> {
    validate_device_path(&job.drive.path)?;
    if job.drive.size == 0 {
        return Err("The selected disk has no reported capacity".into());
    }
    if let Boot::Image(path) = &job.boot
        && (!path.is_absolute() || !path.is_file())
    {
        return Err("The selected image is no longer available".into());
    }
    if !job.fs.supported() || job.wtg.is_some() {
        return Err("This write mode is not supported on macOS".into());
    }
    Ok(())
}

fn validate_authorized_job(job: &Job) -> Result<(), String> {
    validate_job_shape(job)?;
    // Only the authorized process opens the raw disk on macOS.
    drives::verify(&job.drive)
}

struct AuthorizationProcess(Child);
impl Drop for AuthorizationProcess {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

fn authorization_error(stderr: &str) -> String {
    if stderr.contains("(-128)") || stderr.contains("User canceled") {
        CANCELLED.into()
    } else {
        format!("macOS could not authorize disk access: {}", stderr.trim())
    }
}

/// Called on the existing job thread, only after the destructive confirmation.
/// No password is passed to Rustfus; macOS owns the authorization prompt.
pub fn run_job(job: &Job, tx: &Sender<Msg>, cancel: &AtomicBool) -> Result<String, String> {
    if cancel.load(Ordering::Relaxed) {
        return Err(CANCELLED.into());
    }
    validate_job_shape(job)?;
    if effective_uid() == 0 {
        return ops::run_job(job, tx, cancel);
    }
    let image = match &job.boot {
        Boot::Image(path) => {
            let file =
                File::open(path).map_err(|e| format!("Cannot read the selected image: {e}"))?;
            if !file.metadata().map_err(error)?.is_file() {
                return Err("The image must be a regular file".into());
            }
            Some(file)
        }
        _ => None,
    };
    let executable = std::env::current_exe()
        .map_err(error)?
        .canonicalize()
        .map_err(error)?;
    // A short path avoids sockaddr_un's 104-byte macOS limit. tempfile creates a 0700 directory.
    let directory = tempfile::Builder::new()
        .prefix("rustfus-auth-")
        .tempdir_in("/tmp")
        .map_err(error)?;
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).map_err(error)?;
    let socket = directory.path().join("worker.sock");
    let listener = UnixListener::bind(&socket).map_err(error)?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).map_err(error)?;
    listener.set_nonblocking(true).map_err(error)?;
    let mut entropy = [0u8; 32];
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut entropy))
        .map_err(error)?;
    let token: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    let uid = effective_uid().to_string();
    let stderr = tempfile::tempfile().map_err(error)?;
    let stderr_reader = stderr.try_clone().map_err(error)?;
    let _ = tx.send(Msg::Progress(0., "Waiting for macOS authorization…".into()));
    let mut process = AuthorizationProcess(
        Command::new("/usr/bin/osascript")
            .arg("-e")
            .arg(APPLESCRIPT)
            .arg("--")
            .arg(&executable)
            .arg(&socket)
            .arg(&token)
            .arg(uid)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr)
            .spawn()
            .map_err(error)?,
    );
    let start = Instant::now();
    let stream = loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        if let Some(status) = process.0.try_wait().map_err(error)? {
            use std::io::{Seek, SeekFrom};
            let mut stderr = stderr_reader;
            stderr.seek(SeekFrom::Start(0)).map_err(error)?;
            let mut text = String::new();
            stderr
                .take(16384)
                .read_to_string(&mut text)
                .map_err(error)?;
            return Err(if status.success() {
                "The authorized worker did not connect".into()
            } else {
                authorization_error(&text)
            });
        }
        match listener.accept() {
            Ok((stream, _)) => {
                if peer_uid(&stream).map_err(error)? != 0 {
                    continue;
                }
                break stream;
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(WAIT),
            Err(e) => return Err(error(e)),
        }
        if start.elapsed() > AUTH_TIMEOUT {
            return Err("macOS authorization timed out; try again".into());
        }
    };
    stream.set_nonblocking(false).map_err(error)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(error)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(error)?;
    let mut reader = stream.try_clone().map_err(error)?;
    let mut writer = stream;
    let hello: Hello = receive(&mut reader).map_err(error)?;
    if hello.token != token {
        return Err("The authorized worker session did not match".into());
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(CANCELLED.into());
    }
    send(
        &mut writer,
        &Request {
            job: job.clone(),
            language: crate::loc::current(),
        },
    )
    .map_err(error)?;
    if let Some(image) = &image {
        send_image(&writer, image).map_err(error)?;
    }
    reader.set_read_timeout(None).map_err(error)?;
    writer.set_write_timeout(None).map_err(error)?;
    let (events_tx, events_rx) = mpsc::sync_channel(32);
    let reader_task = std::thread::spawn(move || {
        loop {
            let event = receive::<Msg>(&mut reader);
            let finished = matches!(event, Ok(Msg::Done(_)) | Err(_));
            if events_tx.send(event).is_err() || finished {
                break;
            }
        }
    });
    let mut cancellation_sent = false;
    let result = loop {
        if cancel.load(Ordering::Relaxed) && !cancellation_sent {
            let _ = writer.write_all(&[1]);
            cancellation_sent = true;
        }
        match events_rx.recv_timeout(WAIT) {
            Ok(Ok(Msg::Done(result))) => break result,
            Ok(Ok(message)) => {
                if tx.send(message).is_err() {
                    let _ = writer.write_all(&[1]);
                    cancellation_sent = true;
                }
            }
            Ok(Err(e)) => break Err(format!("Lost contact with the authorized disk worker: {e}")),
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                break Err("The authorized disk worker disconnected".into());
            }
        }
    };
    let _ = writer.shutdown(Shutdown::Both);
    drop(events_rx);
    let _ = reader_task.join();
    result
}

/// Internal one-shot entry. It does not accept jobs from files or command-line JSON.
pub fn worker(socket: &Path, token: &str, uid: u32) -> Result<(), String> {
    if effective_uid() != 0 {
        return Err("The disk worker requires macOS authorization".into());
    }
    if token.len() != 64 || !token.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("Invalid worker session".into());
    }
    let parent = socket.parent().ok_or("Invalid worker socket")?;
    let dir = fs::symlink_metadata(parent).map_err(error)?;
    let node = fs::symlink_metadata(socket).map_err(error)?;
    if !dir.is_dir()
        || dir.uid() != uid
        || dir.mode() & 0o777 != 0o700
        || !node.file_type().is_socket()
        || node.uid() != uid
        || node.mode() & 0o777 != 0o600
    {
        return Err("The worker connection is not private to the requesting user".into());
    }
    let mut stream = UnixStream::connect(socket).map_err(error)?;
    stream.set_nonblocking(false).map_err(error)?;
    if peer_uid(&stream).map_err(error)? != uid {
        return Err("The worker connection owner changed".into());
    }
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(error)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(error)?;
    send(
        &mut stream,
        &Hello {
            token: token.into(),
        },
    )
    .map_err(error)?;
    let mut request: Request = receive(&mut stream).map_err(error)?;
    let image = if matches!(request.job.boot, Boot::Image(_)) {
        Some(
            receive_image(&stream)
                .map_err(|e| format!("Receiving selected image descriptor: {e}"))?,
        )
    } else {
        None
    };
    let private = tempfile::Builder::new()
        .prefix("rustfus-worker-")
        .tempdir_in("/private/tmp")
        .map_err(|e| format!("Creating private disk-worker workspace: {e}"))?;
    let home = private.path().join("home");
    let temporary = private.path().join("tmp");
    fs::create_dir(&home).map_err(|e| format!("Creating private worker home: {e}"))?;
    fs::create_dir(&temporary)
        .map_err(|e| format!("Creating private worker temporary directory: {e}"))?;
    if let (Some(image), Boot::Image(path)) = (&image, &mut request.job.boot) {
        let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("img");
        if !extension.bytes().all(|c| c.is_ascii_alphanumeric()) {
            return Err("Invalid image extension".into());
        }
        let pinned = private.path().join(format!("image.{extension}"));
        std::os::unix::fs::symlink(format!("/dev/fd/{}", image.as_raw_fd()), &pinned)
            .map_err(|e| format!("Pinning selected image descriptor: {e}"))?;
        *path = pinned;
    }
    // Never inherit user-controlled executable search paths in the root worker.
    // SAFETY: main invokes this entry before creating any threads.
    unsafe {
        std::env::set_var("PATH", "/usr/bin:/bin:/usr/sbin:/sbin");
        std::env::set_var("HOME", &home);
        std::env::set_var("TMPDIR", &temporary);
        std::env::set_var("CURL_HOME", &home);
    }
    crate::loc::set_language(&request.language);
    stream.set_read_timeout(None).map_err(error)?;
    stream.set_write_timeout(None).map_err(error)?;
    serve(request.job, stream)
}

fn serve(job: Job, mut stream: UnixStream) -> Result<(), String> {
    let cancel = Arc::new(AtomicBool::new(false));
    let mut control = stream.try_clone().map_err(error)?;
    let monitor_cancel = cancel.clone();
    std::thread::scope(|scope| {
        let monitor = scope.spawn(move || {
            let mut signal = [0];
            let _ = control.read(&mut signal);
            monitor_cancel.store(true, Ordering::Relaxed);
        });
        let (tx, rx) = mpsc::channel();
        let worker_tx = tx.clone();
        let worker_cancel = cancel.clone();
        let worker = scope.spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = worker_tx.send(Msg::Log(format!(
                    "macOS disk worker effective UID: {}",
                    effective_uid()
                )));
                if worker_cancel.load(Ordering::Relaxed) {
                    return Err(CANCELLED.into());
                }
                validate_authorized_job(&job)?;
                if worker_cancel.load(Ordering::Relaxed) {
                    return Err(CANCELLED.into());
                }
                ops::run_job(&job, &worker_tx, &worker_cancel)
            }))
            .unwrap_or_else(|_| Err("The disk worker stopped unexpectedly".into()));
            let _ = tx.send(Msg::Done(result));
        });
        let mut result = Ok(());
        for event in rx {
            let done = matches!(event, Msg::Done(_));
            if let Err(e) = send(&mut stream, &event) {
                cancel.store(true, Ordering::Relaxed);
                result = Err(error(e));
                break;
            }
            if done {
                break;
            }
        }
        let _ = stream.shutdown(Shutdown::Both);
        let _ = worker.join();
        let _ = monitor.join();
        result
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transferred_image_is_read_only_and_pinned_when_the_path_is_replaced() {
        use std::io::Seek;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("image.iso");
        fs::write(&path, b"original image bytes").unwrap();
        let original = File::open(&path).unwrap();
        let (sender, receiver) = UnixStream::pair().unwrap();
        send_image(&sender, &original).unwrap();
        fs::rename(&path, directory.path().join("old.iso")).unwrap();
        fs::write(&path, b"replacement bytes").unwrap();
        let received = receive_image(&receiver).unwrap();
        let alias = directory.path().join("pinned.iso");
        std::os::unix::fs::symlink(format!("/dev/fd/{}", received.as_raw_fd()), &alias).unwrap();
        let mut reopened =
            File::open(&alias).expect("reopen the pinned descriptor through its image alias");
        let mut bytes = Vec::new();
        reopened.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"original image bytes");
        reopened.rewind().unwrap();
        assert!(reopened.write_all(b"changed").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"replacement bytes");
    }

    #[test]
    fn image_handoff_rejects_a_writable_descriptor() {
        let file = tempfile::tempfile().unwrap();
        let (sender, receiver) = UnixStream::pair().unwrap();
        send_image(&sender, &file).unwrap();
        assert!(receive_image(&receiver).is_err());
    }

    #[test]
    fn root_eperm_is_explained_as_privacy_or_device_security() {
        let error = io::Error::from_raw_os_error(libc::EPERM);
        let message = disk_open_error_for_uid("/dev/rdisk12", &error, 0);
        assert!(is_disk_privacy_error(&message));
        assert!(message.contains("System Settings"));
        assert!(!message.contains("run as root"));
    }

    #[test]
    fn unprivileged_eperm_keeps_the_authorization_guidance() {
        let error = io::Error::from_raw_os_error(libc::EPERM);
        let message = disk_open_error_for_uid("/dev/rdisk12", &error, 501);
        assert!(!is_disk_privacy_error(&message));
        assert!(message.contains("administrator authorization"));
    }

    #[test]
    fn frames_round_trip_and_reject_unbounded_or_invalid_messages() {
        let mut wire = Vec::new();
        send(&mut wire, &Msg::Progress(0.5, "Writing image: 50%".into())).unwrap();
        assert!(
            matches!(receive::<Msg>(&mut wire.as_slice()).unwrap(), Msg::Progress(p, _) if p == 0.5)
        );
        assert!(receive::<Hello>(&mut &((MAX_FRAME + 1) as u32).to_be_bytes()[..]).is_err());
        assert!(receive::<Hello>(&mut &[0, 0, 0, 0][..]).is_err());
        assert!(receive::<Hello>(&mut &[0, 0, 0, 2, b'{', b'}'][..]).is_err());
        assert!(receive::<Hello>(&mut &[0, 0, 0, 6, b'{'][..]).is_err());
    }

    #[test]
    fn only_whole_external_disk_paths_can_reach_verification() {
        assert!(validate_device_path("/dev/disk26").is_ok());
        for path in [
            "/dev/disk0",
            "/dev/disk",
            "/dev/rdisk26",
            "/dev/disk26s1",
            "/dev/disk26; touch x",
            "/tmp/disk26",
            "/dev/disk../x",
        ] {
            assert!(validate_device_path(path).is_err(), "{path}");
        }
    }

    #[test]
    fn user_cancel_is_not_a_permission_error() {
        assert_eq!(
            authorization_error("execution error: User canceled. (-128)"),
            CANCELLED
        );
        assert!(
            authorization_error("execution error: authorization failed (-60007)")
                .contains("macOS could not authorize")
        );
    }

    #[test]
    fn kernel_peer_identity_matches_current_user() {
        let (a, _b) = UnixStream::pair().unwrap();
        assert_eq!(peer_uid(&a).unwrap(), effective_uid());
    }

    #[test]
    fn cancelled_job_does_not_launch_a_password_prompt() {
        let job: Job = serde_json::from_value(serde_json::json!({
            "drive":{"path":"/dev/disk26","name":"Test","size":8000000000u64,"serial":null},
            "boot":"NonBootable","dd":false,"scheme":"Mbr","target":"BiosUefi","fs":"Fat32",
            "cluster":0,"label":"TEST","quick":true,"ext_label":false,"bad_blocks":null,
            "verify":false,"old_bios":false,"uefi_validation":false,"persistence":0,
            "wue":{"bypass_hw_checks":false,"no_online_account":false,"local_account":null,
            "regional":false,"no_data_collection":false,"no_bitlocker":false,"wtg_no_internal":false},
            "allow_downloads":false,"replace_c32":[],"wtg":null
        })).unwrap();
        let (tx, _rx) = mpsc::channel();
        assert_eq!(
            run_job(&job, &tx, &AtomicBool::new(true)),
            Err(CANCELLED.into())
        );
    }

    #[test]
    fn framed_progress_survives_idle_between_messages() {
        let (mut writer, mut reader) = UnixStream::pair().unwrap();
        let sender = std::thread::spawn(move || {
            send(&mut writer, &Msg::Progress(0.1, "Starting".into())).unwrap();
            std::thread::sleep(Duration::from_millis(150));
            send(&mut writer, &Msg::Done(Ok("Finished".into()))).unwrap();
        });
        reader
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        assert!(matches!(
            receive::<Msg>(&mut reader).unwrap(),
            Msg::Progress(..)
        ));
        assert!(matches!(
            receive::<Msg>(&mut reader).unwrap(),
            Msg::Done(Ok(_))
        ));
        sender.join().unwrap();
    }
}
