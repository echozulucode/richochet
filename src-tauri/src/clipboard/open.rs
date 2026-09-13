//! Opening the clipboard, and waiting properly when someone else has it.
//!
//! Windows lets one window hold the clipboard open at a time, and plenty of things grab it for a
//! moment right after a copy: clipboard history and cloud clipboard (Win+V), the app you copied
//! from while it renders deferred formats, Remote Desktop and VM clipboard sharing, clipboard and
//! password managers. `OpenClipboard` fails with `ERROR_ACCESS_DENIED` for as long as any of them
//! holds it — typically a few to a few tens of milliseconds.
//!
//! This module used to lean on `clipboard_win::Clipboard::new_attempts(10)`, which retries with
//! `Sleep(0)`: that only yields the rest of the current timeslice, so all ten attempts finish in
//! well under a millisecond and absorb essentially no contention. Measured against a process
//! holding the clipboard 40 ms in every 50, it failed 80% of the time — no better than not retrying
//! at all (71%). The symptom in the field was "could not open the clipboard: OSError(5): Access is
//! denied" on Ctrl+V, with retrying by hand or pasting from the context menu working fine, because
//! both add the human delay the retry should have provided.
//!
//! [`BACKOFF`] waits for real. [`open`] also serializes our own clipboard access, so a paste and a
//! copy racing inside Richochet cannot deny each other, and when it does give up it names the
//! process that was holding the clipboard, so a failure in the field identifies its own cause.

use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use super::ClipError;

/// How long to wait before each retry of `OpenClipboard`.
///
/// Starts short, because most holds are a few milliseconds and a paste should not feel slow, then
/// grows to a 50 ms ceiling. The total is about half a second: long enough to outlast every
/// transient hold described in the module docs, short enough that a clipboard which really is
/// stuck fails fast enough to tell the user rather than appearing to hang.
pub(crate) const BACKOFF: &[Duration] = &[
    Duration::from_millis(5),
    Duration::from_millis(10),
    Duration::from_millis(20),
    Duration::from_millis(30),
    Duration::from_millis(40),
    Duration::from_millis(50),
    Duration::from_millis(50),
    Duration::from_millis(50),
    Duration::from_millis(50),
    Duration::from_millis(50),
    Duration::from_millis(50),
    Duration::from_millis(50),
    Duration::from_millis(50),
];

/// Serializes every clipboard session this process opens.
///
/// Clipboard ownership is per thread, and Tauri runs blocking work on a pool, so two of our own
/// operations — a paste and a copy triggered close together — would otherwise deny each other.
static SESSION: Mutex<()> = Mutex::new(());

/// Try `attempt`, and on failure retry after each delay in `delays`, returning the first success
/// or the last error.
///
/// Makes `1 + delays.len()` attempts at most. `sleep` is injected so the schedule can be tested
/// without waiting and without a clipboard.
pub(crate) fn retry_with_backoff<T, E>(
    delays: &[Duration],
    mut attempt: impl FnMut() -> Result<T, E>,
    mut sleep: impl FnMut(Duration),
) -> Result<T, E> {
    let mut last = match attempt() {
        Ok(value) => return Ok(value),
        Err(error) => error,
    };
    for &delay in delays {
        sleep(delay);
        match attempt() {
            Ok(value) => return Ok(value),
            Err(error) => last = error,
        }
    }
    Err(last)
}

/// An open clipboard, closed when dropped.
///
/// Field order is load-bearing: fields drop in declaration order, so the clipboard is closed
/// *before* the in-process lock is released. Releasing the lock first would let another of our own
/// threads try to open a clipboard this thread still holds.
#[cfg(windows)]
pub(crate) struct Session {
    _clipboard: clipboard_win::Clipboard,
    _lock: MutexGuard<'static, ()>,
}

/// Open the clipboard, waiting out transient holds by other applications.
///
/// # Errors
///
/// [`ClipError::Busy`] when the clipboard stayed locked for the whole [`BACKOFF`] window. It names
/// the process holding it when Windows can tell us.
#[cfg(windows)]
pub(crate) fn open() -> Result<Session, ClipError> {
    // A poisoned lock only means an earlier session panicked while holding it; the clipboard itself
    // is released by that session's Drop, so it is safe to carry on.
    let lock = SESSION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let mut holder: Option<String> = None;
    let opened = retry_with_backoff(
        BACKOFF,
        || {
            clipboard_win::Clipboard::new().inspect_err(|_| {
                // Ask on every failure and keep the most recent answer: by the final attempt the
                // original holder may already have let go.
                if let Some(name) = holder_process() {
                    holder = Some(name);
                }
            })
        },
        std::thread::sleep,
    );

    match opened {
        Ok(clipboard) => Ok(Session {
            _clipboard: clipboard,
            _lock: lock,
        }),
        Err(error) => Err(ClipError::Busy {
            holder,
            detail: error.to_string(),
        }),
    }
}

