// Windows container: one job per tree, a single-step stop, and the taskkill fallback.

use std::ffi::c_void;
use std::mem::{MaybeUninit, size_of};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicIsize, Ordering};

use super::{Follow, Grace, Inner, Phase, Process, lock};
use crate::platform::log;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CREATE_SUSPENDED: u32 = 0x0000_0004;
const TH32CS_SNAPTHREAD: u32 = 0x0000_0004;
const THREAD_SUSPEND_RESUME: u32 = 0x0002;
const PROCESS_SET_QUOTA: u32 = 0x0100;
const PROCESS_TERMINATE: u32 = 0x0001;
const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x2000;
const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: u32 = 9;
const JOB_OBJECT_BASIC_ACCOUNTING_INFORMATION: u32 = 1;

#[repr(C)]
struct JobObjectBasicLimitInformation {
    per_process_user_time_limit: i64,
    per_job_user_time_limit: i64,
    limit_flags: u32,
    minimum_working_set_size: usize,
    maximum_working_set_size: usize,
    active_process_limit: u32,
    affinity: usize,
    priority_class: u32,
    scheduling_class: u32,
}

#[repr(C)]
struct IoCounters {
    read_operation_count: u64,
    write_operation_count: u64,
    other_operation_count: u64,
    read_transfer_count: u64,
    write_transfer_count: u64,
    other_transfer_count: u64,
}

#[repr(C)]
struct JobObjectExtendedLimitInformation {
    basic_limit_information: JobObjectBasicLimitInformation,
    io_info: IoCounters,
    process_memory_limit: usize,
    job_memory_limit: usize,
    peak_process_memory_used: usize,
    peak_job_memory_used: usize,
}

#[repr(C)]
struct JobObjectBasicAccountingInformation {
    total_user_time: i64,
    total_kernel_time: i64,
    this_period_total_user_time: i64,
    this_period_total_kernel_time: i64,
    total_page_fault_count: u32,
    total_processes: u32,
    active_processes: u32,
    total_terminated_processes: u32,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> isize;
    fn SetInformationJobObject(job: isize, class: u32, info: *const c_void, len: u32) -> i32;
    fn QueryInformationJobObject(
        job: isize,
        class: u32,
        info: *mut c_void,
        len: u32,
        ret: *mut u32,
    ) -> i32;
    fn AssignProcessToJobObject(job: isize, process: isize) -> i32;
    fn TerminateJobObject(job: isize, exit_code: u32) -> i32;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
    fn CloseHandle(handle: *mut c_void) -> i32;
    fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> isize;
    fn Thread32First(snapshot: isize, entry: *mut ThreadEntry32) -> i32;
    fn Thread32Next(snapshot: isize, entry: *mut ThreadEntry32) -> i32;
    fn OpenThread(access: u32, inherit: i32, id: u32) -> isize;
    fn ResumeThread(thread: isize) -> u32;
}

#[repr(C)]
struct ThreadEntry32 {
    size: u32,
    usage: u32,
    thread_id: u32,
    owner_process_id: u32,
    base_priority: i32,
    delta_priority: i32,
    flags: u32,
}

pub const CONTAINER: &str = "job";

// No console window for a child started from a process that has none. The leader is born suspended: it is put in its job before its first instruction, so nothing it starts can escape.
pub fn prepare(cmd: &mut Command, _follow: Follow) {
    use std::os::windows::process::CommandExt as _;
    cmd.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED);
}

// Let the leader run; every thread it has so far is resumed, or the caller is told why not.
pub fn resume(pid: u32) -> Result<(), String> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == -1 {
        return Err(format!(
            "CreateToolhelp32Snapshot: {}",
            std::io::Error::last_os_error()
        ));
    }
    let outcome = resume_threads(snapshot, pid);
    // SAFETY: the snapshot came from the call above and is closed exactly once here.
    unsafe {
        CloseHandle(snapshot as *mut c_void);
    }
    outcome
}

fn resume_threads(snapshot: isize, pid: u32) -> Result<(), String> {
    // SAFETY: the snapshot stays open for the whole walk; every handle is checked and closed.
    unsafe {
        let mut entry = MaybeUninit::<ThreadEntry32>::zeroed();
        let entry = entry.as_mut_ptr();
        (*entry).size = size_of::<ThreadEntry32>() as u32;
        if Thread32First(snapshot, entry) == 0 {
            return Err(format!(
                "Thread32First: {}",
                std::io::Error::last_os_error()
            ));
        }
        let mut found = false;
        loop {
            if (*entry).owner_process_id == pid {
                found = true;
                let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, (*entry).thread_id);
                if thread == 0 {
                    return Err(format!("OpenThread: {}", std::io::Error::last_os_error()));
                }
                let resumed = ResumeThread(thread);
                let failure =
                    (resumed == u32::MAX).then(|| std::io::Error::last_os_error().to_string());
                CloseHandle(thread as *mut c_void);
                if let Some(reason) = failure {
                    return Err(format!("ResumeThread: {reason}"));
                }
            }
            if Thread32Next(snapshot, entry) == 0 {
                break;
            }
        }
        if found {
            Ok(())
        } else {
            Err(format!("no thread of pid={pid} to resume"))
        }
    }
}

