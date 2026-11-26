// Windows named pipes: one instance per accepted front end.

use std::ffi::{OsStr, c_void};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const INVALID_HANDLE_VALUE: *mut c_void = -1isize as *mut c_void;
const PIPE_ACCESS_DUPLEX: u32 = 0x0000_0003;
const PIPE_REJECT_REMOTE_CLIENTS: u32 = 0x0000_0008;
const PIPE_UNLIMITED_INSTANCES: u32 = 255;
const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const OPEN_EXISTING: u32 = 3;

const ERROR_FILE_NOT_FOUND: i32 = 2;
const ERROR_BROKEN_PIPE: i32 = 109;
const ERROR_PIPE_BUSY: i32 = 231;
const ERROR_NO_DATA: i32 = 232;
const ERROR_PIPE_CONNECTED: i32 = 535;
const ERROR_OPERATION_ABORTED: i32 = 995;
const ERROR_IO_PENDING: i32 = 997;

const WAIT_OBJECT_0: u32 = 0;
const INFINITE: u32 = 0xFFFF_FFFF;

const BUFFER: u32 = 64 * 1024;
const RETRIES: usize = 50;

// The C layout the kernel writes into during overlapped I/O.
#[repr(C)]
struct OverlappedOs {
    internal: usize,
    internal_high: usize,
    offset: u32,
    offset_high: u32,
    event: *mut c_void,
}

unsafe extern "system" {
    fn CreateNamedPipeW(
        name: *const u16,
        open_mode: u32,
        pipe_mode: u32,
        max_instances: u32,
        out_buffer: u32,
        in_buffer: u32,
        timeout: u32,
        attributes: *mut c_void,
    ) -> *mut c_void;
    fn ConnectNamedPipe(pipe: *mut c_void, overlapped: *mut OverlappedOs) -> i32;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        attributes: *mut c_void,
        creation: u32,
        flags: u32,
        template: *mut c_void,
    ) -> *mut c_void;
    fn ReadFile(
        pipe: *mut c_void,
        buffer: *mut u8,
        len: u32,
        read: *mut u32,
        overlapped: *mut OverlappedOs,
    ) -> i32;
    fn WriteFile(
        pipe: *mut c_void,
        buffer: *const u8,
        len: u32,
        written: *mut u32,
        overlapped: *mut OverlappedOs,
    ) -> i32;
    fn CloseHandle(handle: *mut c_void) -> i32;
    fn CreateEventW(
        attributes: *mut c_void,
        manual: i32,
        initial: i32,
        name: *const u16,
    ) -> *mut c_void;
    fn SetEvent(event: *mut c_void) -> i32;
    fn ResetEvent(event: *mut c_void) -> i32;
    fn WaitForSingleObject(handle: *mut c_void, timeout: u32) -> u32;
    fn WaitForMultipleObjects(
        count: u32,
        handles: *const *mut c_void,
        wait_all: i32,
        timeout: u32,
    ) -> u32;
    fn GetOverlappedResult(
        handle: *mut c_void,
        overlapped: *mut OverlappedOs,
        transferred: *mut u32,
        wait: i32,
    ) -> i32;
    fn SetNamedPipeHandleState(
        pipe: *mut c_void,
        mode: *mut u32,
        max_collection: *mut u32,
        timeout: *mut u32,
    ) -> i32;
    fn WaitNamedPipeW(name: *const u16, timeout: u32) -> i32;
    fn CancelIoEx(handle: *mut c_void, overlapped: *mut OverlappedOs) -> i32;
}