/// The executable name of the process that currently has the clipboard open, if Windows will say.
///
/// `GetOpenClipboardWindow` returns the window holding the clipboard *open* — the thing actually
/// blocking us — as distinct from `GetClipboardOwner`, which is whoever last *wrote* to it.
#[cfg(windows)]
fn holder_process() -> Option<String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::DataExchange::GetOpenClipboardWindow;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

    // SAFETY: each call is a plain Win32 query with no preconditions beyond valid arguments. Every
    // handle is checked before use and the process handle is closed on every path that opened it.
    unsafe {
        let window = GetOpenClipboardWindow();
        if window.is_null() {
            return None;
        }

        let mut pid: u32 = 0;
        GetWindowThreadProcessId(window, &mut pid);
        if pid == 0 {
            return None;
        }

        // Limited-information access is granted even for elevated processes we could not
        // otherwise open, which is exactly the case where a name is most useful.
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return Some(format!("process {pid}"));
        }

        let mut buffer = [0u16; 1024];
        let mut length = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut length);
        CloseHandle(process);
        if ok == 0 {
            return Some(format!("process {pid}"));
        }

        let path = String::from_utf16_lossy(&buffer[..length as usize]);
        let name = path.rsplit(['\\', '/']).next().unwrap_or(&path).to_string();
        Some(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_first_success_does_not_sleep() {
        let mut slept = Vec::new();
        let result: Result<u8, ()> = retry_with_backoff(BACKOFF, || Ok(7), |d| slept.push(d));
        assert_eq!(result, Ok(7));
        assert!(slept.is_empty());
    }

    #[test]
    fn retries_until_it_succeeds_and_sleeps_in_between() {
        let mut calls = 0;
        let mut slept = Vec::new();
        let result: Result<&str, &str> = retry_with_backoff(
            BACKOFF,
            || {
                calls += 1;
                if calls < 4 {
                    Err("busy")
                } else {
                    Ok("open")
                }
            },
            |d| slept.push(d),
        );
        assert_eq!(result, Ok("open"));
        assert_eq!(calls, 4);
        // One sleep before each retry, taken from the front of the schedule.
        assert_eq!(slept, &BACKOFF[..3]);
    }

    #[test]
    fn gives_up_with_the_last_error_after_the_whole_schedule() {
        let mut calls: u32 = 0;
        let mut slept = Vec::new();
        let result: Result<(), u32> = retry_with_backoff(
            BACKOFF,
            || {
                calls += 1;
                Err(calls)
            },
            |d| slept.push(d),
        );
        let attempts = BACKOFF.len() as u32 + 1;
        assert_eq!(calls, attempts);
        assert_eq!(result, Err(attempts));
        assert_eq!(slept, BACKOFF);
    }

    #[test]
    fn the_schedule_actually_waits() {
        // The regression this module exists to fix: the old retry spent under a millisecond in
        // total, which absorbs no real contention. Pin the budget to a range that does, without
        // becoming long enough to feel like a hang.
        let total: Duration = BACKOFF.iter().sum();
        assert!(
            total >= Duration::from_millis(300),
            "backoff too short: {total:?}"
        );
        assert!(
            total <= Duration::from_millis(1000),
            "backoff too long: {total:?}"
        );
        // And it must never spin: every retry waits a real, non-zero time.
        assert!(BACKOFF.iter().all(|d| *d >= Duration::from_millis(1)));
    }

    #[test]
    fn the_schedule_never_shrinks() {
        assert!(BACKOFF.windows(2).all(|pair| pair[0] <= pair[1]));
    }

    /// Manual verification against real contention, not part of the normal suite.
    ///
    /// Run a process that holds the clipboard with a real window handle (a `NULL`-handle hold does
    /// not block other processes), then:
    ///
    /// ```text
    /// cargo test -p app --lib clipboard::open::tests::reads_through_real_contention -- --ignored --nocapture
    /// ```
    #[cfg(windows)]
    #[test]
    #[ignore = "needs another process holding the clipboard; run by hand"]
    fn reads_through_real_contention() {
        let mut failures = Vec::new();
        for _ in 0..30 {
            if let Err(error) = crate::clipboard::read() {
                failures.push(error.to_string());
            }
            std::thread::sleep(Duration::from_millis(37));
        }
        println!("failed {} / 30", failures.len());
        for failure in &failures {
            println!("  {failure}");
        }
    }
}