pub struct Group {
    pid: u32,
    follow: Follow,
    backed_off: bool,
    job: AtomicIsize,
}

impl Group {
    // Put the new leader in a job; only a following tree dies with the last handle.
    pub fn attach(pid: u32, follow: Follow) -> Group {
        // SAFETY: every call is checked; a null handle means "fall back to taskkill".
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job == 0 {
                return Group {
                    pid,
                    follow,
                    backed_off: true,
                    job: AtomicIsize::new(0),
                };
            }
            let mut info = MaybeUninit::<JobObjectExtendedLimitInformation>::zeroed();
            if follow == Follow::Main {
                (*info.as_mut_ptr()).basic_limit_information.limit_flags =
                    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            }
            let info = info.assume_init();
            let set = SetInformationJobObject(
                job,
                JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
                &info as *const _ as *const c_void,
                size_of::<JobObjectExtendedLimitInformation>() as u32,
            );
            let process = if set == 0 {
                0
            } else {
                OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid)
            };
            if process == 0 || AssignProcessToJobObject(job, process) == 0 {
                if process != 0 {
                    CloseHandle(process as *mut c_void);
                }
                CloseHandle(job as *mut c_void);
                return Group {
                    pid,
                    follow,
                    backed_off: true,
                    job: AtomicIsize::new(0),
                };
            }
            CloseHandle(process as *mut c_void);
            Group {
                pid,
                follow,
                backed_off: false,
                job: AtomicIsize::new(job),
            }
        }
    }

    pub fn backed_off(&self) -> bool {
        self.backed_off
    }

    // One step: the tree is gone or the call reports why it is not.
    pub fn stop(&self) -> Result<(), String> {
        if self.backed_off {
            return taskkill(self.pid);
        }
        let job = self.job.swap(0, Ordering::SeqCst);
        if job == 0 {
            return Ok(());
        }
        // SAFETY: the handle came from CreateJobObject and is closed exactly once here.
        unsafe {
            let ok = TerminateJobObject(job, 1);
            CloseHandle(job as *mut c_void);
            if ok == 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        Ok(())
    }

    // Whether the leader left members behind; leftovers are killed here.
    pub fn reclaim(&self) -> bool {
        if self.backed_off {
            return false;
        }
        let job = self.job.swap(0, Ordering::SeqCst);
        if job == 0 {
            return false;
        }
        let active = active_processes_settled(job);
        // SAFETY: the handle came from CreateJobObject and is closed exactly once here.
        unsafe {
            if self.follow == Follow::Detached && active > 0 {
                TerminateJobObject(job, 1);
            }
            CloseHandle(job as *mut c_void);
        }
        active > 0
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        // The job handle is closed by stop/reclaim or by the system at exit; closing it here would trip KILL_ON_JOB_CLOSE.
    }
}

fn active_processes_settled(job: isize) -> u32 {
    let first = active_processes(job);
    if first == 0 {
        return 0;
    }
    // The leader counts until the job finishes retiring it; one short recheck settles that.
    std::thread::sleep(std::time::Duration::from_millis(10));
    active_processes(job)
}

fn active_processes(job: isize) -> u32 {
    let mut info = MaybeUninit::<JobObjectBasicAccountingInformation>::zeroed();
    // SAFETY: the buffer matches the class and its length exactly.
    let ok = unsafe {
        QueryInformationJobObject(
            job,
            JOB_OBJECT_BASIC_ACCOUNTING_INFORMATION,
            info.as_mut_ptr() as *mut c_void,
            size_of::<JobObjectBasicAccountingInformation>() as u32,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        0
    } else {
        // SAFETY: a successful query filled every field.
        unsafe { info.assume_init().active_processes }
    }
}

// Fallback for a machine that will not let us own a job; a reused pid is a narrow window.
fn taskkill(pid: u32) -> Result<(), String> {
    use std::os::windows::process::CommandExt as _;
    let mut cmd = Command::new("taskkill");
    cmd.args(["/T", "/F", "/PID", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW);
    match cmd.status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!(
            "taskkill exit {}",
            status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "none".to_string())
        )),
        Err(e) => Err(e.to_string()),
    }
}

// Windows has no polite signal: one call takes the tree down, no window, no waiting window.
pub fn stop(process: &Process) {
    let inner: &Inner = &process.0;
    if inner.exit.published() {
        return;
    }
    {
        let mut phase = lock(&inner.stop_phase);
        if inner.exit.published() || phase.phase != Phase::None {
            return;
        }
        phase.phase = Phase::Terminating;
        phase.grace = Grace::None;
    }
    if let Err(reason) = inner.group.stop() {
        inner.stop_failed.store(true, Ordering::SeqCst);
        log::error(
            "proc",
            format!("pid={} single-process stop failed: {reason}", inner.pid),
        );
    }
}