// A pipe name as UTF-16, NUL-terminated.
fn wide(name: &str) -> Vec<u16> {
    OsStr::new(name)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

// One unused pipe instance: the server's door for the next front end.
fn instance(name: &str) -> io::Result<*mut c_void> {
    let wide = wide(name);
    let handle = unsafe {
        CreateNamedPipeW(
            wide.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
            // The other mode bits (byte type, byte read mode, blocking wait) are zero.
            PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            BUFFER,
            BUFFER,
            0,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    Ok(handle)
}

// A kernel event with the handle that closes it.
struct Event(*mut c_void);

// Signaling an event from any thread is safe; the handle carries no per-thread state.
unsafe impl Send for Event {}
unsafe impl Sync for Event {}

impl Event {
    fn new(manual: bool) -> io::Result<Event> {
        let event = unsafe { CreateEventW(ptr::null_mut(), manual as i32, 0, ptr::null()) };
        if event.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Event(event))
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

// One I/O slot: the kernel-owned layout plus the event that signals its completion.
struct Overlapped {
    os: OverlappedOs,
    event: Event,
}

// One slot belongs to one thread at a time; only its owner moves.
unsafe impl Send for Overlapped {}

// Every use that writes the slot takes `&mut`, so a shared reference reaches nothing that can change under it.
unsafe impl Sync for Overlapped {}

impl Overlapped {
    fn new() -> io::Result<Overlapped> {
        let event = Event::new(false)?;
        let mut os: OverlappedOs = unsafe { std::mem::zeroed() };
        os.event = event.0;
        Ok(Overlapped { os, event })
    }
}

// The listening end. One instance is kept ready for the next front end.
pub struct Listener {
    name: String,
    pending: Mutex<usize>, // the next instance; 0 means none, usize keeps the Listener Send
}

// One connected end.
pub struct Stream {
    handle: Arc<Handle>,
    closed: Arc<AtomicBool>,
    stop: Arc<Event>,  // shared by every clone, set once to wake a blocked read
    read: Overlapped,  // this end's one read slot
    write: Overlapped, // this end's one write slot
}

// A handle is a kernel object, safe to move and to use from either thread its owner picks.
struct Handle(*mut c_void);

unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

impl Stream {
    fn new(handle: Handle) -> io::Result<Stream> {
        Ok(Stream {
            handle: Arc::new(handle),
            closed: Arc::new(AtomicBool::new(false)),
            stop: Arc::new(Event::new(true)?),
            read: Overlapped::new()?,
            write: Overlapped::new()?,
        })
    }
}

// One pipe, one handle: a clone shares the handle and the stop event, and gets its own I/O slots.
impl Clone for Stream {
    fn clone(&self) -> Stream {
        Stream {
            handle: self.handle.clone(),
            closed: self.closed.clone(),
            stop: self.stop.clone(),
            read: Overlapped::new().expect("cannot create the pipe read event"),
            write: Overlapped::new().expect("cannot create the pipe write event"),
        }
    }
}

pub fn bind(name: &str) -> io::Result<Listener> {
    let handle = instance(name)?;
    Ok(Listener {
        name: name.to_string(),
        pending: Mutex::new(handle as usize),
    })
}

impl Listener {
    pub fn accept(&self) -> io::Result<Stream> {
        let pending = {
            let mut slot = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::replace(&mut *slot, 0)
        };
        let handle = if pending == 0 {
            instance(&self.name)?
        } else {
            pending as *mut c_void
        };
        let handle = Handle(handle);
        let mut overlapped = Overlapped::new()?;
        let ok = unsafe { ConnectNamedPipe(handle.0, &mut overlapped.os) };
        if ok == 0 {
            let error = io::Error::last_os_error();
            match error.raw_os_error() {
                // The front end may have connected between creation and the wait.
                Some(ERROR_PIPE_CONNECTED) => {}
                Some(ERROR_IO_PENDING) => {
                    if unsafe { WaitForSingleObject(overlapped.event.0, INFINITE) } != WAIT_OBJECT_0
                    {
                        return Err(io::Error::last_os_error());
                    }
                    let mut transferred = 0u32;
                    if unsafe {
                        GetOverlappedResult(handle.0, &mut overlapped.os, &mut transferred, 0)
                    } == 0
                    {
                        return Err(io::Error::last_os_error());
                    }
                }
                _ => return Err(error),
            }
        }
        // Have the next instance waiting before the loop comes back.
        if let Ok(next) = instance(&self.name) {
            *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = next as usize;
        }
        Stream::new(handle)
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let handle = std::mem::replace(
            &mut *self.pending.lock().unwrap_or_else(|e| e.into_inner()),
            0,
        );
        if handle != 0 {
            unsafe { CloseHandle(handle as *mut c_void) };
        }
    }
}

pub fn connect(name: &str) -> io::Result<Stream> {
    let wide = wide(name);
    let mut tries = 0;
    loop {
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                ptr::null_mut(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                ptr::null_mut(),
            )
        };
        if handle != INVALID_HANDLE_VALUE {
            let mut mode = 0u32; // PIPE_READMODE_BYTE
            unsafe { SetNamedPipeHandleState(handle, &mut mode, ptr::null_mut(), ptr::null_mut()) };
            return Stream::new(Handle(handle));
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_PIPE_BUSY) && tries < RETRIES {
            // Every instance is taken: the server is between accepts. Wait for the next one.
            unsafe { WaitNamedPipeW(wide.as_ptr(), 20) };
            std::thread::sleep(Duration::from_millis(10));
            tries += 1;
            continue;
        }
        return Err(match error.raw_os_error() {
            Some(ERROR_FILE_NOT_FOUND) => io::Error::new(
                io::ErrorKind::NotFound,
                format!("no server listening on {name}"),
            ),
            _ => error,
        });
    }
}

impl Stream {
    // Wake a blocked read; later operations see the flag; a write in flight finishes or fails.
    pub fn shutdown(&self) {
        self.closed.store(true, Ordering::SeqCst);
        unsafe { SetEvent(self.stop.0) };
    }
}

impl io::Read for Stream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.closed.load(Ordering::Acquire) {
            return Ok(0);
        }
        if unsafe { ResetEvent(self.read.event.0) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut read = 0u32;
        let ok = unsafe {
            ReadFile(
                self.handle.0,
                out.as_mut_ptr(),
                out.len() as u32,
                &mut read,
                &mut self.read.os,
            )
        };
        if ok != 0 {
            return if self.closed.load(Ordering::Acquire) {
                Ok(0)
            } else {
                Ok(read as usize)
            };
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_IO_PENDING) {
            return match error.raw_os_error() {
                Some(ERROR_BROKEN_PIPE) | Some(ERROR_NO_DATA) => Ok(0),
                Some(ERROR_OPERATION_ABORTED) if self.closed.load(Ordering::Acquire) => Ok(0),
                _ => Err(error),
            };
        }
        let events = [self.read.event.0, self.stop.0];
        let wait = unsafe { WaitForMultipleObjects(2, events.as_ptr(), 0, INFINITE) };
        if wait == WAIT_OBJECT_0 + 1 {
            unsafe { CancelIoEx(self.handle.0, &mut self.read.os) };
            // Reap the cancelled I/O before the slot is reused by the next read.
            let mut transferred = 0u32;
            unsafe { GetOverlappedResult(self.handle.0, &mut self.read.os, &mut transferred, 1) };
            return Ok(0);
        }
        if wait != WAIT_OBJECT_0 {
            let error = io::Error::last_os_error();
            unsafe { CancelIoEx(self.handle.0, &mut self.read.os) };
            let mut transferred = 0u32;
            unsafe { GetOverlappedResult(self.handle.0, &mut self.read.os, &mut transferred, 1) };
            return Err(error);
        }
        if unsafe { GetOverlappedResult(self.handle.0, &mut self.read.os, &mut read, 0) } == 0 {
            let error = io::Error::last_os_error();
            return match error.raw_os_error() {
                Some(ERROR_BROKEN_PIPE) | Some(ERROR_NO_DATA) => Ok(0),
                Some(ERROR_OPERATION_ABORTED) if self.closed.load(Ordering::Acquire) => Ok(0),
                _ => Err(error),
            };
        }
        if self.closed.load(Ordering::Acquire) {
            return Ok(0);
        }
        Ok(read as usize)
    }
}

impl io::Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.closed.load(Ordering::Acquire) {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "pipe closed"));
        }
        if unsafe { ResetEvent(self.write.event.0) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut written = 0u32;
        let ok = unsafe {
            WriteFile(
                self.handle.0,
                buf.as_ptr(),
                buf.len() as u32,
                &mut written,
                &mut self.write.os,
            )
        };
        if ok != 0 {
            return Ok(written as usize);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_IO_PENDING) {
            return Err(error);
        }
        if unsafe { WaitForSingleObject(self.write.event.0, INFINITE) } != WAIT_OBJECT_0 {
            let error = io::Error::last_os_error();
            unsafe { CancelIoEx(self.handle.0, &mut self.write.os) };
            let mut transferred = 0u32;
            unsafe { GetOverlappedResult(self.handle.0, &mut self.write.os, &mut transferred, 1) };
            return Err(error);
        }
        if unsafe { GetOverlappedResult(self.handle.0, &mut self.write.os, &mut written, 0) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(written as usize)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
